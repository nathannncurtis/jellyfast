//! ============================================================================
//! Jellyfin macOS Client — Single-file baseline ("jellyfin-mac")
//! ============================================================================
//!
//! Build:
//!   cargo build --release
//!   (create Cargo.toml exactly as shown in the CARGO.TOML comment below;
//!    macOS 14+ on aarch64/x86_64, current stable Rust 1.75+)
//!
//! Required system frameworks / capabilities:
//!   AppKit, AVFoundation, AVKit, CoreMedia, Security  — all linked
//!   automatically via #[link] stubs in `mod sys` and the `security-framework`
//!   crate. No provisioning/capabilities needed. App Sandbox is NOT required;
//!   if you enable it, add Keychain access + network client entitlements.
//!
//!   Info.plist notes (you handle packaging):
//!     - CFBundleIdentifier should be a stable reverse-DNS id (device_id and
//!       keychain entries are namespaced by strings in config.rs, not the
//!       bundle id, so changing it later is safe).
//!     - If your Jellyfin server is plain HTTP, ATS will block AVPlayer media
//!       loads. Add NSAppTransportSecurity → NSAllowsArbitraryLoads = YES (or
//!       a per-domain exception) to the Info.plist. The Rust HTTP layer is
//!       unaffected (rustls, no ATS involvement).
//!
//! Minimal configuration:
//!   None required on first launch: the app shows a login card. Enter the
//!   server base URL (e.g. http://192.168.1.10:8096) and either
//!   (a) username + password (recommended; a Jellyfin access token is then
//!       stored in the macOS Keychain), or
//!   (b) an API key (dashboard → API Keys). With an API key the client picks
//!       the first user account for library browsing (documented limitation).
//!   The last successful server/user is remembered in
//!   ~/Library/Application Support/JellyfinMac/config.json; the token lives in
//!   the Keychain (service "app.jellyfin.mac"). RUST_LOG=debug overrides the
//!   default `info` log filter.
//!
//! Player stack choice (and why):
//!   AVKit's AVPlayerView + AVPlayer, driven from a pure-Rust control layer.
//!   This is the "thin binding to AVFoundation/VideoToolbox" option: Apple's
//!   stack performs demuxing, hardware decode (VideoToolbox), audio routing,
//!   subtitles, scrubbing, PiP and fullscreen natively, so the binary stays
//!   tiny and the UI is 100% native (vibrancy, SF Symbols, dark/light).
//!   "Conversion" on-device therefore means: anything within Apple's codec
//!   set (h264/hevc/prores/mpeg4, aac/mp3/ac3/eac3/alac/flac/pcm) direct-plays
//!   with hardware decode — no server transcode. Sources outside that set
//!   (e.g. mkv/DTS/AV1-on-old-Macs) fall back to a server HLS transcode URL;
//!   a fully client-side VideoToolbox transcode pipeline in pure Rust would
//!   require thousands of lines of CoreMedia FFI and is the main known
//!   limitation (see `mod transcode` comments and KNOWN LIMITATIONS below).
//!
//! Architecture notes for future modularization (concrete map):
//!   src/config/mod.rs        ← SECTION: Configuration & Secure Storage
//!   src/storage/keychain.rs  ← keychain helpers inside that section
//!   src/storage/cache.rs     ← poster disk-cache helpers (worker side)
//!   src/jellyfin/client.rs   ← SECTION: Jellyfin API Client (struct impl)
//!   src/jellyfin/models.rs   ← serde models in that section
//!   src/jellyfin/urls.rs     ← pure URL builders in that section
//!   src/transcode/decision.rs← SECTION: Playback Decision & Transcoding
//!   src/platform/appkit.rs   ← SECTION: macOS Bindings (`mod sys`, unsafe)
//!   src/platform/avkit.rs    ← AVPlayerView/player wiring inside `mod sys`
//!   src/ui/shell.rs          ← SECTION: UI Construction (window/menus/sidebar)
//!   src/ui/library.rs        ← poster grid & cells inside UI section
//!   src/ui/login.rs, settings.rs ← login card & settings sheet in UI section
//!   src/app/controller.rs    ← SECTION: Application Controller (message pump)
//!   src/app/worker.rs        ← SECTION: Background Worker (tokio loop)
//!   src/search_integration.rs← SECTION: SearXNG Integration
//!   `mod sys` is the only unsafe code; everything else is safe Rust under
//!   #![deny(unsafe_code)].
//!
//! Known limitations of this single-file version (honest):
//!   - Codec set = Apple's (see player stack choice); unsupported codecs use
//!     a server HLS transcode fallback. AV1 only on M3+ via Apple's decoder.
//!   - No playback queue; "play next episode" not implemented.
//!   - External (sidecar .srt) subtitles only surface when the server-side
//!     HLS path delivers WebVTT; embedded tracks are selectable natively.
//!   - Live TV plays the raw channel TS stream (best-effort, format-limited).
//!   - API-key login picks the first user (Jellyfin has no /Users/Me for keys).
//!   - Music playback is one track/album at a time (no queue/gapless).
//!   - Network-error recovery: automatic retry with backoff (3 attempts),
//!     not a full offline/re-sync engine.
//!   - On Intel Macs, CMTime (24-byte struct) returns go through
//!     objc_msgSend_stret; the `objc` crate handles this, but it is the one
//!     ABI-sensitive spot — keep the pinned objc version on Intel.
//!   - Window state, volume, theme, quality etc. persist; PiP uses the
//!     native AVKit button.
//!
//! ===========================================================================
//! CARGO.TOML (pin exactly these; objc-ecosystem APIs are version-sensitive)
//! ===========================================================================
//! [package]
//! name = "jellyfin-mac"
//! version = "0.1.0"
//! edition = "2021"
//!
//! [dependencies]
//! objc = "0.2.7"                       # Objective-C runtime bindings (tiny, stable)
//! tokio = { version = "1", features = ["rt-multi-thread", "time", "sync"] }
//! reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! tracing = "0.1"
//! tracing-subscriber = { version = "0.3", features = ["env-filter"] }
//! security-framework = "2"             # Keychain (thin binding to Security.framework)
//!
//! # HEAVY-DEPENDENCY JUSTIFICATION (each one, with lighter alternatives):
//! # tokio   — mandated by the spec for async IO. Alternative: std::thread +
//! #           ureq (blocking). Heavier but required here.
//! # reqwest — mandated. rustls-tls avoids OpenSSL/libssl entirely. Lighter
//! #           alternative considered: `ureq` (~10x smaller, blocking) — would
//! #           work fine behind tokio::spawn_blocking, kept reqwest per spec.
//! # security-framework — the only sane Keychain path without hand-rolled
//! #           SecItem FFI (which would add ~200 lines of unsafe). Kept.
//! # objc    — required to talk to AppKit/AVKit. `objc2` is the successor and
//! #           is nicer (typed), but its 0.5→0.6 API churn makes single-shot
//! #           generated code fragile; objc 0.2.7 is frozen and rock-solid.
//! #           When modularizing, MIGRATE mod sys → objc2 (mechanical).
//! # tracing/tracing-subscriber — spec-mandated logging.
//! # serde/serde_json — spec-mandated (REST JSON).
//! # REJECTED: cocoa (deprecated superset of what we need — we hand-roll the
//! #           few geometry structs), block2/dispatch (avoided by using
//! #           target/selector instead of blocks everywhere), objc_id (we use
//! #           a 15-line Ptr RAII wrapper), image crates (NSImage decodes).
//!
//! [profile.release]
//! lto = true
//! strip = true
//! panic = "abort"
//! opt-level = "z"          # small binary; change to 3 if you want max speed
//! ============================================================================

#![deny(unsafe_code)]
// objc 0.2's sel!/msg_send! macros emit cfg(feature = "cargo-clippy"), which new rustc flags.
#![allow(unexpected_cfgs)]

use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::config::{load_config, save_config, Config};
use crate::jellyfin::client::JellyfinClient;
use crate::jellyfin::urls::enc;
use crate::transcode::decision::{build_stream_plan, PlaybackMethod};

pub mod app;
pub mod config;
pub mod jellyfin;
pub mod platform;
pub mod search_integration;
pub mod seerr;
pub mod storage;
pub mod transcode;
pub mod ui;

use app::messages::{MainMsg, PlaybackStart, Settings, UiCmd};

// ============================================================================
// SECTION: main
// ============================================================================

#[allow(unsafe_code)]
fn main() {
    // Logging: quiet by default; RUST_LOG=debug for diagnostics.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyper=warn,reqwest=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cfg = config::load_config();
    let (ui_tx, ui_rx) = unbounded_channel::<UiCmd>();
    let (main_tx, main_rx) = unbounded_channel::<MainMsg>();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    // Worker results arrive on `worker_tx`; the forwarder moves them into the main
    // queue and kicks the main-thread pump so the UI drains them promptly.
    let (worker_tx, mut worker_rx) = unbounded_channel::<MainMsg>();
    let _ = platform::appkit::WORKER_TX.set(worker_tx.clone());
    {
        let main_tx = main_tx.clone();
        rt.spawn(async move {
            while let Some(m) = worker_rx.recv().await {
                if main_tx.send(m).is_ok() {
                    platform::appkit::on_main(app::controller::drain_main);
                }
            }
        });
    }
    if app::selftest::enabled() {
        // Self-test: no worker / network; outgoing UiCmds are only logged.
        let mut ui_rx = ui_rx;
        rt.spawn(async move {
            while let Some(c) = ui_rx.recv().await {
                println!("[selftest]   UiCmd -> {:?}", c);
            }
        });
    } else {
        rt.spawn(app::worker::worker(ui_rx, worker_tx.clone(), cfg.clone()));
    }

    // Register the main-thread receiver the pump drains.
    platform::appkit::install_main_rx(main_rx);

    // SAFETY: bootstrap runs once, on the main thread, before the AppKit loop starts.
    #[allow(unsafe_code)]
    unsafe {
        platform::appkit::bootstrap(cfg, ui_tx, main_tx);
    }
    platform::appkit::run_app();

    // Graceful end: the AppKit loop returned (Quit). Final persist happens in
    // applicationShouldTerminate; give the worker a moment to flush progress.
    rt.shutdown_timeout(Duration::from_millis(500));
}

// ============================================================================
// SECTION: Unit tests (pure functions only — no AppKit, runs via `cargo test`)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::jellyfin::urls as u;
    use super::transcode::decision as d;

    #[test]
    fn normalize_base_url() {
        assert_eq!(u::normalize_base_url(" http://host:8096/ ").unwrap(), "http://host:8096");
        assert!(u::normalize_base_url("ftp://x").is_err());
        assert!(u::normalize_base_url("").is_err());
        assert!(u::normalize_base_url("host:8096").is_err());
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(u::enc("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(u::enc("safe-._~"), "safe-._~");
    }

    #[test]
    fn direct_url_shape() {
        let url = u::build_direct_url("http://s:8096", "item1", "ms1", Some("mkv"), "tok");
        assert!(url.starts_with("http://s:8096/Videos/item1/stream."));
        // mkv is not in the Apple set → decide() must not choose DirectPlay.
        assert_eq!(
            d::decide_playback(Some("mkv"), Some("h264"), Some("aac"), 8_000, 0),
            d::PlaybackMethod::ServerHls
        );
    }

    #[test]
    fn direct_url_no_container() {
        let url = u::build_direct_url("http://s:8096", "i", "m", None, "t");
        assert!(url.contains("stream?static=true"));
        assert!(url.contains("api_key=t"));
    }

    #[test]
    fn hls_url_params() {
        let url = u::build_hls_url("http://s", "i", "m", "tok", 20_000, "eng");
        assert!(url.contains("master.m3u8?MediaSourceId=m"));
        assert!(url.contains("VideoBitrate=20000"));
        assert!(url.contains("VideoCodec=h264%2Chevc"));
    }

    #[test]
    fn playback_decision_matrix() {
        // Fully supported, within bitrate → direct.
        assert_eq!(
            d::decide_playback(Some("mp4"), Some("h264"), Some("aac"), 5_000, 10_000),
            d::PlaybackMethod::DirectPlay
        );
        // DTS audio → HLS.
        assert_eq!(
            d::decide_playback(Some("mp4"), Some("h264"), Some("dts"), 5_000, 10_000),
            d::PlaybackMethod::ServerHls
        );
        // Over bitrate cap → HLS.
        assert_eq!(
            d::decide_playback(Some("mp4"), Some("hevc"), Some("eac3"), 50_000, 20_000),
            d::PlaybackMethod::ServerHls
        );
        // No cap (0) → original allowed.
        assert_eq!(
            d::decide_playback(Some("mov"), Some("prores"), Some("pcm_s16le"), 900_000, 0),
            d::PlaybackMethod::DirectPlay
        );
    }

    #[test]
    fn ticks() {
        assert_eq!(u::ticks_from_secs(1.5), 15_000_000);
        assert!((u::secs_from_ticks(15_000_000) - 1.5).abs() < 1e-9);
        assert_eq!(u::ticks_from_secs(-3.0), 0);
    }

    #[test]
    fn progress_fraction() {
        assert!((u::progress_fraction(5_000_000, Some(10_000_000)) - 0.5).abs() < 1e-9);
        assert_eq!(u::progress_fraction(5, None), 0.0);
        assert_eq!(u::progress_fraction(99, Some(0)), 0.0);
        assert_eq!(u::progress_fraction(20_000_000, Some(10_000_000)), 1.0);
    }

    #[test]
    fn clock_format() {
        assert_eq!(u::fmt_clock(59.4), "0:59");
        assert_eq!(u::fmt_clock(3661.0), "1:01:01");
        assert_eq!(u::fmt_clock(-5.0), "0:00");
    }

    #[test]
    fn runtime_format() {
        assert_eq!(u::fmt_runtime(Some(112 * 60 * 10_000_000)), "1h 52m");
        assert_eq!(u::fmt_runtime(Some(30 * 60 * 10_000_000)), "30m");
        assert_eq!(u::fmt_runtime(None), "");
    }

    #[test]
    fn keychain_account_namespaces_server() {
        assert_eq!(
            crate::config::keychain_account("http://192.168.1.10:8096"),
            "token@http://192.168.1.10:8096"
        );
    }

    #[test]
    fn config_roundtrip() {
        let cfg = crate::config::Config {
            server_url: Some("http://x".into()),
            volume: 0.4,
            ..Default::default()
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: crate::config::Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back.volume, 0.4);
        assert_eq!(back.server_url.as_deref(), Some("http://x"));
    }

    #[test]
    fn web_url_escaping() {
        let url = u::build_web_item_url("http://s:8096", "abc/def");
        assert!(url.contains("abc%2Fdef"));
    }
}
