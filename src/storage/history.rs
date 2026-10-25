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
    let _ = fs::create_dir_all(config_dir());
    let _ = fs::write(history_path(), serde_json::to_string(&h).unwrap_or_default());
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_env::TestEnv;

    fn e(id: &str, at: u64) -> HistoryEntry {
        HistoryEntry { item_id: id.into(), name: format!("n{id}"), at_ms: at }
    }

    #[test]
    fn push_dedups_moves_to_front_and_persists_without_existing_dir() {
        let _env = TestEnv::new();
        let h = push_history(vec![], e("a", 1));
        let h = push_history(h, e("b", 2));
        let h = push_history(h, e("a", 3));
        assert_eq!(h.iter().map(|x| x.item_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        let loaded = load_history();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].at_ms, 3);
    }

    #[test]
    fn push_truncates_to_50() {
        let _env = TestEnv::new();
        let mut h = vec![];
        for i in 0..60 {
            h = push_history(h, e(&i.to_string(), i));
        }
        assert_eq!(h.len(), 50);
        assert_eq!(h[0].item_id, "59");
        assert_eq!(load_history().len(), 50);
    }

    #[test]
    fn load_missing_or_corrupt_is_empty() {
        let _env = TestEnv::new();
        assert!(load_history().is_empty());
        std::fs::create_dir_all(config_dir()).unwrap();
        std::fs::write(history_path(), "garbage").unwrap();
        assert!(load_history().is_empty());
    }
}
