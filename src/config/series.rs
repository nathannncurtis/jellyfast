//! Per-show settings, kept on this Mac and keyed by series id. They never go to the
//! server. They live in their own file, not in `config.json`: the worker writes that
//! file from a snapshot taken at launch, which would undo a show setting saved later.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct SeriesPrefs {
    /// Series id -> "skip intros automatically" for that show. Absent means the global default.
    #[serde(default)]
    pub skip_intros: BTreeMap<String, bool>,
}

pub fn series_prefs_path() -> PathBuf {
    super::config_dir().join("series_prefs.json")
}

/// Missing or unreadable file reads as no per-show settings.
pub fn load_series_prefs() -> SeriesPrefs {
    fs::read_to_string(series_prefs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Atomic write, like `save_config`: temp file, then rename over the target.
pub fn save_series_prefs(p: &SeriesPrefs) -> io::Result<()> {
    let path = series_prefs_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(p).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)
}

/// Effective "skip intros" for a show: its own setting when it has one, else `global_default`.
pub fn skip_intros_for(series_id: &str, global_default: bool) -> bool {
    load_series_prefs().skip_intros.get(series_id).copied().unwrap_or(global_default)
}

/// Record the show's own "skip intros" setting.
pub fn set_skip_intros(series_id: &str, on: bool) -> io::Result<()> {
    let mut p = load_series_prefs();
    p.skip_intros.insert(series_id.to_string(), on);
    save_series_prefs(&p)
}

#[cfg(test)]
mod tests {
    use super::super::test_env::TestEnv;
    use super::*;

    #[test]
    fn missing_file_falls_back_to_the_global_default() {
        let _e = TestEnv::new();
        assert_eq!(load_series_prefs(), SeriesPrefs::default());
        assert!(skip_intros_for("show-1", true));
        assert!(!skip_intros_for("show-1", false));
    }

    #[test]
    fn per_show_setting_persists_and_overrides_the_default() {
        let _e = TestEnv::new();
        set_skip_intros("show-1", true).unwrap();
        set_skip_intros("show-2", false).unwrap();
        // Reads back from disk, not from memory.
        assert!(skip_intros_for("show-1", false));
        assert!(!skip_intros_for("show-2", true), "an explicit off beats a global on");
        assert!(!skip_intros_for("show-3", false), "unknown show uses the default");
        set_skip_intros("show-1", false).unwrap();
        assert!(!skip_intros_for("show-1", true));
        assert_eq!(load_series_prefs().skip_intros.len(), 2, "one entry per series id");
    }

    #[test]
    fn corrupt_file_reads_as_empty_and_is_replaced_on_write() {
        let _e = TestEnv::new();
        fs::create_dir_all(super::super::config_dir()).unwrap();
        fs::write(series_prefs_path(), "{not json").unwrap();
        assert_eq!(load_series_prefs(), SeriesPrefs::default());
        set_skip_intros("s", true).unwrap();
        assert!(skip_intros_for("s", false));
    }

    #[test]
    fn writing_series_prefs_does_not_touch_config_json() {
        let _e = TestEnv::new();
        let mut c = super::super::load_config();
        c.server_url = Some("http://a:1".into());
        super::super::save_config(&c);
        set_skip_intros("s", true).unwrap();
        assert_eq!(super::super::load_config().server_url.as_deref(), Some("http://a:1"));
        assert!(series_prefs_path().exists());
        assert_ne!(series_prefs_path(), super::super::config_path());
    }
}
