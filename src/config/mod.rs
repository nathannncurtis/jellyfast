use super::*;
use std::fs;


pub const APP_DIR_NAME: &str = "JellyfinMac";
pub const KEYCHAIN_SERVICE: &str = "app.jellyfin.mac";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
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
    #[serde(default)]
    pub settings: Settings,
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
                "jellyfin-mac-test-{}-{}",
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
    fn dirs_are_isolated_in_tests() {
        let e = TestEnv::new();
        assert!(config_dir().starts_with(&e.dir));
        assert!(cache_dir().starts_with(&e.dir));
    }
}
