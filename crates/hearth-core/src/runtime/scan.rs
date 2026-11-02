//! Finding models: every `.gguf` under the model folders, split files grouped, vision projectors
//! (mmproj) paired with the models next to them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::gguf::{self, GgufInfo};

/// Folders deeper than this under a model folder aren't searched.
const MAX_DEPTH: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    /// Stable for the same file path.
    pub id: String,
    /// The file to load (the first part, for split models).
    pub path: PathBuf,
    pub file_name: String,
    /// A readable name: the model's own, else the file name without quantization or split.
    pub name: String,
    pub quant: Option<String>,
    pub size_bytes: u64,
    pub parts: u64,
    /// Parts of a split model that aren't there (it can't load).
    pub missing_parts: u64,
    pub architecture: String,
    pub size_label: Option<String>,
    pub context_length: Option<u64>,
    pub layers: Option<u64>,
    pub experts: Option<u64>,
    pub tools: bool,
    pub thinking: bool,
    /// The vision projector found next to it.
    pub mmproj: Option<PathBuf>,
}

static SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(.*)-(\d{5})-of-(\d{5})\.gguf$").expect("valid regex"));
static QUANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[-_.]((?:UD-)?(?:IQ\d_[A-Z]+|I?Q\d(?:_K)?(?:_[A-Z0-9]{1,2})?|BF16|F16|F32|MXFP4(?:_MOE)?))$")
        .expect("valid regex")
});

/// FNV-1a: stable across runs and Rust versions (unlike the standard hasher).
fn stable_id(path: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.display().to_string().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("m-{:012x}", hash & 0xffff_ffff_ffff)
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() && depth < MAX_DEPTH => collect(&path, depth + 1, out),
            Ok(kind) if kind.is_file() && name.ends_with(".gguf") => out.push(path),
            _ => {}
        }
    }
}

/// `Qwen3.6-35B-A3B-UD-IQ4_XS` → (`Qwen3.6-35B-A3B`, `UD-IQ4_XS`).
fn split_quant(stem: &str) -> (String, Option<String>) {
    match QUANT.captures(stem) {
        Some(found) => {
            let quant = found[1].to_owned();
            let name = stem[..stem.len() - quant.len() - 1].to_owned();
            (name, Some(quant.to_uppercase()))
        }
        None => (stem.to_owned(), None),
    }
}

/// Which projector to prefer when a folder has several: F16, then BF16, then F32.
fn projector_rank(info: &GgufInfo, path: &Path) -> u8 {
    let name = path.display().to_string().to_lowercase();
    match info.file_type {
        Some(1) => 0,
        Some(32) => 1,
        Some(0) => 2,
        _ if name.contains("f16") => 0,
        _ => 3,
    }
}

/// Every model under `folders` (missing folders are skipped), sorted by name.
pub fn scan(folders: &[PathBuf]) -> Vec<LocalModel> {
    let mut files = Vec::new();
    for folder in folders {
        collect(folder, 0, &mut files);
    }
    files.sort();
    files.dedup_by(|a, b| {
        a.display()
            .to_string()
            .eq_ignore_ascii_case(&b.display().to_string())
    });

    let mut models: Vec<(LocalModel, PathBuf)> = Vec::new();
    // Projectors by folder: (rank, path).
    let mut projectors: BTreeMap<String, (u8, PathBuf)> = BTreeMap::new();
    // Split parts by (folder, stem): sizes and how many there are.
    let mut parts: BTreeMap<String, (u64, u64)> = BTreeMap::new();

    for path in &files {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if let Some(split) = SPLIT.captures(&file_name) {
            let key = format!("{}|{}", folder.display(), split[1].to_lowercase());
            let entry = parts.entry(key).or_insert((0, 0));
            entry.0 += size;
            entry.1 += 1;
            if &split[2] != "00001" {
                continue;
            }
        }
        let Ok(info) = gguf::read(path) else { continue };
        if info.is_mmproj() || file_name.to_lowercase().contains("mmproj") {
            let rank = projector_rank(&info, path);
            let key = folder.display().to_string().to_lowercase();
            let better = projectors.get(&key).is_none_or(|(best, _)| rank < *best);
            if better {
                projectors.insert(key, (rank, path.clone()));
            }
            continue;
        }
        let stem = file_name
            .strip_suffix(".gguf")
            .or_else(|| file_name.strip_suffix(".GGUF"))
            .unwrap_or(&file_name);
        let (base, total_parts) = match SPLIT.captures(&file_name) {
            Some(split) => (split[1].to_owned(), split[3].parse::<u64>().unwrap_or(1)),
            None => (stem.to_owned(), 1),
        };
        let (file_base, file_quant) = split_quant(&base);
        let quant = file_quant.or_else(|| {
            info.file_type
                .and_then(gguf::file_type_name)
                .map(String::from)
        });
        let name = info
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(file_base);
        let model = LocalModel {
            id: stable_id(path),
            path: path.clone(),
            file_name: file_name.clone(),
            name,
            quant,
            size_bytes: size,
            parts: total_parts,
            missing_parts: 0,
            architecture: info.architecture.clone(),
            size_label: info.size_label.clone(),
            context_length: info.context_length,
            layers: info.block_count,
            experts: info.expert_count,
            tools: info.template_tools,
            thinking: info.template_thinking,
            mmproj: None,
        };
        models.push((model, folder));
    }

    let mut out: Vec<LocalModel> = models
        .into_iter()
        .map(|(mut model, folder)| {
            if model.parts > 1
                && let Some(split) = SPLIT.captures(&model.file_name)
            {
                let key = format!("{}|{}", folder.display(), split[1].to_lowercase());
                if let Some((size, found)) = parts.get(&key) {
                    model.size_bytes = *size;
                    model.missing_parts = model.parts.saturating_sub(*found);
                }
            }
            model.mmproj = projectors
                .get(&folder.display().to_string().to_lowercase())
                .map(|(_, path)| path.clone());
            model
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// LM Studio's model folders, when they exist.
pub fn lm_studio_folders() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    [
        home.join(".lmstudio\\models"),
        home.join(".cache\\lm-studio\\models"),
    ]
    .into_iter()
    .filter(|p| p.is_dir())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::gguf::fixture::{V, gguf};

    fn model(arch: &str, name: Option<&str>, file_type: u32) -> Vec<u8> {
        let mut entries = vec![
            ("general.architecture", V::Str(arch)),
            ("general.file_type", V::U32(file_type)),
            (
                "tokenizer.chat_template",
                V::Str("{% for tool in tools %}{% endfor %}"),
            ),
        ];
        if let Some(name) = name {
            entries.push(("general.name", V::Str(name)));
        }
        entries.push(("qwen3moe.context_length", V::U32(262_144)));
        gguf(&entries, 1000)
    }

    #[test]
    fn finds_models_groups_splits_and_pairs_projectors() {
        let dir = tempfile::tempdir().unwrap();
        let qwen = dir.path().join("qwen");
        let gemma = dir.path().join("deep\\gemma");
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::create_dir_all(&gemma).unwrap();
        std::fs::write(
            qwen.join("Qwen3.6-35B-A3B-UD-IQ4_XS-00001-of-00002.gguf"),
            model("qwen3moe", Some("Qwen3.6 35B A3B"), 30),
        )
        .unwrap();
        std::fs::write(
            qwen.join("Qwen3.6-35B-A3B-UD-IQ4_XS-00002-of-00002.gguf"),
            vec![0u8; 5000],
        )
        .unwrap();
        std::fs::write(
            qwen.join("mmproj-F32.gguf"),
            gguf(
                &[
                    ("general.architecture", V::Str("clip")),
                    ("general.file_type", V::U32(0)),
                ],
                0,
            ),
        )
        .unwrap();
        std::fs::write(
            qwen.join("mmproj-F16.gguf"),
            gguf(
                &[
                    ("general.architecture", V::Str("clip")),
                    ("general.file_type", V::U32(1)),
                ],
                0,
            ),
        )
        .unwrap();
        std::fs::write(
            gemma.join("gemma-4-26b-a4b-it.Q4_K_M.gguf"),
            model("gemma4", None, 15),
        )
        .unwrap();
        std::fs::write(
            gemma.join("gemma-4-26b-a4b-it-mmproj.f16.gguf"),
            b"GGUFbroken",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::write(dir.path().join("broken.gguf"), b"nope").unwrap();

        let found = scan(&[dir.path().to_path_buf(), dir.path().join("missing")]);
        let summary: Vec<(&str, Option<&str>, u64, bool)> = found
            .iter()
            .map(|m| {
                (
                    m.name.as_str(),
                    m.quant.as_deref(),
                    m.parts,
                    m.mmproj.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("gemma-4-26b-a4b-it", Some("Q4_K_M"), 1, false),
                ("Qwen3.6 35B A3B", Some("UD-IQ4_XS"), 2, true)
            ]
        );
        let qwen_model = &found[1];
        assert!(
            qwen_model
                .mmproj
                .as_ref()
                .unwrap()
                .ends_with("mmproj-F16.gguf"),
            "F16 wins over F32"
        );
        assert!(qwen_model.size_bytes > 5000 && qwen_model.missing_parts == 0);
        assert!(qwen_model.tools);
        assert_eq!(qwen_model.context_length, Some(262_144));
        assert_eq!(found[0].id, stable_id(&found[0].path));
        assert_eq!(
            scan(&[dir.path().to_path_buf()])[1].id,
            qwen_model.id,
            "ids are stable"
        );
    }

    #[test]
    fn quantization_comes_from_the_file_name() {
        assert_eq!(
            split_quant("gpt-oss-20b-MXFP4"),
            ("gpt-oss-20b".into(), Some("MXFP4".into()))
        );
        assert_eq!(
            split_quant("gemma-4-26b-a4b-it-heretic.q4_k_m"),
            ("gemma-4-26b-a4b-it-heretic".into(), Some("Q4_K_M".into()))
        );
        assert_eq!(
            split_quant("Llama-3-8B.Q8_0"),
            ("Llama-3-8B".into(), Some("Q8_0".into()))
        );
        assert_eq!(split_quant("model"), ("model".into(), None));
    }
}
