//! Full-text search over conversation titles and finished messages.

use std::collections::HashSet;

use rusqlite::params;
use serde::Serialize;

use super::sessions::Mode;
use super::{Result, Store};

/// One search result. In `snippet`, `\u{2}` and `\u{3}` mark the start and end of each match
/// (the UI turns them into highlights; they can't appear in typed text).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub session_id: String,
    pub message_id: Option<String>,
    pub mode: Mode,
    pub title: String,
    pub snippet: String,
    pub updated_at: i64,
}

impl Store {
    /// Matching conversations, one hit each: title matches first, then message matches.
    pub fn search(&self, query: &str, mode: Option<Mode>, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mode = mode.map(Mode::as_str);
        let conn = self.conn();
        let mut hits = Vec::new();
        let mut seen = HashSet::new();

        let mut titles = conn.prepare(
            "SELECT s.id, s.mode, s.title, s.updated_at
             FROM titles_fts t JOIN sessions s ON s.id = t.session_id
             WHERE titles_fts MATCH ?1 AND s.archived = 0 AND (?2 IS NULL OR s.mode = ?2)
             ORDER BY rank LIMIT ?3",
        )?;
        let rows = titles.query_map(params![fts, mode, limit as i64], |row| {
            let title: String = row.get(2)?;
            Ok(SearchHit {
                session_id: row.get(0)?,
                message_id: None,
                mode: Mode::parse(&row.get::<_, String>(1)?).unwrap_or(Mode::Chat),
                snippet: title.clone(),
                title,
                updated_at: row.get(3)?,
            })
        })?;
        for hit in rows {
            let hit = hit?;
            if seen.insert(hit.session_id.clone()) {
                hits.push(hit);
            }
        }

        let mut messages = conn.prepare(
            "SELECT f.session_id, f.message_id, s.mode, s.title, snippet(messages_fts, 2, char(2), char(3), '…', 12),
                    s.updated_at
             FROM messages_fts f JOIN sessions s ON s.id = f.session_id
             WHERE messages_fts MATCH ?1 AND s.archived = 0 AND (?2 IS NULL OR s.mode = ?2)
             ORDER BY rank LIMIT ?3",
        )?;
        let rows = messages.query_map(params![fts, mode, (limit * 4) as i64], |row| {
            Ok(SearchHit {
                session_id: row.get(0)?,
                message_id: row.get(1)?,
                mode: Mode::parse(&row.get::<_, String>(2)?).unwrap_or(Mode::Chat),
                title: row.get(3)?,
                snippet: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        for hit in rows {
            let hit = hit?;
            if hits.len() >= limit {
                break;
            }
            if seen.insert(hit.session_id.clone()) {
                hits.push(hit);
            }
        }
        hits.truncate(limit);
        Ok(hits)
    }
}

/// What the user typed, as an FTS5 query: every word must appear (as a word prefix).
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::fts_query;

    #[test]
    fn typed_text_becomes_a_safe_prefix_query() {
        assert_eq!(
            fts_query("rust traits").as_deref(),
            Some("\"rust\"* \"traits\"*")
        );
        assert_eq!(
            fts_query("say \"hi\" OR x").as_deref(),
            Some("\"say\"* \"hi\"* \"OR\"* \"x\"*")
        );
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("\"\""), None);
    }
}
