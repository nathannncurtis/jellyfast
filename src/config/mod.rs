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

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME)
}

pub fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("Library").join("Caches").join(APP_DIR_NAME)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> Config {
    match fs::read_to_string(config_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config {
            device_id: generate_device_id(),
            ..Default::default()
        },
    }
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
