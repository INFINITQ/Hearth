//! App settings: a typed struct saved as JSON in the data folder.
//!
//! The UI sends partial updates as JSON merge patches (RFC 7386: objects merge, `null` resets
//! a value to its default). A patch is merged into the current settings and validated by
//! deserialising into [`Settings`] before anything is written, so a bad patch never reaches
//! the disk. Unknown keys are rejected instead of silently dropped, so UI typos show up.

use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fsutil::write_atomic;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow Windows' light/dark app mode.
    #[default]
    System,
    Light,
    Dark,
}

/// Which model to use. For now a model on a connection (a server Hearth talks to); Hearth's own
/// managed llama-server presets join in M5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModelRef {
    Connection {
        connection_id: String,
        model: String,
    },
}

/// The default model for each job (each chat or session can override it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelRoles {
    pub chat: Option<ModelRef>,
    pub code: Option<ModelRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatSettings {
    /// Replaces Hearth's default chat system prompt when set.
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeSettings {
    /// The shell the agent runs commands in (the other one is used if this one is missing).
    pub shell: crate::agent::shell::ShellKind,
}

/// Everything the user can change. New fields need `#[serde(default)]` semantics (the struct
/// is `default`), so settings files from older versions keep loading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub sidebar_collapsed: bool,
    pub models: ModelRoles,
    pub chat: ChatSettings,
    pub code: CodeSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Invalid setting: {0}")]
    Invalid(String),
    #[error("Could not save settings: {0}")]
    Io(#[from] std::io::Error),
}

/// The settings file plus an in-memory copy. Writes are serialised by the lock.
pub struct SettingsStore {
    path: PathBuf,
    current: RwLock<Settings>,
}

impl SettingsStore {
    /// Loads `path`. A missing file means defaults. A damaged file is kept as
    /// `settings.json.bad` (so nothing is lost) and defaults are used.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let current = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_else(|_| {
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        Self {
            path,
            current: RwLock::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Applies a JSON merge patch, saves, and returns the new settings.
    pub fn update(&self, patch: &Value) -> Result<Settings, SettingsError> {
        if !patch.is_object() {
            return Err(SettingsError::Invalid(
                "expected an object of settings".into(),
            ));
        }
        let mut current = self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut merged =
            serde_json::to_value(&*current).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        merge_patch(&mut merged, patch);
        let next: Settings = serde_json::from_value(merged.clone())
            .map_err(|e| SettingsError::Invalid(e.to_string()))?;
        let normalized =
            serde_json::to_value(&next).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        if let Some(key) = unknown_key(&merged, &normalized, "") {
            return Err(SettingsError::Invalid(format!("unknown setting `{key}`")));
        }
        let bytes =
            serde_json::to_vec_pretty(&next).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        write_atomic(&self.path, &bytes)?;
        *current = next.clone();
        Ok(next)
    }
}

/// RFC 7386 JSON merge patch.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(target) = target else {
        return;
    };
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

/// The first key path present in `merged` that serde dropped from `normalized`.
fn unknown_key(merged: &Value, normalized: &Value, prefix: &str) -> Option<String> {
    let (Value::Object(merged), Value::Object(normalized)) = (merged, normalized) else {
        return None;
    };
    for (key, value) in merged {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match normalized.get(key) {
            None => return Some(path),
            Some(inner) => {
                if let Some(found) = unknown_key(value, inner, &path) {
                    return Some(found);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(dir.path().join("settings.json"));
        (dir, store)
    }

    #[test]
    fn a_missing_file_means_defaults() {
        let (_dir, store) = store();
        assert_eq!(store.get(), Settings::default());
        assert_eq!(store.get().theme, Theme::System);
    }

    #[test]
    fn updates_are_saved_and_survive_a_restart() {
        let (dir, store) = store();
        let next = store
            .update(&json!({ "theme": "dark", "sidebarCollapsed": true }))
            .unwrap();
        assert_eq!(next.theme, Theme::Dark);
        assert!(next.sidebar_collapsed);
        let reopened = SettingsStore::open(dir.path().join("settings.json"));
        assert_eq!(reopened.get(), next);
    }

    #[test]
    fn invalid_values_are_rejected_and_nothing_is_written() {
        let (dir, store) = store();
        let err = store.update(&json!({ "theme": "purple" })).unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(_)));
        assert!(!dir.path().join("settings.json").exists());
        assert_eq!(store.get().theme, Theme::System);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let (_dir, store) = store();
        let err = store.update(&json!({ "thme": "dark" })).unwrap_err();
        assert!(err.to_string().contains("thme"), "{err}");
    }

    #[test]
    fn null_resets_a_value_to_its_default() {
        let (_dir, store) = store();
        store.update(&json!({ "theme": "light" })).unwrap();
        let next = store.update(&json!({ "theme": null })).unwrap();
        assert_eq!(next.theme, Theme::System);
    }

    #[test]
    fn a_damaged_file_is_kept_aside_and_defaults_are_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = SettingsStore::open(&path);
        assert_eq!(store.get(), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
    }

    #[test]
    fn merge_patch_follows_rfc_7386() {
        let mut doc = json!({ "a": { "b": 1, "c": 2 }, "d": 3 });
        merge_patch(&mut doc, &json!({ "a": { "b": null, "e": 4 }, "d": [1] }));
        assert_eq!(doc, json!({ "a": { "c": 2, "e": 4 }, "d": [1] }));
    }
}
