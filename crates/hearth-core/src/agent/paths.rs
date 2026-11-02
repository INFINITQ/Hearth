//! Paths the model gives us, made safe and predictable on Windows.
//!
//! Models write paths every way: relative (`src/main.rs`), Windows (`D:\proj\x`, `D:/proj/x`),
//! Git Bash (`/d/proj/x`), WSL (`/mnt/d/proj/x`), or `~/x`. All become absolute Windows paths.
//! Whether a path is inside the project is checked case-insensitively, and again after
//! resolving symlinks and junctions, which could point anywhere.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Absolute Windows path.
    pub abs: PathBuf,
    /// Shown to the model: relative to the project with `/`, or the absolute path if outside.
    pub display: String,
    pub inside: bool,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

impl Workspace {
    /// `root` must exist; it is canonicalized (without the `\\?\` prefix).
    pub fn new(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: dunce::canonicalize(root)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project folder as Git Bash writes it (`/d/proj`).
    pub fn posix_root(&self) -> String {
        to_posix(&self.root)
    }

    pub fn resolve(&self, input: &str) -> Result<Resolved, String> {
        let cleaned = input.trim().trim_matches(['"', '\'', '`']).trim();
        if cleaned.is_empty() {
            return Err("The path is empty.".into());
        }
        let native = to_native(cleaned);
        let path = PathBuf::from(&native);
        let joined = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        let abs = normalize(&joined);
        check_names(&abs)?;
        let inside = contains(&self.root, &abs) && contains(&self.root, &real_location(&abs));
        let display = if inside {
            relative(&self.root, &abs)
        } else {
            abs.display().to_string().replace('\\', "/")
        };
        Ok(Resolved {
            abs,
            display,
            inside,
        })
    }

    /// `path` relative to the project with forward slashes (for listings).
    pub fn display(&self, path: &Path) -> String {
        if contains(&self.root, path) {
            relative(&self.root, path)
        } else {
            path.display().to_string().replace('\\', "/")
        }
    }
}

/// Git Bash and WSL drive paths, `~`, and forward slashes to a Windows path.
fn to_native(input: &str) -> String {
    let mut text = input.to_owned();
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))
        && let Some(home) = std::env::var_os("USERPROFILE")
    {
        text = format!("{}\\{rest}", home.to_string_lossy());
    }
    let lower = text.to_ascii_lowercase();
    let drive_path = |rest: &str| -> Option<String> {
        let mut chars = rest.chars();
        let drive = chars.next()?;
        let after = chars.as_str();
        (drive.is_ascii_alphabetic() && (after.is_empty() || after.starts_with('/'))).then(|| {
            format!(
                "{}:{}",
                drive.to_ascii_uppercase(),
                if after.is_empty() { "/" } else { after }
            )
        })
    };
    let native = if let Some(rest) = lower.strip_prefix("/mnt/") {
        drive_path(&text[5..5 + rest.len()])
    } else if text.starts_with('/') && !text.starts_with("//") {
        drive_path(&text[1..])
    } else {
        None
    };
    if let Some(native) = native {
        text = native;
    }
    text.replace('/', "\\")
}

/// `D:\proj` → `/d/proj`.
pub fn to_posix(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("/{}{}", drive.to_ascii_lowercase(), chars.as_str())
        }
        _ => text,
    }
}

/// Removes `.` and `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn lowered(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// True if `path` is `root` or inside it (case-insensitive, as NTFS is).
fn contains(root: &Path, path: &Path) -> bool {
    let root = lowered(root);
    let path = lowered(path);
    path == root || path.starts_with(&format!("{root}\\"))
}

/// Where a path really is, through symlinks and junctions: the nearest existing ancestor is
/// canonicalized and the rest appended.
fn real_location(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
    let mut real = dunce::canonicalize(&existing).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        real.push(name);
    }
    real
}

fn relative(root: &Path, path: &Path) -> String {
    let root_len = lowered(root).len();
    let full = path.display().to_string().replace('/', "\\");
    let rest = full
        .get(root_len..)
        .unwrap_or_default()
        .trim_start_matches('\\');
    if rest.is_empty() {
        ".".into()
    } else {
        rest.replace('\\', "/")
    }
}

/// Windows refuses (or misbehaves with) device names, trailing dots and spaces, and `:` streams.
fn check_names(path: &Path) -> Result<(), String> {
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = name.to_string_lossy();
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err(format!("`{name}` is a reserved name on Windows."));
        }
        if name.ends_with('.') || name.ends_with(' ') {
            return Err(format!(
                "`{name}` ends with a dot or space, which Windows doesn't allow."
            ));
        }
        if name.contains(':') {
            return Err(format!(
                "`{name}` contains `:`, which isn't allowed in Windows file names."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn every_path_style_lands_in_the_same_place() {
        let (_dir, ws) = workspace();
        let root = ws.root().display().to_string();
        let posix = ws.posix_root();
        let wsl = format!("/mnt{posix}");
        for input in [
            "src/main.rs".to_owned(),
            "./src/../src/main.rs".to_owned(),
            format!("{root}\\src\\main.rs"),
            format!("{}/src/main.rs", root.replace('\\', "/")),
            format!("{posix}/src/main.rs"),
            format!("{wsl}/src/main.rs"),
            format!("\"{root}\\SRC\\main.rs\""),
        ] {
            let resolved = ws.resolve(&input).unwrap();
            assert!(resolved.inside, "{input}");
            assert_eq!(resolved.display.to_lowercase(), "src/main.rs", "{input}");
        }
    }

    #[test]
    fn paths_outside_the_project_are_marked() {
        let (_dir, ws) = workspace();
        let outside = ws.resolve("../elsewhere/x.txt").unwrap();
        assert!(!outside.inside);
        assert!(!ws.resolve("C:/Windows/win.ini").unwrap().inside);
        assert!(ws.resolve(".").unwrap().inside);
    }

    #[test]
    fn windows_name_rules_are_enforced() {
        let (_dir, ws) = workspace();
        for bad in ["src/CON.txt", "nul", "notes.", "file.txt:stream", "dir /x"] {
            assert!(ws.resolve(bad).is_err(), "{bad}");
        }
        assert!(ws.resolve("").is_err());
    }

    #[test]
    fn posix_forms_round_trip() {
        assert_eq!(to_posix(Path::new("D:\\Code\\proj")), "/d/Code/proj");
        assert_eq!(to_native("/d/Code/proj"), "D:\\Code\\proj");
        assert_eq!(to_native("/mnt/c/Users"), "C:\\Users");
        assert_eq!(to_native("relative/path"), "relative\\path");
    }
}
