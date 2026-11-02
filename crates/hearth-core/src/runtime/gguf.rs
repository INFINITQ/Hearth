//! Reading a GGUF file's header: what model it is, how big its context is, whether it's a vision
//! projector (mmproj), and whether its chat template knows about tools. Only the metadata is
//! read; tensor data is never touched, and big arrays (the vocabulary) are skipped.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;

const MAGIC: &[u8; 4] = b"GGUF";
/// More metadata entries than any real model has: the file is damaged or not GGUF.
const MAX_KEYS: u64 = 100_000;
/// Strings longer than this are skipped (chat templates are ~10 KB).
const MAX_STRING: u64 = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GgufInfo {
    pub version: u32,
    pub architecture: String,
    pub name: Option<String>,
    pub size_label: Option<String>,
    /// `general.file_type`: the quantization llama.cpp wrote.
    pub file_type: Option<u64>,
    /// `general.type`: "model", "mmproj", "adapter"…
    pub kind: Option<String>,
    pub context_length: Option<u64>,
    pub block_count: Option<u64>,
    pub expert_count: Option<u64>,
    pub expert_used_count: Option<u64>,
    pub embedding_length: Option<u64>,
    /// The chat template mentions tools, so tool calls should work with `--jinja`.
    pub template_tools: bool,
    /// The chat template has a thinking switch (`enable_thinking`, reasoning effort…).
    pub template_thinking: bool,
    pub split_count: Option<u64>,
    pub split_no: Option<u64>,
}

impl GgufInfo {
    /// A vision projector rather than a language model.
    pub fn is_mmproj(&self) -> bool {
        self.kind.as_deref() == Some("mmproj") || self.architecture == "clip"
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GgufError {
    #[error("not a GGUF file")]
    NotGguf,
    #[error("GGUF version {0} isn't supported")]
    Version(u32),
    #[error("the header is damaged ({0})")]
    Damaged(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

enum Value {
    Uint(u64),
    Int(i64),
    Bool(bool),
    Text(String),
    Other,
}

struct Reader<R> {
    inner: R,
}

impl<R: Read + Seek> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut buf = [0u8; N];
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    fn skip(&mut self, n: u64) -> Result<(), GgufError> {
        let n = i64::try_from(n).map_err(|_| GgufError::Damaged("a length is too big".into()))?;
        self.inner.seek_relative(n)?;
        Ok(())
    }

    /// A string, or `None` when it's too long to keep (it's skipped).
    fn string(&mut self) -> Result<Option<String>, GgufError> {
        let len = self.u64()?;
        if len > MAX_STRING {
            self.skip(len)?;
            return Ok(None);
        }
        let mut buf = vec![0u8; len as usize];
        self.inner.read_exact(&mut buf)?;
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }

    fn value(&mut self, kind: u32) -> Result<Value, GgufError> {
        Ok(match kind {
            0 => Value::Uint(u64::from(u8::from_le_bytes(self.bytes()?))),
            1 => Value::Int(i64::from(i8::from_le_bytes(self.bytes()?))),
            2 => Value::Uint(u64::from(u16::from_le_bytes(self.bytes()?))),
            3 => Value::Int(i64::from(i16::from_le_bytes(self.bytes()?))),
            4 => Value::Uint(u64::from(self.u32()?)),
            5 => Value::Int(i64::from(i32::from_le_bytes(self.bytes()?))),
            6 => {
                self.skip(4)?;
                Value::Other
            }
            7 => Value::Bool(u8::from_le_bytes(self.bytes()?) != 0),
            8 => self.string()?.map_or(Value::Other, Value::Text),
            9 => {
                let element = self.u32()?;
                let count = self.u64()?;
                match element {
                    // Strings have their own lengths: skip them one by one.
                    8 => {
                        for _ in 0..count {
                            let len = self.u64()?;
                            self.skip(len)?;
                        }
                    }
                    9 => return Err(GgufError::Damaged("nested arrays".into())),
                    other => {
                        let size = match other {
                            0 | 1 | 7 => 1,
                            2 | 3 => 2,
                            4..=6 => 4,
                            10..=12 => 8,
                            _ => {
                                return Err(GgufError::Damaged(format!(
                                    "unknown array type {other}"
                                )));
                            }
                        };
                        self.skip(count.saturating_mul(size))?;
                    }
                }
                Value::Other
            }
            10 => Value::Uint(self.u64()?),
            11 => Value::Int(i64::from_le_bytes(self.bytes()?)),
            12 => {
                self.skip(8)?;
                Value::Other
            }
            other => return Err(GgufError::Damaged(format!("unknown value type {other}"))),
        })
    }
}

fn number(value: &Value) -> Option<u64> {
    match value {
        Value::Uint(n) => Some(*n),
        Value::Int(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// Reads the header of a GGUF file.
pub fn read(path: &Path) -> Result<GgufInfo, GgufError> {
    read_from(BufReader::with_capacity(64 * 1024, File::open(path)?))
}

pub fn read_from(source: impl Read + Seek) -> Result<GgufInfo, GgufError> {
    let mut reader = Reader { inner: source };
    let magic: [u8; 4] = reader.bytes().map_err(|_| GgufError::NotGguf)?;
    if &magic != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = reader.u32()?;
    if !(2..=3).contains(&version) {
        return Err(GgufError::Version(version));
    }
    let _tensors = reader.u64()?;
    let keys = reader.u64()?;
    if keys > MAX_KEYS {
        return Err(GgufError::Damaged(format!("{keys} metadata entries")));
    }
    let mut info = GgufInfo {
        version,
        ..GgufInfo::default()
    };
    let mut numbers: Vec<(String, u64)> = Vec::new();
    for _ in 0..keys {
        let key = reader.string()?.unwrap_or_default();
        let kind = reader.u32()?;
        let value = reader.value(kind)?;
        match (key.as_str(), &value) {
            ("general.architecture", Value::Text(text)) => info.architecture.clone_from(text),
            ("general.name", Value::Text(text)) => info.name = Some(text.clone()),
            ("general.size_label", Value::Text(text)) => info.size_label = Some(text.clone()),
            ("general.type", Value::Text(text)) => info.kind = Some(text.clone()),
            ("tokenizer.chat_template", Value::Text(text)) => {
                info.template_tools = text.contains("tools") || text.contains("tool_call");
                info.template_thinking = text.contains("enable_thinking")
                    || text.contains("reasoning_effort")
                    || text.contains("<think>")
                    || text.contains("<|think|>");
            }
            ("clip.has_vision_encoder", Value::Bool(true)) => {
                info.kind.get_or_insert_with(|| "mmproj".into());
            }
            (_, value) => {
                if let Some(n) = number(value) {
                    numbers.push((key, n));
                }
            }
        }
    }
    let arch = info.architecture.clone();
    for (key, n) in numbers {
        let field = key.strip_prefix(&format!("{arch}.")).unwrap_or(&key);
        match field {
            "general.file_type" => info.file_type = Some(n),
            "context_length" => info.context_length = Some(n),
            "block_count" => info.block_count = Some(n),
            "expert_count" => info.expert_count = Some(n),
            "expert_used_count" => info.expert_used_count = Some(n),
            "embedding_length" => info.embedding_length = Some(n),
            "split.count" => info.split_count = Some(n),
            "split.no" => info.split_no = Some(n),
            _ => {}
        }
    }
    Ok(info)
}

/// The name llama.cpp gives a `general.file_type`.
pub fn file_type_name(file_type: u64) -> Option<&'static str> {
    Some(match file_type {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4",
        _ => return None,
    })
}

/// Builds GGUF headers for tests (here and in the runtime and app tests).
pub mod fixture {
    /// A value to write.
    pub enum V<'a> {
        U32(u32),
        U64(u64),
        Bool(bool),
        Str(&'a str),
        Strings(&'a [&'a str]),
    }

    fn string(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&(text.len() as u64).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    /// A GGUF v3 file with this metadata and no tensors (plus `padding` zero bytes, to make
    /// the file look bigger).
    pub fn gguf(entries: &[(&str, V)], padding: usize) -> Vec<u8> {
        let mut out = b"GGUF".to_vec();
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (key, value) in entries {
            string(&mut out, key);
            match value {
                V::U32(n) => {
                    out.extend_from_slice(&4u32.to_le_bytes());
                    out.extend_from_slice(&n.to_le_bytes());
                }
                V::U64(n) => {
                    out.extend_from_slice(&10u32.to_le_bytes());
                    out.extend_from_slice(&n.to_le_bytes());
                }
                V::Bool(b) => {
                    out.extend_from_slice(&7u32.to_le_bytes());
                    out.push(u8::from(*b));
                }
                V::Str(text) => {
                    out.extend_from_slice(&8u32.to_le_bytes());
                    string(&mut out, text);
                }
                V::Strings(items) => {
                    out.extend_from_slice(&9u32.to_le_bytes());
                    out.extend_from_slice(&8u32.to_le_bytes());
                    out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                    for item in *items {
                        string(&mut out, item);
                    }
                }
            }
        }
        out.resize(out.len() + padding, 0);
        out
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::fixture::{V, gguf};
    use super::*;

    #[test]
    fn reads_what_matters_and_skips_the_vocabulary() {
        let tokens: Vec<String> = (0..5000).map(|i| format!("tok{i}")).collect();
        let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let bytes = gguf(
            &[
                ("general.architecture", V::Str("qwen3moe")),
                ("general.name", V::Str("Qwen3.6 35B A3B")),
                ("general.size_label", V::Str("35B-A3B")),
                ("general.file_type", V::U32(30)),
                ("qwen3moe.context_length", V::U32(262_144)),
                ("qwen3moe.block_count", V::U32(48)),
                ("qwen3moe.expert_count", V::U32(128)),
                ("tokenizer.ggml.tokens", V::Strings(&tokens)),
                (
                    "tokenizer.chat_template",
                    V::Str("{% if tools %}…{% if enable_thinking %}<think>{% endif %}"),
                ),
            ],
            0,
        );
        let info = read_from(Cursor::new(bytes)).unwrap();
        assert_eq!(info.architecture, "qwen3moe");
        assert_eq!(info.name.as_deref(), Some("Qwen3.6 35B A3B"));
        assert_eq!(
            (info.context_length, info.block_count, info.expert_count),
            (Some(262_144), Some(48), Some(128))
        );
        assert_eq!(info.file_type.and_then(file_type_name), Some("IQ4_XS"));
        assert!(info.template_tools && info.template_thinking && !info.is_mmproj());
    }

    #[test]
    fn projectors_and_bad_files_are_recognised() {
        let mmproj = gguf(
            &[
                ("general.architecture", V::Str("clip")),
                ("clip.has_vision_encoder", V::Bool(true)),
                ("general.file_type", V::U32(1)),
            ],
            0,
        );
        assert!(read_from(Cursor::new(mmproj)).unwrap().is_mmproj());
        assert!(matches!(
            read_from(Cursor::new(b"not a model".to_vec())),
            Err(GgufError::NotGguf)
        ));
        let mut truncated = gguf(&[("general.architecture", V::Str("llama"))], 0);
        truncated.truncate(30);
        assert!(read_from(Cursor::new(truncated)).is_err());
    }
}
