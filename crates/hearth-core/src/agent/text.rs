//! Text files as the agent sees them: decoded to a `String` with `\n` line endings, and written
//! back with the file's own encoding, byte-order mark and line endings, so an edit never
//! rewrites a whole file's formatting.

use std::path::Path;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

#[derive(Debug, Clone, PartialEq)]
pub struct TextFile {
    /// The content with `\n` line endings.
    pub text: String,
    encoding: &'static Encoding,
    bom: bool,
    crlf: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    Binary,
    Io(String),
    /// Characters the file's encoding can't hold.
    Unencodable(&'static str),
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextError::Binary => write!(f, "This is a binary file, not text."),
            TextError::Io(err) => write!(f, "{err}"),
            TextError::Unencodable(name) => {
                write!(
                    f,
                    "The new text has characters that the file's encoding ({name}) can't store."
                )
            }
        }
    }
}

impl TextFile {
    /// A new file: UTF-8 without a byte-order mark, line endings as given.
    pub fn new(crlf: bool) -> Self {
        Self {
            text: String::new(),
            encoding: UTF_8,
            bom: false,
            crlf,
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TextError> {
        let (encoding, bom_len) = match Encoding::for_bom(bytes) {
            Some((encoding, len)) => (encoding, len),
            None => {
                if bytes.iter().take(8192).any(|&b| b == 0) {
                    return Err(TextError::Binary);
                }
                if std::str::from_utf8(bytes).is_ok() {
                    (UTF_8, 0)
                } else {
                    let mut detector =
                        chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
                    detector.feed(bytes, true);
                    (detector.guess(None, chardetng::Utf8Detection::Allow), 0)
                }
            }
        };
        let (decoded, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        let crlf = decoded.contains("\r\n");
        Ok(Self {
            text: decoded.replace("\r\n", "\n"),
            encoding,
            bom: bom_len > 0,
            crlf,
        })
    }

    pub fn read(path: &Path) -> Result<Self, TextError> {
        let bytes = std::fs::read(path).map_err(|e| TextError::Io(e.to_string()))?;
        Self::decode(&bytes)
    }

    /// Bytes for `text` (with `\n` endings) in this file's format.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, TextError> {
        let text = if self.crlf {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text.to_owned()
        };
        let mut out = Vec::new();
        if self.encoding == UTF_16LE || self.encoding == UTF_16BE {
            if self.bom {
                out.extend_from_slice(if self.encoding == UTF_16LE {
                    &[0xFF, 0xFE]
                } else {
                    &[0xFE, 0xFF]
                });
            }
            for unit in text.encode_utf16() {
                let bytes = if self.encoding == UTF_16LE {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                out.extend_from_slice(&bytes);
            }
            return Ok(out);
        }
        if self.bom && self.encoding == UTF_8 {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        let (encoded, _, unmappable) = self.encoding.encode(&text);
        if unmappable {
            return Err(TextError::Unencodable(self.encoding.name()));
        }
        out.extend_from_slice(&encoded);
        Ok(out)
    }

    pub fn crlf(&self) -> bool {
        self.crlf
    }
}

/// The line ending most files next to `path` use, for new files (CRLF if unsure on Windows).
pub fn sibling_line_endings(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let (mut crlf, mut lf) = (0, 0);
    for entry in entries.flatten().take(20) {
        if let Ok(bytes) = std::fs::read(entry.path())
            && bytes.len() < 512 * 1024
            && !bytes.contains(&0)
        {
            if bytes.windows(2).any(|w| w == b"\r\n") {
                crlf += 1;
            } else if bytes.contains(&b'\n') {
                lf += 1;
            }
        }
    }
    crlf > lf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_bom_survive_an_edit() {
        let original = b"\xEF\xBB\xBFfn main() {\r\n    println!(\"hi\");\r\n}\r\n";
        let file = TextFile::decode(original).unwrap();
        assert_eq!(file.text, "fn main() {\n    println!(\"hi\");\n}\n");
        let edited = file.text.replace("hi", "hello");
        assert_eq!(
            file.encode(&edited).unwrap(),
            b"\xEF\xBB\xBFfn main() {\r\n    println!(\"hello\");\r\n}\r\n"
        );
    }

    #[test]
    fn utf16_files_round_trip() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Write-Host 'ü'\r\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let file = TextFile::decode(&bytes).unwrap();
        assert_eq!(file.text, "Write-Host 'ü'\n");
        assert_eq!(file.encode(&file.text).unwrap(), bytes);
    }

    #[test]
    fn legacy_encodings_are_detected_and_kept() {
        let (bytes, _, _) =
            encoding_rs::WINDOWS_1252.encode("café crème brûlée, naïve über façade");
        let file = TextFile::decode(&bytes).unwrap();
        assert!(file.text.contains("café"));
        assert_eq!(file.encode(&file.text).unwrap(), bytes.to_vec());
        assert!(matches!(
            file.encode("日本語"),
            Err(TextError::Unencodable(_))
        ));
    }

    #[test]
    fn binary_files_are_refused() {
        assert_eq!(
            TextFile::decode(b"\x89PNG\r\n\x1a\n\0\0\0").unwrap_err(),
            TextError::Binary
        );
    }
}
