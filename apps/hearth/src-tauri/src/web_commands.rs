//! Commands for Settings → Web: the search providers' API keys (saved in the database; the UI
//! only learns whether one is set) and a test search with the current settings.

use std::sync::Arc;

use hearth_core::store::Store;
use hearth_core::web::{KEY_BRAVE, KEY_TAVILY, SearchHit, Web};
use serde::Serialize;
use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebKeys {
    brave: bool,
    tavily: bool,
}

fn keys(store: &Store) -> Result<WebKeys> {
    let has = |name| store.kv_get(name).map(|value| value.is_some());
    Ok(WebKeys {
        brave: has(KEY_BRAVE)?,
        tavily: has(KEY_TAVILY)?,
    })
}

/// Which providers have a key.
#[tauri::command]
pub async fn web_keys(store: State<'_, Arc<Store>>) -> Result<WebKeys> {
    keys(&store)
}

/// Saves a provider's API key; an empty one removes it.
#[tauri::command]
pub async fn web_key_set(
    store: State<'_, Arc<Store>>,
    provider: String,
    key: String,
) -> Result<WebKeys> {
    let name = match provider.as_str() {
        "brave" => KEY_BRAVE,
        "tavily" => KEY_TAVILY,
        other => return Err(Error::Invalid(format!("{other} doesn't use an API key."))),
    };
    store.kv_set(name, Some(&key))?;
    keys(&store)
}

/// A search with the current settings, to check that they work.
#[tauri::command]
pub async fn web_test(web: State<'_, Arc<Web>>) -> Result<Vec<SearchHit>> {
    web.search("open source language models", 5, &CancellationToken::new())
        .await
        .map_err(|err| Error::Invalid(err.to_string()))
}
