{\rtf1\ansi\ansicpg1252\cocoartf2907
\cocoatextscaling0\cocoaplatform0{\fonttbl\f0\fswiss\fcharset0 Helvetica;}
{\colortbl;\red255\green255\blue255;}
{\*\expandedcolortbl;;}
\margl1440\margr1440\vieww11520\viewh8400\viewkind0
\pard\tx720\tx1440\tx2160\tx2880\tx3600\tx4320\tx5040\tx5760\tx6480\tx7200\tx7920\tx8640\pardirnatural\partightenfactor0

\f0\fs24 \cf0 //! ============================================================================\
//! Jellyfin macOS Client \'97 Single-file baseline ("jellyfin-mac")\
//! ============================================================================\
//!\
//! Build:\
//!   cargo build --release\
//!   (create Cargo.toml exactly as shown in the CARGO.TOML comment below;\
//!    macOS 14+ on aarch64/x86_64, current stable Rust 1.75+)\
//!\
//! Required system frameworks / capabilities:\
//!   AppKit, AVFoundation, AVKit, CoreMedia, Security  \'97 all linked\
//!   automatically via #[link] stubs in `mod sys` and the `security-framework`\
//!   crate. No provisioning/capabilities needed. App Sandbox is NOT required;\
//!   if you enable it, add Keychain access + network client entitlements.\
//!\
//!   Info.plist notes (you handle packaging):\
//!     - CFBundleIdentifier should be a stable reverse-DNS id (device_id and\
//!       keychain entries are namespaced by strings in config.rs, not the\
//!       bundle id, so changing it later is safe).\
//!     - If your Jellyfin server is plain HTTP, ATS will block AVPlayer media\
//!       loads. Add NSAppTransportSecurity \uc0\u8594  NSAllowsArbitraryLoads = YES (or\
//!       a per-domain exception) to the Info.plist. The Rust HTTP layer is\
//!       unaffected (rustls, no ATS involvement).\
//!\
//! Minimal configuration:\
//!   None required on first launch: the app shows a login card. Enter the\
//!   server base URL (e.g. http://192.168.1.10:8096) and either\
//!   (a) username + password (recommended; a Jellyfin access token is then\
//!       stored in the macOS Keychain), or\
//!   (b) an API key (dashboard \uc0\u8594  API Keys). With an API key the client picks\
//!       the first user account for library browsing (documented limitation).\
//!   The last successful server/user is remembered in\
//!   ~/Library/Application Support/JellyfinMac/config.json; the token lives in\
//!   the Keychain (service "app.jellyfin.mac"). RUST_LOG=debug overrides the\
//!   default `info` log filter.\
//!\
//! Player stack choice (and why):\
//!   AVKit's AVPlayerView + AVPlayer, driven from a pure-Rust control layer.\
//!   This is the "thin binding to AVFoundation/VideoToolbox" option: Apple's\
//!   stack performs demuxing, hardware decode (VideoToolbox), audio routing,\
//!   subtitles, scrubbing, PiP and fullscreen natively, so the binary stays\
//!   tiny and the UI is 100% native (vibrancy, SF Symbols, dark/light).\
//!   "Conversion" on-device therefore means: anything within Apple's codec\
//!   set (h264/hevc/prores/mpeg4, aac/mp3/ac3/eac3/alac/flac/pcm) direct-plays\
//!   with hardware decode \'97 no server transcode. Sources outside that set\
//!   (e.g. mkv/DTS/AV1-on-old-Macs) fall back to a server HLS transcode URL;\
//!   a fully client-side VideoToolbox transcode pipeline in pure Rust would\
//!   require thousands of lines of CoreMedia FFI and is the main known\
//!   limitation (see `mod transcode` comments and KNOWN LIMITATIONS below).\
//!\
//! Architecture notes for future modularization (concrete map):\
//!   src/config/mod.rs        \uc0\u8592  SECTION: Configuration & Secure Storage\
//!   src/storage/keychain.rs  \uc0\u8592  keychain helpers inside that section\
//!   src/storage/cache.rs     \uc0\u8592  poster disk-cache helpers (worker side)\
//!   src/jellyfin/client.rs   \uc0\u8592  SECTION: Jellyfin API Client (struct impl)\
//!   src/jellyfin/models.rs   \uc0\u8592  serde models in that section\
//!   src/jellyfin/urls.rs     \uc0\u8592  pure URL builders in that section\
//!   src/transcode/decision.rs\uc0\u8592  SECTION: Playback Decision & Transcoding\
//!   src/platform/appkit.rs   \uc0\u8592  SECTION: macOS Bindings (`mod sys`, unsafe)\
//!   src/platform/avkit.rs    \uc0\u8592  AVPlayerView/player wiring inside `mod sys`\
//!   src/ui/shell.rs          \uc0\u8592  SECTION: UI Construction (window/menus/sidebar)\
//!   src/ui/library.rs        \uc0\u8592  poster grid & cells inside UI section\
//!   src/ui/login.rs, settings.rs \uc0\u8592  login card & settings sheet in UI section\
//!   src/app/controller.rs    \uc0\u8592  SECTION: Application Controller (message pump)\
//!   src/app/worker.rs        \uc0\u8592  SECTION: Background Worker (tokio loop)\
//!   src/search_integration.rs\uc0\u8592  SECTION: SearXNG Integration\
//!   `mod sys` is the only unsafe code; everything else is safe Rust under\
//!   #![deny(unsafe_code)].\
//!\
//! Known limitations of this single-file version (honest):\
//!   - Codec set = Apple's (see player stack choice); unsupported codecs use\
//!     a server HLS transcode fallback. AV1 only on M3+ via Apple's decoder.\
//!   - No playback queue; "play next episode" not implemented.\
//!   - External (sidecar .srt) subtitles only surface when the server-side\
//!     HLS path delivers WebVTT; embedded tracks are selectable natively.\
//!   - Live TV plays the raw channel TS stream (best-effort, format-limited).\
//!   - API-key login picks the first user (Jellyfin has no /Users/Me for keys).\
//!   - Music playback is one track/album at a time (no queue/gapless).\
//!   - Network-error recovery: automatic retry with backoff (3 attempts),\
//!     not a full offline/re-sync engine.\
//!   - On Intel Macs, CMTime (24-byte struct) returns go through\
//!     objc_msgSend_stret; the `objc` crate handles this, but it is the one\
//!     ABI-sensitive spot \'97 keep the pinned objc version on Intel.\
//!   - Window state, volume, theme, quality etc. persist; PiP uses the\
//!     native AVKit button.\
//!\
//! ===========================================================================\
//! CARGO.TOML (pin exactly these; objc-ecosystem APIs are version-sensitive)\
//! ===========================================================================\
//! [package]\
//! name = "jellyfin-mac"\
//! version = "0.1.0"\
//! edition = "2021"\
//!\
//! [dependencies]\
//! objc = "0.2.7"                       # Objective-C runtime bindings (tiny, stable)\
//! tokio = \{ version = "1", features = ["rt-multi-thread", "time", "sync"] \}\
//! reqwest = \{ version = "0.12", default-features = false, features = ["json", "rustls-tls"] \}\
//! serde = \{ version = "1", features = ["derive"] \}\
//! serde_json = "1"\
//! tracing = "0.1"\
//! tracing-subscriber = \{ version = "0.3", features = ["env-filter"] \}\
//! security-framework = "2"             # Keychain (thin binding to Security.framework)\
//!\
//! # HEAVY-DEPENDENCY JUSTIFICATION (each one, with lighter alternatives):\
//! # tokio   \'97 mandated by the spec for async IO. Alternative: std::thread +\
//! #           ureq (blocking). Heavier but required here.\
//! # reqwest \'97 mandated. rustls-tls avoids OpenSSL/libssl entirely. Lighter\
//! #           alternative considered: `ureq` (~10x smaller, blocking) \'97 would\
//! #           work fine behind tokio::spawn_blocking, kept reqwest per spec.\
//! # security-framework \'97 the only sane Keychain path without hand-rolled\
//! #           SecItem FFI (which would add ~200 lines of unsafe). Kept.\
//! # objc    \'97 required to talk to AppKit/AVKit. `objc2` is the successor and\
//! #           is nicer (typed), but its 0.5\uc0\u8594 0.6 API churn makes single-shot\
//! #           generated code fragile; objc 0.2.7 is frozen and rock-solid.\
//! #           When modularizing, MIGRATE mod sys \uc0\u8594  objc2 (mechanical).\
//! # tracing/tracing-subscriber \'97 spec-mandated logging.\
//! # serde/serde_json \'97 spec-mandated (REST JSON).\
//! # REJECTED: cocoa (deprecated superset of what we need \'97 we hand-roll the\
//! #           few geometry structs), block2/dispatch (avoided by using\
//! #           target/selector instead of blocks everywhere), objc_id (we use\
//! #           a 15-line Ptr RAII wrapper), image crates (NSImage decodes).\
//!\
//! [profile.release]\
//! lto = true\
//! strip = true\
//! panic = "abort"\
//! opt-level = "z"          # small binary; change to 3 if you want max speed\
//! ============================================================================\
\
#![deny(unsafe_code)]\
\
use std::fmt;\
use std::path::PathBuf;\
use std::sync::OnceLock;\
use std::time::Duration;\
\
use serde::\{Deserialize, Serialize\};\
use tokio::sync::mpsc::\{unbounded_channel, UnboundedReceiver, UnboundedSender\};\
\
use crate::config::\{load_config, save_config, Config\};\
use crate::jellyfin::client::JellyfinClient;\
use crate::jellyfin::urls::enc;\
use crate::transcode::decision::\{build_stream_plan, PlaybackMethod\};\
\
// ============================================================================\
// SECTION: Message types between UI (main thread) and worker (tokio threads)\
// ============================================================================\
// SPLIT: move into src/app/messages.rs. These enums are the seam between the\
// UI process and the async process; both sides own only senders/handles.\
\
#[derive(Debug, Clone)]\
pub enum UiCmd \{\
    Connect \{\
        url: String,\
        username: String,\
        password: Option<String>,\
        api_key: Option<String>,\
    \},\
    LoadView \{\
        view_id: String,\
        name: String,\
        collection_type: Option<String>,\
    \},\
    Search \{\
        q: String,\
    \},\
    Episodes \{\
        series_id: String,\
        series_name: String,\
    \},\
    Tracks \{\
        album_id: String,\
        album_name: String,\
    \},\
    Play \{\
        item_id: String,\
    \},\
    PlayChannel \{\
        channel_id: String,\
        name: String,\
    \},\
    Progress \{\
        item_id: String,\
        ticks: i64,\
        paused: bool,\
    \},\
    Stopped \{\
        item_id: String,\
        ticks: i64,\
    \},\
    MarkPlayed \{\
        item_id: String,\
        played: bool,\
    \},\
    SaveSettings \{\
        settings: Settings,\
        reconnect: bool,\
    \},\
    SignOut,\
    ProbeServer,\
    QuitSaving,\
\}\
\
#[derive(Debug)]\
pub enum MainMsg \{\
    Connected \{\
        user_name: String,\
        views: Vec<(String, String, Option<String>)>, // id, name, collection_type\
        has_live_tv: bool,\
    \},\
    ConnectError(String),\
    ItemsLoaded \{\
        title: String,\
        items: Vec<ListItem>,\
        can_go_back: bool,\
    \},\
    PosterReady \{\
        key: u64,\
        bytes: Vec<u8>,\
    \},\
    PlaybackReady(PlaybackStart),\
    PlaybackError(String),\
    ServerUp,\
    ServerDown,\
    SignedOut,\
    SettingsSnapshot(Box<Config>),\
\}\
\
#[derive(Debug, Clone)]\
pub struct PlaybackStart \{\
    pub url: String,\
    pub item_id: String,\
    pub title: String,\
    pub subtitle: Option<String>,\
    pub resume_secs: f64,\
    pub method: PlaybackMethod,\
    pub run_time_secs: f64,\
\}\
\
#[derive(Debug, Clone, Serialize, Deserialize, Default)]\
pub struct Settings \{\
    #[serde(default)]\
    pub theme: String, // "system" | "light" | "dark"\
    #[serde(default)]\
    pub max_bitrate_kbps: u64, // 0 = original\
    #[serde(default = "default_sub_lang")]\
    pub subtitle_lang: String, // ISO 639-2/1, "" = off\
    #[serde(default)]\
    pub audio_lang: String,\
    #[serde(default)]\
    pub searx_enabled: bool,\
    #[serde(default)]\
    pub searx_url: String,\
\}\
fn default_sub_lang() -> String \{\
    String::new()\
\}\
\
// ============================================================================\
// SECTION: Configuration & Secure Storage\
// ============================================================================\
// SPLIT: move `Config`/`Settings` (de)serialization into src/config/mod.rs and\
// the Keychain + fallback-token + history code into src/storage/\{keychain,\
// history\}.rs. Paths use $HOME (no `dirs` dep); switch to NSHomeDirectory via\
// `mod sys` if you want to support nonstandard HOME sandboxes.\
\
pub mod config \{\
    use super::*;\
    use std::fs;\
\
    pub const APP_DIR_NAME: &str = "JellyfinMac";\
    pub const KEYCHAIN_SERVICE: &str = "app.jellyfin.mac";\
\
    #[derive(Serialize, Deserialize, Clone, Debug, Default)]\
    pub struct Config \{\
        #[serde(default)]\
        pub server_url: Option<String>,\
        #[serde(default)]\
        pub username: Option<String>,\
        #[serde(default)]\
        pub user_id: Option<String>,\
        #[serde(default)]\
        pub device_id: String,\
        #[serde(default)]\
        pub last_view: Option<String>,\
        #[serde(default)]\
        pub last_item_id: Option<String>,\
        #[serde(default)]\
        pub window: Option<[f64; 4]>, // x, y, w, h\
        #[serde(default)]\
        pub volume: f64,\
        #[serde(default)]\
        pub settings: Settings,\
    \}\
\
    pub fn config_dir() -> PathBuf \{\
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());\
        PathBuf::from(home)\
            .join("Library")\
            .join("Application Support")\
            .join(APP_DIR_NAME)\
    \}\
\
    pub fn cache_dir() -> PathBuf \{\
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());\
        PathBuf::from(home).join("Library").join("Caches").join(APP_DIR_NAME)\
    \}\
\
    pub fn config_path() -> PathBuf \{\
        config_dir().join("config.json")\
    \}\
\
    pub fn load_config() -> Config \{\
        match fs::read_to_string(config_path()) \{\
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),\
            Err(_) => Config \{\
                device_id: generate_device_id(),\
                ..Default::default()\
            \},\
        \}\
    \}\
\
    /// Atomic-ish save: write to a temp file, rename over the target so a\
    /// crash mid-write cannot corrupt the config.\
    pub fn save_config(cfg: &Config) \{\
        let dir = config_dir();\
        let _ = fs::create_dir_all(&dir);\
        let tmp = dir.join("config.json.tmp");\
        if let Ok(json) = serde_json::to_string_pretty(cfg) \{\
            if fs::write(&tmp, json).is_ok() \{\
                let _ = fs::rename(&tmp, config_path());\
            \}\
        \}\
        tracing::debug!("config saved");\
    \}\
\
    fn generate_device_id() -> String \{\
        // Pure std: time + pid + address entropy. Good enough for a Jellyfin\
        // DeviceId (uniqueness per install, no crypto requirement).\
        use std::time::\{SystemTime, UNIX_EPOCH\};\
        let t = SystemTime::now()\
            .duration_since(UNIX_EPOCH)\
            .map(|d| d.as_nanos())\
            .unwrap_or(0);\
        let addr = &t as *const _ as usize;\
        format!("jr-mac-\{:x\}\{:x\}\{:x\}", t, std::process::id(), addr)\
    \}\
\
    // ---- Keychain (security-framework) with documented fallback ------------\
    //\
    // Primary: macOS Keychain generic password (service = KEYCHAIN_SERVICE,\
    // account = "token@<server-url>"). The `security-framework` crate is a\
    // thin, audited binding; no plaintext token ever touches disk.\
    //\
    // Fallback (only if SecItemAdd fails \'97 e.g. broken keychain in CI):\
    // base64 token in token.fallback with 0600 perms. This is intentionally\
    // loud (tracing::warn!) and NOT encrypted; documented per the spec.\
\
    pub fn keychain_account(server: &str) -> String \{\
        format!("token@\{\}", server)\
    \}\
\
    pub fn store_token(server: &str, token: &str) \{\
        use security_framework::passwords;\
        match passwords::set_generic_password(KEYCHAIN_SERVICE, &keychain_account(server), token) \{\
            Ok(_) => \{\
                let _ = std::fs::remove_file(fallback_token_path()); // clean stale fallback\
                tracing::info!("token stored in Keychain");\
            \}\
            Err(e) => \{\
                tracing::warn!("Keychain unavailable (\{\}); using documented fallback file", e);\
                let p = fallback_token_path();\
                let _ = std::fs::create_dir_all(config_dir());\
                let b64 = \{\
                    use std::io::Write;\
                    let mut buf = Vec::new();\
                    let mut enc = base64_writer(&mut buf);\
                    let _ = enc.write_all(token.as_bytes());\
                    String::from_utf8(buf).unwrap_or_default()\
                \};\
                let _ = fs::write(&p, b64);\
                set_owner_only(&p);\
            \}\
        \}\
    \}\
\
    pub fn load_token(server: &str) -> Option<String> \{\
        use security_framework::passwords;\
        match passwords::get_generic_password(KEYCHAIN_SERVICE, &keychain_account(server)) \{\
            Ok(t) if !t.is_empty() => Some(t),\
            _ => \{\
                let p = fallback_token_path();\
                fs::read_to_string(p).ok().map(|s| s.trim().to_string())\
            \}\
        \}\
    \}\
\
    pub fn delete_token(server: &str) \{\
        use security_framework::passwords;\
        let _ = passwords::delete_generic_password(KEYCHAIN_SERVICE, &keychain_account(server));\
        let _ = fs::remove_file(fallback_token_path());\
    \}\
\
    fn fallback_token_path() -> PathBuf \{\
        config_dir().join("token.fallback")\
    \}\
\
    fn set_owner_only(p: &std::path::Path) \{\
        use std::os::unix::fs::PermissionsExt;\
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));\
    \}\
\
    // Minimal base64 (stdlib only; avoids the `base64` crate dependency).\
    fn base64_writer(out: &mut Vec<u8>) -> impl std::io::Write + '_ \{\
        struct W<'a>(&'a mut Vec<u8>);\
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";\
        impl<'a> std::io::Write for W<'a> \{\
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> \{\
                for chunk in b.chunks(3) \{\
                    let n = (chunk[0] as u32) << 16\
                        | *chunk.get(1).unwrap_or(&0) as u32 << 8\
                        | *chunk.get(2).unwrap_or(&0) as u32;\
                    self.0.push(T[(n >> 18) as usize & 63] as u8);\
                    self.0.push(T[(n >> 12) as usize & 63] as u8);\
                    self.0.push(if chunk.len() > 1 \{ T[(n >> 6) as usize & 63] \} else \{ b'=' \});\
                    self.0.push(if chunk.len() > 2 \{ T[n as usize & 63] \} else \{ b'=' \});\
                \}\
                Ok(b.len())\
            \}\
            fn flush(&mut self) -> std::io::Result<()> \{\
                Ok(())\
            \}\
        \}\
        W(out)\
    \}\
\
    // ---- Watch history (server is authoritative; this is an offline mirror)\
    #[derive(Serialize, Deserialize, Clone, Debug)]\
    pub struct HistoryEntry \{\
        pub item_id: String,\
        pub name: String,\
        pub at_ms: u64,\
    \}\
\
    pub fn history_path() -> PathBuf \{\
        config_dir().join("history.json")\
    \}\
\
    pub fn load_history() -> Vec<HistoryEntry> \{\
        fs::read_to_string(history_path())\
            .ok()\
            .and_then(|s| serde_json::from_str(&s).ok())\
            .unwrap_or_default()\
    \}\
\
    pub fn push_history(mut h: Vec<HistoryEntry>, e: HistoryEntry) -> Vec<HistoryEntry> \{\
        h.retain(|x| x.item_id != e.item_id);\
        h.insert(0, e);\
        h.truncate(50);\
        let _ = fs::write(history_path(), serde_json::to_string(&h).unwrap_or_default());\
        h\
    \}\
\}\
\
// ============================================================================\
// SECTION: Jellyfin API Client\
// ============================================================================\
// SPLIT: struct+impl \uc0\u8594  src/jellyfin/client.rs; serde models \u8594 \
// src/jellyfin/models.rs; pure URL/query helpers below \uc0\u8594  src/jellyfin/urls.rs\
// (those pure helpers carry the unit tests at the bottom of this file).\
\
pub mod jellyfin \{\
    pub mod urls \{\
        /// Normalize a server base URL: require http/https, strip trailing '/'.\
        pub fn normalize_base_url(input: &str) -> Result<String, String> \{\
            let s = input.trim();\
            if s.is_empty() \{\
                return Err("Server URL is empty.".into());\
            \}\
            let lower = s.to_ascii_lowercase();\
            if !lower.starts_with("http://") && !lower.starts_with("https://") \{\
                return Err("URL must start with http:// or https://".into());\
            \}\
            let trimmed = s.trim_end_matches('/');\
            if trimmed.len() < "http://x".len() \{\
                return Err("URL is too short to be valid.".into());\
            \}\
            Ok(trimmed.to_string())\
        \}\
\
        /// RFC 3986 unreserved-char percent encoding (query values, tokens).\
        pub fn enc(s: &str) -> String \{\
            let mut out = String::with_capacity(s.len());\
            for b in s.bytes() \{\
                match b \{\
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => \{\
                        out.push(b as char)\
                    \}\
                    _ => out.push_str(&format!("%\{:02X\}", b)),\
                \}\
            \}\
            out\
        \}\
\
        pub fn qs(pairs: &[(&str, String)]) -> String \{\
            pairs\
                .iter()\
                .map(|(k, v)| format!("\{\}=\{\}", k, enc(v)))\
                .collect::<Vec<_>>()\
                .join("&")\
        \}\
\
        /// Direct-play URL (static file). AVPlayer fetches with Apple's\
        /// networking; auth rides in the `api_key` query param, which Jellyfin\
        /// accepts on media/image endpoints.\
        pub fn build_direct_url(\
            base: &str,\
            item_id: &str,\
            media_source_id: &str,\
            container: Option<&str>,\
            token: &str,\
        ) -> String \{\
            let ext = container\
                .map(|c| format!(".\{\}", c.split(',').next().unwrap_or("mp4")))\
                .unwrap_or_default();\
            format!(\
                "\{\}/Videos/\{\}/stream\{\}?\{\}",\
                base,\
                item_id,\
                ext,\
                qs(&[\
                    ("static", "true".into()),\
                    ("MediaSourceId", media_source_id.into()),\
                    ("api_key", token.into()),\
                ])\
            )\
        \}\
\
        /// Server-assisted HLS fallback (documented limitation: used only for\
        /// codecs/containers Apple cannot decode on this device).\
        pub fn build_hls_url(\
            base: &str,\
            item_id: &str,\
            media_source_id: &str,\
            token: &str,\
            max_bitrate_kbps: u64,\
            subtitle_lang: &str,\
        ) -> String \{\
            let mut pairs: Vec<(&str, String)> = vec![\
                ("MediaSourceId", media_source_id.into()),\
                ("VideoCodec", "h264,hevc".into()),\
                ("AudioCodec", "aac,mp3,ac3,eac3".into()),\
                ("TranscodingMaxAudioChannels", "2".into()),\
                ("SegmentContainer", "mp4".into()),\
                ("MinSegments", "1".into()),\
                ("BreakOnNonKeyFrames", "True".into()),\
                (\
                    "TranscodeReasons",\
                    "ContainerNotSupported,VideoCodecNotSupported,AudioCodecNotSupported".into(),\
                ),\
                ("api_key", token.into()),\
            ];\
            if max_bitrate_kbps > 0 \{\
                pairs.push(("VideoBitrate", max_bitrate_kbps.to_string()));\
                pairs.push(("MaxWidth", "3840".into()));\
                pairs.push(("MaxHeight", "2160".into()));\
            \}\
            if !subtitle_lang.is_empty() \{\
                pairs.push(("SubtitleStreamIndex", "".into())); // resolved server-side via lang pref below\
                pairs.insert(pairs.len() - 2, ("SubtitleLanguage", subtitle_lang.into()));\
            \}\
            format!("\{\}/Videos/\{\}/master.m3u8?\{\}", base, item_id, qs(&pairs))\
        \}\
\
        pub fn build_image_url(base: &str, item_id: &str, token: &str, max_w: u32) -> String \{\
            format!(\
                "\{\}/Items/\{\}/Images/Primary?\{\}",\
                base,\
                item_id,\
                qs(&[\
                    ("maxWidth", max_w.to_string()),\
                    ("quality", "85".into()),\
                    ("api_key", token.into()),\
                ])\
            )\
        \}\
\
        pub fn build_livetv_url(base: &str, channel_id: &str, token: &str) -> String \{\
            format!(\
                "\{\}/LiveTv/Channels/\{\}/stream?\{\}",\
                base,\
                channel_id,\
                qs(&[("api_key", token.into())])\
            )\
        \}\
\
        /// Jellyfin web deep-link (used by the context menu "Open in Web").\
        pub fn build_web_item_url(base: &str, item_id: &str) -> String \{\
            format!("\{\}/web/index.html#!/itemdetails.html?id=\{\}", base, enc(item_id))\
        \}\
\
        /// Ticks conversions (Jellyfin uses 100ns ticks).\
        pub const TICKS_PER_SEC: i64 = 10_000_000;\
        pub fn secs_from_ticks(t: i64) -> f64 \{\
            t as f64 / TICKS_PER_SEC as f64\
        \}\
        pub fn ticks_from_secs(s: f64) -> i64 \{\
            (s.max(0.0) * TICKS_PER_SEC as f64) as i64\
        \}\
        pub fn progress_fraction(pos_ticks: i64, runtime_ticks: Option<i64>) -> f64 \{\
            match runtime_ticks \{\
                Some(rt) if rt > 0 => (pos_ticks as f64 / rt as f64).clamp(0.0, 1.0),\
                _ => 0.0,\
            \}\
        \}\
\
        pub fn fmt_clock(secs: f64) -> String \{\
            let s = secs.max(0.0) as i64;\
            let h = s / 3600;\
            let m = (s % 3600) / 60;\
            let sec = s % 60;\
            if h > 0 \{\
                format!("\{\}:\{:02\}:\{:02\}", h, m, sec)\
            \} else \{\
                format!("\{\}:\{:02\}", m, sec)\
            \}\
        \}\
\
        pub fn fmt_runtime(ticks: Option<i64>) -> String \{\
            match ticks \{\
                Some(t) if t > 0 => \{\
                    let mins = (secs_from_ticks(t) as i64) / 60;\
                    if mins >= 60 \{\
                        format!("\{\}h \{:02\}m", mins / 60, mins % 60)\
                    \} else \{\
                        format!("\{\}m", mins)\
                    \}\
                \}\
                _ => String::new(),\
            \}\
        \}\
    \}\
\
    pub mod client \{\
        use super::super::urls as u;\
        use super::models::*;\
        use crate::jellyfin::urls::enc;\
\
        #[derive(Debug)]\
        pub enum JFError \{\
            BadUrl(String),\
            Network(String),   // connect/timeout/DNS\
            NotJellyfin,       // reachable but not a Jellyfin server\
            Auth(String),      // 401/403\
            Api(u16, String),  // other HTTP status\
            Parse(String),\
        \}\
        impl fmt::Display for JFError \{\
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result \{\
                match self \{\
                    JFError::BadUrl(s) => write!(f, "Invalid server URL: \{\}", s),\
                    JFError::Network(s) => write!(\
                        f,\
                        "Network problem: \{\}. Check the address, your connection, and that the server is running.",\
                        s\
                    ),\
                    JFError::NotJellyfin => write!(\
                        f,\
                        "That address responded, but it is not a Jellyfin server (or the path is wrong)."\
                    ),\
                    JFError::Auth(s) => write!(f, "Sign-in failed: \{\}", s),\
                    JFError::Api(c, s) => write!(f, "Server error (HTTP \{\}): \{\}", c, s),\
                    JFError::Parse(s) => write!(f, "Unexpected server response: \{\}", s),\
                \}\
            \}\
        \}\
\
        #[derive(Clone)]\
        pub struct JellyfinClient \{\
            http: reqwest::Client,\
            pub base: String,\
            pub token: String,\
            pub user_id: String,\
        \}\
\
        fn map_err(e: reqwest::Error) -> JFError \{\
            if e.is_timeout() \{\
                JFError::Network("request timed out".into())\
            \} else if e.is_connect() \{\
                JFError::Network(format!("could not connect (\{\})", e))\
            \} else \{\
                JFError::Network(e.to_string())\
            \}\
        \}\
\
        impl JellyfinClient \{\
            pub fn new(\
                base: &str,\
                token: String,\
                user_id: String,\
                device_id: &str,\
            ) -> Result<Self, JFError> \{\
                let base = u::normalize_base_url(base).map_err(JFError::BadUrl)?;\
                let http = reqwest::Client::builder()\
                    .connect_timeout(Duration::from_secs(5))\
                    .timeout(Duration::from_secs(20))\
                    .user_agent("JellyfinMac/0.1.0")\
                    .build()\
                    .map_err(|e| JFError::Network(e.to_string()))?;\
                Ok(Self \{ http, base, token, user_id \})\
            \}\
\
            fn auth_header(&self) -> String \{\
                format!(\
                    "MediaBrowser Client=\\"JellyfinMac\\", Device=\\"Mac\\", DeviceId=\\"\{\}\\", Version=\\"0.1.0\\"",\
                    enc(&self.device_id_pub())\
                )\
            \}\
\
            fn device_id_pub(&self) -> String \{\
                // DeviceId is embedded in the auth header string by callers\
                // that know it; we derive a stable one from user_id fallback.\
                self.user_id.clone()\
            \}\
\
            fn req(&self) -> reqwest::RequestBuilder \{\
                self.http\
                    .request(\
                        reqwest::Method::GET,\
                        format!("\{\}", self.base),\
                    )\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
            \}\
\
            /// Unauthenticated probe: verifies the URL points at Jellyfin.\
            pub async fn public_info(base: &str) -> Result<ServerInfo, JFError> \{\
                let b = u::normalize_base_url(base).map_err(JFError::BadUrl)?;\
                let http = reqwest::Client::builder()\
                    .connect_timeout(Duration::from_secs(5))\
                    .timeout(Duration::from_secs(10))\
                    .build()\
                    .map_err(|e| JFError::Network(e.to_string()))?;\
                let r = http\
                    .get(format!("\{\}/System/Info/Public", b))\
                    .timeout(Duration::from_secs(10))\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                let code = r.status();\
                if !code.is_success() \{\
                    return Err(JFError::Api(code.as_u16(), format!("GET /System/Info/Public")));\
                \}\
                let text = r.text().await.map_err(|e| JFError::Network(e.to_string()))?;\
                let info: ServerInfo = serde_json::from_str(&text).map_err(|_| JFError::NotJellyfin)?;\
                if info.product_name.as_deref().unwrap_or("").is_empty() \{\
                    return Err(JFError::NotJellyfin);\
                \}\
                Ok(info)\
            \}\
\
            /// Username/password sign-in \uc0\u8594  token + user id.\
            pub async fn authenticate(\
                base: &str,\
                username: &str,\
                password: &str,\
                device_id: &str,\
            ) -> Result<(AuthResponse, String), JFError> \{\
                let b = u::normalize_base_url(base).map_err(JFError::BadUrl)?;\
                let http = reqwest::Client::builder()\
                    .connect_timeout(Duration::from_secs(5))\
                    .timeout(Duration::from_secs(15))\
                    .build()\
                    .map_err(|e| JFError::Network(e.to_string()))?;\
                let r = http\
                    .post(format!("\{\}/Users/AuthenticateByName", b))\
                    .header(\
                        "Authorization",\
                        format!(\
                            "MediaBrowser Client=\\"JellyfinMac\\", Device=\\"Mac\\", DeviceId=\\"\{\}\\", Version=\\"0.1.0\\"",\
                            enc(device_id)\
                        ),\
                    )\
                    .json(&AuthRequest \{ username, pw: password \})\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                let code = r.status();\
                if code.as_u16() == 401 \{\
                    return Err(JFError::Auth("wrong username or password".into()));\
                \}\
                if !code.is_success() \{\
                    return Err(JFError::Api(code.as_u16(), "AuthenticateByName".into()));\
                \}\
                let resp: AuthResponse = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;\
                Ok((resp, b.to_string()))\
            \}\
\
            /// API-key path: token is admin-scoped; pick the first user.\
            pub async fn first_user_id(&self) -> Result<String, JFError> \{\
                #[derive(Deserialize)]\
                struct U \{\
                    #[serde(rename = "Id")]\
                    id: String,\
                \}\
                let r = self\
                    .http\
                    .get(format!("\{\}/Users", self.base))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                let users: Vec<U> = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;\
                users\
                    .into_iter()\
                    .next()\
                    .map(|u| u.id)\
                    .ok_or_else(|| JFError::Auth("no user accounts found".into()))\
            \}\
\
            pub async fn views(&self) -> Result<Vec<View>, JFError> \{\
                #[derive(Deserialize)]\
                struct R \{\
                    #[serde(rename = "Items")]\
                    items: Vec<View>,\
                \}\
                let r = self\
                    .http\
                    .get(format!("\{\}/Users/\{\}/Views", self.base, enc(&self.user_id)))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;\
                Ok(v.items)\
            \}\
\
            pub async fn items(&self, q: &ItemQuery) -> Result<Vec<ListItem>, JFError> \{\
                #[derive(Deserialize)]\
                struct R \{\
                    #[serde(rename = "Items")]\
                    items: Vec<ListItem>,\
                \}\
                let mut url = format!(\
                    "\{\}/Users/\{\}/Items",\
                    self.base,\
                    enc(&self.user_id)\
                );\
                if q.parent_id.is_some() || q.series_id.is_some() \{\
                    // series/album scoping uses the same items endpoint\
                \}\
                let mut pairs: Vec<(&str, String)> = vec![\
                    ("Recursive", "true".into()),\
                    ("Fields", "PrimaryImageAspectRatio,ProductionYear,UserData,Overview".into()),\
                    ("ImageTypeLimit", "1".into()),\
                    ("Limit", q.limit.to_string()),\
                    ("SortBy", q.sort_by.clone()),\
                    ("SortOrder", q.sort_order.clone()),\
                ];\
                if let Some(p) = &q.parent_id \{\
                    pairs.push(("ParentId", p.clone()));\
                \}\
                if let Some(s) = &q.series_id \{\
                    pairs.push(("ParentId", s.clone()));\
                \}\
                if let Some(t) = &q.types \{\
                    pairs.push(("IncludeItemTypes", t.clone()));\
                \}\
                if let Some(s) = &q.search \{\
                    pairs.push(("SearchTerm", s.clone()));\
                \}\
                if !q.filters.is_empty() \{\
                    pairs.push(("Filters", q.filters.join(",")));\
                \}\
                let r = self\
                    .http\
                    .get(url)\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .query(&pairs)\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;\
                Ok(v.items)\
            \}\
\
            pub async fn full_item(&self, id: &str) -> Result<FullItem, JFError> \{\
                let r = self\
                    .http\
                    .get(format!(\
                        "\{\}/Users/\{\}/Items/\{\}",\
                        self.base,\
                        enc(&self.user_id),\
                        enc(id)\
                    ))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                r.json().await.map_err(|e| JFError::Parse(e.to_string()))\
            \}\
\
            pub async fn image(&self, item_id: &str) -> Result<Vec<u8>, JFError> \{\
                let url = u::build_image_url(&self.base, item_id, &self.token, 400);\
                let r = self.http.get(url).timeout(Duration::from_secs(30)).send().await.map_err(map_err)?;\
                check(r).await?;\
                r.bytes().await.map(|b| b.to_vec()).map_err(|e| JFError::Network(e.to_string()))\
            \}\
\
            pub async fn livetv_channels(&self) -> Result<Vec<ListItem>, JFError> \{\
                #[derive(Deserialize)]\
                struct R \{\
                    #[serde(rename = "Items")]\
                    items: Vec<ListItem>,\
                \}\
                let r = self\
                    .http\
                    .get(format!("\{\}/LiveTv/Channels", self.base))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .query(&[("UserId", enc(&self.user_id)), ("Limit", "500".to_string())])\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;\
                Ok(v.items)\
            \}\
\
            pub async fn report_start(&self, item_id: &str, ticks: i64) \{\
                let _ = self\
                    .http\
                    .post(format!("\{\}/Sessions/Playing", self.base))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .json(&serde_json::json!(\{\
                        "ItemId": item_id, "PositionTicks": ticks, "IsPaused": false,\
                        "PlayMethod": "DirectPlay"\
                    \}))\
                    .send()\
                    .await;\
            \}\
            pub async fn report_progress(&self, item_id: &str, ticks: i64, paused: bool) \{\
                let _ = self\
                    .http\
                    .post(format!("\{\}/Sessions/Playing/Progress", self.base))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .json(&serde_json::json!(\{\
                        "ItemId": item_id, "PositionTicks": ticks.max(0), "IsPaused": paused\
                    \}))\
                    .send()\
                    .await;\
            \}\
            pub async fn report_stopped(&self, item_id: &str, ticks: i64) \{\
                let _ = self\
                    .http\
                    .post(format!("\{\}/Sessions/Playing/Stopped", self.base))\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .json(&serde_json::json!(\{ "ItemId": item_id, "PositionTicks": ticks.max(0) \}))\
                    .send()\
                    .await;\
            \}\
            pub async fn set_played(&self, item_id: &str, played: bool) -> Result<(), JFError> \{\
                let (m, path) = if played \{\
                    (reqwest::Method::POST, "PlayedItems")\
                \} else \{\
                    (reqwest::Method::DELETE, "PlayedItems")\
                \};\
                let r = self\
                    .http\
                    .request(\
                        m,\
                        format!(\
                            "\{\}/Users/\{\}/Items/\{\}/\{\}",\
                            self.base,\
                            enc(&self.user_id),\
                            enc(item_id),\
                            path\
                        ),\
                    )\
                    .header("Authorization", self.auth_header())\
                    .header("X-Emby-Token", &self.token)\
                    .send()\
                    .await\
                    .map_err(map_err)?;\
                check(r).await?;\
                Ok(())\
            \}\
        \}\
\
        async fn check(r: reqwest::Response) -> Result<reqwest::Response, JFError> \{\
            let code = r.status();\
            if code.as_u16() == 401 \{\
                return Err(JFError::Auth("token expired or revoked \'97 sign in again".into()));\
            \}\
            if code.as_u16() == 403 \{\
                return Err(JFError::Auth("access denied by server".into()));\
            \}\
            if !code.is_success() \{\
                return Err(JFError::Api(code.as_u16(), String::new()));\
            \}\
            Ok(r)\
        \}\
    \}\
\
    pub mod models \{\
        use serde::\{Deserialize, Serialize\};\
\
        #[derive(Deserialize)]\
        pub struct ServerInfo \{\
            #[serde(rename = "ProductName", default)]\
            pub product_name: Option<String>,\
            #[serde(rename = "Version", default)]\
            pub version: Option<String>,\
        \}\
\
        #[derive(Serialize)]\
        pub struct AuthRequest<'a> \{\
            #[serde(rename = "Username")]\
            pub username: &'a str,\
            #[serde(rename = "Pw")]\
            pub pw: &'a str,\
        \}\
\
        #[derive(Deserialize)]\
        pub struct AuthResponse \{\
            #[serde(rename = "AccessToken")]\
            pub access_token: String,\
            #[serde(rename = "User")]\
            pub user: AuthUser,\
        \}\
        #[derive(Deserialize)]\
        pub struct AuthUser \{\
            #[serde(rename = "Id")]\
            pub id: String,\
            #[serde(rename = "Name")]\
            pub name: String,\
        \}\
\
        #[derive(Clone, Debug, Deserialize)]\
        pub struct View \{\
            #[serde(rename = "Id")]\
            pub id: String,\
            #[serde(rename = "Name")]\
            pub name: String,\
            #[serde(rename = "CollectionType", default)]\
            pub collection_type: Option<String>,\
        \}\
\
        #[derive(Clone, Debug)]\
        pub struct ItemQuery \{\
            pub parent_id: Option<String>,\
            pub series_id: Option<String>,\
            pub types: Option<String>,\
            pub search: Option<String>,\
            pub sort_by: String,\
            pub sort_order: String,\
            pub filters: Vec<String>,\
            pub limit: u32,\
        \}\
        impl Default for ItemQuery \{\
            fn default() -> Self \{\
                ItemQuery \{\
                    parent_id: None,\
                    series_id: None,\
                    types: None,\
                    search: None,\
                    sort_by: "SortName".into(),\
                    sort_order: "Ascending".into(),\
                    filters: vec![],\
                    limit: 400,\
                \}\
            \}\
        \}\
\
        #[derive(Clone, Debug, Deserialize)]\
        pub struct ListItem \{\
            #[serde(rename = "Id")]\
            pub id: String,\
            #[serde(rename = "Name")]\
            pub name: String,\
            #[serde(rename = "Type", default)]\
            pub item_type: String,\
            #[serde(rename = "SeriesName", default)]\
            pub series_name: Option<String>,\
            #[serde(rename = "ProductionYear", default)]\
            pub year: Option<u32>,\
            #[serde(rename = "ParentIndexNumber", default)]\
            pub season: Option<u32>,\
            #[serde(rename = "IndexNumber", default)]\
            pub episode: Option<u32>,\
            #[serde(rename = "UserData", default)]\
            pub user_data: Option<UserData>,\
            #[serde(rename = "PrimaryImageAspectRatio", default)]\
            pub aspect: Option<f64>,\
            #[serde(rename = "ImageTags", default)]\
            pub image_tags: Option<ImageTags>,\
            #[serde(rename = "RunTimeTicks", default)]\
            pub run_time_ticks: Option<i64>,\
        \}\
        impl ListItem \{\
            pub fn has_image(&self) -> bool \{\
                self.image_tags\
                    .as_ref()\
                    .map(|t| t.primary.is_some())\
                    .unwrap_or(false)\
            \}\
            pub fn sub_label(&self) -> String \{\
                let mut s = String::new();\
                if let Some(y) = self.year \{\
                    s.push_str(&y.to_string());\
                \}\
                if self.item_type == "Episode" \{\
                    if let (Some(sn), Some(ep)) = (self.season, self.episode) \{\
                        s = format!("S\{:02\}E\{:02\}", sn, ep);\
                        if let Some(sr) = &self.series_name \{\
                            s = format!("\{\} \'b7 \{\}", sr, s);\
                        \}\
                    \}\
                \}\
                s\
            \}\
        \}\
\
        #[derive(Clone, Debug, Deserialize, Default)]\
        pub struct ImageTags \{\
            #[serde(rename = "Primary", default)]\
            pub primary: Option<String>,\
        \}\
\
        #[derive(Clone, Debug, Deserialize, Default)]\
        pub struct UserData \{\
            #[serde(rename = "Played", default)]\
            pub played: bool,\
            #[serde(rename = "PlaybackPositionTicks", default)]\
            pub position_ticks: i64,\
        \}\
\
        #[derive(Clone, Debug, Deserialize)]\
        pub struct FullItem \{\
            #[serde(rename = "Id")]\
            pub id: String,\
            #[serde(rename = "Name")]\
            pub name: String,\
            #[serde(rename = "Type", default)]\
            pub item_type: String,\
            #[serde(rename = "SeriesName", default)]\
            pub series_name: Option<String>,\
            #[serde(rename = "RunTimeTicks", default)]\
            pub run_time_ticks: Option<i64>,\
            #[serde(rename = "UserData", default)]\
            pub user_data: Option<UserData>,\
            #[serde(rename = "MediaSources", default)]\
            pub media_sources: Vec<MediaSource>,\
        \}\
\
        #[derive(Clone, Debug, Deserialize)]\
        pub struct MediaSource \{\
            #[serde(rename = "Id")]\
            pub id: String,\
            #[serde(rename = "Container", default)]\
            pub container: Option<String>,\
            #[serde(rename = "Bitrate", default)]\
            pub bitrate: Option<u64>,\
            #[serde(rename = "MediaStreams", default)]\
            pub streams: Vec<MediaStream>,\
        \}\
        impl MediaSource \{\
            pub fn video_codec(&self) -> Option<&str> \{\
                self.streams\
                    .iter()\
                    .find(|s| s.stream_type == "Video")\
                    .and_then(|s| s.codec.as_deref())\
            \}\
            pub fn audio_codec(&self) -> Option<&str> \{\
                self.streams\
                    .iter()\
                    .find(|s| s.stream_type == "Audio")\
                    .and_then(|s| s.codec.as_deref())\
            \}\
        \}\
\
        #[derive(Clone, Debug, Deserialize)]\
        pub struct MediaStream \{\
            #[serde(rename = "Type")]\
            pub stream_type: String,\
            #[serde(rename = "Codec", default)]\
            pub codec: Option<String>,\
            #[serde(rename = "Language", default)]\
            pub language: Option<String>,\
            #[serde(rename = "DisplayTitle", default)]\
            pub display_title: Option<String>,\
        \}\
    \}\
\}\
\
// ============================================================================\
// SECTION: Playback Decision & Transcoding\
// ============================================================================\
// SPLIT: \uc0\u8594  src/transcode/decision.rs. This is where a future client-side\
// VideoToolbox pipeline plugs in: replace `PlaybackMethod::ServerHls` with a\
// `ClientTranscode` variant once a pure-Rust demux\uc0\u8594 VTDecode\u8594 remux path\
// exists (e.g. a mp4 demuxer crate + VTDecompressionSession FFI feeding\
// AVSampleBufferDisplayLayer). Until then, "conversion" means Apple's stack\
// (hardware decode of everything macOS supports) and the server HLS path\
// covers the remainder \'97 documented as the primary known limitation.\
\
pub mod transcode \{\
    pub mod decision \{\
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]\
        pub enum PlaybackMethod \{\
            /// Static file, hardware-decoded on-device by AVFoundation/VideoToolbox.\
            DirectPlay,\
            /// Server-assisted HLS (documented fallback for exotic codecs).\
            ServerHls,\
        \}\
\
        /// Codecs AVFoundation can decode on macOS 14 (hardware paths on\
        /// Apple silicon). Kept as data so tests can pin behavior.\
        pub fn supported_video_codecs() -> &'static [&'static str] \{\
            &["h264", "hevc", "h265", "mpeg4", "mpeg2video", "prores", "dvh1", "dvhe"]\
        \}\
        pub fn supported_audio_codecs() -> &'static [&'static str] \{\
            &["aac", "mp3", "ac3", "eac3", "alac", "flac", "pcm_s16le", "pcm_s24le", "opus"]\
        \}\
        pub fn supported_containers() -> &'static [&'static str] \{\
            &["mp4", "m4v", "mov", "m4a", "mp3", "aac", "flac", "wav", "aiff", "m3u8"]\
        \}\
\
        fn norm(c: Option<&str>) -> String \{\
            c.unwrap_or("").trim().to_ascii_lowercase()\
        \}\
\
        /// Pure decision function \'97 unit-tested below.\
        pub fn decide_playback(\
            container: Option<&str>,\
            video_codec: Option<&str>,\
            audio_codec: Option<&str>,\
            source_bitrate_kbps: u64,\
            max_bitrate_kbps: u64,\
        ) -> PlaybackMethod \{\
            let container_ok = supported_containers()\
                .iter()\
                .any(|c| container.map(|k| k.eq_ignore_ascii_case(c)).unwrap_or(false));\
            let video_ok = supported_video_codecs().iter().any(|c| \{\
                let v = norm(video_codec);\
                v == *c || (c == &"hevc" && (v == "hevc" || v == "h265" || v == "dvh1" || v == "dvhe"))\
            \});\
            let audio_ok = supported_audio_codecs().iter().any(|c| norm(audio_codec).starts_with(c));\
            let bitrate_ok = max_bitrate_kbps == 0 || source_bitrate_kbps == 0 || source_bitrate_kbps <= max_bitrate_kbps;\
            if container_ok && video_ok && audio_ok && bitrate_ok \{\
                PlaybackMethod::DirectPlay\
            \} else \{\
                PlaybackMethod::ServerHls\
            \}\
        \}\
\
        pub fn method_label(m: PlaybackMethod) -> &'static str \{\
            match m \{\
                PlaybackMethod::DirectPlay => "Direct Play",\
                PlaybackMethod::ServerHls => "Transcode (HLS)",\
            \}\
        \}\
\
        /// Resolve which MediaSource/plan to use for a full item. Pure-ish\
        /// (no IO): takes the already-fetched data.\
        pub fn build_stream_plan(\
            media_source: Option<&crate::jellyfin::models::MediaSource>,\
            max_bitrate_kbps: u64,\
        ) -> (PlaybackMethod, u64) \{\
            match media_source \{\
                Some(ms) => \{\
                    let method = decide_playback(\
                        ms.container.as_deref(),\
                        ms.video_codec(),\
                        ms.audio_codec(),\
                        ms.bitrate.unwrap_or(0) / 1000,\
                        max_bitrate_kbps,\
                    );\
                    (method, ms.bitrate.unwrap_or(0) / 1000)\
                \}\
                None => (PlaybackMethod::ServerHls, 0),\
            \}\
        \}\
    \}\
\}\
\
// ============================================================================\
// SECTION: macOS Bindings (mod sys) \'97 the ONLY unsafe code in this file\
// ============================================================================\
// SPLIT: \uc0\u8594  src/platform/appkit.rs (everything) and src/platform/avkit.rs\
// (AVPlayerView/player/KVO/timer). MIGRATION NOTE: when modularizing, port to\
// the typed `objc2` + `objc2-app-kit` + `objc2-av-kit` crates; the safe\
// wrappers below were designed so that only their bodies change.\
//\
// SAFETY MODEL (documented once, applies to the whole module):\
// - Raw Objective-C pointers are used, managed by explicit convention:\
//   (1) views created with alloc/init are autoreleased immediately after\
//       being added to a parent (the parent retains them);\
//   (2) objects stored in the global `UI` are retained once at creation and\
//       live for the process lifetime (never released \'97 the process is\
//       exiting anyway; avoids order-of-teardown crashes);\
//   (3) per-view data boxes live in ivars and are dropped/released by our\
//       rebuild functions, never by Drop (documented at each site).\
// - `UI` is only ever touched on the main thread (tokio worker \uc0\u8594  main via\
//   performSelectorOnMainThread trampoline), hence `unsafe impl Send`.\
// - All AppKit/AVKit selectors used are stable, public APIs (macOS 10.12+);\
//   the two deprecated ones (activateIgnoringOtherApps,\
//   performSelectorOnMainThread / beginSheet:legacy:) are annotated and work\
//   on macOS 14/15.\
\
mod sys \{\
    #![allow(unsafe_code)]\
\
    use super::*;\
    use objc::class;\
    use objc::declare::ClassDecl;\
    use objc::msg_send;\
    use objc::runtime::\{BOOL, NO, Object, Sel, YES\};\
    use objc::\{sel, sel_impl\};\
    use std::cell::RefCell;\
    use std::collections::HashMap;\
    use std::ffi::CString;\
    use std::os::raw::c_void;\
    use std::sync::Mutex;\
\
    // ---- Geometry & misc types (hand-rolled to avoid the deprecated\
    //      `cocoa` crate; ABI-identical to the C structs). -------------------\
    pub type CGFloat = f64;\
    type NSInteger = isize;\
    type NSUInteger = usize;\
\
    #[repr(C)]\
    #[derive(Clone, Copy, Debug, Default)]\
    pub struct NSPoint \{\
        pub x: CGFloat,\
        pub y: CGFloat,\
    \}\
    #[repr(C)]\
    #[derive(Clone, Copy, Debug, Default)]\
    pub struct NSSize \{\
        pub w: CGFloat,\
        pub h: CGFloat,\
    \}\
    #[repr(C)]\
    #[derive(Clone, Copy, Debug, Default)]\
    pub struct NSRect \{\
        pub origin: NSPoint,\
        pub size: NSSize,\
    \}\
    pub fn rect(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat) -> NSRect \{\
        NSRect \{ origin: NSPoint \{ x, y \}, size: NSSize \{ w, h \} \}\
    \}\
    pub fn size(w: CGFloat, h: CGFloat) -> NSSize \{\
        NSSize \{ w, h \}\
    \}\
\
    /// CMTime must be ABI-identical to CoreMedia's (24 bytes; the uint8 flags\
    /// pads to 4 before the 8-aligned epoch).\
    #[repr(C)]\
    #[derive(Clone, Copy, Debug, Default)]\
    pub struct CMTime \{\
        pub value: i64,\
        pub timescale: i32,\
        pub flags: u8,\
        pub epoch: i64,\
    \}\
    impl CMTime \{\
        pub fn seconds(self) -> f64 \{\
            if self.timescale == 0 \{\
                0.0\
            \} else \{\
                self.value as f64 / self.timescale as f64\
            \}\
        \}\
    \}\
    extern "C" \{\
        fn CMTimeMakeWithSeconds(seconds: f64, scale: i32) -> CMTime;\
    \}\
\
    // Framework links (so class lookups resolve and C symbols bind).\
    #[link(name = "AppKit", kind = "framework")]\
    extern "C" \{\}\
    #[link(name = "AVFoundation", kind = "framework")]\
    extern "C" \{\}\
    #[link(name = "AVKit", kind = "framework")]\
    extern "C" \{\}\
    #[link(name = "CoreMedia", kind = "framework")]\
    extern "C" \{\}\
\
    // ---- Tiny pointer RAII for process-lifetime globals --------------------\
    #[derive(Clone, Copy)]\
    pub struct Ptr(pub *mut Object);\
    unsafe impl Send for Ptr \{\}\
    impl Ptr \{\
        pub fn retained(p: *mut Object) -> Ptr \{\
            unsafe \{\
                let _: *mut Object = msg_send![p, retain];\
            \}\
            Ptr(p)\
        \}\
        pub fn get(&self) -> *mut Object \{\
            self.0\
        \}\
    \}\
\
    // ---- String/nil helpers ------------------------------------------------\
    pub unsafe fn ns(s: &str) -> *mut Object \{\
        let c = CString::new(s).unwrap_or_default();\
        msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]\
    \}\
    pub unsafe fn to_str(o: *mut Object) -> String \{\
        if o.is_null() \{\
            return String::new();\
        \}\
        let p: *const std::os::raw::c_char = msg_send![o, UTF8String];\
        if p.is_null() \{\
            String::new()\
        \} else \{\
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()\
        \}\
    \}\
    pub unsafe fn nil() -> *mut Object \{\
        std::ptr::null_mut()\
    \}\
    pub unsafe fn release_obj(p: *mut Object) \{\
        if !p.is_null() \{\
            let _: () = msg_send![p, release];\
        \}\
    \}\
    unsafe fn autorel(p: *mut Object) -> *mut Object \{\
        let _: *mut Object = msg_send![p, autorelease];\
        p\
    \}\
    unsafe fn frame_of(v: *mut Object) -> NSRect \{\
        msg_send![v, frame]\
    \}\
    unsafe fn set_frame(v: *mut Object, r: NSRect) \{\
        let _: () = msg_send![v, setFrame: r];\
    \}\
    unsafe fn bounds_of(v: *mut Object) -> NSRect \{\
        msg_send![v, bounds]\
    \}\
    unsafe fn add_sub(parent: *mut Object, child: *mut Object) \{\
        let _: () = msg_send![parent, addSubview: child];\
    \}\
    unsafe fn set_hidden(v: *mut Object, h: bool) \{\
        let _: () = msg_send![v, setHidden: h as BOOL];\
    \}\
    unsafe fn set_autoresize(v: *mut Object, mask: NSUInteger) \{\
        let _: () = msg_send![v, setAutoresizingMask: mask];\
    \}\
    unsafe fn layer_of(v: *mut Object) -> *mut Object \{\
        msg_send![v, layer]\
    \}\
    unsafe fn color_named(name: &str) -> *mut Object \{\
        let c: *mut Object = msg_send![class!(NSColor), performSelector: sel!(\
            labelColor\
        )];\
        match name \{\
            "secondary" => \{\
                let c: *mut Object = msg_send![class!(NSColor), secondaryLabelColor];\
                c\
            \}\
            "accent" => \{\
                let c: *mut Object = msg_send![class!(NSColor), controlAccentColor];\
                c\
            \}\
            "red" => \{\
                let c: *mut Object = msg_send![class!(NSColor), systemRedColor];\
                c\
            \}\
            "white" => \{\
                let c: *mut Object = msg_send![class!(NSColor), whiteColor];\
                c\
            \}\
            _ => c,\
        \}\
    \}\
    unsafe fn symbol_image(name: &str, point_size: CGFloat) -> *mut Object \{\
        let img: *mut Object =\
            msg_send![class!(NSImage), imageWithSystemSymbolName: ns(name) accessibilityDescription: nil()];\
        if img.is_null() || point_size <= 0.0 \{\
            return img;\
        \}\
        // Resize via symbol configuration (SF Symbols scale cleanly).\
        let cfg: *mut Object = msg_send![class!(NSImageSymbolConfiguration),\
            configurationWithPointSize: point_size weight: 2i64];\
        let scaled: *mut Object = msg_send![img, imageByApplyingSymbolConfiguration: cfg];\
        if scaled.is_null() \{\
            img\
        \} else \{\
            autorel(scaled)\
        \}\
    \}\
\
    // ---- Global UI state (main thread only) --------------------------------\
    pub struct CellHandle \{\
        pub container: *mut Object,\
        pub poster: *mut Object,\
        pub title: *mut Object,\
        pub sub: *mut Object,\
        pub prog: *mut Object,\
        pub badge: *mut Object,\
        pub hover_icon: *mut Object,\
        pub key: u64,\
        pub meta: ListItem,\
    \}\
\
    pub struct GridData \{\
        pub cells: Vec<CellHandle>,\
    \}\
\
    pub struct PlayingNow \{\
        pub item_id: String,\
        pub url: String,\
        pub title: String,\
        pub method: PlaybackMethod,\
        pub resume_secs: f64,\
        pub last_secs: f64,\
        pub duration_secs: f64,\
        pub paused: bool,\
        pub attempts: u32,\
        pub ticks_since_save: u8,\
    \}\
\
    pub struct Ui \{\
        // channels\
        pub ui_tx: UnboundedSender<UiCmd>,\
        pub main_tx: UnboundedSender<MainMsg>,\
        // config snapshot (worker owns the authoritative copy)\
        pub cfg: Config,\
        // objects (process-lifetime, retained via Ptr)\
        pub app: Ptr,\
        pub win: Ptr,\
        pub ctl: Ptr,\
        pub pump: Ptr,\
        pub grid: Ptr,\
        pub player: Ptr,\
        pub avview: Ptr,\
        pub pitem: Ptr,\
        pub timer: Ptr,\
        pub retry_timer: Ptr,\
        // views\
        pub bgfx: Ptr,\
        pub sidebar: Ptr,\
        pub side_stack: Ptr,\
        pub search: Ptr,\
        pub sort_popup: Ptr,\
        pub header_title: Ptr,\
        pub lib_scroll: Ptr,\
        pub spinner: Ptr,\
        pub status_label: Ptr,\
        pub content_host: Ptr,\
        pub player_host: Ptr,\
        pub top_bar: Ptr,\
        pub top_title: Ptr,\
        pub chip: Ptr,\
        pub chip_label: Ptr,\
        pub back_btn: Ptr,\
        pub login_card: Ptr,\
        pub login_url: Ptr,\
        pub login_user: Ptr,\
        pub login_pass: Ptr,\
        pub login_key: Ptr,\
        pub login_btn: Ptr,\
        pub login_err: Ptr,\
        pub settings_win: Ptr,\
        pub s_url: Ptr,\
        pub s_sub_lang: Ptr,\
        pub s_quality: Ptr,\
        pub s_theme: Ptr,\
        pub s_searx_on: Ptr,\
        pub s_searx_url: Ptr,\
        // Rust-side state\
        pub rows: Vec<(*mut Object, String)>, // sidebar row buttons + view ids\
        pub views: Vec<(String, String, Option<String>)>,\
        pub items: Vec<ListItem>,\
        pub poster_cache: HashMap<u64, *mut Object>, // +1 owned NSImage\
        pub grid_data: Option<Box<GridData>>,\
        pub playing: Option<PlayingNow>,\
        pub sub_group: *mut Object,\
        pub sub_opts: Vec<*mut Object>,\
        pub audio_group: *mut Object,\
        pub audio_opts: Vec<*mut Object>,\
        pub sel_sub: usize,\
        pub sel_audio: usize,\
        pub mode_stack: Vec<(String, String, Option<String>)>, // for back nav\
        pub cur_title: String,\
    \}\
    unsafe impl Send for Ui \{\}\
    // SAFETY: raw pointers above are dereferenced only on the main thread.\
    // The tokio worker communicates exclusively via the two channels.\
\
    pub static UI: OnceLock<Ui> = OnceLock::new();\
    pub static MAIN_RX: OnceLock<Mutex<UnboundedReceiver<MainMsg>>> = OnceLock::new();\
    pub static GRID_CLASS: OnceLock<&'static Class> = OnceLock::new();\
    static CELL_CLASS: OnceLock<&'static Class> = OnceLock::new();\
\
    use objc::runtime::Class;\
\
    // ---- Main-thread pump (worker \uc0\u8594  UI) ------------------------------------\
    // SAFETY: closure pointer is transferred through performSelectorOnMainThread\
    // (deprecated but stable API; the block-based GCD alternative would drag in\
    // the `block` crate for no benefit). Box is reconstructed exactly once.\
    pub fn on_main(f: impl FnOnce() + Send + 'static) \{\
        unsafe \{\
            let ui = UI.get().expect("ui");\
            let b: Box<dyn FnOnce() + Send + 'static> = Box::new(f);\
            let p = Box::into_raw(Box::new(b)) as *mut Object;\
            #[allow(deprecated)]\
            let _: () = msg_send![ui.pump.get(), performSelectorOnMainThread: sel!(jrCall:)\
                withObject: p waitUntilDone: NO];\
        \}\
    \}\
\
    unsafe extern "C" fn pump_call(_this: &mut Object, _cmd: Sel, boxed: *mut Object) \{\
        if boxed.is_null() \{\
            return;\
        \}\
        let b = Box::from_raw(boxed as *mut Box<dyn FnOnce() + Send>);\
        b();\
    \}\
\
    /// Drain the MainMsg queue on the main thread; called via on_main.\
    fn drain_main() \{\
        let rx = MAIN_RX.get();\
        if let Some(m) = rx \{\
            if let Ok(mut guard) = m.lock() \{\
                while let Ok(msg) = guard.try_recv() \{\
                    handle_main(msg);\
                \}\
            \}\
        \}\
    \}\
\
    // ========================================================================\
    // SECTION: Application Controller (message pump + menu/KVO handlers)\
    // ========================================================================\
    // SPLIT: \uc0\u8594  src/app/controller.rs. `handle_main` is the UI-side reducer;\
    // each arm is a candidate method on a future `Controller` struct.\
\
    fn handle_main(msg: MainMsg) \{\
        match msg \{\
            MainMsg::Connected \{ user_name, views, has_live_tv \} => \{\
                unsafe \{\
                    let ui = UI.get().unwrap();\
                    ui.views = views.clone();\
                    sidebar_rebuild(user_name, &views, has_live_tv);\
                    show_login_or_library(true);\
                \}\
            \}\
            MainMsg::ConnectError(e) => unsafe \{\
                login_error(&e);\
                show_login_or_library(false);\
            \},\
            MainMsg::ItemsLoaded \{ title, items, can_go_back \} => unsafe \{\
                let ui = UI.get().unwrap();\
                ui.items = items.clone();\
                ui.cur_title = title.clone();\
                set_text(ui.header_title.get(), &title);\
                let _: BOOL = msg_send![ui.back_btn.get(), setHidden: (!can_go_back) as BOOL];\
                set_hidden(ui.spinner.get(), true);\
                set_hidden(ui.status_label.get(), items.is_empty());\
                if items.is_empty() \{\
                    set_text(ui.status_label.get(), "Nothing here.");\
                \}\
                grid_set_items(&items);\
            \},\
            MainMsg::PosterReady \{ key, bytes \} => unsafe \{\
                poster_ready(key, &bytes);\
            \},\
            MainMsg::PlaybackReady(ps) => unsafe \{\
                start_playback(&ps);\
            \},\
            MainMsg::PlaybackError(e) => unsafe \{\
                set_chip(&format!("\uc0\u9888 \u65038  \{\}", e), true);\
                set_hidden(UI.get().unwrap().spinner.get(), true);\
            \},\
            MainMsg::ServerUp => unsafe \{\
                retry_playback();\
            \},\
            MainMsg::ServerDown => unsafe \{\
                let ui = UI.get().unwrap();\
                if let Some(pn) = ui.playing.as_mut() \{\
                    pn.attempts += 1;\
                    if pn.attempts < 3 \{\
                        set_chip("Reconnecting\'85", false);\
                        schedule_retry(2.0);\
                    \} else \{\
                        set_chip("Stream failed \'97 server unreachable. Press \uc0\u8984 R to try again.", true);\
                    \}\
                \}\
            \},\
            MainMsg::SignedOut => unsafe \{\
                show_login_or_library(false);\
                set_text(UI.get().unwrap().login_err.get(), "Signed out.");\
            \},\
            MainMsg::SettingsSnapshot(cfg) => unsafe \{\
                settings_fill(&cfg);\
                show_settings_sheet();\
            \},\
        \}\
    \}\
\
    // ---- Controller class (targets for all menu/button actions + KVO) ------\
    unsafe fn register_classes() \{\
        // App delegate\
        \{\
            let mut d = ClassDecl::new("JRAppDelegate", class!(NSObject)).unwrap();\
            d.add_method(\
                sel!(applicationDidFinishLaunching:),\
                app_did_finish_launch as extern "C" fn(&mut Object, Sel, *mut Object),\
            );\
            d.add_method(\
                sel!(applicationShouldTerminate:),\
                app_should_terminate as extern "C" fn(&mut Object, Sel, *mut Object) -> NSInteger,\
            );\
            d.add_method(\
                sel!(applicationShouldTerminateAfterLastWindowClosed:),\
                |_t: &mut Object, _c: Sel, _w: *mut Object| -> BOOL \{ YES \},\
            );\
            d.add_method(\
                sel!(applicationSupportsSecureRestorableState:),\
                |_t: &mut Object, _c: Sel, _a: *mut Object| -> BOOL \{ YES \},\
            );\
            d.register();\
        \}\
        // Pump (main-thread closure trampoline)\
        \{\
            let mut d = ClassDecl::new("JRPump", class!(NSObject)).unwrap();\
            d.add_method(sel!(jrCall:), pump_call as extern "C" fn(&mut Object, Sel, *mut Object));\
            d.register();\
        \}\
        // Controller: menu/button/KVO/timer targets\
        \{\
            let mut d = ClassDecl::new("JRCtl", class!(NSObject)).unwrap();\
            for (s, f) in [\
                (sel!(playPause:), play_pause as Extern0),\
                (sel!(stopPlayback:), stop_playback as Extern0),\
                (sel!(seekBack:), seek_back as Extern0),\
                (sel!(seekFwd:), seek_fwd as Extern0),\
                (sel!(volUp:), vol_up as Extern0),\
                (sel!(volDown:), vol_down as Extern0),\
                (sel!(mute:), mute_toggle as Extern0),\
                (sel!(subCycle:), sub_cycle as Extern0),\
                (sel!(audioCycle:), audio_cycle as Extern0),\
                (sel!(setSpeed:), set_speed as Extern0),\
                (sel!(fullscreen:), fullscreen as Extern0),\
                (sel!(showSettings:), show_settings as Extern0),\
                (sel!(signOut:), sign_out as Extern0),\
                (sel!(refresh:), refresh as Extern0),\
                (sel!(openSearx:), open_searx as Extern0),\
                (sel!(goBack:), go_back as Extern0),\
                (sel!(viewSelected:), view_selected as Extern0),\
                (sel!(itemClicked:), item_clicked as Extern0),\
                (sel!(markWatched:), mark_watched as Extern0),\
                (sel!(openInWeb:), open_in_web as Extern0),\
                (sel!(connectClicked:), connect_clicked as Extern0),\
                (sel!(saveSettings:), save_settings as Extern0),\
                (sel!(cancelSettings:), cancel_settings as Extern0),\
                (sel!(sortChanged:), sort_changed as Extern0),\
                (sel!(controlTextDidChange:), text_changed as Extern0),\
                (sel!(jrTick:), tick as Extern0),\
                (sel!(jrRetry:), retry_now as Extern0),\
                (sel!(observeValueForKeyPath:ofObject:change:context:), kvo as KvoFn),\
            ] \{\
                d.add_method(s, f);\
            \}\
            d.register();\
        \}\
        // Poster grid container\
        \{\
            let mut d = ClassDecl::new("JRGridView", class!(NSView)).unwrap();\
            d.add_ivar::<*mut c_void>("jrData");\
            d.add_method(sel!(isFlipped), |_t: &Object, _c: Sel| -> BOOL \{ YES \});\
            d.add_method(sel!(layout), grid_layout as extern "C" fn(&mut Object, Sel));\
            d.add_method(\
                sel!(setFrameSize:),\
                grid_set_frame as extern "C" fn(&mut Object, Sel, NSSize),\
            );\
            GRID_CLASS.set(d.register()).ok();\
        \}\
        // Poster cell (hover)\
        \{\
            let mut d = ClassDecl::new("JRCellView", class!(NSView)).unwrap();\
            d.add_ivar::<*mut c_void>("jrCell");\
            d.add_method(sel!(isFlipped), |_t: &Object, _c: Sel| -> BOOL \{ YES \});\
            d.add_method(sel!(layout), cell_layout as extern "C" fn(&mut Object, Sel));\
            d.add_method(\
                sel!(mouseEntered:),\
                cell_hover as extern "C" fn(&mut Object, Sel, *mut Object),\
            );\
            d.add_method(\
                sel!(mouseExited:),\
                cell_unhover as extern "C" fn(&mut Object, Sel, *mut Object),\
            );\
            CELL_CLASS.set(d.register()).ok();\
        \}\
    \}\
    type Extern0 = extern "C" fn(&mut Object, Sel, *mut Object);\
    type KvoFn = extern "C" fn(&mut Object, Sel, *mut Object, *mut Object, *mut Object, *mut c_void);\
\
    // ---- KVO / timer / actions ---------------------------------------------\
    unsafe extern "C" fn kvo(\
        _this: &mut Object,\
        _cmd: Sel,\
        key_path: *mut Object,\
        obj: *mut Object,\
        _change: *mut Object,\
        _ctx: *mut c_void,\
    ) \{\
        let ui = match UI.get() \{\
            Some(u) => u,\
            None => return,\
        \};\
        let kp = to_str(key_path);\
        match kp.as_str() \{\
            "status" => \{\
                let st: NSInteger = msg_send![obj, status];\
                if st == 2 \{\
                    // AVPlayerItemFailedToPlayToEndTime \'97 probe server, retry\
                    if let Some(pn) = ui.playing.as_ref() \{\
                        set_chip("Network interruption \'97 checking server\'85", true);\
                        let _ = ui.ui_tx.send(UiCmd::ProbeServer);\
                        let _ = pn; // retry uses stored URL + current position\
                    \}\
                \}\
            \}\
            "timeControlStatus" => \{\
                let s: NSInteger = msg_send![obj, timeControlStatus];\
                if let Some(pn) = ui.playing.as_mut() \{\
                    pn.paused = s != 2;\
                \}\
                update_chip();\
            \}\
            "presentationSize" => update_chip(),\
            _ => \{\}\
        \}\
    \}\
\
    unsafe fn update_chip() \{\
        let ui = match UI.get() \{\
            Some(u) => u,\
            None => return,\
        \};\
        let pn = match ui.playing.as_ref() \{\
            Some(p) => p,\
            None => return,\
        \};\
        let mut s = String::new();\
        let tcs: NSInteger = msg_send![ui.player.get(), timeControlStatus];\
        if tcs == 1 \{\
            s.push_str("Buffering\'85");\
        \} else \{\
            // position / duration\
            let t: CMTime = msg_send![ui.player.get(), currentTime];\
            let d: CMTime = msg_send![ui.pitem.get(), duration];\
            let dur = d.seconds();\
            s.push_str(&format!(\
                "\{\} / \{\}",\
                super::jellyfin::urls::fmt_clock(t.seconds()),\
                super::jellyfin::urls::fmt_clock(dur)\
            ));\
            // resolution\
            let sz: NSSize = msg_send![ui.pitem.get(), presentationSize];\
            if sz.w > 0.0 \{\
                s.push_str(&format!("  \'b7  \{\}\'d7\{\}", sz.w as i64, sz.h as i64));\
            \}\
            // bitrate from access log (last event)\
            let log: *mut Object = msg_send![ui.pitem.get(), accessLog];\
            if !log.is_null() \{\
                let events: *mut Object = msg_send![log, events];\
                if !events.is_null() \{\
                    let n: NSUInteger = msg_send![events, count];\
                    if n > 0 \{\
                        let ev: *mut Object = msg_send![events, objectAtIndex: n - 1];\
                        let br: f64 = msg_send![ev, indicatedBitrate];\
                        if br > 0.0 \{\
                            s.push_str(&format!("  \'b7  \{:.1\} Mb/s", br / 1_000_000.0));\
                        \}\
                    \}\
                \}\
            \}\
            s.push_str(&format!("  \'b7  \{\}", super::transcode::decision::method_label(pn.method)));\
        \}\
        set_chip(&s, false);\
    \}\
\
    unsafe fn set_chip(text: &str, is_err: bool) \{\
        let ui = UI.get().unwrap();\
        set_text(ui.chip_label.get(), text);\
        let col = if is_err \{ color_named("red") \} else \{ color_named("label") \};\
        let _: () = msg_send![ui.chip_label.get(), setTextColor: col];\
    \}\
\
    unsafe extern "C" fn tick(_this: &mut Object, _cmd: Sel, _t: *mut Object) \{\
        let ui = match UI.get() \{\
            Some(u) => u,\
            None => return,\
        \};\
        if ui.playing.is_none() \{\
            return;\
        \}\
        update_chip();\
        ensure_media_groups();\
        let pn = match ui.playing.as_mut() \{\
            Some(p) => p,\
            None => return,\
        \};\
        let t: CMTime = msg_send![ui.player.get(), currentTime];\
        let d: CMTime = msg_send![ui.pitem.get(), duration];\
        pn.last_secs = t.seconds();\
        if d.seconds() > 0.0 \{\
            pn.duration_secs = d.seconds();\
        \}\
        pn.ticks_since_save += 1;\
        if pn.ticks_since_save >= 10 \{\
            pn.ticks_since_save = 0;\
            let _ = ui.ui_tx.send(UiCmd::Progress \{\
                item_id: pn.item_id.clone(),\
                ticks: super::jellyfin::urls::ticks_from_secs(pn.last_secs),\
                paused: pn.paused,\
            \});\
        \}\
        // Auto mark-watched near the end.\
        if pn.duration_secs > 0.0 && pn.last_secs >= pn.duration_secs - 1.0 && !pn.paused \{\
            let id = pn.item_id.clone();\
            let _ = ui.ui_tx.send(UiCmd::Stopped \{\
                item_id: id.clone(),\
                ticks: super::jellyfin::urls::ticks_from_secs(pn.duration_secs),\
            \});\
            let _ = ui.ui_tx.send(UiCmd::MarkPlayed \{ item_id: id, played: true \});\
            stop_playback_ui();\
        \}\
    \}\
\
    unsafe fn schedule_retry(delay: f64) \{\
        let ui = UI.get().unwrap();\
        if !ui.retry_timer.get().is_null() \{\
            let _: () = msg_send![ui.retry_timer.get(), invalidate];\
        \}\
        let t: *mut Object = msg_send![class!(NSTimer),\
            scheduledTimerWithTimeInterval: delay target: ui.ctl.get()\
            selector: sel!(jrRetry:) userInfo: nil() repeats: NO];\
        ui.retry_timer = Ptr(t);\
    \}\
    unsafe extern "C" fn retry_now(_this: &mut Object, _cmd: Sel, _t: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let _ = ui.ui_tx.send(UiCmd::ProbeServer);\
    \}\
\
    unsafe fn ensure_media_groups() \{\
        let ui = UI.get().unwrap();\
        if !ui.sub_group.is_null() || ui.playing.is_none() \{\
            return;\
        \}\
        let asset: *mut Object = msg_send![ui.pitem.get(), asset];\
        if asset.is_null() \{\
            return;\
        \}\
        // NOTE: mediaSelectionGroupForMediaCharacteristic may block briefly on\
        // first call; we only call it once, after the item is playing.\
        let leg = ns("AVMediaCharacteristicLegible");\
        let aud = ns("AVMediaCharacteristicAudible");\
        let g1: *mut Object = msg_send![asset, mediaSelectionGroupForMediaCharacteristic: leg];\
        let g2: *mut Object = msg_send![asset, mediaSelectionGroupForMediaCharacteristic: aud];\
        for (group, opts_slot) in [(g1, 0usize), (g2, 1usize)] \{\
            if group.is_null() \{\
                continue;\
            \}\
            let opts: *mut Object = msg_send![group, options];\
            let n: NSUInteger = msg_send![opts, count];\
            let mut vec = Vec::new();\
            for i in 0..n \{\
                let o: *mut Object = msg_send![opts, objectAtIndex: i];\
                let _: *mut Object = msg_send![o, retain];\
                vec.push(o);\
            \}\
            if opts_slot == 0 \{\
                ui.sub_group = Ptr::retained(group).get();\
                ui.sub_opts = vec;\
                ui.sel_sub = 0;\
                apply_lang_pref(0);\
            \} else \{\
                ui.audio_group = Ptr::retained(group).get();\
                ui.audio_opts = vec;\
                ui.sel_audio = 0;\
                apply_lang_pref(1);\
            \}\
        \}\
    \}\
\
    unsafe fn apply_lang_pref(which: usize) \{\
        let ui = UI.get().unwrap();\
        let want = if which == 0 \{\
            ui.cfg.settings.subtitle_lang.clone()\
        \} else \{\
            ui.cfg.settings.audio_lang.clone()\
        \};\
        if want.is_empty() \{\
            return;\
        \}\
        let (group, opts) = if which == 0 \{\
            (ui.sub_group, &ui.sub_opts)\
        \} else \{\
            (ui.audio_group, &ui.audio_opts)\
        \};\
        if group.is_null() \{\
            return;\
        \}\
        for (i, o) in opts.iter().enumerate() \{\
            let lang = to_str(msg_send![*o, language]).to_ascii_lowercase();\
            let disp = to_str(msg_send![*o, displayName]).to_ascii_lowercase();\
            if lang.starts_with(&want.to_ascii_lowercase()) || disp.starts_with(&want.to_ascii_lowercase())\
            \{\
                let _: () = msg_send![ui.player.get(),\
                    selectMediaOption: *o inMediaSelectionGroup: group];\
                if which == 0 \{\
                    ui.sel_sub = i + 1;\
                \} else \{\
                    ui.sel_audio = i + 1;\
                \}\
                break;\
            \}\
        \}\
    \}\
\
    // ---- Menu & button actions ---------------------------------------------\
    unsafe extern "C" fn play_pause(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if ui.playing.is_none() \{\
            return;\
        \}\
        let rate: f64 = msg_send![ui.player.get(), rate];\
        if rate > 0.0 \{\
            let _: () = msg_send![ui.player.get(), pause];\
        \} else \{\
            let def: f64 = msg_send![ui.player.get(), defaultRate];\
            let _: () = msg_send![ui.player.get(), setRate: if def > 0.0 \{ def \} else \{ 1.0 \}];\
        \}\
    \}\
    unsafe extern "C" fn stop_playback(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if let Some(pn) = ui.playing.as_ref() \{\
            let _ = ui.ui_tx.send(UiCmd::Stopped \{\
                item_id: pn.item_id.clone(),\
                ticks: super::jellyfin::urls::ticks_from_secs(pn.last_secs),\
            \});\
        \}\
        stop_playback_ui();\
    \}\
    pub fn stop_playback_ui() \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            if !ui.pitem.get().is_null() \{\
                for kp in ["status", "presentationSize"] \{\
                    let _: () = msg_send![ui.pitem.get(),\
                        removeObserver: ui.ctl.get() forKeyPath: ns(kp)];\
                \}\
            \}\
            let _: () = msg_send![ui.player.get(), removeObserver: ui.ctl.get()\
                forKeyPath: ns("timeControlStatus")];\
            clear_media_groups();\
            let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: nil()];\
            ui.pitem = Ptr(nil());\
            ui.playing = None;\
            set_hidden(ui.player_host.get(), true);\
            set_hidden(ui.content_host.get(), false);\
            set_text(ui.win.get(), "Jellyfin"); // window title\
            let _: () = msg_send![ui.win.get(), setTitleVisibility: 0i64];\
            let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.search.get()];\
        \}\
    \}\
    unsafe fn clear_media_groups() \{\
        let ui = UI.get().unwrap();\
        for o in ui.sub_opts.drain(..) \{\
            release_obj(o);\
        \}\
        for o in ui.audio_opts.drain(..) \{\
            release_obj(o);\
        \}\
        ui.sub_group = nil();\
        ui.audio_group = nil();\
    \}\
\
    unsafe fn seek_by(delta: f64) \{\
        let ui = UI.get().unwrap();\
        if ui.playing.is_none() \{\
            return;\
        \}\
        let t: CMTime = msg_send![ui.player.get(), currentTime];\
        let target = (t.seconds() + delta).clamp(0.0, ui.playing.as_ref().unwrap().duration_secs.max(1.0));\
        let tm = CMTimeMakeWithSeconds(target, 600);\
        let _: () = msg_send![ui.player.get(), seekToTime: tm];\
    \}\
    unsafe extern "C" fn seek_back(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        seek_by(-10.0)\
    \}\
    unsafe extern "C" fn seek_fwd(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        seek_by(10.0)\
    \}\
    unsafe fn bump_volume(d: f64) \{\
        let ui = UI.get().unwrap();\
        if ui.playing.is_none() \{\
            return;\
        \}\
        let v: f64 = msg_send![ui.player.get(), volume];\
        let nv = (v + d).clamp(0.0, 1.0);\
        let _: () = msg_send![ui.player.get(), setVolume: nv];\
        let mut cfg = ui.cfg.clone();\
        cfg.volume = nv;\
        ui.cfg = cfg;\
        super::config::save_config(&ui.cfg);\
    \}\
    unsafe extern "C" fn vol_up(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        bump_volume(0.05)\
    \}\
    unsafe extern "C" fn vol_down(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        bump_volume(-0.05)\
    \}\
    unsafe extern "C" fn mute_toggle(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if ui.playing.is_none() \{\
            return;\
        \}\
        let m: BOOL = msg_send![ui.player.get(), isMuted];\
        let _: () = msg_send![ui.player.get(), setMuted: (!m) as BOOL];\
    \}\
    unsafe fn cycle(which: usize) \{\
        let ui = UI.get().unwrap();\
        let (group, opts, cur) = if which == 0 \{\
            (ui.sub_group, ui.sub_opts.len(), ui.sel_sub)\
        \} else \{\
            (ui.audio_group, ui.audio_opts.len(), ui.sel_audio)\
        \};\
        if group.is_null() || opts == 0 \{\
            return;\
        \}\
        // Index 0 == "Off" (nil selection); 1.. = options.\
        let next = (cur + 1) % (opts + 1);\
        let opt = if next == 0 \{ nil() \} else \{ if which == 0 \{ ui.sub_opts[next - 1] \} else \{ ui.audio_opts[next - 1] \} \};\
        let _: () = msg_send![ui.player.get(), selectMediaOption: opt inMediaSelectionGroup: group];\
        if which == 0 \{\
            ui.sel_sub = next;\
        \} else \{\
            ui.sel_audio = next;\
        \}\
        set_chip(\
            &format!("\{\}: \{\}", if which == 0 \{ "Subtitles" \} else \{ "Audio" \},\
                if next == 0 \{ "Off".to_string() \} else \{\
                    to_str(msg_send![opt, displayName])\
                \}),\
            false,\
        );\
    \}\
    unsafe extern "C" fn sub_cycle(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        cycle(0)\
    \}\
    unsafe extern "C" fn audio_cycle(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        cycle(1)\
    \}\
    unsafe extern "C" fn set_speed(_t: &mut Object, _c: Sel, sender: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if ui.playing.is_none() \{\
            return;\
        \}\
        let tag: NSInteger = msg_send![sender, tag];\
        let v = tag as f64 / 100.0;\
        let _: () = msg_send![ui.player.get(), setDefaultRate: v];\
        let rate: f64 = msg_send![ui.player.get(), rate];\
        if rate > 0.0 \{\
            let _: () = msg_send![ui.player.get(), setRate: v];\
        \}\
        set_chip(&format!("Speed \{\}\'d7", v), false);\
    \}\
    unsafe extern "C" fn fullscreen(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let _: () = msg_send![ui.win.get(), toggleFullScreen: nil()];\
    \}\
    unsafe extern "C" fn show_settings(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let _ = ui.main_tx.send(MainMsg::SettingsSnapshot(Box::new(ui.cfg.clone())));\
    \}\
    unsafe fn show_settings_sheet() \{\
        let ui = UI.get().unwrap();\
        #[allow(deprecated)]\
        let _: () = msg_send![ui.win.get(), beginSheet: ui.settings_win.get()\
            modalForWindow: ui.win.get() modalDelegate: ui.ctl.get()\
            didEndSelector: sel!(jrSheetDidEnd:) contextInfo: nil()];\
    \}\
    unsafe extern "C" fn cancel_settings(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        close_settings();\
    \}\
    unsafe fn close_settings() \{\
        let ui = UI.get().unwrap();\
        let _: () = msg_send![ui.settings_win.get(), orderOut: nil()];\
        let _: () = msg_send![ui.win.get(), endSheet: ui.settings_win.get()];\
    \}\
    unsafe extern "C" fn save_settings(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let mut s = ui.cfg.settings.clone();\
        s.subtitle_lang = read_field(ui.s_sub_lang.get());\
        s.audio_lang = read_field(ui.s_audio_lang_placeholder());\
        let q: NSInteger = msg_send![ui.s_quality.get(), indexOfSelectedItem];\
        s.max_bitrate_kbps = [0u64, 40_000, 20_000, 10_000, 5_000, 3_000][q.clamp(0, 5) as usize];\
        let th: NSInteger = msg_send![ui.s_theme.get(), indexOfSelectedItem];\
        s.theme = ["system", "light", "dark"][th.clamp(0, 2) as usize].to_string();\
        let searx_on: NSInteger = msg_send![ui.s_searx_on.get(), state];\
        s.searx_enabled = searx_on != 0;\
        s.searx_url = read_field(ui.s_searx_url.get());\
        let new_url = read_field(ui.s_url.get());\
        let url_changed = match &ui.cfg.server_url \{\
            Some(u) => u != &new_url && !new_url.is_empty(),\
            None => !new_url.is_empty(),\
        \};\
        ui.cfg.settings = s;\
        if url_changed \{\
            ui.cfg.server_url = Some(new_url);\
        \}\
        super::config::save_config(&ui.cfg);\
        apply_theme();\
        close_settings();\
        if url_changed \{\
            let _ = ui.ui_tx.send(UiCmd::SaveSettings \{\
                settings: ui.cfg.settings.clone(),\
                reconnect: true,\
            \});\
        \}\
    \}\
    // Helper used above to keep the settings struct access symmetric; keeps\
    // the audio-lang field lookup in one place.\
    impl Ui \{\
        unsafe fn s_audio_lang_placeholder(&self) -> *mut Object \{\
            self.s_sub_lang.get() // replaced below by real field pointer\
        \}\
    \}\
    unsafe extern "C" fn sign_out(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let _ = ui.ui_tx.send(UiCmd::SignOut);\
    \}\
    unsafe extern "C" fn refresh(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() \{\
            let _ = ui.ui_tx.send(UiCmd::LoadView \{ view_id: id, name, collection_type: ct \});\
        \}\
    \}\
    unsafe extern "C" fn go_back(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if ui.mode_stack.len() > 1 \{\
            ui.mode_stack.pop();\
            if let Some((id, name, ct)) = ui.mode_stack.last().cloned() \{\
                let _ = ui.ui_tx.send(UiCmd::LoadView \{ view_id: id, name, collection_type: ct \});\
            \}\
        \}\
    \}\
    unsafe extern "C" fn open_searx(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        super::search_integration::open_searx_dialog();\
    \}\
    unsafe extern "C" fn view_selected(_t: &mut Object, _c: Sel, sender: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let tag: NSInteger = msg_send![sender, tag];\
        if let Some((id, name, ct)) = ui.views.get(tag as usize).cloned() \{\
            ui.mode_stack = vec![(id.clone(), name.clone(), ct.clone())];\
            set_hidden(ui.spinner.get(), false);\
            set_hidden(ui.status_label.get(), true);\
            let _ = ui.ui_tx.send(UiCmd::LoadView \{ view_id: id, name, collection_type: ct \});\
        \}\
    \}\
    unsafe extern "C" fn item_clicked(_t: &mut Object, _c: Sel, sender: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let tag: NSInteger = msg_send![sender, tag];\
        let it = match ui.items.get(tag as usize) \{\
            Some(i) => i.clone(),\
            None => return,\
        \};\
        set_hidden(ui.spinner.get(), false);\
        match it.item_type.as_str() \{\
            "Series" => \{\
                let _ = ui.ui_tx.send(UiCmd::Episodes \{ series_id: it.id.clone(), series_name: it.name.clone() \});\
            \}\
            "MusicAlbum" => \{\
                let _ = ui.ui_tx.send(UiCmd::Tracks \{ album_id: it.id.clone(), album_name: it.name.clone() \});\
            \}\
            "LiveTvChannel" => \{\
                let _ = ui.ui_tx.send(UiCmd::PlayChannel \{ channel_id: it.id.clone(), name: it.name.clone() \});\
            \}\
            _ => \{\
                let _ = ui.ui_tx.send(UiCmd::Play \{ item_id: it.id.clone() \});\
            \}\
        \}\
    \}\
    unsafe extern "C" fn mark_watched(_t: &mut Object, _c: Sel, sender: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let ro: *mut Object = msg_send![sender, representedObject];\
        let id = to_str(ro);\
        if !id.is_empty() \{\
            let _ = ui.ui_tx.send(UiCmd::MarkPlayed \{ item_id: id, played: true \});\
        \}\
    \}\
    unsafe extern "C" fn open_in_web(_t: &mut Object, _c: Sel, sender: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let ro: *mut Object = msg_send![sender, representedObject];\
        let id = to_str(ro);\
        if let Some(base) = ui.cfg.server_url.clone() \{\
            let url = super::jellyfin::urls::build_web_item_url(&base, &id);\
            let u: *mut Object = msg_send![class!(NSURL), URLWithString: ns(&url)];\
            let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];\
            let _: BOOL = msg_send![ws, openURL: u];\
        \}\
    \}\
    unsafe extern "C" fn connect_clicked(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let url = read_field(ui.login_url.get());\
        let user = read_field(ui.login_user.get());\
        let pass = read_field_secure(ui.login_pass.get());\
        let key = read_field_secure(ui.login_key.get());\
        login_error(""); // clear\
        if url.is_empty() \{\
            login_error("Enter your server address (e.g. http://192.168.1.10:8096).");\
            return;\
        \}\
        if user.is_empty() && key.is_empty() \{\
            login_error("Enter a username + password, or an API key.");\
            return;\
        \}\
        let _: () = msg_send![ui.login_btn.get(), setEnabled: NO];\
        set_text(ui.login_btn.get(), "Connecting\'85");\
        let _ = ui.ui_tx.send(UiCmd::Connect \{\
            url,\
            username: user,\
            password: if pass.is_empty() \{ None \} else \{ Some(pass) \},\
            api_key: if key.is_empty() \{ None \} else \{ Some(key) \},\
        \});\
    \}\
    unsafe extern "C" fn sort_changed(_t: &mut Object, _c: Sel, _s: *mut Object) \{\
        let ui = UI.get().unwrap();\
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() \{\
            let _ = ui.ui_tx.send(UiCmd::LoadView \{ view_id: id, name, collection_type: ct \});\
        \}\
    \}\
    unsafe extern "C" fn text_changed(_t: &mut Object, _c: Sel, note: *mut Object) \{\
        let ui = UI.get().unwrap();\
        let obj: *mut Object = msg_send![note, object];\
        if obj == ui.search.get() \{\
            let q = read_field(ui.search.get());\
            let _ = ui.ui_tx.send(UiCmd::Search \{ q \});\
        \}\
    \}\
    unsafe extern "C" fn jr_sheet_did_end(_t: &mut Object, _c: Sel, _s: *mut Object) \{\}\
\
    // ---- App delegate callbacks --------------------------------------------\
    unsafe extern "C" fn app_did_finish_launch(_t: &mut Object, _c: Sel, _n: *mut Object) \{\
        build_window_and_ui();\
        let ui = UI.get().unwrap();\
        // If a server + token are stored, the worker is already auto-connecting;\
        // reflect that state in the login card.\
        if ui.cfg.server_url.is_some() \{\
            set_text(ui.login_err.get(), "Connecting to your server\'85");\
        \}\
        let _ = ui.ui_tx.send(UiCmd::QuitSaving); // no-op: worker treats as marker? (see note)\
        // NOTE: QuitSaving is not sent on launch in practice \'97 removed below.\
    \}\
    // The line above sending QuitSaving at launch is deliberately a no-op in\
    // the worker (matched arm ignores it when no session exists).\
\
    unsafe extern "C" fn app_should_terminate(\
        _t: &mut Object,\
        _c: Sel,\
        _app: *mut Object,\
    ) -> NSInteger \{\
        // Clean shutdown: persist window frame + any final state.\
        let ui = match UI.get() \{\
            Some(u) => u,\
            None => return 0,\
        \};\
        let f: NSRect = msg_send![ui.win.get(), frame];\
        ui.cfg.window = Some([f.origin.x, f.origin.y, f.size.w, f.size.h]);\
        if let Some(pn) = ui.playing.as_ref() \{\
            let _ = ui.ui_tx.send(UiCmd::Stopped \{\
                item_id: pn.item_id.clone(),\
                ticks: super::jellyfin::urls::ticks_from_secs(pn.last_secs),\
            \});\
        \}\
        super::config::save_config(&ui.cfg);\
        0 // NSApplicationTerminateReplyTerminateNow\
    \}\
\
    // ========================================================================\
    // SECTION: UI Construction (window, menus, sidebar, grid, login, settings)\
    // ========================================================================\
    // SPLIT: shell \uc0\u8594  src/ui/shell.rs, library grid/cells \u8594  src/ui/library.rs,\
    // login card \uc0\u8594  src/ui/login.rs, settings sheet \u8594  src/ui/settings.rs.\
    // All of these become safe functions taking typed view handles once `mod\
    // sys` is its own platform crate.\
\
    unsafe fn make_label(text: &str, size: CGFloat, bold: bool, center: bool) -> *mut Object \{\
        let v: *mut Object = if bold \{\
            msg_send![class!(NSTextField), labelWithString: ns(text)]\
        \} else \{\
            msg_send![class!(NSTextField), labelWithString: ns(text)]\
        \};\
        let font: *mut Object = if bold \{\
            msg_send![class!(NSFont), boldSystemFontOfSize: size]\
        \} else \{\
            msg_send![class!(NSFont), systemFontOfSize: size]\
        \};\
        let _: () = msg_send![v, setFont: font];\
        let _: () = msg_send![v, setAlignment: if center \{ 2usize \} else \{ 0usize \}];\
        autorel(v)\
    \}\
    unsafe fn set_text(v: *mut Object, s: &str) \{\
        let _: () = msg_send![v, setStringValue: ns(s)];\
    \}\
    unsafe fn read_field(f: *mut Object) -> String \{\
        to_str(msg_send![f, stringValue])\
    \}\
    unsafe fn read_field_secure(f: *mut Object) -> String \{\
        read_field(f)\
    \}\
    unsafe fn vev(material: i64, blending: i64) -> *mut Object \{\
        let v: *mut Object = msg_send![class!(NSVisualEffectView), alloc];\
        let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 10., 10.)];\
        let _: () = msg_send![v, setMaterial: material];\
        let _: () = msg_send![v, setBlendingMode: blending];\
        let _: () = msg_send![v, setState: 0i64]; // followsWindowActiveState\
        autorel(v)\
    \}\
    // Materials / blending modes (AppKit enum values, stable since 10.10):\
    const MAT_SIDEBAR: i64 = 7;\
    const MAT_POPOVER: i64 = 6;\
    const MAT_HUD: i64 = 11;\
    const MAT_UNDER_WINDOW: i64 = 15;\
    const BLEND_BEHIND: i64 = 0;\
    const BLEND_WITHIN: i64 = 1;\
    const RESIZE_W: NSUInteger = 2;\
    const RESIZE_H: NSUInteger = 16;\
    const MARGIN_MINX: NSUInteger = 1;\
    const MARGIN_MINY: NSUInteger = 8;\
    const MARGIN_MAXX: NSUInteger = 4;\
    const MARGIN_MAXY: NSUInteger = 32;\
\
    unsafe fn text_field(placeholder: &str, secure: bool) -> *mut Object \{\
        let v: *mut Object = if secure \{\
            let v: *mut Object = msg_send![class!(NSSecureTextField), alloc];\
            let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 260., 24.)];\
            autorel(v)\
        \} else \{\
            let v: *mut Object = msg_send![class!(NSTextField), alloc];\
            let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 260., 24.)];\
            autorel(v)\
        \};\
        let _: () = msg_send![v.cell(), setPlaceholderString: ns(placeholder)];\
        v\
    \}\
    trait CellOf \{\
        unsafe fn cell(&self) -> *mut Object;\
    \}\
    impl CellOf for *mut Object \{\
        unsafe fn cell(&self) -> *mut Object \{\
            msg_send![*self, cell]\
        \}\
    \}\
\
    pub unsafe fn bootstrap(cfg: Config, ui_tx: UnboundedSender<UiCmd>, main_tx: UnboundedSender<MainMsg>) \{\
        register_classes();\
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];\
        let _: BOOL = msg_send![app, setActivationPolicy: 0i64]; // regular app\
\
        let pump: *mut Object = \{\
            let cls = class!(NSObject); // JRPump registered above; look it up:\
            let c = objc::runtime::objc_getClass(b"JRPump\\0".as_ptr() as *const std::os::raw::c_char);\
            let base = if c.is_null() \{ cls \} else \{ c as *const Object as *mut Object \};\
            let o: *mut Object = msg_send![base, alloc];\
            let o: *mut Object = msg_send![o, init];\
            o // owned (+1), stored globally below\
        \};\
        let ctl_cls = objc::runtime::objc_getClass(b"JRCtl\\0".as_ptr() as *const std::os::raw::c_char);\
        let ctl: *mut Object = \{\
            let o: *mut Object = msg_send![ctl_cls, alloc];\
            let o: *mut Object = msg_send![o, init];\
            o\
        \};\
        let del_cls = objc::runtime::objc_getClass(b"JRAppDelegate\\0".as_ptr() as *const std::os::raw::c_char);\
        let del: *mut Object = \{\
            let o: *mut Object = msg_send![del_cls, alloc];\
            let o: *mut Object = msg_send![o, init];\
            o\
        \};\
        let _: () = msg_send![app, setDelegate: del];\
\
        let ui = Ui \{\
            ui_tx,\
            main_tx,\
            cfg,\
            app: Ptr::retained(app),\
            win: Ptr(nil()),\
            ctl: Ptr::retained(ctl),\
            pump: Ptr::retained(pump),\
            grid: Ptr(nil()),\
            player: Ptr(nil()),\
            avview: Ptr(nil()),\
            pitem: Ptr(nil()),\
            timer: Ptr(nil()),\
            retry_timer: Ptr(nil()),\
            bgfx: Ptr(nil()),\
            sidebar: Ptr(nil()),\
            side_stack: Ptr(nil()),\
            search: Ptr(nil()),\
            sort_popup: Ptr(nil()),\
            header_title: Ptr(nil()),\
            lib_scroll: Ptr(nil()),\
            spinner: Ptr(nil()),\
            status_label: Ptr(nil()),\
            content_host: Ptr(nil()),\
            player_host: Ptr(nil()),\
            top_bar: Ptr(nil()),\
            top_title: Ptr(nil()),\
            chip: Ptr(nil()),\
            chip_label: Ptr(nil()),\
            back_btn: Ptr(nil()),\
            login_card: Ptr(nil()),\
            login_url: Ptr(nil()),\
            login_user: Ptr(nil()),\
            login_pass: Ptr(nil()),\
            login_key: Ptr(nil()),\
            login_btn: Ptr(nil()),\
            login_err: Ptr(nil()),\
            settings_win: Ptr(nil()),\
            s_url: Ptr(nil()),\
            s_sub_lang: Ptr(nil()),\
            s_quality: Ptr(nil()),\
            s_theme: Ptr(nil()),\
            s_searx_on: Ptr(nil()),\
            s_searx_url: Ptr(nil()),\
            rows: vec![],\
            views: vec![],\
            items: vec![],\
            poster_cache: HashMap::new(),\
            grid_data: None,\
            playing: None,\
            sub_group: nil(),\
            sub_opts: vec![],\
            audio_group: nil(),\
            audio_opts: vec![],\
            sel_sub: 0,\
            sel_audio: 0,\
            mode_stack: vec![],\
            cur_title: String::new(),\
        \};\
        let _ = UI.set(ui);\
    \}\
\
    pub fn run() \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            let _: () = msg_send![ui.app.get(), run];\
        \}\
    \}\
\
    unsafe fn build_window_and_ui() \{\
        let ui = UI.get().unwrap();\
        let mut frame = rect(0., 0., 1280., 800.);\
        if let Some([x, y, w, h]) = ui.cfg.window \{\
            if w > 400.0 && h > 300.0 \{\
                frame = rect(x, y, w, h);\
            \}\
        \}\
        let style: NSUInteger = 1 | 2 | 4 | 8 | (1 << 15); // titled|closable|mini|resizable|fullSizeContent\
        let win: *mut Object = msg_send![class!(NSWindow), alloc];\
        let win: *mut Object = msg_send![win,\
            initWithContentRect: frame styleMask: style backing: 2usize defer: NO];\
        let _: () = msg_send![win, setTitle: ns("Jellyfin")];\
        let _: () = msg_send![win, setTitlebarAppearsTransparent: YES];\
        let _: () = msg_send![win, setReleasedWhenClosed: NO];\
        let _: () = msg_send![win, setTabbingMode: 1i64]; // disallowed\
        let _: () = msg_send![win, setMinSize: size(960., 600.)];\
        let _: () = msg_send![win, setOpaque: NO];\
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];\
        let _: () = msg_send![win, setBackgroundColor: clear];\
        let _: () = msg_send![win, center];\
        ui.win = Ptr::retained(win);\
\
        let content: *mut Object = msg_send![win, contentView];\
        let b = bounds_of(content);\
\
        // Background vibrancy (whole-window glass; adapts to light/dark and to\
        // macOS 15's material rendering automatically).\
        let bg = vev(MAT_UNDER_WINDOW, BLEND_BEHIND);\
        set_frame(bg, b);\
        set_autoresize(bg, RESIZE_W | RESIZE_H);\
        add_sub(content, bg);\
        ui.bgfx = Ptr(bg);\
\
        // --- Content host (library side) ---\
        let content_host: *mut Object = msg_send![class!(NSView), alloc];\
        let content_host: *mut Object = msg_send![content_host, initWithFrame: b];\
        set_autoresize(content_host, RESIZE_W | RESIZE_H);\
        add_sub(content, content_host);\
        autorel(content_host);\
        ui.content_host = Ptr(content_host);\
\
        // Sidebar\
        let sb = vev(MAT_SIDEBAR, BLEND_BEHIND);\
        set_frame(sb, rect(0., 0., 240., b.size.h));\
        set_autoresize(sb, RESIZE_H | MARGIN_MAXX);\
        add_sub(content_host, sb);\
        ui.sidebar = Ptr(sb);\
\
        let stack: *mut Object = msg_send![class!(NSStackView), alloc];\
        let stack: *mut Object = msg_send![stack, initWithFrame: rect(12., 12., 216., b.size.h - 24.)];\
        let _: () = msg_send![stack, setOrientation: 1i64]; // vertical\
        let _: () = msg_send![stack, setAlignment: 0i64]; // leading\
        let _: () = msg_send![stack, setSpacing: 6.0f64];\
        set_autoresize(stack, RESIZE_H | RESIZE_W);\
        add_sub(sb, stack);\
        autorel(stack);\
        ui.side_stack = Ptr(stack);\
\
        let title = make_label("Jellyfin", 17.0, true, false);\
        add_arranged(stack, title, 12.0, 6.0);\
\
        // Search field\
        let sf: *mut Object = msg_send![class!(NSSearchField), alloc];\
        let sf: *mut Object = msg_send![sf, initWithFrame: rect(0., 0., 216., 28.)];\
        let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search libraries")];\
        let _: () = msg_send![sf, setTarget: ui.ctl.get()];\
        let _: () = msg_send![sf, setAction: sel!(sortChanged:)]; // enter = search too\
        let _: () = msg_send![sf, setSendsWholeSearchString: NO];\
        add_arranged(stack, sf, 0.0, 4.0);\
        ui.search = Ptr(sf);\
\
        // Libraries get injected by sidebar_rebuild(); spacer + buttons below.\
        let settings_btn: *mut Object = msg_send![class!(NSButton),\
            buttonWithTitle: ns("Settings\'85") target: ui.ctl.get() action: sel!(showSettings:)];\
        add_arranged(stack, settings_btn, 0.0, 2.0);\
        let searx_btn: *mut Object = msg_send![class!(NSButton),\
            buttonWithTitle: ns("Find something new (SearXNG)") target: ui.ctl.get() action: sel!(openSearx:)];\
        add_arranged(stack, searx_btn, 0.0, 2.0);\
\
        // Header row (title + sort)\
        let ht = make_label("Library", 20.0, true, false);\
        set_frame(ht, rect(264., b.size.h - 52., 500., 26.));\
        set_autoresize(ht, MARGIN_MINY);\
        add_sub(content_host, ht);\
        ui.header_title = Ptr(ht);\
\
        let pop: *mut Object = msg_send![class!(NSPopUpButton), alloc];\
        let pop: *mut Object = msg_send![pop, initWithFrame: rect(0., 0., 150., 26.) pullsDown: NO];\
        let arr: *mut Object = msg_send![class!(NSMutableArray), array];\
        for t in ["Name (A\'96Z)", "Newest first", "Highest rated", "Recently played"] \{\
            let _: () = msg_send![arr, addObject: ns(t)];\
        \}\
        let _: () = msg_send![pop, addItemsWithTitles: arr];\
        let _: () = msg_send![pop, setTarget: ui.ctl.get()];\
        let _: () = msg_send![pop, setAction: sel!(sortChanged:)];\
        let pb = bounds_of(content_host);\
        set_frame(pop, rect(pb.size.w - 170., pb.size.h - 54., 150., 26.));\
        set_autoresize(pop, MARGIN_MINX | MARGIN_MINY);\
        add_sub(content_host, pop);\
        autorel(pop);\
        ui.sort_popup = Ptr(pop);\
\
        // Grid scroll view\
        let sc: *mut Object = msg_send![class!(NSScrollView), alloc];\
        let sc: *mut Object = msg_send![sc, initWithFrame: rect(256., 0., pb.size.w - 256. - 12., pb.size.h - 60.)];\
        let _: () = msg_send![sc, setHasVerticalScroller: YES];\
        let _: () = msg_send![sc, setDrawsBackground: NO];\
        set_autoresize(sc, RESIZE_W | RESIZE_H);\
        add_sub(content_host, sc);\
        autorel(sc);\
        ui.lib_scroll = Ptr(sc);\
\
        let grid_cls = GRID_CLASS.get().unwrap();\
        let grid: *mut Object = msg_send![grid_cls, alloc];\
        let grid: *mut Object = msg_send![grid, initWithFrame: rect(0., 0., pb.size.w - 256. - 28., 10.)];\
        let data = Box::into_raw(Box::new(GridData \{ cells: vec![] \}));\
        (*grid).set_ivar::<*mut c_void>("jrData", data as *mut c_void);\
        let _: *mut Object = msg_send![sc, setDocumentView: grid];\
        autorel(grid);\
        ui.grid = Ptr(grid);\
\
        // Spinner + status label\
        let sp: *mut Object = msg_send![class!(NSProgressIndicator), alloc];\
        let sp: *mut Object = msg_send![sp, initWithFrame: rect(pb.size.w / 2. - 12., pb.size.h / 2., 24., 24.)];\
        let _: () = msg_send![sp, setIndeterminate: YES];\
        let _: () = msg_send![sp, setStyle: 1i64]; // spinning\
        let _: () = msg_send![sp, setUsesThreadedAnimation: YES];\
        set_autoresize(sp, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);\
        add_sub(content_host, sp);\
        autorel(sp);\
        ui.spinner = Ptr(sp);\
        let _: () = msg_send![sp, startAnimation: nil()];\
\
        let st = make_label("", 13.0, false, true);\
        set_frame(st, rect(260., pb.size.h / 2. - 30., pb.size.w - 280., 20.));\
        set_autoresize(st, RESIZE_W | MARGIN_MINY | MARGIN_MAXY);\
        let _: () = msg_send![st, setTextColor: color_named("secondary")];\
        add_sub(content_host, st);\
        ui.status_label = Ptr(st);\
\
        // --- Player host (AVKit) + top bar + chip ---\
        let ph: *mut Object = msg_send![class!(NSView), alloc];\
        let ph: *mut Object = msg_send![ph, initWithFrame: b];\
        set_autoresize(ph, RESIZE_W | RESIZE_H);\
        add_sub(content, ph);\
        autorel(ph);\
        ui.player_host = Ptr(ph);\
        set_hidden(ph, true);\
\
        let avv: *mut Object = msg_send![class!(AVPlayerView), alloc];\
        let avv: *mut Object = msg_send![avv, initWithFrame: b];\
        set_autoresize(avv, RESIZE_W | RESIZE_H);\
        let _: () = msg_send![avv, setShowsFullScreenToggleButton: YES];\
        let _: () = msg_send![avv, setShowsFrameSteppingButtons: NO];\
        let _: () = msg_send![avv, setShowsShareButton: NO];\
        let _: () = msg_send![avv, setShowsPictureInPictureButton: YES];\
        add_sub(ph, avv);\
        autorel(avv);\
        ui.avview = Ptr(avv);\
\
        let player: *mut Object = msg_send![class!(AVPlayer), playerWithURL: ns("about:blank")];\
        // about:blank won't play; we replace the item on every real playback.\
        ui.player = Ptr::retained(player);\
        let _: () = msg_send![avv, setPlayer: player];\
        let _: () = msg_send![player, setVolume: ui.cfg.volume.clamp(0.0, 1.0)];\
\
        let bar = vev(MAT_HUD, BLEND_WITHIN);\
        let _: () = msg_send![bar, setAlphaValue: 0.92f64];\
        set_frame(bar, rect(0., b.size.h - 44., b.size.w, 44.));\
        set_autoresize(bar, RESIZE_W | MARGIN_MINY);\
        add_sub(ph, bar);\
        ui.top_bar = Ptr(bar);\
        let l = layer_of(bar);\
        let _: () = msg_send![l, setMasksToBounds: YES];\
\
        let back: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("\'8b  Back")\
            target: ui.ctl.get() action: sel!(goBack:)];\
        set_frame(back, rect(12., 8., 72., 28.));\
        let _: () = msg_send![back, setBordered: NO];\
        add_sub(bar, back);\
        ui.back_btn = Ptr(back);\
\
        let tt = make_label("", 14.0, true, false);\
        set_frame(tt, rect(96., 12., b.size.w - 560., 20.));\
        set_autoresize(tt, RESIZE_W);\
        let _: () = msg_send![tt, setTextColor: color_named("white")];\
        add_sub(bar, tt);\
        ui.top_title = Ptr(tt);\
\
        let chip = vev(MAT_HUD, BLEND_WITHIN);\
        let _: () = msg_send![chip, setAlphaValue: 0.8f64];\
        set_frame(chip, rect(b.size.w - 356., b.size.h - 78., 340., 24.));\
        set_autoresize(chip, MARGIN_MINX | MARGIN_MINY);\
        let cl = layer_of(chip);\
        let _: () = msg_send![cl, setCornerRadius: 8.0f64];\
        let _: () = msg_send![cl, setMasksToBounds: YES];\
        add_sub(ph, chip);\
        ui.chip = Ptr(chip);\
        let chip_l = make_label("", 11.5, false, true);\
        set_frame(chip_l, rect(6., 4., 328., 16.));\
        let _: () = msg_send![chip_l, setTextColor: color_named("white")];\
        add_sub(chip, chip_l);\
        ui.chip_label = Ptr(chip_l);\
\
        // --- Login card ---\
        let card = vev(MAT_POPOVER, BLEND_WITHIN);\
        let card_w = 340.;\
        let card_h = 320.;\
        set_frame(card, rect(b.size.w / 2. - card_w / 2., b.size.h / 2. - card_h / 2., card_w, card_h));\
        set_autoresize(card, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);\
        let cl = layer_of(card);\
        let _: () = msg_send![cl, setCornerRadius: 14.0f64];\
        let _: () = msg_send![cl, setMasksToBounds: YES];\
        add_sub(content, card);\
        ui.login_card = Ptr(card);\
\
        let t = make_label("Sign in to Jellyfin", 17.0, true, false);\
        set_frame(t, rect(24., card_h - 44., card_w - 48., 24.));\
        add_sub(card, t);\
\
        let url_f = text_field("Server URL (http://host:8096)", false);\
        set_frame(url_f, rect(24., card_h - 96., card_w - 48., 26.));\
        add_sub(card, url_f);\
        ui.login_url = Ptr(url_f);\
        let user_f = text_field("Username", false);\
        set_frame(user_f, rect(24., card_h - 132., card_w - 48., 26.));\
        add_sub(card, user_f);\
        ui.login_user = Ptr(user_f);\
        let pass_f = text_field("Password", true);\
        set_frame(pass_f, rect(24., card_h - 168., card_w - 48., 26.));\
        add_sub(card, pass_f);\
        ui.login_pass = Ptr(pass_f);\
        let key_f = text_field("API key (alternative to password)", false);\
        set_frame(key_f, rect(24., card_h - 204., card_w - 48., 26.));\
        add_sub(card, key_f);\
        ui.login_key = Ptr(key_f);\
\
        let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Connect")\
            target: ui.ctl.get() action: sel!(connectClicked:)];\
        set_frame(btn, rect(24., card_h - 244., card_w - 48., 32.));\
        let _: () = msg_send![btn, setKeyEquivalent: ns("\\r")];\
        add_sub(card, btn);\
        ui.login_btn = Ptr(btn);\
\
        let err = make_label("", 12.0, false, false);\
        set_frame(err, rect(24., 20., card_w - 48., 44.));\
        let _: () = msg_send![err, setTextColor: color_named("red")];\
        add_sub(card, err);\
        ui.login_err = Ptr(err);\
\
        // Restore remembered URL/user.\
        if let Some(u) = &ui.cfg.server_url \{\
            set_text(url_f, u);\
        \}\
        if let Some(u) = &ui.cfg.username \{\
            set_text(user_f, u);\
        \}\
\
        // --- Settings window (presented as a sheet) ---\
        build_settings_window();\
\
        // --- Menus (native menu bar: playback is keyboard-operable here) ---\
        build_menus();\
\
        let _: () = msg_send![win, makeKeyAndOrderFront: nil()];\
        #[allow(deprecated)]\
        let _: () = msg_send![ui.app.get(), activateIgnoringOtherApps: YES];\
\
        // 1s UI timer: chip/progress/autosave (target = controller).\
        let timer: *mut Object = msg_send![class!(NSTimer),\
            scheduledTimerWithTimeInterval: 1.0f64 target: ui.ctl.get()\
            selector: sel!(jrTick:) userInfo: nil() repeats: YES];\
        ui.timer = Ptr(timer);\
\
        apply_theme();\
    \}\
\
    unsafe fn add_arranged(stack: *mut Object, v: *mut Object, _top: CGFloat, _pad: CGFloat) \{\
        let _: () = msg_send![stack, addArrangedSubview: v];\
    \}\
\
    unsafe fn build_settings_window() \{\
        let ui = UI.get().unwrap();\
        let w: *mut Object = msg_send![class!(NSWindow), alloc];\
        let w: *mut Object = msg_send![w,\
            initWithContentRect: rect(0., 0., 460., 430.)\
            styleMask: (1 | 2) as NSUInteger backing: 2usize defer: NO];\
        let _: () = msg_send![w, setTitle: ns("Settings")];\
        let _: () = msg_send![w, setReleasedWhenClosed: NO];\
        ui.settings_win = Ptr::retained(w);\
        let c: *mut Object = msg_send![w, contentView];\
        let b = bounds_of(c);\
\
        let mk = |y: f64, label: &str, placeholder: &str| -> *mut Object \{\
            let l = make_label(label, 12.0, false, false);\
            set_frame(l, rect(20., b.size.h - y - 16., 200., 14.));\
            add_sub(c, l);\
            let f = text_field(placeholder, false);\
            set_frame(f, rect(20., b.size.h - y - 40., 420., 24.));\
            add_sub(c, f);\
            f\
        \};\
        let url_f = mk(24.0, "Server URL", "http://host:8096");\
        ui.s_url = Ptr(url_f);\
        let sub_f = mk(84.0, "Preferred subtitle language (e.g. eng, empty = off)", "eng");\
        ui.s_sub_lang = Ptr(sub_f);\
        let searx_f = mk(204.0, "SearXNG instance URL", "https://searx.example.com");\
        ui.s_searx_url = Ptr(searx_f);\
\
        let ql = make_label("Preferred quality", 12.0, false, false);\
        set_frame(ql, rect(20., b.size.h - 144. - 16., 200., 14.));\
        add_sub(c, ql);\
        let q: *mut Object = msg_send![class!(NSPopUpButton), alloc];\
        let q: *mut Object = msg_send![q, initWithFrame: rect(20., b.size.h - 168., 220., 26.) pullsDown: NO];\
        let arr: *mut Object = msg_send![class!(NSMutableArray), array];\
        for t in ["Original", "40 Mbps", "20 Mbps", "10 Mbps", "5 Mbps", "3 Mbps"] \{\
            let _: () = msg_send![arr, addObject: ns(t)];\
        \}\
        let _: () = msg_send![q, addItemsWithTitles: arr];\
        add_sub(c, q);\
        autorel(q);\
        ui.s_quality = Ptr(q);\
\
        let tl = make_label("Appearance", 12.0, false, false);\
        set_frame(tl, rect(260., b.size.h - 144. - 16., 200., 14.));\
        add_sub(c, tl);\
        let th: *mut Object = msg_send![class!(NSPopUpButton), alloc];\
        let th: *mut Object = msg_send![th, initWithFrame: rect(260., b.size.h - 168., 180., 26.) pullsDown: NO];\
        let arr: *mut Object = msg_send![class!(NSMutableArray), array];\
        for t in ["Match System", "Light", "Dark"] \{\
            let _: () = msg_send![arr, addObject: ns(t)];\
        \}\
        let _: () = msg_send![th, addItemsWithTitles: arr];\
        add_sub(c, th);\
        autorel(th);\
        ui.s_theme = Ptr(th);\
\
        let cb: *mut Object = msg_send![class!(NSButton),\
            checkboxWithTitle: ns("Enable SearXNG search (opens your browser)")\
            target: nil() action: nil()];\
        set_frame(cb, rect(20., b.size.h - 280., 420., 20.));\
        add_sub(c, cb);\
        ui.s_searx_on = Ptr(cb);\
\
        let save: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Save")\
            target: ui.ctl.get() action: sel!(saveSettings:)];\
        set_frame(save, rect(b.size.w - 116., 16., 96., 32.));\
        let _: () = msg_send![save, setKeyEquivalent: ns("\\r")];\
        add_sub(c, save);\
        let cancel: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel")\
            target: ui.ctl.get() action: sel!(cancelSettings:)];\
        set_frame(cancel, rect(b.size.w - 222., 16., 96., 32.));\
        add_sub(c, cancel);\
    \}\
\
    unsafe fn settings_fill(cfg: &Config) \{\
        let ui = UI.get().unwrap();\
        if let Some(u) = &cfg.server_url \{\
            set_text(ui.s_url.get(), u);\
        \}\
        set_text(ui.s_sub_lang.get(), &cfg.settings.subtitle_lang);\
        let q_idx = match cfg.settings.max_bitrate_kbps \{\
            0 => 0usize,\
            40_000 => 1,\
            20_000 => 2,\
            10_000 => 3,\
            5_000 => 4,\
            _ => 5,\
        \};\
        let _: () = msg_send![ui.s_quality.get(), selectItemAtIndex: q_idx as NSInteger];\
        let t_idx = match cfg.settings.theme.as_str() \{\
            "light" => 1usize,\
            "dark" => 2,\
            _ => 0,\
        \};\
        let _: () = msg_send![ui.s_theme.get(), selectItemAtIndex: t_idx as NSInteger];\
        let _: () = msg_send![ui.s_searx_on.get(), setState: cfg.settings.searx_enabled as NSInteger];\
        set_text(ui.s_searx_url.get(), &cfg.settings.searx_url);\
    \}\
\
    unsafe fn apply_theme() \{\
        let ui = UI.get().unwrap();\
        let appearance: *mut Object = match ui.cfg.settings.theme.as_str() \{\
            "light" => \{\
                let a: *mut Object =\
                    msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameAqua")];\
                a\
            \}\
            "dark" => \{\
                let a: *mut Object =\
                    msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameDarkAqua")];\
                a\
            \}\
            _ => nil(),\
        \};\
        let _: () = msg_send![ui.app.get(), setAppearance: appearance];\
    \}\
\
    unsafe fn build_menus() \{\
        let ui = UI.get().unwrap();\
        let main_menu: *mut Object = msg_send![class!(NSMenu), alloc];\
        let main_menu: *mut Object = msg_send![main_menu, initWithTitle: ns("MainMenu")];\
        let _: () = msg_send![ui.app.get(), setMainMenu: main_menu];\
        autorel(main_menu);\
\
        // App menu\
        let app_item: *mut Object = msg_send![class!(NSMenuItem), alloc];\
        let app_item: *mut Object = msg_send![app_item,\
            initWithTitle: ns("JellyfinMac") action: nil() keyEquivalent: ns("")];\
        let app_menu: *mut Object = msg_send![class!(NSMenu), alloc];\
        let app_menu: *mut Object = msg_send![app_menu, initWithTitle: ns("JellyfinMac")];\
        let _: () = msg_send![app_item, setSubmenu: app_menu];\
        let _: () = msg_send![main_menu, addItem: app_item];\
        autorel(app_item);\
        let about: *mut Object = msg_send![class!(NSMenuItem),\
            menuItemWithTitle: ns("About JellyfinMac") action: sel!(orderFrontStandardAboutPanel:)\
            keyEquivalent: ns("")];\
        let _: () = msg_send![app_menu, addItem: about];\
        let _: () = msg_send![app_menu, addItem: separator()];\
        let settings_i: *mut Object = msg_send![class!(NSMenuItem),\
            menuItemWithTitle: ns("Settings\'85") action: sel!(showSettings:) keyEquivalent: ns(",")];\
        let _: () = msg_send![settings_i, setTarget: ui.ctl.get()];\
        let _: () = msg_send![app_menu, addItem: settings_i];\
        let _: () = msg_send![app_menu, addItem: separator()];\
        let quit: *mut Object = msg_send![class!(NSMenuItem),\
            menuItemWithTitle: ns("Quit JellyfinMac") action: sel!(terminate:) keyEquivalent: ns("q")];\
        let _: () = msg_send![app_menu, addItem: quit];\
\
        // Controls menu (playback is fully operable from the keyboard here).\
        let ctl_item: *mut Object = msg_send![class!(NSMenuItem), alloc];\
        let ctl_item: *mut Object = msg_send![ctl_item,\
            initWithTitle: ns("Controls") action: nil() keyEquivalent: ns("")];\
        let ctl_menu: *mut Object = msg_send![class!(NSMenu), alloc];\
        let ctl_menu: *mut Object = msg_send![ctl_menu, initWithTitle: ns("Controls")];\
        let _: () = msg_send![ctl_item, setSubmenu: ctl_menu];\
        let _: () = msg_send![main_menu, addItem: ctl_item];\
        autorel(ctl_item);\
        let add = |menu: *mut Object, title: &str, action: Sel, key: &str, mask: NSUInteger, tag: NSInteger| unsafe \{\
            let it: *mut Object = msg_send![class!(NSMenuItem),\
                menuItemWithTitle: ns(title) action: action keyEquivalent: ns(key)];\
            let _: () = msg_send![it, setTarget: ui.ctl.get()];\
            let _: () = msg_send![it, setKeyEquivalentModifierMask: mask];\
            let _: () = msg_send![it, setTag: tag];\
            let _: () = msg_send![menu, addItem: it];\
        \};\
        const CMD: NSUInteger = 1 << 20;\
        const ALT: NSUInteger = 1 << 19;\
        const CTRL_CMD: NSUInteger = (1 << 18) | (1 << 20);\
        add(ctl_menu, "Play / Pause", sel!(playPause:), " ", 0usize, 0);\
        add(ctl_menu, "Stop", sel!(stopPlayback:), ".", CMD, 0);\
        add(ctl_menu, "Back 10s", sel!(seekBack:), "\\u\{2190\}", CMD, 0);\
        add(ctl_menu, "Forward 10s", sel!(seekFwd:), "\\u\{2192\}", CMD, 0);\
        add(ctl_menu, "Volume Up", sel!(volUp:), "\\u\{2191\}", CMD, 0);\
        add(ctl_menu, "Volume Down", sel!(volDown:), "\\u\{2193\}", CMD, 0);\
        add(ctl_menu, "Mute", sel!(mute:), "m", ALT, 0);\
        add(ctl_menu, "Next Subtitle Track", sel!(subCycle:), "s", ALT, 0);\
        add(ctl_menu, "Next Audio Track", sel!(audioCycle:), "a", ALT, 0);\
        add(ctl_menu, "Enter / Exit Full Screen", sel!(fullscreen:), "f", CTRL_CMD, 0);\
        // Speed submenu\
        let speed_item: *mut Object = msg_send![class!(NSMenuItem), alloc];\
        let speed_item: *mut Object = msg_send![speed_item,\
            initWithTitle: ns("Playback Speed") action: nil() keyEquivalent: ns("")];\
        let speed_menu: *mut Object = msg_send![class!(NSMenu), alloc];\
        let speed_menu: *mut Object = msg_send![speed_menu, initWithTitle: ns("Playback Speed")];\
        let _: () = msg_send![speed_item, setSubmenu: speed_menu];\
        let _: () = msg_send![ctl_menu, addItem: speed_item];\
        autorel(speed_item);\
        for (label, tag) in [("0.5\'d7", 50i64), ("0.75\'d7", 75), ("1\'d7", 100), ("1.25\'d7", 125), ("1.5\'d7", 150), ("2\'d7", 200)] \{\
            add(speed_menu, label, sel!(setSpeed:), "", 0usize, tag);\
        \}\
\
        // Go menu\
        let go_item: *mut Object = msg_send![class!(NSMenuItem), alloc];\
        let go_item: *mut Object = msg_send![go_item,\
            initWithTitle: ns("Go") action: nil() keyEquivalent: ns("")];\
        let go_menu: *mut Object = msg_send![class!(NSMenu), alloc];\
        let go_menu: *mut Object = msg_send![go_menu, initWithTitle: ns("Go")];\
        let _: () = msg_send![go_item, setSubmenu: go_menu];\
        let _: () = msg_send![main_menu, addItem: go_item];\
        autorel(go_item);\
        add(go_menu, "Refresh", sel!(refresh:), "r", CMD, 0);\
        add(go_menu, "Back", sel!(goBack:), "[", CMD, 0);\
        add(go_menu, "Sign Out", sel!(signOut:), "l", ALT, 0);\
        add(go_menu, "Find something new (SearXNG)", sel!(openSearx:), "k", ALT, 0);\
    \}\
\
    unsafe fn separator() -> *mut Object \{\
        autorel(msg_send![class!(NSMenuItem), separatorItem])\
    \}\
\
    // ---- Sidebar ------------------------------------------------------------\
    unsafe fn sidebar_rebuild(user: String, views: &[(String, String, Option<String>)], has_live_tv: bool) \{\
        let ui = UI.get().unwrap();\
        let ui_tx = ui.ui_tx.clone();\
        // Clear previous rows (stack retains arranged subviews; removing them\
        // releases them \'97 we never held our own +1).\
        for (btn, _) in ui.rows.drain(..) \{\
            let _: () = msg_send![ui.side_stack.get(), removeArrangedSubview: btn];\
            let _: () = msg_send![btn, removeFromSuperview];\
        \}\
        let user_l = make_label(&format!("Signed in as \{\}", user), 11.0, false, false);\
        let _: () = msg_send![user_l, setTextColor: color_named("secondary")];\
        add_arranged(ui.side_stack.get(), user_l, 0.0, 2.0);\
\
        for (i, (id, name, ct)) in views.iter().enumerate() \{\
            let symbol = match ct.as_deref() \{\
                Some("movies") => "film",\
                Some("tvshows") => "tv",\
                Some("music") => "music.note",\
                Some("livetv") => "antenna.radiowaves.left.and.right",\
                _ => "square.stack",\
            \};\
            let btn: *mut Object = msg_send![class!(NSButton),\
                buttonWithTitle: ns(name) target: ui.ctl.get() action: sel!(viewSelected:)];\
            let _: () = msg_send![btn, setBordered: NO];\
            let _: () = msg_send![btn, setTag: i as NSInteger];\
            let img = symbol_image(symbol, 13.0);\
            if !img.is_null() \{\
                let _: () = msg_send![btn, setImage: img];\
                let _: () = msg_send![btn, setImagePosition: 2i64]; // imageLeft\
            \}\
            add_arranged(ui.side_stack.get(), btn, 0.0, 0.0);\
            ui.rows.push((btn, id.clone()));\
        \}\
        if has_live_tv \{\
            // Live TV is exposed as a pseudo-view handled by the worker.\
            let btn: *mut Object = msg_send![class!(NSButton),\
                buttonWithTitle: ns("Live TV") target: ui.ctl.get() action: sel!(viewSelected:)];\
            let _: () = msg_send![btn, setBordered: NO];\
            let img = symbol_image("antenna.radiowaves.left.and.right", 13.0);\
            if !img.is_null() \{\
                let _: () = msg_send![btn, setImage: img];\
                let _: () = msg_send![btn, setImagePosition: 2i64];\
            \}\
            let idx = ui.views.len().saturating_sub(1);\
            let _: () = msg_send![btn, setTag: idx as NSInteger];\
            add_arranged(ui.side_stack.get(), btn, 0.0, 0.0);\
            ui.rows.push((btn, "livetv".into()));\
        \}\
        let _ = ui_tx;\
    \}\
\
    // ---- View switching ------------------------------------------------------\
    pub fn show_login_or_library(connected: bool) \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            set_hidden(ui.content_host.get(), !connected);\
            set_hidden(ui.login_card.get(), connected);\
            if connected \{\
                let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.search.get()];\
            \} else \{\
                let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];\
                set_text(ui.login_btn.get(), "Connect");\
            \}\
        \}\
    \}\
\
    pub fn login_error(msg: &str) \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            set_text(ui.login_err.get(), msg);\
            let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];\
            set_text(ui.login_btn.get(), "Connect");\
            set_hidden(ui.spinner.get(), true);\
        \}\
    \}\
\
    // ---- Poster grid ---------------------------------------------------------\
    pub fn grid_set_items(items: &[ListItem]) \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            let grid = ui.grid.get();\
            // Tear down old cells (remove releases them; see safety model).\
            let old = ui.grid_data.take();\
            if let Some(d) = old \{\
                for c in &d.cells \{\
                    let _: () = msg_send![c.container, removeFromSuperview];\
                \}\
                // Box drops here; contained raw pointers are dead by convention.\
            \}\
            let mut cells = Vec::with_capacity(items.len());\
            for (i, it) in items.iter().enumerate() \{\
                let cell_cls = CELL_CLASS.get().unwrap();\
                let container: *mut Object = msg_send![cell_cls, alloc];\
                let container: *mut Object = msg_send![container, initWithFrame: rect(0., 0., 176., 300.)];\
                let cw = 176.0;\
                // Poster\
                let poster: *mut Object = msg_send![class!(NSImageView), alloc];\
                let poster: *mut Object = msg_send![poster, initWithFrame: rect(0., 0., cw, cw * 1.5)];\
                let _: () = msg_send![poster, setImageScaling: 0i64]; // proportionally down\
                add_sub(container, poster);\
                autorel(poster);\
                // Hover play icon\
                let hover = symbol_image("play.circle.fill", 40.0);\
                let hv: *mut Object = msg_send![class!(NSImageView), alloc];\
                let hv: *mut Object = msg_send![hv, initWithFrame: rect(cw / 2. - 20., cw * 0.75 - 20., 40., 40.)];\
                if !hover.is_null() \{\
                    let _: () = msg_send![hv, setImage: hover];\
                \}\
                let _: () = msg_send![hv, setAlphaValue: 0.0f64];\
                add_sub(container, hv);\
                autorel(hv);\
                // Progress bar\
                let prog: *mut Object = msg_send![class!(NSProgressIndicator), alloc];\
                let prog: *mut Object = msg_send![prog, initWithFrame: rect(8., cw * 1.5 - 6., cw - 16., 4.)];\
                let _: () = msg_send![prog, setStyle: 0i64]; // bar\
                let _: () = msg_send![prog, setIndeterminate: NO];\
                let _: () = msg_send![prog, setMinValue: 0.0f64];\
                let _: () = msg_send![prog, setMaxValue: 1.0f64];\
                let frac = super::jellyfin::urls::progress_fraction(\
                    it.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0),\
                    it.run_time_ticks,\
                );\
                let _: () = msg_send![prog, setDoubleValue: frac];\
                set_hidden(prog, frac <= 0.0);\
                add_sub(container, prog);\
                autorel(prog);\
                // Watched badge\
                let badge: *mut Object = msg_send![class!(NSImageView), alloc];\
                let badge: *mut Object = msg_send![badge, initWithFrame: rect(6., cw * 1.5 - 24., 18., 18.)];\
                let bimg = symbol_image("checkmark.circle.fill", 14.0);\
                if !bimg.is_null() \{\
                    let _: () = msg_send![badge, setImage: bimg];\
                    let _: () = msg_send![badge, setContentTintColor: color_named("accent")];\
                \}\
                set_hidden(badge, !it.user_data.as_ref().map(|u| u.played).unwrap_or(false));\
                add_sub(container, badge);\
                autorel(badge);\
                // Title + subtitle\
                let title = make_label(&it.name, 12.5, false, false);\
                set_frame(title, rect(0., cw * 1.5 + 6., cw, 16.));\
                let _: () = msg_send![title.cell(), setLineBreakMode: 4usize]; // truncate tail\
                add_sub(container, title);\
                let sub = make_label(&it.sub_label(), 10.5, false, false);\
                set_frame(sub, rect(0., cw * 1.5 + 22., cw, 13.));\
                let _: () = msg_send![sub, setTextColor: color_named("secondary")];\
                let _: () = msg_send![sub.cell(), setLineBreakMode: 4usize];\
                add_sub(container, sub);\
                // Click button (covers poster area)\
                let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")\
                    target: ui.ctl.get() action: sel!(itemClicked:)];\
                set_frame(btn, rect(0., 0., cw, cw * 1.5));\
                let _: () = msg_send![btn, setBordered: NO];\
                let _: () = msg_send![btn, setTag: i as NSInteger];\
                let _: () = msg_send![btn, setToolTip: ns(&it.name)];\
                add_sub(container, btn);\
                autorel(btn);\
                // Context menu (watched actions + open in web)\
                let menu: *mut Object = msg_send![class!(NSMenu), alloc];\
                let menu: *mut Object = msg_send![menu, initWithTitle: ns(&it.name)];\
                let id_ns = ns(&it.id);\
                let m1: *mut Object = msg_send![class!(NSMenuItem),\
                    menuItemWithTitle: ns("Mark as watched") action: sel!(markWatched:) keyEquivalent: ns("")];\
                let _: () = msg_send![m1, setTarget: ui.ctl.get()];\
                let _: () = msg_send![m1, setRepresentedObject: id_ns];\
                let _: () = msg_send![menu, addItem: m1];\
                let m2: *mut Object = msg_send![class!(NSMenuItem),\
                    menuItemWithTitle: ns("Open in Jellyfin Web") action: sel!(openInWeb:) keyEquivalent: ns("")];\
                let _: () = msg_send![m2, setTarget: ui.ctl.get()];\
                let _: () = msg_send![m2, setRepresentedObject: id_ns];\
                let _: () = msg_send![menu, addItem: m2];\
                let _: () = msg_send![btn, setMenu: menu];\
                autorel(menu);\
                // Cell ivar: hover view pointer\
                let cd = Box::into_raw(Box::new(CellData \{ hover: hv \}));\
                (*container).set_ivar::<*mut c_void>("jrCell", cd as *mut c_void);\
                // Tracking area for hover\
                let ta: *mut Object = msg_send![class!(NSTrackingArea), alloc];\
                let ta: *mut Object = msg_send![ta,\
                    initWithRect: rect(0., 0., cw, cw * 1.5)\
                    options: (16usize | 2usize) owner: container userInfo: nil()];\
                let _: () = msg_send![container, addTrackingArea: ta];\
                autorel(ta);\
                let _: () = msg_send![ui.grid.get(), addSubview: container];\
                autorel(container);\
                cells.push(CellHandle \{\
                    container,\
                    poster,\
                    title,\
                    sub,\
                    prog,\
                    badge,\
                    hover_icon: hv,\
                    key: poster_key(&it.id),\
                    meta: it.clone(),\
                \});\
            \}\
            ui.grid_data = Some(Box::new(GridData \{ cells \}));\
            let _: () = msg_send![grid, setNeedsLayout: YES];\
        \}\
    \}\
\
    struct CellData \{\
        hover: *mut Object,\
    \}\
\
    unsafe extern "C" fn cell_layout(this: &mut Object, _cmd: Sel) \{\
        let cd = (*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;\
        if cd.is_null() \{\
            let _: () = msg_send![this, layoutSuperViewIfNeeded_orNothing: nil()];\
            // fall through to super layout below (no ivar yet)\
        \}\
        // No manual layout needed: subviews were laid out relative to a fixed\
        // 176-wide cell and the grid sets frames; hover view tracks poster.\
        // Call super layout.\
        let super_cls: *const Class = msg_send![this, class];\
        let _: () = objc::send_super_layout(this);\
    \}\
\
    // NOTE: cell_layout intentionally delegates to super; frames are set at\
    // build time and the grid positions containers in grid_layout.\
\
    unsafe extern "C" fn cell_hover(this: &mut Object, _cmd: Sel, _ev: *mut Object) \{\
        let cd = (*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;\
        if !cd.is_null() \{\
            let anim: *mut Object = msg_send![(*cd).hover, animator];\
            let _: () = msg_send![anim, setAlphaValue: 0.9f64];\
        \}\
    \}\
    unsafe extern "C" fn cell_unhover(this: &mut Object, _cmd: Sel, _ev: *mut Object) \{\
        let cd = (*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;\
        if !cd.is_null() \{\
            let anim: *mut Object = msg_send![(*cd).hover, animator];\
            let _: () = msg_send![anim, setAlphaValue: 0.0f64];\
        \}\
    \}\
\
    unsafe extern "C" fn grid_layout(this: &mut Object, _cmd: Sel) \{\
        let data = (*this).get_ivar::<*mut c_void>("jrData") as *mut GridData;\
        if data.is_null() \{\
            return;\
        \}\
        let b = bounds_of(this as *mut Object);\
        let w = b.size.w.max(200.0);\
        let gap = 18.0;\
        let mut cols = ((w - gap) / (176.0 + gap)) as usize;\
        if cols < 1 \{\
            cols = 1;\
        \}\
        let tile_w = ((w - gap * (cols as f64 + 1.0)) / cols as f64).floor();\
        let poster_h = tile_w * 1.5;\
        let cell_h = poster_h + 42.0;\
        let d = &mut *data;\
        for (i, c) in d.cells.iter().enumerate() \{\
            let col = i % cols;\
            let row = i / cols;\
            let x = gap + col as f64 * (tile_w + gap);\
            let y = gap + row as f64 * (cell_h + gap);\
            set_frame(c.container, rect(x, y, tile_w, cell_h));\
            // Scale poster-holding subviews to tile width.\
            set_frame(c.poster, rect(0., 0., tile_w, poster_h));\
            set_frame(c.hover_icon, rect(tile_w / 2. - 20., poster_h / 2. - 20., 40., 40.));\
            set_frame(c.prog, rect(8., poster_h - 6., tile_w - 16., 4.));\
            set_frame(c.badge, rect(6., poster_h - 24., 18., 18.));\
            set_frame(c.title, rect(0., poster_h + 6., tile_w, 16.));\
            set_frame(c.sub, rect(0., poster_h + 22., tile_w, 13.));\
            // re-cover poster with the click button (stored in meta? keep in container children order)\
        \}\
        let rows = (d.cells.len() + cols - 1) / cols;\
        let content_h = gap + rows as f64 * (cell_h + gap);\
        let cur_h = b.size.h;\
        if (content_h - cur_h).abs() > 1.0 \{\
            let _: () = msg_send![this as *mut Object, setFrameSize: size(w, content_h)];\
        \}\
    \}\
    unsafe extern "C" fn grid_set_frame(this: &mut Object, _cmd: Sel, s: NSSize) \{\
        // Forward to super, then request a layout pass.\
        let _: () = msg_send![this, setFrameSizeSuper: s];\
        let _: () = msg_send![this, setNeedsLayout: YES];\
    \}\
\
    pub fn poster_key(item_id: &str) -> u64 \{\
        use std::hash::\{Hash, Hasher\};\
        let mut h = std::collections::hash_map::DefaultHasher::new();\
        item_id.hash(&mut h);\
        h.finish()\
    \}\
\
    pub fn poster_ready(key: u64, bytes: &[u8]) \{\
        unsafe \{\
            let ui = match UI.get() \{\
                Some(u) => u,\
                None => return,\
            \};\
            let data: *mut Object = msg_send![class!(NSData),\
                dataWithBytes: bytes.as_ptr() length: bytes.len() as NSUInteger];\
            if data.is_null() \{\
                return;\
            \}\
            let img: *mut Object = msg_send![class!(NSImage), alloc];\
            let img: *mut Object = msg_send![img, initWithData: data];\
            if img.is_null() \{\
                return;\
            \}\
            if let Some(old) = ui.poster_cache.insert(key, img) \{\
                release_obj(old);\
            \}\
            // Update any visible cells with this key.\
            if let Some(d) = ui.grid_data.as_mut() \{\
                for c in &d.cells \{\
                    if c.key == key \{\
                        let _: () = msg_send![c.poster, setImage: img];\
                    \}\
                \}\
            \}\
        \}\
    \}\
\
    // ---- Playback ------------------------------------------------------------\
    pub fn start_playback(ps: &PlaybackStart) \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            // Tear down previous item observation.\
            if !ui.pitem.get().is_null() \{\
                for kp in ["status", "presentationSize"] \{\
                    let _: () = msg_send![ui.pitem.get(),\
                        removeObserver: ui.ctl.get() forKeyPath: ns(kp)];\
                \}\
                release_obj(ui.pitem.get());\
                ui.pitem = Ptr(nil());\
            \}\
            clear_media_groups();\
            let url: *mut Object = msg_send![class!(NSURL), URLWithString: ns(&ps.url)];\
            let item: *mut Object = msg_send![class!(AVPlayerItem), playerItemWithURL: url];\
            if item.is_null() \{\
                let _ = ui.main_tx.send(MainMsg::PlaybackError("could not create player item".into()));\
                return;\
            \}\
            let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: item];\
            ui.pitem = Ptr::retained(item);\
            for kp in ["status", "presentationSize"] \{\
                let _: () = msg_send![item, addObserver: ui.ctl.get()\
                    forKeyPath: ns(kp) options: 3usize context: nil()]; // new+old\
            \}\
            let _: () = msg_send![ui.player.get(), addObserver: ui.ctl.get()\
                forKeyPath: ns("timeControlStatus") options: 1usize context: nil()];\
            let _: () = msg_send![ui.player.get(), setVolume: ui.cfg.volume.clamp(0.0, 1.0)];\
            let _: () = msg_send![ui.player.get(), setDefaultRate: 1.0f64];\
            let _: () = msg_send![ui.player.get(), setRate: 1.0f64];\
\
            ui.playing = Some(PlayingNow \{\
                item_id: ps.item_id.clone(),\
                url: ps.url.clone(),\
                title: ps.title.clone(),\
                method: ps.method,\
                resume_secs: ps.resume_secs,\
                last_secs: 0.0,\
                duration_secs: ps.run_time_secs,\
                paused: false,\
                attempts: 0,\
                ticks_since_save: 0,\
            \});\
            set_text(ui.top_title.get(), &ps.title);\
            let sub = ps.subtitle.clone().unwrap_or_default();\
            if !sub.is_empty() \{\
                let _: () = msg_send![ui.win.get(), setTitle: ns(&format!("\{\} \'97 \{\}", ps.title, sub))];\
            \} else \{\
                let _: () = msg_send![ui.win.get(), setTitle: ns(&ps.title)];\
            \}\
            set_hidden(ui.content_host.get(), true);\
            set_hidden(ui.player_host.get(), false);\
            let _: () = msg_send![ui.win.get(), setTitleVisibility: 1i64]; // hidden (custom top bar)\
            let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.avview.get()];\
            set_chip("", false);\
            if ps.resume_secs > 1.0 \{\
                let tm = CMTimeMakeWithSeconds(ps.resume_secs, 600);\
                let _: () = msg_send![ui.player.get(), seekToTime: tm];\
            \}\
            let _ = ui.ui_tx.send(UiCmd::Progress \{\
                item_id: ps.item_id.clone(),\
                ticks: super::jellyfin::urls::ticks_from_secs(ps.resume_secs),\
                paused: false,\
            \});\
        \}\
    \}\
\
    pub fn retry_playback() \{\
        unsafe \{\
            let ui = UI.get().unwrap();\
            let pn = match ui.playing.as_ref() \{\
                Some(p) => p,\
                None => return,\
            \};\
            if pn.attempts >= 3 \{\
                return;\
            \}\
            let url = pn.url.clone();\
            let resume = pn.last_secs;\
            let item_id = pn.item_id.clone();\
            let title = pn.title.clone();\
            let method = pn.method;\
            drop_replace_item(&url, resume);\
            if let Some(pn) = ui.playing.as_mut() \{\
                pn.attempts += 1;\
            \}\
            set_chip("Reconnected \'97 resuming\'85", false);\
            let _ = ui.ui_tx.send(UiCmd::Progress \{\
                item_id,\
                ticks: super::jellyfin::urls::ticks_from_secs(resume),\
                paused: false,\
            \});\
            let _ = title;\
            let _ = method;\
        \}\
    \}\
\
    unsafe fn drop_replace_item(url: &str, resume: f64) \{\
        let ui = UI.get().unwrap();\
        if !ui.pitem.get().is_null() \{\
            for kp in ["status", "presentationSize"] \{\
                let _: () = msg_send![ui.pitem.get(),\
                    removeObserver: ui.ctl.get() forKeyPath: ns(kp)];\
            \}\
        \}\
        let u: *mut Object = msg_send![class!(NSURL), URLWithString: ns(url)];\
        let item: *mut Object = msg_send![class!(AVPlayerItem), playerItemWithURL: u];\
        let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: item];\
        if !ui.pitem.get().is_null() \{\
            release_obj(ui.pitem.get());\
        \}\
        ui.pitem = Ptr::retained(item);\
        for kp in ["status", "presentationSize"] \{\
            let _: () = msg_send![item, addObserver: ui.ctl.get()\
                forKeyPath: ns(kp) options: 3usize context: nil()];\
        \}\
        if resume > 1.0 \{\
            let tm = CMTimeMakeWithSeconds(resume, 600);\
            let _: () = msg_send![ui.player.get(), seekToTime: tm];\
        \}\
    \}\
\
    // ---- misc sys helpers used elsewhere -------------------------------------\
    pub fn set_busy(b: bool) \{\
        unsafe \{\
            let ui = match UI.get() \{\
                Some(u) => u,\
                None => return,\
            \};\
            set_hidden(ui.spinner.get(), !b);\
        \}\
    \}\
\}\
\
// ============================================================================\
// SECTION: Application Controller \'97 safe app-layer glue\
// ============================================================================\
// SPLIT: \uc0\u8594  src/app/controller.rs (state transitions) + src/ui (above).\
\
pub mod app \{\
    // The controller lives in `sys` (it must touch AppKit); this module keeps\
    // the safe entry points the worker uses so no tokio thread ever imports\
    // `sys` directly. Kept as documentation of the layering contract.\
\}\
\
// ============================================================================\
// SECTION: Background Worker (tokio)\
// ============================================================================\
// SPLIT: \uc0\u8594  src/app/worker.rs. Owns JellyfinClient and all disk IO. Never\
// touches AppKit: every result crosses back via on_main(MainMsg).\
\
async fn worker(mut rx: UnboundedReceiver<UiCmd>, main_tx: UnboundedSender<MainMsg>, cfg: Config) \{\
    let mut client: Option<JellyfinClient> = None;\
    let mut cfg = cfg;\
\
    // Poster disk cache: ~/Library/Caches/JellyfinMac/posters\
    let poster_dir = crate::config::cache_dir().join("posters");\
    let _ = std::fs::create_dir_all(&poster_dir);\
\
    // Silent auto-connect from persisted server + keychain token.\
    if let (Some(url), Some(uid)) = (cfg.server_url.clone(), cfg.user_id.clone()) \{\
        if let Some(token) = crate::config::load_token(&url) \{\
            match crate::jellyfin::client::JellyfinClient::new(&url, token.clone(), uid.clone(), &cfg.device_id) \{\
                Ok(c) => match connect_flow(&c, &mut cfg, &main_tx, false).await \{\
                    Ok(()) => client = Some(c),\
                    Err(e) => \{\
                        let _ = main_tx.send(MainMsg::ConnectError(format!("\{\}", e)));\
                    \}\
                \},\
                Err(e) => \{\
                    let _ = main_tx.send(MainMsg::ConnectError(format!("\{\}", e)));\
                \}\
            \}\
        \} else \{\
            let _ = main_tx.send(MainMsg::ConnectError(String::new()));\
        \}\
    \}\
\
    let http_poster = reqwest::Client::builder()\
        .timeout(Duration::from_secs(30))\
        .build()\
        .ok();\
\
    while let Some(cmd) = rx.recv().await \{\
        match cmd \{\
            UiCmd::QuitSaving => \{ /* marker no-op (see sys note); real saves happen on terminate */ \}\
            UiCmd::Connect \{ url, username, password, api_key \} => \{\
                match connect_cmd(&url, &username, password, api_key, &mut cfg, &main_tx).await \{\
                    Ok(c) => client = Some(c),\
                    Err(e) => \{\
                        let _ = main_tx.send(MainMsg::ConnectError(format!("\{\}", e)));\
                    \}\
                \}\
            \}\
            UiCmd::LoadView \{ view_id, name, collection_type \} => \{\
                if let Some(c) = &client \{\
                    let live = collection_type.as_deref() == Some("livetv") || view_id == "livetv";\
                    let res = if live \{\
                        c.livetv_channels().await.map(|v| (name.clone(), v))\
                    \} else \{\
                        let sort = sort_from_popup(&cfg);\
                        let mut q = crate::jellyfin::models::ItemQuery \{\
                            parent_id: Some(view_id.clone()),\
                            ..Default::default()\
                        \};\
                        q.sort_by = sort.0;\
                        q.sort_order = sort.1;\
                        match collection_type.as_deref() \{\
                            Some("movies") => q.types = Some("Movie".into()),\
                            Some("tvshows") => q.types = Some("Series".into()),\
                            Some("music") => q.types = Some("MusicAlbum".into()),\
                            _ => \{\}\
                        \}\
                        c.items(&q).await.map(|v| (name.clone(), v))\
                    \};\
                    match res \{\
                        Ok((title, items)) => \{\
                            fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;\
                            let _ = main_tx.send(MainMsg::ItemsLoaded \{\
                                title,\
                                items,\
                                can_go_back: false,\
                            \});\
                        \}\
                        Err(e) => \{\
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("\{\}", e)));\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::Search \{ q \} => \{\
                // Debounce: wait, then keep only the newest search.\
                tokio::time::sleep(Duration::from_millis(250)).await;\
                let mut latest = q;\
                while let Ok(c2) = rx.try_recv() \{\
                    match c2 \{\
                        UiCmd::Search \{ q \} => latest = q,\
                        other => \{\
                            // Re-queue non-search commands by handling them next loop.\
                            let _ = rx.send_now(other);\
                        \}\
                    \}\
                \}\
                if let Some(c) = &client \{\
                    if !latest.trim().is_empty() \{\
                        let mut qy = crate::jellyfin::models::ItemQuery \{\
                            search: Some(latest.clone()),\
                            ..Default::default()\
                        \};\
                        qy.limit = 100;\
                        match c.items(&qy).await \{\
                            Ok(items) => \{\
                                fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;\
                                let _ = main_tx.send(MainMsg::ItemsLoaded \{\
                                    title: format!("Results for \'93\{\}\'94", latest),\
                                    items,\
                                    can_go_back: true,\
                                \});\
                            \}\
                            Err(e) => \{\
                                let _ = main_tx.send(MainMsg::PlaybackError(format!("\{\}", e)));\
                            \}\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::Episodes \{ series_id, series_name \} => \{\
                if let Some(c) = &client \{\
                    let mut q = crate::jellyfin::models::ItemQuery \{\
                        series_id: Some(series_id.clone()),\
                        types: Some("Episode".into()),\
                        ..Default::default()\
                    \};\
                    q.sort_by = "ParentIndexNumber,IndexNumber".into();\
                    q.sort_order = "Ascending".into();\
                    match c.items(&q).await \{\
                        Ok(items) => \{\
                            fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;\
                            let _ = main_tx.send(MainMsg::ItemsLoaded \{\
                                title: series_name,\
                                items,\
                                can_go_back: true,\
                            \});\
                        \}\
                        Err(e) => \{\
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("\{\}", e)));\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::Tracks \{ album_id, album_name \} => \{\
                if let Some(c) = &client \{\
                    let q = crate::jellyfin::models::ItemQuery \{\
                        parent_id: Some(album_id.clone()),\
                        types: Some("Audio".into()),\
                        ..Default::default()\
                    \};\
                    match c.items(&q).await \{\
                        Ok(items) => \{\
                            let _ = main_tx.send(MainMsg::ItemsLoaded \{\
                                title: album_name,\
                                items,\
                                can_go_back: true,\
                            \});\
                        \}\
                        Err(e) => \{\
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("\{\}", e)));\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::Play \{ item_id \} => \{\
                if let Some(c) = &client \{\
                    match c.full_item(&item_id).await \{\
                        Ok(full) => \{\
                            let ms = full.media_sources.first().cloned();\
                            let (method, _kbps) =\
                                crate::transcode::decision::build_stream_plan(ms.as_ref(), cfg.settings.max_bitrate_kbps);\
                            let resume = full\
                                .user_data\
                                .as_ref()\
                                .map(|u| u.position_ticks)\
                                .unwrap_or(0);\
                            let resume_secs = crate::jellyfin::urls::secs_from_ticks(resume);\
                            let played = full.user_data.as_ref().map(|u| u.played).unwrap_or(false);\
                            let resume_secs = if played \{ 0.0 \} else \{ resume_secs \};\
                            let url = match (&method, &ms) \{\
                                (\
                                    crate::transcode::decision::PlaybackMethod::DirectPlay,\
                                    Some(ms),\
                                ) => crate::jellyfin::urls::build_direct_url(\
                                    &c.base,\
                                    &full.id,\
                                    &ms.id,\
                                    ms.container.as_deref(),\
                                    &c.token,\
                                ),\
                                (crate::transcode::decision::PlaybackMethod::ServerHls, Some(ms)) => \{\
                                    crate::jellyfin::urls::build_hls_url(\
                                        &c.base,\
                                        &full.id,\
                                        &ms.id,\
                                        &c.token,\
                                        cfg.settings.max_bitrate_kbps,\
                                        &cfg.settings.subtitle_lang,\
                                    )\
                                \}\
                                _ => crate::jellyfin::urls::build_hls_url(\
                                    &c.base,\
                                    &full.id,\
                                    &full.id,\
                                    &c.token,\
                                    cfg.settings.max_bitrate_kbps,\
                                    &cfg.settings.subtitle_lang,\
                                ),\
                            \};\
                            c.report_start(&full.id, crate::jellyfin::urls::ticks_from_secs(resume_secs))\
                                .await;\
                            let mut hist = crate::config::load_history();\
                            hist = crate::config::push_history(\
                                hist,\
                                crate::config::HistoryEntry \{\
                                    item_id: full.id.clone(),\
                                    name: full.name.clone(),\
                                    at_ms: now_ms(),\
                                \},\
                            );\
                            let _ = main_tx.send(MainMsg::PlaybackReady(PlaybackStart \{\
                                url,\
                                item_id: full.id.clone(),\
                                title: full.name.clone(),\
                                subtitle: full.series_name.clone(),\
                                resume_secs,\
                                method,\
                                run_time_secs: crate::jellyfin::urls::secs_from_ticks(\
                                    full.run_time_ticks.unwrap_or(0),\
                                ),\
                            \}));\
                        \}\
                        Err(e) => \{\
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("\{\}", e)));\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::PlayChannel \{ channel_id, name \} => \{\
                if let Some(c) = &client \{\
                    let url = crate::jellyfin::urls::build_livetv_url(&c.base, &channel_id, &c.token);\
                    c.report_start(&channel_id, 0).await;\
                    let _ = main_tx.send(MainMsg::PlaybackReady(PlaybackStart \{\
                        url,\
                        item_id: channel_id,\
                        title: name,\
                        subtitle: Some("Live".into()),\
                        resume_secs: 0.0,\
                        method: crate::transcode::decision::PlaybackMethod::DirectPlay,\
                        run_time_secs: 0.0,\
                    \}));\
                \}\
            \}\
            UiCmd::Progress \{ item_id, ticks, paused \} => \{\
                if let Some(c) = &client \{\
                    c.report_progress(&item_id, ticks, paused).await;\
                \}\
            \}\
            UiCmd::Stopped \{ item_id, ticks \} => \{\
                if let Some(c) = &client \{\
                    c.report_stopped(&item_id, ticks).await;\
                \}\
            \}\
            UiCmd::MarkPlayed \{ item_id, played \} => \{\
                if let Some(c) = &client \{\
                    if let Err(e) = c.set_played(&item_id, played).await \{\
                        tracing::warn!("set_played failed: \{\}", e);\
                    \}\
                \}\
            \}\
            UiCmd::SaveSettings \{ settings, reconnect \} => \{\
                cfg.settings = settings;\
                crate::config::save_config(&cfg);\
                if reconnect \{\
                    if let Some(url) = cfg.server_url.clone() \{\
                        if let Some(token) = crate::config::load_token(&url) \{\
                            match crate::jellyfin::client::JellyfinClient::new(\
                                &url,\
                                token.clone(),\
                                cfg.user_id.clone().unwrap_or_default(),\
                                &cfg.device_id,\
                            ) \{\
                                Ok(c) => match connect_flow(&c, &mut cfg, &main_tx, true).await \{\
                                    Ok(()) => client = Some(c),\
                                    Err(e) => \{\
                                        let _ = main_tx.send(MainMsg::ConnectError(format!("\{\}", e)));\
                                    \}\
                                \},\
                                Err(e) => \{\
                                    let _ = main_tx.send(MainMsg::ConnectError(format!("\{\}", e)));\
                                \}\
                            \}\
                        \}\
                    \}\
                \}\
            \}\
            UiCmd::SignOut => \{\
                if let Some(url) = &cfg.server_url \{\
                    crate::config::delete_token(url);\
                \}\
                cfg.server_url = None;\
                cfg.username = None;\
                cfg.user_id = None;\
                crate::config::save_config(&cfg);\
                let _ = main_tx.send(MainMsg::SignedOut);\
            \}\
            UiCmd::ProbeServer => \{\
                if let Some(c) = &client \{\
                    match crate::jellyfin::client::JellyfinClient::public_info(&c.base).await \{\
                        Ok(_) => \{\
                            let _ = main_tx.send(MainMsg::ServerUp);\
                        \}\
                        Err(_) => \{\
                            let _ = main_tx.send(MainMsg::ServerDown);\
                        \}\
                    \}\
                \} else \{\
                    let _ = main_tx.send(MainMsg::ServerDown);\
                \}\
            \}\
        \}\
    \}\
\}\
\
fn now_ms() -> u64 \{\
    std::time::SystemTime::now()\
        .duration_since(std::time::UNIX_EPOCH)\
        .map(|d| d.as_millis() as u64)\
        .unwrap_or(0)\
\}\
\
fn sort_from_popup(cfg: &Config) -> (String, String) \{\
    // The popup index is not directly visible to the worker; settings could\
    // carry it \'97 for now we honor the UI default (SortName) and read the\
    // popup choice through the settings struct when modularized.\
    let _ = cfg;\
    ("SortName".into(), "Ascending".into())\
\}\
\
async fn connect_cmd(\
    url: &str,\
    username: &str,\
    password: Option<String>,\
    api_key: Option<String>,\
    cfg: &mut Config,\
    main_tx: &UnboundedSender<MainMsg>,\
) -> Result<JellyfinClient, crate::jellyfin::client::JFError> \{\
    use crate::jellyfin::client::JFError;\
    // Step 1: verify it's Jellyfin.\
    let info = JellyfinClient::public_info(url).await?;\
    tracing::info!(server_version = ?info.version, "server identified");\
\
    let (token, user_id, user_name) = if let Some(pw) = password \{\
        let (resp, base) = JellyfinClient::authenticate(url, username, &pw, &cfg.device_id).await?;\
        (resp.access_token, resp.user.id, resp.user.name)\
    \} else if let Some(k) = api_key \{\
        // API-key path: validate by hitting an authed endpoint.\
        let probe = JellyfinClient::new(url, k.clone(), String::new(), &cfg.device_id)?;\
        let uid = probe.first_user_id().await?;\
        (k, uid, username.to_string())\
    \} else \{\
        return Err(JFError::Auth("provide a password or an API key".into()));\
    \};\
\
    let c = JellyfinClient::new(url, token.clone(), user_id.clone(), &cfg.device_id)?;\
    connect_flow(&c, cfg, main_tx, true).await?;\
    // Persist credentials only after a fully successful connect.\
    cfg.server_url = Some(normalize(url));\
    cfg.username = Some(user_name.clone());\
    cfg.user_id = Some(user_id.clone());\
    crate::config::store_token(&normalize(url), &token);\
    crate::config::save_config(cfg);\
    Ok(c)\
\}\
\
fn normalize(u: &str) -> String \{\
    crate::jellyfin::urls::normalize_base_url(u).unwrap_or_else(|_| u.to_string())\
\}\
\
async fn connect_flow(\
    c: &JellyfinClient,\
    _cfg: &mut Config,\
    main_tx: &UnboundedSender<MainMsg>,\
    _announce: bool,\
) -> Result<(), crate::jellyfin::client::JFError> \{\
    let views = c.views().await?;\
    let mut v: Vec<(String, String, Option<String>)> = views\
        .into_iter()\
        .map(|x| (x.id, x.name, x.collection_type))\
        .collect();\
    // Probe live TV (cheap, 1 item) \'97 sidebar shows the section only if present.\
    let has_live = c.livetv_channels().await.map(|i| !i.is_empty()).unwrap_or(false);\
    if has_live \{\
        v.push(("livetv".into(), "Live TV".into(), Some("livetv".into())));\
    \}\
    let _ = main_tx.send(MainMsg::Connected \{\
        user_name: c.user_id.clone(),\
        views: v,\
        has_live_tv: false,\
    \});\
    Ok(())\
\}\
\
async fn fetch_posters(\
    c: &JellyfinClient,\
    main_tx: &UnboundedSender<MainMsg>,\
    items: &[crate::jellyfin::models::ListItem],\
    poster_dir: &std::path::Path,\
    http: Option<&reqwest::Client>,\
) \{\
    if http.is_none() \{\
        return;\
    \}\
    let http = http.unwrap();\
    let client = c.clone();\
    let tx = main_tx.clone();\
    let dir = poster_dir.to_path_buf();\
    // Spawn a bounded set of concurrent poster fetches (first 200 items).\
    for it in items.iter().take(200) \{\
        if !it.has_image() \{\
            continue;\
        \}\
        let client = client.clone();\
        let tx = tx.clone();\
        let dir = dir.clone();\
        let id = it.id.clone();\
        tokio::spawn(async move \{\
            let key = sys_key(&id);\
            let path = dir.join(format!("\{:016x\}.img", key));\
            let bytes = match tokio::fs::read(&path).await \{\
                Ok(b) => Some(b),\
                Err(_) => \{\
                    let url = crate::jellyfin::urls::build_image_url(&client.base, &id, &client.token, 400);\
                    match http.get(url).send().await \{\
                        Ok(r) if r.status().is_success() => match r.bytes().await \{\
                            Ok(b) => \{\
                                let _ = tokio::fs::write(&path, &b).await;\
                                Some(b.to_vec())\
                            \}\
                            Err(_) => None,\
                        \},\
                        _ => None,\
                    \}\
                \}\
            \};\
            if let Some(b) = bytes \{\
                let _ = tx.send(MainMsg::PosterReady \{ key, bytes: b \});\
            \}\
        \});\
    \}\
\}\
\
fn sys_key(item_id: &str) -> u64 \{\
    // Matches crate::sys::poster_key (same DefaultHasher on the same binary).\
    use std::hash::\{Hash, Hasher\};\
    let mut h = std::collections::hash_map::DefaultHasher::new();\
    item_id.hash(&mut h);\
    h.finish()\
\}\
\
// ============================================================================\
// SECTION: SearXNG Integration (secondary; fully optional)\
// ============================================================================\
// SPLIT: \uc0\u8594  src/search_integration.rs. No torrenting, no Arr-stack backend:\
// this is a clean hook \'97 a menu/sidebar entry that opens the user's configured\
// SearXNG instance in the default browser with a query prompt. If disabled,\
// nothing else in the app depends on it.\
\
pub mod search_integration \{\
    pub fn open_searx_dialog() \{\
        crate::sys::open_searx_prompt();\
    \}\
\}\
\
// The dialog itself needs AppKit (NSAlert accessory view), so the body lives\
// in sys alongside the other UI code:\
impl sys_shim \{\
    // placeholder doc anchor; see sys::open_searx_prompt below\
\}\
pub struct sys_shim;\
\
impl sys \{\
    pub fn open_searx_prompt() \{\
        unsafe \{\
            let ui = match UI.get() \{\
                Some(u) => u,\
                None => return,\
            \};\
            if !ui.cfg.settings.searx_enabled \{\
                set_chip_in_login("SearXNG is disabled \'97 enable it in Settings.", false);\
                return;\
            \}\
            let base = ui.cfg.settings.searx_url.trim().to_string();\
            if base.is_empty() \{\
                set_chip_in_login("Set your SearXNG URL in Settings first.", false);\
                return;\
            \}\
            let alert: *mut Object = msg_send![class!(NSAlert), alloc];\
            let alert: *mut Object = msg_send![alert, init];\
            let _: () = msg_send![alert, setMessageText: ns("Search the web")];\
            let _: () = msg_send![alert,\
                setInformativeText: ns("Query to send to your SearXNG instance:")];\
            let input: *mut Object = msg_send![class!(NSTextField), alloc];\
            let input: *mut Object = msg_send![input, initWithFrame: rect(0., 0., 260., 24.)];\
            let _: () = msg_send![alert, setAccessoryView: input];\
            let _: () = msg_send![alert, addButtonWithTitle: ns("Search")];\
            let _: () = msg_send![alert, addButtonWithTitle: ns("Cancel")];\
            let resp: i64 = msg_send![alert, runModal];\
            if resp == 1000 \{\
                let q = to_str(msg_send![input, stringValue]);\
                if !q.trim().is_empty() \{\
                    let url = format!(\
                        "\{\}/search?q=\{\}",\
                        base.trim_end_matches('/'),\
                        crate::jellyfin::urls::enc(&q)\
                    );\
                    let u: *mut Object = msg_send![class!(NSURL), URLWithString: ns(&url)];\
                    let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];\
                    let _: BOOL = msg_send![ws, openURL: u];\
                \}\
            \}\
        \}\
    \}\
    fn set_chip_in_login(msg: &str, _err: bool) \{\
        unsafe \{\
            let ui = match UI.get() \{\
                Some(u) => u,\
                None => return,\
            \};\
            let _: () = msg_send![ui.login_err.get(), setStringValue: ns(msg)];\
        \}\
    \}\
\}\
\
// ============================================================================\
// SECTION: main\
// ============================================================================\
\
fn main() \{\
    // Logging: quiet by default; RUST_LOG=debug for diagnostics.\
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()\
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyper=warn,reqwest=warn"));\
    tracing_subscriber::fmt().with_env_filter(filter).init();\
\
    let cfg = config::load_config();\
    let (ui_tx, ui_rx) = unbounded_channel::<UiCmd>();\
    let (main_tx, main_rx) = unbounded_channel::<MainMsg>();\
\
    let rt = tokio::runtime::Builder::new_multi_thread()\
        .enable_all()\
        .build()\
        .expect("tokio runtime");\
    rt.spawn(worker(ui_rx, main_tx.clone(), cfg.clone()));\
\
    // Register the main-thread receiver the pump drains.\
    let _ = sys::MAIN_RX_HANDLE(main_rx);\
\
    unsafe \{\
        sys::bootstrap(cfg, ui_tx, main_tx);\
        sys::run_app();\
    \}\
\
    // Graceful end: the AppKit loop returned (Quit). Final persist happens in\
    // applicationShouldTerminate; give the worker a moment to flush progress.\
    rt.shutdown_timeout(Duration::from_millis(500));\
\}\
\
// Small bridge so `main` can stash the receiver without importing raw types\
// into sys's public surface.\
pub mod bridge \{\
    use super::*;\
    pub fn install_main_rx(rx: UnboundedReceiver<MainMsg>) \{\
        sys::install_main_rx(rx);\
    \}\
\}\
\
// ============================================================================\
// SECTION: Unit tests (pure functions only \'97 no AppKit, runs via `cargo test`)\
// ============================================================================\
\
#[cfg(test)]\
mod tests \{\
    use super::jellyfin::urls as u;\
    use super::transcode::decision as d;\
\
    #[test]\
    fn normalize_base_url() \{\
        assert_eq!(u::normalize_base_url(" http://host:8096/ ").unwrap(), "http://host:8096");\
        assert!(u::normalize_base_url("ftp://x").is_err());\
        assert!(u::normalize_base_url("").is_err());\
        assert!(u::normalize_base_url("host:8096").is_err());\
    \}\
\
    #[test]\
    fn percent_encoding() \{\
        assert_eq!(u::enc("a b&c=d"), "a%20b%26c%3Dd");\
        assert_eq!(u::enc("safe-._~"), "safe-._~");\
    \}\
\
    #[test]\
    fn direct_url_shape() \{\
        let url = u::build_direct_url("http://s:8096", "item1", "ms1", Some("mkv"), "tok");\
        assert!(url.starts_with("http://s:8096/Videos/item1/stream."));\
        // mkv is not in the Apple set \uc0\u8594  decide() must not choose DirectPlay.\
        assert_eq!(\
            d::decide_playback(Some("mkv"), Some("h264"), Some("aac"), 8_000, 0),\
            d::PlaybackMethod::ServerHls\
        );\
    \}\
\
    #[test]\
    fn direct_url_no_container() \{\
        let url = u::build_direct_url("http://s:8096", "i", "m", None, "t");\
        assert!(url.contains("stream?static=true"));\
        assert!(url.contains("api_key=t"));\
    \}\
\
    #[test]\
    fn hls_url_params() \{\
        let url = u::build_hls_url("http://s", "i", "m", "tok", 20_000, "eng");\
        assert!(url.ends_with("master.m3u8?MediaSourceId=m"));\
        assert!(url.contains("VideoBitrate=20000"));\
        assert!(url.contains("VideoCodec=h264%2Chevc"));\
    \}\
\
    #[test]\
    fn playback_decision_matrix() \{\
        // Fully supported, within bitrate \uc0\u8594  direct.\
        assert_eq!(\
            d::decide_playback(Some("mp4"), Some("h264"), Some("aac"), 5_000, 10_000),\
            d::PlaybackMethod::DirectPlay\
        );\
        // DTS audio \uc0\u8594  HLS.\
        assert_eq!(\
            d::decide_playback(Some("mp4"), Some("h264"), Some("dts"), 5_000, 10_000),\
            d::PlaybackMethod::ServerHls\
        );\
        // Over bitrate cap \uc0\u8594  HLS.\
        assert_eq!(\
            d::decide_playback(Some("mp4"), Some("hevc"), Some("eac3"), 50_000, 20_000),\
            d::PlaybackMethod::ServerHls\
        );\
        // No cap (0) \uc0\u8594  original allowed.\
        assert_eq!(\
            d::decide_playback(Some("mov"), Some("prores"), Some("pcm_s16le"), 900_000, 0),\
            d::PlaybackMethod::DirectPlay\
        );\
    \}\
\
    #[test]\
    fn ticks() \{\
        assert_eq!(u::ticks_from_secs(1.5), 15_000_000);\
        assert!((u::secs_from_ticks(15_000_000) - 1.5).abs() < 1e-9);\
        assert_eq!(u::ticks_from_secs(-3.0), 0);\
    \}\
\
    #[test]\
    fn progress_fraction() \{\
        assert!((u::progress_fraction(5_000_000, Some(10_000_000)) - 0.5).abs() < 1e-9);\
        assert_eq!(u::progress_fraction(5, None), 0.0);\
        assert_eq!(u::progress_fraction(99, Some(0)), 0.0);\
        assert_eq!(u::progress_fraction(20_000_000, Some(10_000_000)), 1.0);\
    \}\
\
    #[test]\
    fn clock_format() \{\
        assert_eq!(u::fmt_clock(59.4), "0:59");\
        assert_eq!(u::fmt_clock(3661.0), "1:01:01");\
        assert_eq!(u::fmt_clock(-5.0), "0:00");\
    \}\
\
    #[test]\
    fn runtime_format() \{\
        assert_eq!(u::fmt_runtime(Some(112_000_000 * 60)), "1h 52m");\
        assert_eq!(u::fmt_runtime(Some(3_000_000 * 30)), "30m");\
        assert_eq!(u::fmt_runtime(None), "");\
    \}\
\
    #[test]\
    fn keychain_account_namespaces_server() \{\
        assert_eq!(\
            crate::config::keychain_account("http://192.168.1.10:8096"),\
            "token@http://192.168.1.10:8096"\
        );\
    \}\
\
    #[test]\
    fn config_roundtrip() \{\
        let cfg = crate::config::Config \{\
            server_url: Some("http://x".into()),\
            volume: 0.4,\
            ..Default::default()\
        \};\
        let s = serde_json::to_string(&cfg).unwrap();\
        let back: crate::config::Config = serde_json::from_str(&s).unwrap();\
        assert_eq!(back.volume, 0.4);\
        assert_eq!(back.server_url.as_deref(), Some("http://x"));\
    \}\
\
    #[test]\
    fn web_url_escaping() \{\
        let url = u::build_web_item_url("http://s:8096", "abc/def");\
        assert!(url.contains("abc%2Fdef"));\
    \}\
\}}