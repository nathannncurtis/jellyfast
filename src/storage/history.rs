use crate::config::config_dir;
use crate::*;
use std::fs;
use std::path::PathBuf;

// ---- Watch history (server is authoritative; this is an offline mirror)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryEntry {
    pub item_id: String,
    pub name: String,
    pub at_ms: u64,
}

pub fn history_path() -> PathBuf {
    config_dir().join("history.json")
}

pub fn load_history() -> Vec<HistoryEntry> {
    fs::read_to_string(history_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn push_history(mut h: Vec<HistoryEntry>, e: HistoryEntry) -> Vec<HistoryEntry> {
    h.retain(|x| x.item_id != e.item_id);
    h.insert(0, e);
    h.truncate(50);
    let _ = fs::write(history_path(), serde_json::to_string(&h).unwrap_or_default());
    h
}
