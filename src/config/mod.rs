use super::*;
use std::fs;

pub mod series;
pub use series::{set_skip_intros, skip_intros_for, SeriesPrefs};


pub const APP_DIR_NAME: &str = "Jellyfast";
pub const KEYCHAIN_SERVICE: &str = "to.nathancurtis.jellyfast";
/// Names used before the rename to Jellyfast; read once and migrated.
pub const LEGACY_APP_DIR_NAME: &str = "JellyfinMac";
pub const LEGACY_KEYCHAIN_SERVICE: &str = "app.jellyfin.mac";

/// How the sidebar is organised. `Separate` lists every library (the classic
/// layout); `Unified` shows Home, Browse, Search and Requests with one mixed grid.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    Separate,
    Unified,
}

/// Which tab the one-big-library Home shows: the mixed Home, or one media type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HomeSection {
    #[default]
    Home,
    Movies,
    TvShows,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub last_view: Option<String>,
    #[serde(default)]
    pub last_item_id: Option<String>,
    #[serde(default)]
    pub window: Option<[f64; 4]>, // x, y, w, h
    #[serde(default)]
    pub volume: f64,
    /// `None` until the first-run onboarding is answered; that is what shows it.
    #[serde(default)]
    pub layout_mode: Option<LayoutMode>,
    /// Last Home tab chosen in the one-big-library layout (`None` means Home).
    #[serde(default)]
    pub home_section: Option<HomeSection>,
    #[serde(default)]
    pub settings: Settings,
    /// "Remember me" on the login card. When true, the session token is kept in
    /// the Keychain and the app signs in on launch; when false nothing persists.
    #[serde(default = "default_remember_me")]
    pub remember_me: bool,
    /// Library view ids in the user's chosen sidebar order. Unlisted libraries follow in server order.
    #[serde(default)]
    pub library_order: Vec<String>,
    /// Library view ids the user hid from the sidebar.
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
    /// Header sort popup choice: the Jellyfin `SortBy` key (see `app::library_query::SORT_CHOICES`).
    /// `None` means Name.
    #[serde(default)]
    pub sort_by: Option<String>,
    /// Last accepted JSON text per palette id, used when a palette file goes missing or invalid.
    #[serde(default)]
    pub palette_cache: std::collections::BTreeMap<String, String>,
}

fn default_remember_me() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server_url: None,
            username: None,
            user_id: None,
            device_id: String::new(),
            last_view: None,
            last_item_id: None,
            window: None,
            volume: 0.0,
            settings: Settings::default(),
            remember_me: default_remember_me(),
            library_order: Vec::new(),
            hidden_libraries: Vec::new(),
            sort_by: None,
            layout_mode: None,
            home_section: None,
            palette_cache: Default::default(),
        }
    }
}

/// Test seam: tests point the app dirs at a temp directory and serialize
/// themselves via `test_env()`. Absent in non-test builds.
#[cfg(test)]
pub(crate) static DIR_OVERRIDE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn dir_override() -> Option<PathBuf> {
    DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(d) = dir_override() {
        return d.join("config");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME)
}

/// Move `~/Library/{Application Support,Caches}/JellyfinMac` to the Jellyfast names
/// when only the old directory exists. Best effort: failures leave the old data in place.
pub fn migrate_legacy_dirs() {
    #[cfg(test)]
    if dir_override().is_some() {
        return;
    }
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    for base in [home.join("Library").join("Application Support"), home.join("Library").join("Caches")] {
        let (old, new) = (base.join(LEGACY_APP_DIR_NAME), base.join(APP_DIR_NAME));
        if old.is_dir() && !new.exists() {
            let _ = fs::rename(&old, &new);
        }
    }
}

pub fn cache_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(d) = dir_override() {
        return d.join("cache");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("Library").join("Caches").join(APP_DIR_NAME)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> Config {
    let mut cfg: Config = match fs::read_to_string(config_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    };
    // A missing or corrupt file (or one lacking a device id) must still yield a
    // usable DeviceId for Jellyfin.
    if cfg.device_id.is_empty() {
        cfg.device_id = generate_device_id();
    }
    cfg
}

/// Atomic-ish save: write to a temp file, rename over the target so a
/// crash mid-write cannot corrupt the config.
pub fn save_config(cfg: &Config) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join("config.json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, config_path());
        }
    }
    tracing::debug!("config saved");
}

/// Save a UI-side snapshot of the config without clobbering the session fields.
///
/// The UI holds its own copy loaded at launch, while the worker writes
/// `server_url`, `user_id`, `username` and `remember_me` when a login succeeds.
/// Writing the stale snapshot back (on quit, last-view change, settings save)
/// erased the session, so the next launch had nothing to auto-connect with.
/// The worker is the only writer of these fields, so take them from disk.
pub fn save_config_keeping_session(cfg: &Config) {
    let disk = load_config();
    let mut merged = cfg.clone();
    merged.server_url = disk.server_url;
    merged.username = disk.username;
    merged.user_id = disk.user_id;
    merged.remember_me = disk.remember_me;
    save_config(&merged);
}

fn generate_device_id() -> String {
    // Pure std: time + pid + address entropy. Good enough for a Jellyfin
    // DeviceId (uniqueness per install, no crypto requirement).
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let addr = &t as *const _ as usize;
    format!("jr-mac-{:x}{:x}{:x}", t, std::process::id(), addr)
}

pub use crate::storage::history::*;
pub use crate::storage::keychain::*;

/// Test harness: serializes tests that touch global app state, redirects the
/// config/cache dirs to a fresh temp dir and clears the in-memory token store.
#[cfg(test)]
pub(crate) mod test_env {
    use super::*;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    pub(crate) struct TestEnv {
        _g: std::sync::MutexGuard<'static, ()>,
        pub dir: PathBuf,
    }
    impl TestEnv {
        pub(crate) fn new() -> Self {
            let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!(
                "jellyfast-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            *DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
            crate::storage::keychain::test_store_clear();
            TestEnv { _g: g, dir }
        }
    }
    impl Drop for TestEnv {
        fn drop(&mut self) {
            *DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_env::TestEnv;
    use super::*;

    #[test]
    fn load_config_fresh_has_device_id() {
        let _e = TestEnv::new();
        let c = load_config();
        assert!(c.device_id.starts_with("jr-mac-"));
        assert!(c.server_url.is_none());
    }

    #[test]
    fn save_load_roundtrip_and_creates_dir() {
        let _e = TestEnv::new();
        let mut c = load_config();
        c.server_url = Some("http://x:1".into());
        c.settings.max_bitrate_kbps = 1234;
        save_config(&c);
        let l = load_config();
        assert_eq!(l.server_url.as_deref(), Some("http://x:1"));
        assert_eq!(l.settings.max_bitrate_kbps, 1234);
        assert_eq!(l.device_id, c.device_id);
    }

    #[test]
    fn corrupt_config_still_gets_device_id() {
        let _e = TestEnv::new();
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(config_path(), "{not json").unwrap();
        assert!(!load_config().device_id.is_empty());
        fs::write(config_path(), "{\"server_url\":\"http://a\"}").unwrap();
        let c = load_config();
        assert_eq!(c.server_url.as_deref(), Some("http://a"));
        assert!(!c.device_id.is_empty());
    }

    #[test]
    fn library_prefs_default_when_absent_and_roundtrip() {
        let _e = TestEnv::new();
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(config_path(), "{\"server_url\":\"http://a\"}").unwrap();
        let c = load_config();
        assert!(c.library_order.is_empty() && c.hidden_libraries.is_empty());
        let mut c = c;
        c.library_order = vec!["v2".into(), "v1".into()];
        c.hidden_libraries = vec!["v3".into()];
        save_config(&c);
        let l = load_config();
        assert_eq!(l.library_order, ["v2", "v1"]);
        assert_eq!(l.hidden_libraries, ["v3"]);
    }

    #[test]
    fn home_section_defaults_to_home_and_round_trips() {
        let _e = TestEnv::new();
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(config_path(), "{\"server_url\":\"http://a\"}").unwrap();
        assert_eq!(load_config().home_section, None, "missing key means the Home tab");
        fs::write(config_path(), "{\"home_section\":\"tv_shows\"}").unwrap();
        assert_eq!(load_config().home_section, Some(HomeSection::TvShows));
        let mut c = load_config();
        c.home_section = Some(HomeSection::Movies);
        save_config(&c);
        assert_eq!(load_config().home_section, Some(HomeSection::Movies));
        assert_eq!(HomeSection::default(), HomeSection::Home);
    }

    #[test]
    fn layout_mode_defaults_to_unanswered_and_round_trips() {
        let _e = TestEnv::new();
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(config_path(), "{\"server_url\":\"http://a\"}").unwrap();
        assert_eq!(load_config().layout_mode, None, "missing key means onboarding is still due");
        fs::write(config_path(), "{\"layout_mode\":\"unified\"}").unwrap();
        assert_eq!(load_config().layout_mode, Some(LayoutMode::Unified));
        fs::write(config_path(), "{\"layout_mode\":\"separate\"}").unwrap();
        assert_eq!(load_config().layout_mode, Some(LayoutMode::Separate));
        let mut c = load_config();
        c.layout_mode = Some(LayoutMode::Unified);
        save_config(&c);
        assert!(fs::read_to_string(config_path()).unwrap().contains("\"unified\""));
        assert_eq!(load_config().layout_mode, Some(LayoutMode::Unified));
    }

    #[test]
    fn dirs_are_isolated_in_tests() {
        let e = TestEnv::new();
        assert!(config_dir().starts_with(&e.dir));
        assert!(cache_dir().starts_with(&e.dir));
    }
}

#[cfg(test)]
mod remember_me_tests {
    use super::test_env::TestEnv;
    use super::*;

    #[test]
    fn remember_me_defaults_to_on_for_fresh_and_legacy_configs() {
        let _e = TestEnv::new();
        assert!(load_config().remember_me, "fresh install");
        assert!(Config::default().remember_me);
        // A config written before the option existed has no key at all.
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(config_path(), r#"{"server_url":"http://x:1","user_id":"u"}"#).unwrap();
        let c = load_config();
        assert!(c.remember_me);
        assert_eq!(c.server_url.as_deref(), Some("http://x:1"));
    }

    #[test]
    fn remember_me_false_round_trips() {
        let _e = TestEnv::new();
        let mut c = load_config();
        c.remember_me = false;
        save_config(&c);
        assert!(!load_config().remember_me);
    }

    #[test]
    fn ui_snapshot_save_keeps_session_written_by_worker() {
        let _e = TestEnv::new();
        // UI copy taken at launch, before any login.
        let ui_snapshot = load_config();
        // Worker signs in and persists the session.
        let mut worker_cfg = ui_snapshot.clone();
        worker_cfg.server_url = Some("http://srv:8096".into());
        worker_cfg.user_id = Some("u1".into());
        worker_cfg.username = Some("alice".into());
        save_config(&worker_cfg);
        // UI quits with its stale copy (last_view changed, session fields empty).
        let mut ui = ui_snapshot;
        ui.last_view = Some("v-mov".into());
        save_config_keeping_session(&ui);
        let l = load_config();
        assert_eq!(l.server_url.as_deref(), Some("http://srv:8096"));
        assert_eq!(l.user_id.as_deref(), Some("u1"));
        assert_eq!(l.username.as_deref(), Some("alice"));
        assert_eq!(l.last_view.as_deref(), Some("v-mov"));
    }

    #[test]
    fn ui_snapshot_save_does_not_resurrect_a_cleared_session() {
        let _e = TestEnv::new();
        let mut signed_in = load_config();
        signed_in.server_url = Some("http://srv:8096".into());
        signed_in.user_id = Some("u1".into());
        save_config(&signed_in);
        // UI still holds the signed-in copy after the worker signed out.
        let ui = signed_in.clone();
        let mut cleared = load_config();
        cleared.server_url = None;
        cleared.user_id = None;
        save_config(&cleared);
        save_config_keeping_session(&ui);
        let l = load_config();
        assert!(l.server_url.is_none() && l.user_id.is_none());
    }
}
