use crate::config::Config;
use crate::jellyfin::models::ListItem;
use crate::seerr::models::{SeerrRequestItem, SeerrResult};
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
    /// Debounced like `Search`. Blank query yields empty `SeerrResults`.
    SeerrSearch {
        q: String,
    },
    /// `media_type` is "movie" or "tv" (tv requests all seasons).
    SeerrRequest {
        media_type: String,
        tmdb_id: u64,
        title: String,
    },
    /// The signed-in Seerr user's recent requests, newest first.
    SeerrRequests,
    /// Validate the connection, then persist. A non-blank `username` selects
    /// Jellyfin-account sign-in (`password: None`/blank reuses the Keychain
    /// password for this URL); a blank one selects API-key mode (`api_key:
    /// None`/blank reuses the stored key).
    SaveSeerr {
        enabled: bool,
        url: String,
        username: String,
        password: Option<String>,
        api_key: Option<String>,
    },
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
    SeerrResults(Vec<SeerrResult>),
    SeerrRequested {
        title: String,
    },
    SeerrRequestList(Vec<SeerrRequestItem>),
    /// Search/request/list failure, "already requested", or Seerr disabled/unconfigured.
    SeerrError(String),
    /// Outcome of `SaveSeerr` (ok=false: nothing was persisted).
    SeerrStatus {
        ok: bool,
        message: String,
    },
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
    /// `poster_key` of the item's artwork (for the buffering backdrop), when it has some.
    pub image_key: Option<u64>,
    pub media_kind: MediaKind,
}

/// Whether playback shows the full-window video view or the now-playing bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
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
    pub seerr_enabled: bool,
    /// Base URL of the Seerr instance. The API key lives in the Keychain
    /// (account "seerr@<url>"), never here.
    #[serde(default)]
    pub seerr_url: String,
    /// Non-empty = sign in with a Jellyfin account (password in the Keychain,
    /// account `seerr-login@<url>`); empty = API-key mode.
    #[serde(default)]
    pub seerr_username: String,
    /// Show the playback-info panel (time, resolution, bitrate, method) over the video.
    #[serde(default)]
    pub show_stats: bool,
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

    #[test]
    fn seerr_settings_default_and_unknown_keys_ignored() {
        let s: Settings = serde_json::from_str(r#"{"legacy_flag":true,"legacy_url":"https://s.x"}"#).unwrap();
        assert!(!s.seerr_enabled && s.seerr_url.is_empty());
        let s: Settings = serde_json::from_str(r#"{"seerr_enabled":true,"seerr_url":"http://h:5055"}"#).unwrap();
        assert!(s.seerr_enabled);
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("seerr_url") );
        assert!(!j.contains("api_key"));
    }
}
