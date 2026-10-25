use crate::config::Config;
use crate::jellyfin::models::ListItem;
use crate::transcode::decision::PlaybackMethod;
use serde::{Deserialize, Serialize};

// ============================================================================
// SECTION: Message types between UI (main thread) and worker (tokio threads)
// ============================================================================
// SPLIT: move into src/app/messages.rs. These enums are the seam between the
// UI process and the async process; both sides own only senders/handles.

#[derive(Debug, Clone)]
pub enum UiCmd {
    Connect {
        url: String,
        username: String,
        password: Option<String>,
        api_key: Option<String>,
    },
    LoadView {
        view_id: String,
        name: String,
        collection_type: Option<String>,
    },
    Search {
        q: String,
    },
    Episodes {
        series_id: String,
        series_name: String,
    },
    Tracks {
        album_id: String,
        album_name: String,
    },
    Play {
        item_id: String,
    },
    PlayChannel {
        channel_id: String,
        name: String,
    },
    Progress {
        item_id: String,
        ticks: i64,
        paused: bool,
    },
    Stopped {
        item_id: String,
        ticks: i64,
    },
    MarkPlayed {
        item_id: String,
        played: bool,
    },
    SaveSettings {
        settings: Settings,
        reconnect: bool,
    },
    SignOut,
    ProbeServer,
    QuitSaving,
}

#[derive(Debug)]
pub enum MainMsg {
    Connected {
        user_name: String,
        views: Vec<(String, String, Option<String>)>, // id, name, collection_type
        has_live_tv: bool,
    },
    ConnectError(String),
    ItemsLoaded {
        title: String,
        items: Vec<ListItem>,
        can_go_back: bool,
    },
    PosterReady {
        key: u64,
        bytes: Vec<u8>,
    },
    PlaybackReady(PlaybackStart),
    PlaybackError(String),
    ServerUp,
    ServerDown,
    SignedOut,
    SettingsSnapshot(Box<Config>),
}

#[derive(Debug, Clone)]
pub struct PlaybackStart {
    pub url: String,
    pub item_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub resume_secs: f64,
    pub method: PlaybackMethod,
    pub run_time_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub theme: String, // "system" | "light" | "dark"
    #[serde(default)]
    pub max_bitrate_kbps: u64, // 0 = original
    #[serde(default = "default_sub_lang")]
    pub subtitle_lang: String, // ISO 639-2/1, "" = off
    #[serde(default)]
    pub audio_lang: String,
    #[serde(default)]
    pub searx_enabled: bool,
    #[serde(default)]
    pub searx_url: String,
}
fn default_sub_lang() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_deserialize_with_defaults_and_ignore_unknown() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!((s.theme.as_str(), s.max_bitrate_kbps, s.subtitle_lang.as_str()), ("", 0, ""));
        let s: Settings = serde_json::from_str(r#"{"theme":"dark","max_bitrate_kbps":5000,"extra":1}"#).unwrap();
        assert_eq!((s.theme.as_str(), s.max_bitrate_kbps), ("dark", 5000));
    }
}
