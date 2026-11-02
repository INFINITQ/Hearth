//! Finding the text an edit replaces. Local models often get whitespace slightly wrong or paste
//! line numbers from `read_file`, so after an exact match fails, looser matches are tried in
//! order: trailing whitespace ignored, then indentation ignored (the new text is re-indented to
//! fit), each also with line-number prefixes stripped. A match must be unique unless the caller
//! asks to replace every occurrence.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchError {
    Empty,
    NotFound { closest: Option<Closest> },
    Ambiguous { lines: Vec<usize> },
}

/// The most similar part of the file, to show the model what is really there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closest {
    /// 1-based line number where it starts.
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub replacements: usize,
    /// 1-based line where the first replacement starts.
    pub first_line: usize,
    /// How the match was found, for diagnostics.
    pub how: &'static str,
}

pub fn apply(content: &str, old: &str, new: &str, replace_all: bool) -> Result<Edited, MatchError> {
    let old = old.replace("\r\n", "\n");
    let new = new.replace("\r\n", "\n");
    if old.is_empty() {
        return Err(MatchError::Empty);
    }
    if let Some(result) = exact(content, &old, &new, replace_all)? {
        return Ok(result);
    }
    let (old_stripped, new_stripped) = (strip_line_numbers(&old), strip_line_numbers(&new));
    let mut candidates = vec![(old.as_str(), new.as_str(), false)];
    if old_stripped != old {
        candidates.push((old_stripped.as_str(), new_stripped.as_str(), true));
    }
    for (old, new, numbers_removed) in candidates {
        // The original text's exact match was tried above.
        if numbers_removed && let Some(result) = exact(content, old, new, replace_all)? {
            return Ok(Edited {
                how: "exact, line numbers removed",
                ..result
            });
        }
        if let Some(result) = by_lines(content, old, new, replace_all, Loose::TrailingSpace)? {
            let how = if numbers_removed {
                "trailing spaces ignored, line numbers removed"
            } else {
                "trailing spaces ignored"
            };
            return Ok(Edited { how, ..result });
        }
        if let Some(result) = by_lines(content, old, new, replace_all, Loose::Indentation)? {
            let how = if numbers_removed {
                "indentation ignored, line numbers removed"
            } else {
                "indentation ignored"
            };
            return Ok(Edited { how, ..result });
        }
    }
    Err(MatchError::NotFound {
        closest: closest(content, &old),
    })
}

fn line_of(content: &str, byte: usize) -> usize {
    content[..byte].matches('\n').count() + 1
}

fn exact(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Option<Edited>, MatchError> {
    let positions: Vec<usize> = content.match_indices(old).map(|(i, _)| i).collect();
    match positions.len() {
        0 => Ok(None),
        1 => Ok(Some(Edited {
            text: content.replacen(old, new, 1),
            replacements: 1,
            first_line: line_of(content, positions[0]),
            how: "exact",
        })),
        n if replace_all => Ok(Some(Edited {
            text: content.replace(old, new),
            replacements: n,
            first_line: line_of(content, positions[0]),
            how: "exact",
        })),
        _ => Err(MatchError::Ambiguous {
            lines: positions.iter().map(|&p| line_of(content, p)).collect(),
        }),
    }
}

#[derive(Clone, Copy)]
enum Loose {
    TrailingSpace,
    Indentation,
}

fn same_line(a: &str, b: &str, loose: Loose) -> bool {
    match loose {
        Loose::TrailingSpace => a.trim_end() == b.trim_end(),
        Loose::Indentation => a.trim() == b.trim(),
    }
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Whole-line matching with a looser comparison. The replacement keeps the file's indentation.
fn by_lines(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    loose: Loose,
) -> Result<Option<Edited>, MatchError> {
    let old_lines: Vec<&str> = old.trim_matches('\n').split('\n').collect();
    if old_lines.iter().all(|l| l.trim().is_empty()) {
        return Ok(None);
    }
    let lines: Vec<&str> = content.split('\n').collect();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + old_lines.len() <= lines.len() {
        if old_lines
            .iter()
            .enumerate()
            .all(|(k, o)| same_line(lines[i + k], o, loose))
        {
            starts.push(i);
            i += old_lines.len();
        } else {
            i += 1;
        }
    }
    match starts.len() {
        0 => return Ok(None),
        n if n > 1 && !replace_all => {
            return Err(MatchError::Ambiguous {
                lines: starts.iter().map(|s| s + 1).collect(),
            });
        }
        _ => {}
    }
    let new_lines: Vec<&str> = new.trim_matches('\n').split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = 0;
    for &start in &starts {
        out.extend(lines[cursor..start].iter().map(|l| (*l).to_owned()));
        // Shift the new text by the difference between the file's and the model's indentation.
        let file_indent = indent_of(lines[start]);
        let model_indent = indent_of(old_lines[0]);
        for line in &new_lines {
            let adjusted = match loose {
                Loose::Indentation if line.starts_with(model_indent) => {
                    format!("{file_indent}{}", &line[model_indent.len()..])
                }
                Loose::Indentation if !line.trim().is_empty() => {
                    format!("{file_indent}{}", line.trim_start())
                }
                _ => (*line).to_owned(),
            };
            out.push(adjusted);
        }
        cursor = start + old_lines.len();
    }
    out.extend(lines[cursor..].iter().map(|l| (*l).to_owned()));
    Ok(Some(Edited {
        text: out.join("\n"),
        replacements: starts.len(),
        first_line: starts[0] + 1,
        how: "",
    }))
}

/// `   12→text` or `    12\ttext` (what `read_file` and `cat -n` print) back to `text`.
fn strip_line_numbers(text: &str) -> String {
    let stripped: Vec<&str> = text
        .split('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            let rest = &trimmed[digits..];
            if digits > 0
                && let Some(after) = rest.strip_prefix('→').or_else(|| rest.strip_prefix('\t'))
            {
                return after;
            }
            line
        })
        .collect();
    stripped.join("\n")
}

fn closest(content: &str, old: &str) -> Option<Closest> {
    let old_lines: Vec<&str> = old.trim_matches('\n').split('\n').collect();
    let lines: Vec<&str> = content.split('\n').collect();
    if lines.is_empty() || old_lines.is_empty() {
        return None;
    }
    let window = old_lines.len().min(lines.len());
    let target = old_lines.join("\n");
    let mut best: Option<(f64, usize)> = None;
    for start in 0..=lines.len() - window {
        let candidate = lines[start..start + window].join("\n");
        let score = strsim::normalized_levenshtein(&candidate, &target);
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, start));
        }
    }
    let (score, start) = best?;
    (score > 0.4).then(|| Closest {
        line: start + 1,
        text: lines[start..start + window].join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n";

    #[test]
    fn exact_matches_replace_once() {
        let edited = apply(FILE, "let x = 1;", "let x = 2;", false).unwrap();
        assert_eq!(edited.text, FILE.replace("1;", "2;"));
        assert_eq!(
            (edited.replacements, edited.first_line, edited.how),
            (1, 2, "exact")
        );
    }

    #[test]
    fn duplicates_need_replace_all() {
        let file = "a\nb\na\n";
        assert_eq!(
            apply(file, "a", "c", false).unwrap_err(),
            MatchError::Ambiguous { lines: vec![1, 3] }
        );
        assert_eq!(apply(file, "a", "c", true).unwrap().text, "c\nb\nc\n");
    }

    #[test]
    fn wrong_indentation_still_matches_and_is_reindented() {
        let edited = apply(
            FILE,
            "let x = 1;\nprintln!(\"{x}\");",
            "let x = 1;\nlet y = x + 1;\nprintln!(\"{y}\");",
            false,
        )
        .unwrap();
        assert_eq!(
            edited.text,
            "fn main() {\n    let x = 1;\n    let y = x + 1;\n    println!(\"{y}\");\n}\n"
        );
        assert_eq!(edited.how, "indentation ignored");
    }

    #[test]
    fn trailing_spaces_and_crlf_in_the_request_are_tolerated() {
        let edited = apply(FILE, "    let x = 1;   \r\n", "    let x = 3;\r\n", false).unwrap();
        assert!(edited.text.contains("let x = 3;"));
    }

    #[test]
    fn line_numbers_pasted_from_read_file_are_ignored() {
        let edited = apply(
            FILE,
            "     2→    let x = 1;",
            "     2→    let x = 9;",
            false,
        )
        .unwrap();
        assert!(edited.text.contains("    let x = 9;\n"), "{}", edited.text);
    }

    #[test]
    fn misses_report_the_closest_text() {
        match apply(FILE, "    let x = 11;", "z", false).unwrap_err() {
            MatchError::NotFound {
                closest: Some(closest),
            } => {
                assert_eq!(closest.line, 2);
                assert_eq!(closest.text, "    let x = 1;");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(apply(FILE, "", "x", false).unwrap_err(), MatchError::Empty);
    }
}
