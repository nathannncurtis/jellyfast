//! Headless UI self-test (JELLYFIN_MAC_SELFTEST=1). Inert unless the env var is set.
//!
//! Drives every UI code path with fake data and no server: MainMsg values go
//! through `handle_main` (the same reducer the worker results use), controller
//! actions are invoked through real Objective-C dispatch on the JRCtl object,
//! and outgoing UiCmds are logged by a drain task in main.rs. Steps run one per
//! run-loop turn (optionally delayed) so AppKit layout/timers/KVO really fire.
//! Video: $JELLYFIN_MAC_SELFTEST_VIDEO, else a system .m4v/.mov.
#![allow(unsafe_code)]
use crate::app::controller::handle_main;
use crate::app::messages::*;
use crate::config::Config;
use crate::jellyfin::models::{ImageTags, ListItem, UserData};
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::transcode::decision::PlaybackMethod;
use crate::ui::library::poster_key;
use objc::msg_send;
use objc::runtime::{Object, BOOL, NO, YES};
use objc::{class, sel, sel_impl};

pub fn enabled() -> bool {
    std::env::var("JELLYFIN_MAC_SELFTEST").map(|v| v == "1").unwrap_or(false)
}

type Step = (&'static str, fn(), u64);

fn log(s: &str) {
    println!("[selftest] {}", s);
}

/// Called once from applicationDidFinishLaunching after the UI is built.
pub fn start() {
    log("start");
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
        ("mark watched / open web / searx", s_misc_actions, 50),
        ("stop when not playing", s_stop_idle, 50),
        ("playback error (idle)", s_playback_error, 50),
        ("server down/up while idle", s_server_idle, 50),
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
        "/System/Library/Frameworks/ImagePlayground.framework/Versions/A/Resources/ReframeOrExpand.m4v".into()
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
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.searx_enabled = false;
        act(sel!(openSearx:), nil()); // disabled -> chip message, no modal
        ui.cfg.settings.searx_enabled = true;
        ui.cfg.settings.searx_url = String::new();
        act(sel!(openSearx:), nil()); // empty URL -> chip message, no modal
        ui.cfg.settings.searx_enabled = false;
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
    cfg.settings.searx_enabled = true;
    cfg.settings.searx_url = "https://searx.example".into();
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    // cancel is invoked on the next step via a helper below
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        on_main(|| unsafe {
            log("  cancelling settings sheet");
            act(sel!(cancelSettings:), nil());
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
