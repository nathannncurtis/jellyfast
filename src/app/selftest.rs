//! Headless UI self-test (JELLYFIN_MAC_SELFTEST=1). Inert unless the env var is set.
//!
//! Drives every UI code path with fake data and no server: MainMsg values go
//! through `handle_main` (the same reducer the worker results use), controller
//! actions are invoked through real Objective-C dispatch on the JRCtl object,
//! and outgoing UiCmds are logged by a drain task in main.rs. Steps run one per
//! run-loop turn (optionally delayed) so AppKit layout/timers/KVO really fire.
//! Video: $JELLYFIN_MAC_SELFTEST_VIDEO, else assets/selftest.mp4 (a plain gradient clip
//! with two audio tracks and one subtitle track).
#![allow(unsafe_code)]
use crate::app::controller::handle_main;
use crate::app::messages::*;
use crate::config::Config;
use crate::jellyfin::models::{ImageTags, ListItem, UserData};
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use crate::transcode::decision::PlaybackMethod;
use std::sync::Mutex;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use crate::ui::library::poster_key;
use objc::msg_send;
use objc::runtime::{Object, BOOL, NO, YES};
use objc::{class, sel, sel_impl};

pub fn enabled() -> bool {
    std::env::var("JELLYFIN_MAC_SELFTEST").map(|v| v == "1").unwrap_or(false)
}

type Step = (&'static str, fn(), u64);

static CMD_RX: Mutex<Option<UnboundedReceiver<UiCmd>>> = Mutex::new(None);

/// Take (and log) every UiCmd emitted since the last call.
fn take_cmds() -> Vec<UiCmd> {
    let mut out = vec![];
    if let Some(rx) = CMD_RX.lock().unwrap().as_mut() {
        while let Ok(c) = rx.try_recv() {
            println!("[selftest]   UiCmd -> {:?}", c);
            out.push(c);
        }
    }
    out
}

fn log(s: &str) {
    println!("[selftest] {}", s);
}

/// Called once from applicationDidFinishLaunching after the UI is built.
pub fn start() {
    log("start");
    // Intercept outgoing UiCmds so steps can assert on them (they are still logged).
    unsafe {
        let ui = UI.get_mut().unwrap();
        let (tx, rx) = unbounded_channel::<UiCmd>();
        ui.ui_tx = tx;
        *CMD_RX.lock().unwrap() = Some(rx);
    }
    next(0);
}

fn next(i: usize) {
    let steps = steps();
    if i >= steps.len() {
        log("ALL STEPS DONE");
        unsafe {
            let ui = UI.get_mut().unwrap();
            let _: () = msg_send![ui.app.get(), terminate: nil()];
        }
        return;
    }
    let (name, f, delay) = steps[i];
    log(&format!("step {:02} {}", i, name));
    f();
    take_cmds(); // log anything the step emitted
    // Let the run loop turn (and timers/KVO fire) before the next step.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay.max(30)));
        on_main(move || next(i + 1));
    });
}

fn steps() -> Vec<Step> {
    vec![
        ("login: empty connect", s_login_empty, 50),
        ("login: filled connect", s_login_filled, 50),
        ("connect error", s_connect_error, 50),
        ("connected (views + livetv)", s_connected, 100),
        ("connected again (sidebar rebuild)", s_connected, 100),
        ("items: movies", s_items_movies, 100),
        ("worker->UI path (WORKER_TX)", s_worker_path, 400),
        ("worker->UI path check", s_worker_check, 50),
        ("posters (png, jpeg, garbage)", s_posters, 100),
        ("resize window", s_resize, 200),
        ("hover cells", s_hover, 50),
        ("sidebar click", s_sidebar_click, 50),
        ("items: series", s_items_series, 100),
        ("items: episodes", s_items_episodes, 100),
        ("items: audio", s_items_audio, 100),
        ("items: empty", s_items_empty, 50),
        ("items: many (grid scroll)", s_items_many, 100),
        ("resize window (many)", s_resize, 200),
        ("item clicks", s_item_clicks, 50),
        ("sort/search/refresh/back", s_nav_actions, 50),
        ("mark watched / open web", s_misc_actions, 50),
        ("stop when not playing", s_stop_idle, 50),
        ("playback error (idle)", s_playback_error, 50),
        ("server down/up while idle", s_server_idle, 50),
        ("seerr: open while disabled (inline hint)", s_seerr_disabled, 100),
        ("seerr: open while enabled", s_seerr_enabled, 100),
        ("seerr: results (all availabilities)", s_seerr_results, 100),
        ("seerr: type in search field", s_seerr_typing, 50),
        ("seerr: click Request", s_seerr_request, 100),
        ("seerr: requested + error", s_seerr_replies, 100),
        ("seerr: My requests tab", s_seerr_requests_tab, 100),
        ("seerr: resize window", s_seerr_resize, 150),
        ("seerr: close and reopen", s_seerr_reopen, 100),
        ("seerr: empty results", s_seerr_empty, 100),
        ("seerr: settings save (open)", s_seerr_settings_open, 700),
        ("seerr: settings status failure", s_seerr_settings_fail, 100),
        ("seerr: settings status ok", s_seerr_settings_ok, 100),
        ("seerr: settings save while disabled", s_seerr_settings_off, 700),
        ("seerr: settings API-key mode", s_seerr_settings_key, 700),
        ("seerr: wrap up", s_seerr_end, 50),
        ("settings: open+cancel", s_settings_cancel, 600),
        ("settings: open+save", s_settings_save, 600),
        ("playback: start", s_play_start, 2500),
        ("playback: controls", s_play_controls, 300),
        ("playback: subtitle/audio cycle", s_play_cycle, 300),
        ("playback: speed", s_play_speed, 300),
        ("playback: server down/up/error", s_play_net, 500),
        ("playback: replace item", s_play_start, 1500),
        ("playback: stop", s_play_stop, 200),
        ("fullscreen toggle x2", s_fullscreen, 2500),
        ("playback: run to end (auto mark watched)", s_play_to_end, 8000),
        ("signed out", s_signed_out, 100),
        ("quit", s_quit_prep, 50),
    ]
}

// ---- helpers --------------------------------------------------------------
unsafe fn ctl() -> *mut Object {
    UI.get_mut().unwrap().ctl.get()
}
unsafe fn act(sel: objc::runtime::Sel, sender: *mut Object) {
    let _: *mut Object = msg_send![ctl(), performSelector: sel withObject: sender];
}
unsafe fn tagged_button(tag: isize) -> *mut Object {
    let b: *mut Object = msg_send![class!(NSButton), new];
    let _: () = msg_send![b, setTag: tag];
    let _: *mut Object = msg_send![b, autorelease];
    b
}
fn item(id: &str, name: &str, ty: &str, played: bool, pos: i64, run: i64) -> ListItem {
    ListItem {
        id: id.into(),
        name: name.into(),
        item_type: ty.into(),
        series_name: if ty == "Episode" { Some("Some Show".into()) } else { None },
        year: Some(1999),
        season: Some(1),
        episode: Some(2),
        user_data: Some(UserData { played, position_ticks: pos }),
        aspect: Some(0.667),
        image_tags: Some(ImageTags { primary: Some("tag".into()) }),
        run_time_ticks: Some(run),
    }
}
fn items_of(ty: &str, n: usize) -> Vec<ListItem> {
    (0..n)
        .map(|i| {
            item(
                &format!("{}-{}", ty, i),
                &format!("{} number {} with a rather long title to truncate", ty, i),
                ty,
                i % 3 == 0,
                if i % 2 == 0 { 3_000_000_000 } else { 0 },
                6_000_000_000,
            )
        })
        .collect()
}
fn send_items(title: &str, items: Vec<ListItem>, back: bool) {
    handle_main(MainMsg::ItemsLoaded { title: title.into(), items, can_go_back: back });
}
fn image_bytes(kind: usize, w: usize, h: usize, shade: u8) -> Vec<u8> {
    unsafe {
        let rep: *mut Object = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: *mut Object = msg_send![rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
            pixelsWide: w as isize pixelsHigh: h as isize bitsPerSample: 8isize
            samplesPerPixel: 4isize hasAlpha: YES isPlanar: NO
            colorSpaceName: ns("NSDeviceRGBColorSpace")
            bytesPerRow: (w * 4) as isize bitsPerPixel: 32isize];
        let px: *mut u8 = msg_send![rep, bitmapData];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                *px.add(o) = (x * 255 / w) as u8;
                *px.add(o + 1) = (y * 255 / h) as u8;
                *px.add(o + 2) = shade;
                *px.add(o + 3) = 255;
            }
        }
        let props: *mut Object = msg_send![class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypeJPEG = 3, PNG = 4
        let data: *mut Object = msg_send![rep, representationUsingType: if kind == 0 { 4usize } else { 3usize } properties: props];
        let len: usize = msg_send![data, length];
        let p: *const u8 = msg_send![data, bytes];
        let v = std::slice::from_raw_parts(p, len).to_vec();
        let _: () = msg_send![rep, release];
        v
    }
}
fn video_url() -> String {
    let path = std::env::var("JELLYFIN_MAC_SELFTEST_VIDEO").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/selftest.mp4").into()
    });
    let enc: Vec<String> = path.split('/').map(crate::jellyfin::urls::enc).collect();
    format!("file://{}", enc.join("/"))
}
fn playback(resume: f64, run: f64) -> MainMsg {
    MainMsg::PlaybackReady(PlaybackStart {
        url: video_url(),
        item_id: "movie-1".into(),
        title: "Fake Movie".into(),
        subtitle: Some("Subtitle line".into()),
        resume_secs: resume,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: run,
    })
}
fn state_check(what: &str, ok: bool) {
    log(&format!("  check {}: {}", what, if ok { "ok" } else { "FAILED" }));
    if !ok {
        std::process::exit(2);
    }
}

// ---- steps ----------------------------------------------------------------
fn s_login_empty() {
    unsafe { act(sel!(connectClicked:), nil()) }
}
fn s_login_filled() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_url.get(), "http://127.0.0.1:1");
        set_text(ui.login_user.get(), "me");
        set_text(ui.login_pass.get(), "pw");
        act(sel!(connectClicked:), nil());
        set_text(ui.login_key.get(), "key");
        act(sel!(connectClicked:), nil());
    }
}
fn s_connect_error() {
    handle_main(MainMsg::ConnectError("Bad password".into()));
}
fn s_connected() {
    handle_main(MainMsg::Connected {
        user_name: "tester".into(),
        views: vec![
            ("v1".into(), "Movies".into(), Some("movies".into())),
            ("v2".into(), "Shows".into(), Some("tvshows".into())),
            ("v3".into(), "Music".into(), Some("music".into())),
            ("v4".into(), "Live TV".into(), Some("livetv".into())),
            ("v5".into(), "Misc".into(), None),
        ],
        has_live_tv: true,
    });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("login hidden", {
            let h: BOOL = msg_send![ui.login_card.get(), isHidden];
            h == YES
        });
    }
}
fn s_worker_path() {
    let tx = WORKER_TX.get().expect("worker tx");
    let _ = tx.send(MainMsg::ItemsLoaded { title: "ViaWorker".into(), items: items_of("Movie", 3), can_go_back: false });
}
fn s_worker_check() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("worker msg delivered", ui.cur_title == "ViaWorker" && ui.items.len() == 3);
    }
    send_items("Movies", items_of("Movie", 12), false);
}
fn s_items_movies() {
    send_items("Movies", items_of("Movie", 12), false);
}
fn s_posters() {
    let png = image_bytes(0, 200, 300, 40);
    let jpg = image_bytes(1, 200, 300, 200);
    state_check("png magic", png.starts_with(&[0x89, b'P', b'N', b'G']));
    state_check("jpeg magic", jpg.starts_with(&[0xFF, 0xD8]));
    for i in 0..12 {
        let key = poster_key(&format!("Movie-{}", i));
        handle_main(MainMsg::PosterReady { key, bytes: if i % 2 == 0 { png.clone() } else { jpg.clone() } });
    }
    // replace existing cache entry, unknown key, garbage, empty
    handle_main(MainMsg::PosterReady { key: poster_key("Movie-0"), bytes: jpg.clone() });
    handle_main(MainMsg::PosterReady { key: 12345, bytes: png.clone() });
    handle_main(MainMsg::PosterReady { key: poster_key("Movie-1"), bytes: vec![1, 2, 3, 4] });
    handle_main(MainMsg::PosterReady { key: poster_key("Movie-1"), bytes: vec![] });
}
fn s_resize() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        for (w, h) in [(1000.0, 650.0), (1700.0, 1000.0), (960.0, 600.0), (1280.0, 800.0)] {
            let _: () = msg_send![ui.win.get(), setContentSize: size(w, h)];
            let _: () = msg_send![ui.win.get(), displayIfNeeded];
            let _: () = msg_send![ui.grid.get(), layoutSubtreeIfNeeded];
        }
    }
}
fn s_hover() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(d) = ui.grid_data.as_ref() {
            for c in &d.cells {
                let _: *mut Object = msg_send![c.container, mouseEntered: nil()];
                let _: *mut Object = msg_send![c.container, mouseExited: nil()];
            }
        }
    }
}
fn s_sidebar_click() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tags: Vec<*mut Object> = ui.rows.iter().map(|r| r.0).collect();
        state_check("sidebar rows == label + 5 views (livetv already present)", tags.len() == 6);
        for b in tags.into_iter().skip(1) {
            act(sel!(viewSelected:), b);
        }
    }
}
fn s_items_series() {
    send_items("Shows", items_of("Series", 5), true);
}
fn s_items_episodes() {
    send_items("Some Show", items_of("Episode", 8), true);
}
fn s_items_audio() {
    send_items("Album", items_of("Audio", 4), true);
    send_items("Albums", items_of("MusicAlbum", 4), false);
    send_items("Channels", items_of("LiveTvChannel", 4), false);
}
fn s_items_empty() {
    send_items("Empty", vec![], false);
    unsafe {
        let ui = UI.get().unwrap();
        let h: BOOL = msg_send![ui.status_label.get(), isHidden];
        state_check("status label visible", h == NO);
    }
}
fn s_items_many() {
    send_items("Many", items_of("Movie", 300), false);
    for i in (0..300).step_by(7) {
        handle_main(MainMsg::PosterReady {
            key: poster_key(&format!("Movie-{}", i)),
            bytes: image_bytes(0, 80, 120, (i % 255) as u8),
        });
    }
}
fn s_item_clicks() {
    unsafe {
        send_items("Mixed", {
            let mut v = items_of("Series", 1);
            v.extend(items_of("MusicAlbum", 1));
            v.extend(items_of("LiveTvChannel", 1));
            v.extend(items_of("Movie", 1));
            v.extend(items_of("Episode", 1));
            v
        }, true);
        for t in 0..6 {
            act(sel!(itemClicked:), tagged_button(t));
        }
    }
}
fn s_nav_actions() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        act(sel!(sortChanged:), nil());
        let idx = 2isize;
        let _: () = msg_send![ui.sort_popup.get(), selectItemAtIndex: idx];
        act(sel!(sortChanged:), nil());
        set_text(ui.search.get(), "query");
        let note: *mut Object = msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification") object: ui.search.get()];
        act(sel!(controlTextDidChange:), note);
        let note2: *mut Object = msg_send![class!(NSNotification), notificationWithName: ns("x") object: ui.login_url.get()];
        act(sel!(controlTextDidChange:), note2);
        act(sel!(refresh:), nil());
        act(sel!(goBack:), nil());
        act(sel!(signOut:), nil());
    }
}
fn s_misc_actions() {
    unsafe {
        let mi: *mut Object = msg_send![class!(NSMenuItem), new];
        let _: () = msg_send![mi, setRepresentedObject: ns("Movie-1")];
        act(sel!(markWatched:), mi);
        act(sel!(openInWeb:), mi); // no server_url configured -> no browser launch
        let _: *mut Object = msg_send![mi, autorelease];
    }
}
fn s_stop_idle() {
    unsafe {
        act(sel!(stopPlayback:), nil());
        for s in [sel!(playPause:), sel!(seekBack:), sel!(seekFwd:), sel!(volUp:), sel!(volDown:), sel!(mute:), sel!(subCycle:), sel!(audioCycle:)] {
            act(s, nil());
        }
        act(sel!(setSpeed:), tagged_button(150));
        update_chip();
    }
}
fn s_playback_error() {
    handle_main(MainMsg::PlaybackError("boom".into()));
}
fn s_server_idle() {
    handle_main(MainMsg::ServerDown);
    handle_main(MainMsg::ServerUp);
}
fn s_settings_cancel() {
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.settings.theme = "dark".into();
    cfg.settings.max_bitrate_kbps = 20_000;
    cfg.settings.subtitle_lang = "eng".into();
    cfg.settings.audio_lang = "spa".into();
    cfg.settings.seerr_enabled = true;
    cfg.settings.seerr_url = "https://seerr.example".into();
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    // cancel is invoked on the next step via a helper below
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            log("  cancelling settings sheet");
            let ui = UI.get_mut().unwrap();
            state_check("seerr fields filled", read_field(ui.seerr.s_url.get()) == "https://seerr.example");
            set_text(ui.seerr.s_key.get(), "typed-then-cancelled");
            act(sel!(cancelSettings:), nil());
            state_check("key cleared on cancel", read_field(ui.seerr.s_key.get()).is_empty());
            state_check("cancel sends nothing", take_cmds().is_empty());
        });
    });
}
fn s_settings_save() {
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.settings.theme = "light".into();
    cfg.settings.audio_lang = "spa".into();
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            log("  saving settings sheet");
            act(sel!(saveSettings:), nil());
            let ui = UI.get().unwrap();
            state_check("audio lang saved", ui.cfg.settings.audio_lang == "spa");
            state_check("theme saved", ui.cfg.settings.theme == "light");
        });
    });
}
fn s_play_start() {
    handle_main(playback(0.0, 6.0));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("playing set", ui.playing.is_some());
    }
}
fn s_play_controls() {
    unsafe {
        act(sel!(playPause:), nil());
        act(sel!(seekFwd:), nil());
        act(sel!(seekBack:), nil());
        act(sel!(volUp:), nil());
        act(sel!(volDown:), nil());
        act(sel!(mute:), nil());
        act(sel!(mute:), nil());
        act(sel!(playPause:), nil());
        update_chip();
    }
}
fn s_play_cycle() {
    unsafe {
        let ui = UI.get().unwrap();
        log(&format!("  subs={} audio={}", ui.sub_opts.len(), ui.audio_opts.len()));
        for _ in 0..4 {
            act(sel!(subCycle:), nil());
            act(sel!(audioCycle:), nil());
        }
    }
}
fn s_play_speed() {
    unsafe {
        for t in [50, 75, 100, 125, 150, 200, 100] {
            act(sel!(setSpeed:), tagged_button(t));
        }
    }
}
fn s_play_net() {
    handle_main(MainMsg::ServerDown);
    handle_main(MainMsg::ServerUp);
    handle_main(MainMsg::ServerDown);
    handle_main(MainMsg::ServerDown);
    handle_main(MainMsg::ServerDown);
    handle_main(MainMsg::ServerUp);
    handle_main(MainMsg::PlaybackError("stream broke".into()));
    unsafe { act(sel!(jrRetry:), nil()) }
}
fn s_play_stop() {
    unsafe {
        act(sel!(stopPlayback:), nil());
        let ui = UI.get().unwrap();
        state_check("playing cleared", ui.playing.is_none());
        act(sel!(stopPlayback:), nil()); // double stop
    }
}
fn s_fullscreen() {
    handle_main(playback(0.0, 6.0));
    unsafe {
        act(sel!(fullscreen:), nil());
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        on_main(|| unsafe {
            act(sel!(fullscreen:), nil());
        });
    });
}
fn s_play_to_end() {
    // Wait for the stop from step above to settle, then play from near the end.
    unsafe { act(sel!(stopPlayback:), nil()) }
    handle_main(playback(3.0, 6.0));
}
fn s_signed_out() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.playing.is_some() {
            log("  (still playing at end; stopping)");
            act(sel!(stopPlayback:), nil());
        }
    }
    handle_main(MainMsg::SignedOut);
}
fn s_quit_prep() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("no sheet left", {
            let s: *mut Object = msg_send![ui.win.get(), attachedSheet];
            s.is_null()
        });
    }
}

// ---- Seerr steps ------------------------------------------------------------
fn seerr_result(media: &str, id: u64, title: &str, year: Option<u16>, a: Availability) -> SeerrResult {
    SeerrResult {
        media_type: media.into(),
        tmdb_id: id,
        title: title.into(),
        year,
        poster_path: None,
        overview: "overview".into(),
        availability: a,
        request_status: None,
    }
}
fn seerr_mixed() -> Vec<SeerrResult> {
    vec![
        seerr_result("movie", 11, "Fresh Movie", Some(2010), Availability::NotRequested),
        seerr_result("tv", 22, "Fresh Show", None, Availability::NotRequested),
        seerr_result("movie", 33, "Pending Movie", Some(1999), Availability::Pending),
        seerr_result("tv", 44, "Processing Show", Some(2021), Availability::Processing),
        seerr_result("movie", 55, "Partial Movie", None, Availability::PartiallyAvailable),
        seerr_result("tv", 66, "Library Show", Some(2005), Availability::Available),
        seerr_result("movie", 77, "A very long blocklisted title that has to be truncated somewhere around here", Some(2001), Availability::Blocklisted),
    ]
}
fn seerr_hidden(v: *mut Object) -> bool {
    unsafe {
        let h: BOOL = msg_send![v, isHidden];
        h == YES
    }
}
fn seerr_visible_win() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        let v: BOOL = msg_send![ui.seerr.win.get(), isVisible];
        v == YES
    }
}
fn sheet_open() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        let a: *mut Object = msg_send![ui.win.get(), attachedSheet];
        !a.is_null()
    }
}
/// Number of rows in the Seerr list that carry a Request button.
fn seerr_button_rows() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr
            .rows
            .iter()
            .filter(|r| {
                let subs: *mut Object = msg_send![**r, subviews];
                let n: usize = msg_send![subs, count];
                n == 5 // title, sub, badge, button, separator
            })
            .count()
    }
}
fn seerr_status_text() -> String {
    unsafe { read_field(UI.get().unwrap().seerr.status.get()) }
}
fn seerr_note(field: *mut Object) -> *mut Object {
    unsafe {
        msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification") object: field]
    }
}

fn s_seerr_disabled() {
    unsafe {
        take_cmds();
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = false;
        ui.cfg.settings.seerr_url = String::new();
        act(sel!(openSeerr:), nil());
        state_check("seerr window visible", seerr_visible_win());
        state_check("hint shown when disabled", !seerr_hidden(ui.seerr.hint.get()));
        state_check("search hidden when disabled", seerr_hidden(ui.seerr.search.get()));
        ui.cfg.settings.seerr_enabled = true; // enabled but no URL is still unconfigured
        act(sel!(openSeerr:), nil());
        state_check("hint shown when URL empty", !seerr_hidden(ui.seerr.hint.get()));
        // Messages that arrive while unconfigured must not crash.
        handle_main(MainMsg::SeerrError("Seerr is turned off. Enable it in Settings to search and request.".into()));
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check("no commands sent", take_cmds().is_empty());
    }
}
fn s_seerr_enabled() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = true;
        ui.cfg.settings.seerr_url = "http://seerr.example:5055".into();
        act(sel!(openSeerr:), nil()); // same window reused
        state_check("hint hidden when configured", seerr_hidden(ui.seerr.hint.get()));
        state_check("search visible", !seerr_hidden(ui.seerr.search.get()));
        state_check("empty state row", ui.seerr.rows.len() == 1);
    }
}
fn s_seerr_results() {
    handle_main(MainMsg::SeerrResults(seerr_mixed()));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("7 rows", ui.seerr.rows.len() == 7);
        state_check("2 request buttons (NotRequested only)", seerr_button_rows() == 2);
    }
}
fn s_seerr_typing() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        take_cmds();
        set_text(ui.seerr.search.get(), "incep");
        act(sel!(controlTextDidChange:), seerr_note(ui.seerr.search.get()));
        let cmds = take_cmds();
        state_check(
            "SeerrSearch emitted",
            matches!(cmds.as_slice(), [UiCmd::SeerrSearch { q }] if q == "incep"),
        );
        state_check("searching status", seerr_status_text() == "Searching…");
        handle_main(MainMsg::SeerrResults(seerr_mixed()));
        set_text(ui.seerr.search.get(), "");
        act(sel!(controlTextDidChange:), seerr_note(ui.seerr.search.get()));
        let cmds = take_cmds();
        state_check(
            "blank search emitted and results cleared",
            matches!(cmds.as_slice(), [UiCmd::SeerrSearch { q }] if q.is_empty()) && ui.seerr.results.is_empty(),
        );
        handle_main(MainMsg::SeerrResults(seerr_mixed()));
    }
}
fn s_seerr_request() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        take_cmds();
        act(sel!(seerrRequest:), tagged_button(0));
        let cmds = take_cmds();
        state_check(
            "movie request emitted",
            matches!(cmds.as_slice(), [UiCmd::SeerrRequest { media_type, tmdb_id: 11, title }]
                if media_type == "movie" && title == "Fresh Movie"),
        );
        act(sel!(seerrRequest:), tagged_button(0));
        state_check("double click ignored", take_cmds().is_empty());
        act(sel!(seerrRequest:), tagged_button(1));
        let cmds = take_cmds();
        state_check(
            "tv request emitted",
            matches!(cmds.as_slice(), [UiCmd::SeerrRequest { media_type, tmdb_id: 22, .. }] if media_type == "tv"),
        );
        act(sel!(seerrRequest:), tagged_button(2)); // Pending: no request
        act(sel!(seerrRequest:), tagged_button(5)); // Available: no request
        act(sel!(seerrRequest:), tagged_button(99)); // out of range
        act(sel!(seerrRequest:), tagged_button(-1));
        state_check("non-requestable clicks ignored", take_cmds().is_empty());
        state_check("two in flight", ui.seerr.inflight.len() == 2);
    }
}
fn s_seerr_replies() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        handle_main(MainMsg::SeerrRequested { title: "Fresh Movie".into() });
        state_check("confirmation shown", seerr_status_text() == "Requested “Fresh Movie”.");
        state_check("row state refreshed", ui.seerr.results[0].availability == Availability::Pending);
        state_check("one request button left", seerr_button_rows() == 1);
        state_check("other still in flight", ui.seerr.inflight.len() == 1);
        handle_main(MainMsg::SeerrError("“Fresh Show” has already been requested.".into()));
        state_check("error shown", seerr_status_text().contains("already been requested"));
        state_check("in flight cleared", ui.seerr.inflight.is_empty());
        state_check("button re-enabled (still NotRequested)", seerr_button_rows() == 1);
        handle_main(MainMsg::SeerrRequested { title: "Never Sent".into() }); // unknown title: harmless
    }
}
fn seerr_request_item(id: u64, title: &str, st: RequestStatus, av: Availability) -> SeerrRequestItem {
    SeerrRequestItem {
        request_id: id,
        media_type: if id % 2 == 0 { "tv".into() } else { "movie".into() },
        tmdb_id: id * 10,
        title: title.into(),
        year: if id % 3 == 0 { None } else { Some(2000 + id as u16) },
        poster_path: None,
        status: st,
        availability: av,
        requested_by: if id == 4 { String::new() } else { "alice".into() },
        created_at: "2025-01-02T03:04:05.000Z".into(),
    }
}
fn s_seerr_requests_tab() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        take_cmds();
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: 1isize];
        act(sel!(seerrTab:), ui.seerr.tabs.get());
        state_check("SeerrRequests emitted", matches!(take_cmds().as_slice(), [UiCmd::SeerrRequests]));
        state_check("loading row", ui.seerr.rows.len() == 1);
        handle_main(MainMsg::SeerrRequestList(vec![
            seerr_request_item(1, "One", RequestStatus::Pending, Availability::Pending),
            seerr_request_item(2, "Two", RequestStatus::Approved, Availability::Processing),
            seerr_request_item(3, "Three", RequestStatus::Declined, Availability::NotRequested),
            seerr_request_item(4, "Four", RequestStatus::Failed, Availability::NotRequested),
            seerr_request_item(5, "Five", RequestStatus::Completed, Availability::Available),
            seerr_request_item(6, "Six", RequestStatus::Unknown, Availability::Blocklisted),
        ]));
        state_check("6 request rows", ui.seerr.rows.len() == 6);
        state_check("no request buttons in My requests", seerr_button_rows() == 0);
        handle_main(MainMsg::SeerrRequestList(vec![]));
        state_check("empty requests -> 1 info row", ui.seerr.rows.len() == 1);
        handle_main(MainMsg::SeerrRequestList(vec![seerr_request_item(1, "One", RequestStatus::Pending, Availability::Pending)]));
        // typing in the search field jumps back to the search tab
        set_text(ui.seerr.search.get(), "x");
        act(sel!(controlTextDidChange:), seerr_note(ui.seerr.search.get()));
        state_check("typing returns to search", !ui.seerr.requests_mode);
        let sel_seg: isize = msg_send![ui.seerr.tabs.get(), selectedSegment];
        state_check("tab control follows", sel_seg == 0);
        take_cmds();
        handle_main(MainMsg::SeerrResults(seerr_mixed()));
        // error while on the requests tab, then back to search
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: 1isize];
        act(sel!(seerrTab:), ui.seerr.tabs.get());
        handle_main(MainMsg::SeerrError("Seerr request failed (HTTP 500)".into()));
        take_cmds();
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: 0isize];
        act(sel!(seerrTab:), ui.seerr.tabs.get());
        state_check("back on search tab", !ui.seerr.requests_mode && ui.seerr.rows.len() == 7);
    }
}
fn s_seerr_resize() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        for (w, h) in [(460.0, 360.0), (900.0, 800.0), (620.0, 640.0)] {
            let _: () = msg_send![ui.seerr.win.get(), setContentSize: size(w, h)];
            let _: () = msg_send![ui.seerr.win.get(), displayIfNeeded];
            let _: () = msg_send![ui.seerr.doc.get(), layoutSubtreeIfNeeded];
        }
        handle_main(MainMsg::SeerrResults(seerr_mixed())); // rebuild at the new width
    }
}
fn s_seerr_reopen() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let w = ui.seerr.win.get();
        let _: () = msg_send![w, close];
        state_check("window closed", !seerr_visible_win());
        // messages while the window is hidden are applied, not dropped
        handle_main(MainMsg::SeerrRequested { title: "Fresh Show".into() });
        act(sel!(openSeerr:), nil());
        state_check("window reopened", seerr_visible_win());
        state_check("same window instance", ui.seerr.win.get() == w);
        state_check("results survive close/reopen", ui.seerr.results.len() == 7 && ui.seerr.rows.len() == 7);
    }
}
fn s_seerr_empty() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.seerr.search.get(), "zzzz");
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check("no results -> 1 info row", ui.seerr.rows.len() == 1 && seerr_button_rows() == 0);
        set_text(ui.seerr.search.get(), "");
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check("blank -> 1 info row", ui.seerr.rows.len() == 1);
        take_cmds();
    }
}
fn s_seerr_settings_open() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = false;
        ui.cfg.settings.seerr_url = String::new();
        take_cmds();
    }
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.username = Some("nathan".into());
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            log("  saving seerr settings");
            let ui = UI.get_mut().unwrap();
            let on: isize = msg_send![ui.seerr.s_on.get(), state];
            state_check("checkbox off from snapshot", on == 0);
            let mode: isize = msg_send![ui.seerr.s_mode.get(), indexOfSelectedItem];
            state_check("defaults to Jellyfin-account mode", mode == 0);
            state_check("username prefilled from Jellyfin login", read_field(ui.seerr.s_user.get()) == "nathan");
            state_check("password/key empty", read_field(ui.seerr.s_pass.get()).is_empty() && read_field(ui.seerr.s_key.get()).is_empty());
            state_check("key field hidden in login mode", seerr_hidden(ui.seerr.s_key.get()) && !seerr_hidden(ui.seerr.s_pass.get()));
            let _: () = msg_send![ui.seerr.s_on.get(), setState: 1isize];
            set_text(ui.seerr.s_url.get(), "  http://seerr.example:5055 ");
            // blank username in login mode: refuse locally, sheet stays open, nothing sent
            set_text(ui.seerr.s_user.get(), "");
            set_text(ui.seerr.s_pass.get(), "super-secret-pw");
            act(sel!(saveSettings:), nil());
            state_check("blank username sends no SaveSeerr", !take_cmds().iter().any(|c| matches!(c, UiCmd::SaveSeerr { .. })));
            state_check("blank username keeps sheet open", sheet_open());
            set_text(ui.seerr.s_user.get(), " nathan ");
            set_text(ui.seerr.s_pass.get(), "super-secret-pw");
            act(sel!(saveSettings:), nil());
            let cmds = take_cmds();
            let order_ok = match cmds.as_slice() {
                [UiCmd::SaveSettings { settings, .. }, UiCmd::SaveSeerr { enabled, url, username, password, api_key }] => {
                    // SaveSettings must carry the last confirmed Seerr values (not the unconfirmed edit)
                    !settings.seerr_enabled
                        && *enabled
                        && url == "http://seerr.example:5055"
                        && username == "nathan"
                        && password.as_deref() == Some("super-secret-pw")
                        && api_key.is_none()
                }
                _ => false,
            };
            state_check("SaveSettings then SaveSeerr emitted", order_ok);
            state_check(
                "secret fields cleared after save",
                read_field(ui.seerr.s_key.get()).is_empty() && read_field(ui.seerr.s_pass.get()).is_empty(),
            );
            state_check("sheet stays open while checking", sheet_open());
            state_check("checking label", read_field(ui.seerr.s_status.get()) == "Checking connection…");
            state_check("settings not yet changed", !ui.cfg.settings.seerr_enabled);
            let on_disk = std::fs::read_to_string(crate::config::config_path()).unwrap_or_default();
            state_check("password not in config.json", !on_disk.contains("super-secret"));
            state_check("password not in memory config", !format!("{:?}", ui.cfg).contains("super-secret"));
        });
    });
}
fn s_seerr_settings_fail() {
    handle_main(MainMsg::SeerrStatus { ok: false, message: "Seerr rejected the API key.".into() });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("failure shown in sheet", read_field(ui.seerr.s_status.get()) == "Seerr rejected the API key.");
        state_check("sheet still open after failure", sheet_open());
        state_check("settings unchanged after failure", !ui.cfg.settings.seerr_enabled);
    }
}
fn s_seerr_settings_ok() {
    handle_main(MainMsg::SeerrStatus { ok: true, message: "Connected to Seerr 2.7.3 as admin.".into() });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("sheet closed after ok", !sheet_open());
        state_check(
            "confirmed settings applied",
            ui.cfg.settings.seerr_enabled
                && ui.cfg.settings.seerr_url == "http://seerr.example:5055"
                && ui.cfg.settings.seerr_username == "nathan",
        );
        state_check("seerr window shows search UI", seerr_hidden(ui.seerr.hint.get()));
        state_check("success message in seerr status line", seerr_status_text().starts_with("Connected to Seerr"));
    }
    // A stray status with nothing pending must be harmless.
    handle_main(MainMsg::SeerrStatus { ok: true, message: "Seerr is turned off.".into() });
}
fn s_seerr_settings_off() {
    // Turn it off again through the sheet; the stored key is left alone.
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.settings.seerr_enabled = true;
    cfg.settings.seerr_url = "http://seerr.example:5055".into();
    cfg.settings.seerr_username = "nathan".into();
    take_cmds();
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            let ui = UI.get_mut().unwrap();
            let _: () = msg_send![ui.seerr.s_on.get(), setState: 0isize];
            act(sel!(saveSettings:), nil());
            let cmds = take_cmds();
            state_check(
                "disable emits SaveSeerr without key",
                matches!(cmds.as_slice(), [UiCmd::SaveSettings { .. }, UiCmd::SaveSeerr { enabled: false, password: None, api_key: None, .. }]),
            );
            handle_main(MainMsg::SeerrStatus { ok: true, message: "Seerr is turned off.".into() });
            state_check("disabled applied", !ui.cfg.settings.seerr_enabled);
            state_check("hint back after disable", !seerr_hidden(ui.seerr.hint.get()));
            state_check("sheet closed", !sheet_open());
        });
    });
}
fn s_seerr_settings_key() {
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.settings.seerr_enabled = false;
    cfg.settings.seerr_url = "http://seerr.example:5055".into();
    cfg.settings.seerr_username = String::new(); // saved URL without a username = API-key mode
    take_cmds();
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            let ui = UI.get_mut().unwrap();
            let mode: isize = msg_send![ui.seerr.s_mode.get(), indexOfSelectedItem];
            state_check("saved URL without username opens in API-key mode", mode == 1);
            state_check("login fields hidden in key mode", seerr_hidden(ui.seerr.s_pass.get()) && !seerr_hidden(ui.seerr.s_key.get()));
            // switch the popup the way a click would
            let _: () = msg_send![ui.seerr.s_mode.get(), selectItemAtIndex: 0isize];
            act(sel!(seerrModeChanged:), nil());
            state_check("switching shows login fields", !seerr_hidden(ui.seerr.s_pass.get()) && seerr_hidden(ui.seerr.s_key.get()));
            let _: () = msg_send![ui.seerr.s_mode.get(), selectItemAtIndex: 1isize];
            act(sel!(seerrModeChanged:), nil());
            let _: () = msg_send![ui.seerr.s_on.get(), setState: 1isize];
            set_text(ui.seerr.s_user.get(), "ignored-in-key-mode");
            set_text(ui.seerr.s_key.get(), "api-key-123");
            act(sel!(saveSettings:), nil());
            let cmds = take_cmds();
            state_check(
                "API-key SaveSeerr has no username/password",
                matches!(cmds.as_slice(), [UiCmd::SaveSettings { .. }, UiCmd::SaveSeerr { enabled: true, username, password: None, api_key: Some(k), .. }]
                    if username.is_empty() && k == "api-key-123"),
            );
            handle_main(MainMsg::SeerrStatus { ok: true, message: "Connected to Seerr 3.5.1 as admin.".into() });
            state_check("username cleared in key mode", ui.cfg.settings.seerr_username.is_empty() && ui.cfg.settings.seerr_enabled);
            state_check("sheet closed", !sheet_open());
            ui.cfg.settings.seerr_enabled = false;
        });
    });
}
fn s_seerr_end() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.seerr.win.get(), close];
        take_cmds();
    }
}
