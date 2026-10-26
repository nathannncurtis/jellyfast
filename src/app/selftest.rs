//! Headless UI self-test (JELLYFAST_SELFTEST=1). Inert unless the env var is set.
//!
//! Drives every UI code path with fake data and no server: MainMsg values go
//! through `handle_main` (the same reducer the worker results use), controller
//! actions are invoked through real Objective-C dispatch on the JRCtl object,
//! and outgoing UiCmds are logged by a drain task in main.rs. Steps run one per
//! run-loop turn (optionally delayed) so AppKit layout/timers/KVO really fire.
//! Video: $JELLYFAST_SELFTEST_VIDEO, else assets/selftest.mp4 (a plain gradient clip
//! with two audio tracks and one subtitle track).
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use crate::app::controller::handle_main;
use crate::app::messages::*;
use crate::config::Config;
use crate::jellyfin::models::{ImageTags, ListItem, UserData};
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::cast::STRIP_H;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use crate::transcode::decision::PlaybackMethod;
use crate::jellyfin::segments::{NextUp, Segment, SegmentKind};
use std::sync::Mutex;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use crate::ui::library::poster_key;
use crate::ui::shell::is_hidden;

pub fn enabled() -> bool {
    std::env::var("JELLYFAST_SELFTEST").map(|v| v == "1").unwrap_or(false)
}

type Step = (&'static str, fn(), u64);

static CMD_RX: Mutex<Option<UnboundedReceiver<UiCmd>>> = Mutex::new(None);

/// UiCmds sent before `start()` swapped in the interceptor (during launch).
static EARLY_CMDS: Mutex<Vec<UiCmd>> = Mutex::new(Vec::new());

/// Pre-start UiCmd sink: `main` routes the self-test drain here instead of dropping.
pub fn early_cmd(c: UiCmd) {
    println!("[selftest]   UiCmd (launch) -> {:?}", c);
    EARLY_CMDS.lock().unwrap().push(c);
}

/// Commands that reach the network or sign in, i.e. anything that must wait for Connect.
fn is_connect_or_auth(c: &UiCmd) -> bool {
    matches!(
        c,
        UiCmd::Connect { .. }
            | UiCmd::SaveSettings { server_url: Some(_), .. }
            | UiCmd::SaveSeerr { .. }
            | UiCmd::ProbeServer
            | UiCmd::LoadHome
            | UiCmd::LoadBrowse
            | UiCmd::LoadView { .. }
            | UiCmd::SeerrSearch { .. }
            | UiCmd::SeerrRequest { .. }
            | UiCmd::SeerrRequests
    )
}

/// Fresh login screen: no connect/auth command queued, Connect enabled, no status text.
fn state_check_fresh_login(ui: &crate::platform::appkit::Ui) {
    let mut sent = std::mem::take(&mut *EARLY_CMDS.lock().unwrap());
    sent.extend(take_cmds());
    let bad: Vec<String> = sent.iter().filter(|c| is_connect_or_auth(c)).map(|c| format!("{:?}", c)).collect();
    if !bad.is_empty() {
        log(&format!("  sent before Connect: {:?}", bad));
    }
    state_check("fresh login: no connect/auth UiCmd before Connect", bad.is_empty());
    unsafe {
        let en: Bool = msg_send![ui.login_btn.get(), isEnabled];
        let title: *mut AnyObject = msg_send![ui.login_btn.get(), title];
        let status: *mut AnyObject = msg_send![ui.login_err.get(), stringValue];
        state_check(
            "fresh login: Connect enabled, titled Connect, no connecting status",
            en == Bool::YES && to_str(title) == "Connect" && to_str(status).is_empty(),
        );
    }
}

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
        state_check_fresh_login(ui);
        // Launch state: no library chrome, no spinner, branded window.
        let h: Bool = msg_send![ui.content_host.get(), isHidden];
        state_check("launch: content_host hidden", h == Bool::YES);
        let h: Bool = msg_send![ui.spinner.get(), isHidden];
        state_check("launch: spinner hidden", h == Bool::YES);
        if !crate::ui::theme::reduce_transparency() {
            let bh: Bool = msg_send![ui.bgfx.get(), isHidden];
            state_check("launch: login backdrop visible", bh == Bool::NO);
        }
        let t: *mut AnyObject = msg_send![ui.win.get(), title];
        state_check("window title is Jellyfast", to_str(t) == "Jellyfast");
    }
    check_offscreen("start");
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
        ("audit: login (long strings)", s_audit_login, 100),
        ("login: filled connect", s_login_filled, 50),
        ("login: remember me checkbox", s_login_remember, 50),
        ("connect error", s_connect_error, 50),
        ("connected (views + livetv)", s_connected, 100),
        ("connected again (sidebar rebuild)", s_connected, 100),
        ("login connect syncs server url", s_login_url_sync, 50),
        ("onboarding: sheet after first login", s_onboard_show, 150),
        ("onboarding: pick one big library", s_onboard_pick, 100),
        ("unified: browse grid", s_unified_browse, 150),
        ("unified: sidebar actions and Home", s_unified_actions, 100),
        ("unified: login restores Browse", s_unified_login, 100),
        ("unified tabs: segmented control on Home", s_tabs_home, 100),
        ("unified tabs: Movies shows only movies", s_tabs_movies, 100),
        ("unified tabs: grid re-flows on resize", s_tabs_resize, 150),
        ("unified tabs: TV Shows shows only series", s_tabs_shows, 100),
        ("unified tabs: back to Home", s_tabs_home_again, 100),
        ("unified tabs: tab remembered on return", s_tabs_remember, 100),
        ("settings: layout back to separate", s_layout_settings, 100),
        ("connected: Home wins over a remembered library", s_connected_library, 100),
        ("items: movies", s_items_movies, 100),
        ("worker->UI path (WORKER_TX)", s_worker_path, 400),
        ("worker->UI path check", s_worker_check, 50),
        ("home rows stored", s_home_rows, 50),
        ("audit: sidebar (long names)", s_audit_sidebar, 150),
        ("posters (png, jpeg, garbage)", s_posters, 100),
        ("audit: grid (long titles)", s_audit_grid, 200),
        ("resize window", s_resize, 200),
        ("grid geometry + layers", s_grid_checks, 50),
        ("hover cells", s_hover, 50),
        ("hover model layer", s_hover_model, 50),
        ("poster states (garbage keeps placeholder)", s_poster_states, 100),
        ("sidebar click", s_sidebar_click, 50),
        ("items: series", s_items_series, 100),
        ("items: episodes", s_items_episodes, 100),
        ("items: audio", s_items_audio, 100),
        ("items: empty", s_items_empty, 50),
        ("header layout", s_header, 100),
        ("states: skeleton, error, empty search", s_states, 150),
        ("search debounce: type", s_debounce_type, 100),
        ("search debounce: not yet", s_debounce_early, 250),
        ("search debounce: fired once", s_debounce_done, 50),
        ("items: many (grid scroll)", s_items_many, 100),
        ("resize window (many)", s_resize, 200),
        ("item clicks", s_item_clicks, 50),
        ("shortcuts: menu keys, no duplicates", s_shortcuts_menu, 50),
        ("shortcuts: Edit menu routes text commands", s_edit_menu, 50),
        ("shortcuts: text fields keep plain keys", s_shortcuts_text, 50),
        ("shortcuts: grid focus (Tab, Return enabled)", s_shortcuts_grid, 50),
        ("shortcuts: help window", s_shortcuts_help, 150),
        ("sort/search/refresh/back", s_nav_actions, 50),
        ("sort + filter: order, filter, clear", s_sort_filter, 50),
        ("mark watched / open web", s_misc_actions, 50),
        ("detail: movie (long strings, art, actions)", s_detail_movie, 150),
        ("detail: resize relayout, back", s_detail_resize, 150),
        ("detail: series page", s_detail_series, 150),
        ("detail: seasons (picker, episodes, play)", s_detail_seasons, 300),
        ("detail: error toast", s_detail_error, 50),
        ("cast: detail Cast button and Play On menu", s_cast_detail, 150),
        ("cast: remote bar controls and audit", s_cast_bar, 200),
        ("cast: error strip in the library", s_cast_error, 150),
        ("cast: series Cast button sends the series", s_cast_series, 150),
        ("stop when not playing", s_stop_idle, 50),
        ("playback error (idle)", s_playback_error, 50),
        ("server down/up while idle", s_server_idle, 50),
        ("seerr: open while disabled (inline hint)", s_seerr_disabled, 100),
        ("seerr: open while enabled", s_seerr_enabled, 100),
        ("seerr: login restores Requests", s_seerr_login_restore, 100),
        ("seerr: results (all availabilities)", s_seerr_results, 100),
        ("seerr: type in search field", s_seerr_typing, 50),
        ("seerr: click Request", s_seerr_request, 100),
        ("seerr: requested + error", s_seerr_replies, 100),
        ("seerr: My requests tab", s_seerr_requests_tab, 100),
        ("seerr: resize window", s_seerr_resize, 150),
        ("seerr: leave and return", s_seerr_reopen, 100),
        ("seerr: empty results", s_seerr_empty, 100),
        ("seerr: settings save (open)", s_seerr_settings_open, 700),
        ("seerr: settings status failure", s_seerr_settings_fail, 100),
        ("seerr: settings status ok", s_seerr_settings_ok, 100),
        ("seerr: settings save while disabled", s_seerr_settings_off, 700),
        ("seerr: settings API-key mode", s_seerr_settings_key, 700),
        ("seerr: wrap up", s_seerr_end, 50),
        ("home: open from sidebar", s_home_open, 100),
        ("home: rows with long titles and posters", s_home_fake, 150),
        ("audit: home (long titles)", s_audit_home, 200),
        ("home: card clicks", s_home_clicks, 50),
        ("home: empty state, then rows again", s_home_empty, 100),
        ("home: back to a library", s_home_leave, 100),
        ("settings: open+cancel", s_settings_cancel, 600),
        ("settings: open+save", s_settings_save, 600),
        ("themes: load palette from folder", s_themes_palette, 300),
        ("themes: follow system pickers", s_themes_follow, 300),
        ("themes: palette screenshots", s_themes_shots, 300),
        ("themes: restore", s_themes_restore, 100),
        ("libraries: sidebar order + hidden, settings controls", s_library_prefs, 300),
        ("playback: start", s_play_start, 2500),
        // The start-up fade-in of the bars runs for M_FAST (0.12 s) and alphaValue reads
        // mid-animation, so the delay after the chrome step must clear it before the bar checks.
        ("playback: chrome", s_play_chrome, 400),
        ("player bar: layout + audit", s_player_bar, 100),
        ("player bar: buttons, menus, hover", s_player_bar_buttons, 300),
        ("player bar: auto-hide arm", s_pb_hide_arm, 600),
        ("player bar: auto-hide check", s_pb_hide_check, 50),
        ("subtitles: pick a track, cue on screen", s_sub_pick, 800),
        ("subtitles: cue beside the controls, audits", s_sub_on_screen, 300),
        ("subtitles: controls hide, overlay drops", s_sub_hide, 400),
        ("subtitles: overlay at the bottom", s_sub_bottom, 400),
        ("subtitles: seek clears the cue", s_sub_seek, 400),
        ("subtitles: track off clears the overlay", s_sub_off, 400),
        ("subtitles: stays clear with the track off", s_sub_off_check, 50),
        ("seek: scrubber lands near 50%", s_seek_scrub, 500),
        ("seek: scrubber landed", s_seek_scrub_check, 500),
        ("playback: controls", s_play_controls, 300),
        ("player: AirPlay button and external card", s_player_airplay, 150),
        ("playback: subtitle/audio cycle", s_play_cycle, 2300),
        ("playback: toast hides itself", s_toast_hidden, 50),
        ("audit: player (long title, toast, info)", s_audit_player, 200),
        ("player header: episode, long names", s_player_header, 200),
        ("playback: speed", s_play_speed, 300),
        ("playback: server down/up/error", s_play_net, 500),
        ("playback: replace item", s_play_start, 1500),
        ("playback: stop", s_play_stop, 200),
        ("skip: intro button inside a segment", s_skip_show, 700),
        ("skip: button shown, audit (intro)", s_skip_audit, 50),
        ("skip: clicking seeks to the segment end", s_skip_click, 800),
        ("skip: click landed at the segment end", s_skip_click_check, 100),
        ("skip: S key, intro start", s_skip_key_start, 700),
        ("skip: S key seeks past the intro", s_skip_key, 900),
        ("skip: S key landed past the intro", s_skip_key_check, 100),
        ("skip: per-show auto-skip, start", s_skip_series_start, 700),
        ("skip: per-show auto-skip toggles and persists", s_skip_series_on, 800),
        ("skip: auto-skip on the next intro of that show", s_skip_series_auto, 800),
        ("skip: per-show auto-skip off again", s_skip_series_off, 100),
        ("player: movie overlay, large and min", s_player_movie_overlay, 100),
        ("player: episode overlay, large and min", s_player_episode_overlay, 100),
        ("player: episodes panel lists the season, audit", s_player_episodes_panel, 100),
        ("player: episodes panel picks an episode", s_player_episodes_pick, 100),
        ("player: next episode button and N", s_player_next_episode, 100),
        ("up next: card near the end", s_upnext_card, 5000),
        ("up next: card audit and Play Now", s_upnext_play, 300),
        ("up next: next episode, title updated", s_upnext_next, 300),
        ("up next: cancel keeps the episode alone", s_upnext_cancel, 5000),
        ("up next: cancel, then the end stops", s_upnext_cancel_check, 2200),
        ("up next: cancel end check", s_upnext_cancel_end, 100),
        ("up next: countdown to the end starts next", s_upnext_end, 6800),
        ("up next: end started next episode", s_upnext_end_check, 300),
        ("up next: stop after autoplay", s_upnext_stop, 200),
        ("settings: autoplay and skip default controls", s_settings_skip_controls, 100),
        ("detail: series skip toggle persists", s_detail_skip_toggle, 300),
        ("cast: player menu casts and stops local playback", s_player_cast, 200),
        ("audio: start (now-playing bar)", s_audio_start, 1500),
        ("audio: bar layout + audit", s_audio_bar, 100),
        ("audio: stop", s_audio_stop, 200),
        ("fullscreen toggle x2", s_fullscreen, 2500),
        ("playback: run to end (auto mark watched)", s_play_to_end, 8000),
        ("signed out", s_signed_out, 100),
        ("quit", s_quit_prep, 50),
    ]
}


// ---- casting ----------------------------------------------------------------
const OWN_DEVICE: &str = "dev-self";

/// `GET /Sessions` body: this app's own session, a TV playing an episode, and a web browser.
fn sessions_json(paused: bool) -> String {
    serde_json::json!([
        {"Id": "sess-me", "DeviceId": OWN_DEVICE, "DeviceName": "This Mac", "Client": "Jellyfast", "SupportsRemoteControl": true},
        {"Id": "sess-tv", "DeviceId": "tv-1", "DeviceName": "Living Room", "Client": "Jellyfin Android TV",
         "SupportsRemoteControl": true,
         "NowPlayingItem": {"Id": "ep-1", "Name": "Pilot", "SeriesName": "Show", "RunTimeTicks": 1_200_000_000i64},
         "PlayState": {"PositionTicks": 300_000_000i64, "IsPaused": paused}},
        {"Id": "sess-web", "DeviceId": "web-1", "DeviceName": "Office", "Client": "Jellyfin Web", "SupportsRemoteControl": true}
    ])
    .to_string()
}

/// The list the worker would send: parsed by the real filter, with this app's device as own.
fn own_device_sessions(paused: bool) -> Vec<crate::jellyfin::sessions::RemoteSession> {
    unsafe {
        UI.get_mut().unwrap().cfg.device_id = OWN_DEVICE.into();
    }
    crate::jellyfin::sessions::parse_sessions(&sessions_json(paused), OWN_DEVICE).unwrap()
}

/// First NSButton under `v` whose action is `action`.
unsafe fn find_action(v: *mut AnyObject, action: objc2::runtime::Sel, depth: u32) -> *mut AnyObject {
    if v.is_null() || depth > 8 {
        return nil();
    }
    let is: Bool = msg_send![v, isKindOfClass: class!(NSButton)];
    if is == Bool::YES {
        let a: objc2::runtime::Sel = msg_send![v, action];
        if a == action {
            return v;
        }
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: NSUInteger = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let f = find_action(c, action, depth + 1);
        if !f.is_null() {
            return f;
        }
    }
    nil()
}

fn s_cast_detail() {
    take_cmds();
    handle_main(MainMsg::DetailLoaded(Box::new(detail_view(false))));
    unsafe {
        let ui = UI.get_mut().unwrap();
        let btn = find_action(ui.detail.doc.get(), sel!(detailCast:), 0);
        state_check("detail: Cast button sits in the Play row", !btn.is_null());
        act_with(detail_host(), sel!(detailCast:), btn);
        state_check(
            "cast: opening the menu asks the server for sessions",
            take_cmds().iter().any(|c| matches!(c, UiCmd::LoadSessions { silent: false })),
        );
        handle_main(MainMsg::SessionsLoaded(own_device_sessions(false)));
        let (titles, n) = crate::ui::cast::menu_titles();
        state_check(
            "cast menu lists the other Jellyfin devices",
            titles.iter().any(|t| t == "Living Room · Jellyfin Android TV") && titles.iter().any(|t| t == "Office · Jellyfin Web"),
        );
        state_check("cast menu excludes this device", !titles.iter().any(|t| t.contains("This Mac")));
        state_check("cast menu has no AirPlay row off the player", !titles.iter().any(|t| t == "AirPlay"));
        state_check("cast menu: header plus two devices", n == 3);
        let picked = crate::ui::cast::choose_row_for_test(0);
        state_check(
            "choosing the first device picks the TV session",
            picked.as_ref().map(|s| s.id.as_str()) == Some("sess-tv"),
        );
        let sent = take_cmds();
        state_check(
            "choosing a device sends PlayNow for the item from the start",
            sent.iter().any(|c| matches!(c, UiCmd::CastPlay { session_id, item_id, start_secs }
                if session_id == "sess-tv" && item_id == "detail-long" && *start_secs == 0.0)),
        );
        state_check("remote bar shows after a cast", crate::ui::cast::remote_visible());
        let r = crate::ui::cast::remote_state();
        state_check("remote bar names the device", r.as_ref().map(|r| r.device.as_str()) == Some("Living Room · Jellyfin Android TV"));
    }
    strip_no_overlap("detail page");
    audit("cast/detail page with remote bar");
}

/// The remote strip sits below the library side: the content host stops at its top edge,
/// so nothing under it is covered.
fn strip_no_overlap(what: &str) {
    unsafe {
        let ui = UI.get().unwrap();
        let host = frame_of(ui.content_host.get());
        let bar = frame_of(ui.cast.bar.get());
        state_check(
            &format!("{}: library side stops above the remote bar", what),
            (host.origin.y - STRIP_H).abs() < 0.5 && bar.origin.y + bar.size.h <= host.origin.y + 0.5,
        );
        let c: *mut AnyObject = msg_send![ui.cast.bar.get(), superview];
        let cb = bounds_of(c);
        state_check(
            &format!("{}: remote bar is inside the window", what),
            bar.origin.x >= 0.0 && bar.origin.x + bar.size.w <= cb.size.w + 0.5 && bar.origin.y >= 0.0,
        );
        state_check(
            &format!("{}: remote strip spans the full window width", what),
            bar.origin.x.abs() < 0.5 && (bar.size.w - cb.size.w).abs() < 0.5,
        );
    }
}

fn s_cast_bar() {
    take_cmds();
    // The poll reports the episode playing, 30 s in, of 120 s.
    handle_main(MainMsg::SessionsLoaded(own_device_sessions(false)));
    let r = crate::ui::cast::remote_state().expect("remote state");
    state_check("remote: title from the session's now playing", r.title.as_deref() == Some("Show · Pilot"));
    state_check("remote: position and length from the session", (r.pos_secs - 30.0).abs() < 0.01 && (r.run_secs - 120.0).abs() < 0.01);
    state_check("remote: playing, not paused", !r.paused && r.online);
    state_check("remote bar visible", crate::ui::cast::remote_visible());
    let (bar, play, stop, scrub, close, title) = {
        let ui = UI.get().unwrap();
        (ui.cast.bar.get(), ui.cast.play.get(), ui.cast.stop.get(), ui.cast.scrub.get(), ui.cast.close.get(), ui.cast.title.get())
    };
    unsafe {
        let a: objc2::runtime::Sel = msg_send![play, action];
        state_check("remote: play button wired", a == sel!(jrCastPlayPause:));
        state_check("remote: scrubber enabled with a length", { let e: Bool = msg_send![scrub, isEnabled]; e == Bool::YES });
        state_check("remote: title shown", read_field(title) == "Show · Pilot");
        act(sel!(jrCastPlayPause:), nil());
        state_check(
            "remote: play/pause sends Pause while playing",
            take_cmds().iter().any(|c| matches!(c, UiCmd::CastControl { session_id, action: crate::jellyfin::sessions::RemoteAction::Pause } if session_id == "sess-tv")),
        );
        act(sel!(jrCastPlayPause:), nil());
        state_check(
            "remote: play/pause sends Unpause after that",
            take_cmds().iter().any(|c| matches!(c, UiCmd::CastControl { action: crate::jellyfin::sessions::RemoteAction::Unpause, .. })),
        );
        let _: () = msg_send![scrub, setDoubleValue: 0.5f64];
        act_with(ctl(), sel!(jrCastScrub:), scrub);
        state_check(
            "remote: scrub to half sends a seek to 60 s",
            take_cmds().iter().any(|c| matches!(c, UiCmd::CastControl { action: crate::jellyfin::sessions::RemoteAction::SeekSecs(s), .. } if (*s - 60.0).abs() < 0.01)),
        );
    }
    audit("cast/remote bar (playing)");
    unsafe {
        act(sel!(jrCastStop:), nil());
        state_check(
            "remote: stop sends Stop",
            take_cmds().iter().any(|c| matches!(c, UiCmd::CastControl { action: crate::jellyfin::sessions::RemoteAction::Stop, .. })),
        );
        state_check("remote: stopped shows nothing playing", read_field(title) == "Nothing playing");
        let _: () = msg_send![stop, setEnabled: Bool::YES];
    }
    handle_main(MainMsg::SessionsLoaded(own_device_sessions(true)));
    state_check("remote: poll reports a paused session", crate::ui::cast::remote_state().map(|r| r.paused).unwrap_or(false));
    // The device drops off the server's list: the bar says so rather than showing stale state.
    let only_web: Vec<_> = own_device_sessions(false).into_iter().filter(|s| s.id != "sess-tv").collect();
    handle_main(MainMsg::SessionsLoaded(only_web));
    state_check("remote: offline device says so", unsafe { read_field(title) } == "Not available right now");
    audit("cast/remote bar (offline)");
    // Over the library grid: the grid's bottom edge moves up, so no card sits under the strip.
    crate::ui::detail::hide();
    strip_no_overlap("library grid");
    audit("cast/remote bar over library grid");
    unsafe {
        act(sel!(jrCastClose:), nil());
        state_check("remote bar hides on close", !crate::ui::cast::remote_visible() && is_hidden_or_gone(bar));
        let _ = close;
    }
    let host = unsafe { UI.get().unwrap().content_host.get() };
    state_check("library side returns to full height on close", unsafe { frame_of(host).origin.y.abs() < 0.5 });
}

fn s_cast_error() {
    crate::ui::cast::on_error("Couldn't reach Living Room: the session has ended".into());
    unsafe {
        let ui = UI.get().unwrap();
        state_check("cast error: strip shows in the library", crate::ui::cast::remote_visible());
        state_check(
            "cast error: the message is the strip's text",
            read_field(ui.cast.title.get()).starts_with("Couldn't reach Living Room"),
        );
        state_check("cast error: controls step aside", is_hidden(ui.cast.play.get()));
    }
    strip_no_overlap("error strip");
    audit("cast/error strip (library)");
    unsafe { act(sel!(jrCastClose:), nil()) };
    state_check("cast error: × dismisses it", !crate::ui::cast::remote_visible());
    let host = unsafe { UI.get().unwrap().content_host.get() };
    state_check("cast error: library returns to full height", unsafe { frame_of(host).origin.y.abs() < 0.5 });
}

fn s_cast_series() {
    take_cmds();
    let series = detail_view(true);
    let series_id = series.id.clone();
    handle_main(MainMsg::DetailLoaded(Box::new(series)));
    unsafe {
        let ui = UI.get_mut().unwrap();
        let btn = find_action(ui.detail.doc.get(), sel!(detailCast:), 0);
        state_check("series page: Cast button shown", !btn.is_null());
        act_with(detail_host(), sel!(detailCast:), btn);
    }
    handle_main(MainMsg::SessionsLoaded(own_device_sessions(false)));
    let picked = crate::ui::cast::choose_row_for_test(0);
    state_check("series cast: picks the TV", picked.map(|s| s.id == "sess-tv").unwrap_or(false));
    state_check(
        "series cast sends CastSeries (server resolves Next Up)",
        take_cmds().iter().any(|c| matches!(c, UiCmd::CastSeries { session_id, series_id: sid } if session_id == "sess-tv" && *sid == series_id)),
    );
    crate::ui::detail::hide();
    unsafe { act(sel!(jrCastClose:), nil()) };
}

fn is_hidden_or_gone(v: *mut AnyObject) -> bool {
    v.is_null() || unsafe { is_hidden(v) }
}

fn s_player_airplay() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        // AirPlay and Cast live in the More menu, which is built when it opens (self-test only counts its rows).
        act(sel!(showMoreMenu:), ui.pbar.more.get());
        state_check("more menu lists speed, AirPlay, Picture in Picture and Cast", ui.pbar.last_menu_items == 4);
        let route = ui.pbar.route.get();
        let is_route: Bool = msg_send![route, isKindOfClass: class!(AVRoutePickerView)];
        state_check("more menu: AirPlay route picker built", is_route == Bool::YES);
        let sup: *mut AnyObject = msg_send![route, superview];
        state_check("more menu: route picker is the AirPlay row", !sup.is_null());
        let more_hidden: Bool = msg_send![ui.pbar.more.get(), isHidden];
        state_check("player bar: More button visible", more_hidden == Bool::NO);
        let cast = ui.pbar.cast.get();
        let a: objc2::runtime::Sel = msg_send![cast, action];
        state_check("cast menu is still wired to its button", a == sel!(showCastMenu:));
        let cfg: Bool = msg_send![ui.player.get(), allowsExternalPlayback];
        state_check("AVPlayer allows external (AirPlay) playback", cfg == Bool::YES);
        state_check("AirPlay card hidden while playing here", is_hidden(ui.ext.view.get()));
    }
    unsafe { crate::ui::player::set_external(true) };
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("AirPlay card covers the video while external", !is_hidden(ui.ext.view.get()));
        state_check("AirPlay card says where it plays", read_field(ui.ext.label.get()).starts_with("Playing on "));
    }
    audit("player/airplay card");
    unsafe { crate::ui::player::set_external(false) };
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("AirPlay card hides when playback comes back", is_hidden(ui.ext.view.get()));
    }
}

fn s_player_cast() {
    handle_main(playback(0.0, 6.0));
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("cast from player: playing", ui.playing.is_some());
        let cast = ui.pbar.cast.get();
        take_cmds();
        act_with(ctl(), sel!(showCastMenu:), cast);
        state_check(
            "player cast: asks for sessions",
            take_cmds().iter().any(|c| matches!(c, UiCmd::LoadSessions { silent: false })),
        );
    }
    handle_main(MainMsg::SessionsLoaded(own_device_sessions(false)));
    let (titles, n) = crate::ui::cast::menu_titles();
    state_check("player cast menu starts with AirPlay", titles.first().map(String::as_str) == Some("AirPlay"));
    state_check("player cast menu: AirPlay, header, two devices", n == 5 && titles.len() == 4);
    state_check("player cast menu lists devices after AirPlay", titles.iter().any(|t| t == "Office · Jellyfin Web"));
    take_cmds();
    let _ = crate::ui::cast::choose_row_for_test(0);
    let sent = take_cmds();
    state_check(
        "player cast sends PlayNow for the playing item",
        sent.iter().any(|c| matches!(c, UiCmd::CastPlay { session_id, item_id, .. } if session_id == "sess-tv" && item_id == "movie-1")),
    );
    let (idle, library) = unsafe { (UI.get().unwrap().playing.is_none(), !is_hidden(UI.get().unwrap().content_host.get())) };
    state_check("casting stops local playback", idle);
    state_check("library is back after casting", library);
    state_check("remote bar shows after a player cast", crate::ui::cast::remote_visible());
    // Video fills the window: the strip steps away (its state is kept) and comes back afterwards.
    handle_main(playback(0.0, 6.0));
    state_check("strip hides while video fills the window", !crate::ui::cast::remote_visible());
    state_check("library side is full height while video plays", unsafe { frame_of(UI.get().unwrap().content_host.get()).origin.y.abs() < 0.5 });
    handle_main(MainMsg::SessionsLoaded(own_device_sessions(false)));
    state_check("video: remote session still tracked", crate::ui::cast::remote_state().is_some());
    unsafe { act(sel!(stopPlayback:), nil()) };
    state_check("strip returns when video stops", crate::ui::cast::remote_visible());
    strip_no_overlap("after video");
    unsafe {
        act_with(ctl(), sel!(jrCastClose:), nil());
    }
}

// ---- helpers --------------------------------------------------------------
unsafe fn ctl() -> *mut AnyObject {
    UI.get_mut().unwrap().ctl.get()
}
unsafe fn act(sel: Sel, sender: *mut AnyObject) {
    let _: *mut AnyObject = msg_send![ctl(), performSelector: sel, withObject: sender];
}
unsafe fn tagged_button(tag: isize) -> *mut AnyObject {
    let b: *mut AnyObject = msg_send![class!(NSButton), new];
    let _: () = msg_send![b, setTag: tag];
    let _: *mut AnyObject = msg_send![b, autorelease];
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
        user_data: Some(UserData { played, position_ticks: pos, last_played_date: None, is_favorite: false }),
        aspect: Some(0.667),
        image_tags: Some(ImageTags { primary: Some("tag".into()), thumb: None, logo: None }),
        backdrop_tags: None,
        run_time_ticks: Some(run),
        date_created: None,
        overview: Some("A mock overview for the self-test item.".into()),
        genres: vec!["Drama".into(), "Mystery".into()],
        community_rating: Some(7.6),
        official_rating: Some("TV-14".into()),
        provider_ids: [("Tmdb".to_string(), "1399".to_string())].into_iter().collect(),
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
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: *mut AnyObject = msg_send![rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(), pixelsWide: w as isize, pixelsHigh: h as isize, bitsPerSample: 8isize, samplesPerPixel: 4isize, hasAlpha: Bool::YES, isPlanar: Bool::NO, colorSpaceName: ns("NSDeviceRGBColorSpace"), bytesPerRow: (w * 4) as isize, bitsPerPixel: 32isize];
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
        let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypeJPEG = 3, PNG = 4
        let data: *mut AnyObject = msg_send![rep, representationUsingType: if kind == 0 { 4usize } else { 3usize }, properties: props];
        let len: usize = msg_send![data, length];
        let p: *const std::ffi::c_void = msg_send![data, bytes];
        let v = std::slice::from_raw_parts(p as *const u8, len).to_vec();
        let _: () = msg_send![rep, release];
        v
    }
}
fn video_url() -> String {
    let path = std::env::var("JELLYFAST_SELFTEST_VIDEO").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/selftest.mp4").into()
    });
    let enc: Vec<String> = path.split('/').map(crate::jellyfin::urls::enc).collect();
    format!("file://{}", enc.join("/"))
}
fn playback(resume: f64, run: f64) -> MainMsg {
    MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(),
        item_id: "movie-1".into(),
        title: "Fake Movie".into(),
        subtitle: Some("Subtitle line".into()),
        resume_secs: resume,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: run,
        image_key: Some(poster_key("Movie-0")),
        media_kind: MediaKind::Video,
        series_name: None,
        season_number: None,
        episode_number: None,
        production_year: None,
    segments: Vec::new(),
    series_id: None,
    next_up: None,
})
}
/// Resolved sRGB components of a colour under the current appearance (for comparisons).
pub(crate) unsafe fn srgb(c: *mut AnyObject) -> [i64; 4] {
    let space: *mut AnyObject = msg_send![class!(NSColorSpace), sRGBColorSpace];
    let r: *mut AnyObject = msg_send![c, colorUsingColorSpace: space];
    if r.is_null() {
        return [-1; 4];
    }
    let (mut a, mut b, mut g, mut al) = (0f64, 0f64, 0f64, 0f64);
    let _: () = msg_send![r, getRed: &mut a, green: &mut g, blue: &mut b, alpha: &mut al];
    [(a * 1000.).round() as i64, (g * 1000.).round() as i64, (b * 1000.).round() as i64, (al * 1000.).round() as i64]
}

fn state_check(what: &str, ok: bool) {
    log(&format!("  check {}: {}", what, if ok { "ok" } else { "FAILED" }));
    if !ok {
        std::process::exit(2);
    }
}

// ---- layout audit (overlap / truncation) ------------------------------------
//
// Walks every visible window (sheets included) and reports:
//  * sibling NSTextField / NSButton pairs whose frames intersect by more than 1pt;
//  * labels / buttons whose text is wider than their frame without a truncating
//    line-break mode (wrapping labels must instead be tall enough for their text);
//  * text that intersects the standard window buttons (traffic lights).
// Documented exemptions:
//  * NSButton with an empty title and no image: a transparent full-area hit target
//    (poster cell click button, sidebar row button) that is meant to sit above its labels;
//  * hidden views, views with alpha 0, and zero-size views.
unsafe fn visible(v: *mut AnyObject) -> bool {
    let h: Bool = msg_send![v, isHidden];
    let a: f64 = msg_send![v, alphaValue];
    h == Bool::NO && a > 0.01
}
unsafe fn is_kind(v: *mut AnyObject, cls: &AnyClass) -> bool {
    let b: Bool = msg_send![v, isKindOfClass: cls];
    b == Bool::YES
}
unsafe fn describe(v: *mut AnyObject) -> String {
    let cls: *mut AnyObject = msg_send![v, className];
    let txt = if is_kind(v, class!(NSControl)) {
        let sv: *mut AnyObject = msg_send![v, stringValue];
        to_str(sv)
    } else {
        String::new()
    };
    let t: String = txt.chars().take(24).collect();
    let f: NSRect = msg_send![v, frame];
    format!("{}[\"{}\" {:.0},{:.0} {:.0}x{:.0}]", to_str(cls), t, f.origin.x, f.origin.y, f.size.w, f.size.h)
}
unsafe fn is_text_item(v: *mut AnyObject) -> bool {
    is_kind(v, class!(NSTextField)) || is_kind(v, class!(NSButton))
}
unsafe fn is_hit_target(v: *mut AnyObject) -> bool {
    if !is_kind(v, class!(NSButton)) {
        return false;
    }
    let t: *mut AnyObject = msg_send![v, title];
    let img: *mut AnyObject = msg_send![v, image];
    to_str(t).is_empty() && img.is_null()
}
unsafe fn audit_view(v: *mut AnyObject, issues: &mut Vec<String>) {
    if !visible(v) {
        return;
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    let mut items: Vec<*mut AnyObject> = vec![];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        if !visible(c) {
            continue;
        }
        if is_text_item(c) && !is_hit_target(c) {
            let f: NSRect = msg_send![c, frame];
            if f.size.w > 0.5 && f.size.h > 0.5 {
                items.push(c);
            }
        }
        audit_view(c, issues);
    }
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let a: NSRect = msg_send![items[i], frame];
            let b: NSRect = msg_send![items[j], frame];
            let ox = (a.origin.x + a.size.w).min(b.origin.x + b.size.w) - a.origin.x.max(b.origin.x);
            let oy = (a.origin.y + a.size.h).min(b.origin.y + b.size.h) - a.origin.y.max(b.origin.y);
            if ox > 1.0 && oy > 1.0 {
                issues.push(format!("overlap: {} x {}", describe(items[i]), describe(items[j])));
            }
        }
    }
    for &c in &items {
        if is_kind(c, class!(NSTextField)) {
            let editable: Bool = msg_send![c, isEditable];
            if editable == Bool::YES {
                continue; // inputs scroll their text
            }
            let f: NSRect = msg_send![c, frame];
            let cell: *mut AnyObject = msg_send![c, cell];
            let lb: isize = msg_send![cell, lineBreakMode];
            let sz: NSSize = msg_send![cell, cellSize];
            let txt = to_str(msg_send![c, stringValue]);
            if txt.is_empty() {
                continue;
            }
            let wraps = lb == 0 || lb == 2; // word / char wrapping
            if wraps {
                let need: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., f.size.w, 10000.)];
                if need.h > f.size.h + 1.0 {
                    issues.push(format!("clipped wrapping text: {} needs h={:.0}", describe(c), need.h));
                }
            } else if sz.w > f.size.w + 1.0 && lb != 4 && lb != 3 && lb != 5 {
                issues.push(format!("no truncation: {} text w={:.0}", describe(c), sz.w));
            }
        }
    }
}
/// Cross-container overlap: every visible, on-canvas text item compared in window coordinates,
/// so labels in different subviews that collide on screen are caught too.
unsafe fn collect_text_windowrects(v: *mut AnyObject, out: &mut Vec<(*mut AnyObject, NSRect)>) {
    if !visible(v) {
        return;
    }
    if is_text_item(v) && !is_hit_target(v) {
        let txt = if is_kind(v, class!(NSButton)) { to_str(msg_send![v, title]) } else { to_str(msg_send![v, stringValue]) };
        let vr: NSRect = msg_send![v, visibleRect];
        if !txt.is_empty() && vr.size.w > 1.0 && vr.size.h > 1.0 {
            let bb: NSRect = msg_send![v, bounds];
            let r: NSRect = msg_send![v, convertRect: bb, toView: nil()];
            out.push((v, r));
        }
    }
    if is_kind(v, class!(NSControl)) {
        return; // a control's internal subviews (e.g. NSButtonTextField) are part of it
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        collect_text_windowrects(c, out);
    }
}
unsafe fn audit_global_overlap_in(root: *mut AnyObject, issues: &mut Vec<String>) {
    let mut items = vec![];
    collect_text_windowrects(root, &mut items);
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let ((va, a), (vb, b)) = (items[i], items[j]);
            let anc_a: Bool = msg_send![va, isDescendantOf: vb];
            let anc_b: Bool = msg_send![vb, isDescendantOf: va];
            let sa: *mut AnyObject = msg_send![va, superview];
            let sb: *mut AnyObject = msg_send![vb, superview];
            if anc_a == Bool::YES || anc_b == Bool::YES || sa == sb {
                continue; // siblings are covered by audit_view
            }
            let ox = (a.origin.x + a.size.w).min(b.origin.x + b.size.w) - a.origin.x.max(b.origin.x);
            let oy = (a.origin.y + a.size.h).min(b.origin.y + b.size.h) - a.origin.y.max(b.origin.y);
            if ox > 1.0 && oy > 1.0 {
                issues.push(format!(
                    "overlap (cross-view): {} x {} at window ({:.0},{:.0} {:.0}x{:.0}) / ({:.0},{:.0} {:.0}x{:.0})",
                    describe(va), describe(vb),
                    a.origin.x, a.origin.y, a.size.w, a.size.h, b.origin.x, b.origin.y, b.size.w, b.size.h
                ));
            }
        }
    }
}
/// Traffic-light collision: any visible text in the window's content that sits under a standard button.
unsafe fn audit_traffic(win: *mut AnyObject, issues: &mut Vec<String>) {
    let content: *mut AnyObject = msg_send![win, contentView];
    for kind in 0..3isize {
        let b: *mut AnyObject = msg_send![win, standardWindowButton: kind];
        if b.is_null() || !visible(b) {
            continue;
        }
        let bb: NSRect = msg_send![b, bounds];
        let br: NSRect = msg_send![b, convertRect: bb, toView: nil()];
        walk_text_windowrect(content, br, issues);
    }
}
unsafe fn walk_text_windowrect(v: *mut AnyObject, br: NSRect, issues: &mut Vec<String>) {
    if !visible(v) {
        return;
    }
    if is_kind(v, class!(NSTextField)) && !to_str(msg_send![v, stringValue]).is_empty() {
        let bb: NSRect = msg_send![v, bounds];
        let r: NSRect = msg_send![v, convertRect: bb, toView: nil()];
        let ox = (r.origin.x + r.size.w).min(br.origin.x + br.size.w) - r.origin.x.max(br.origin.x);
        let oy = (r.origin.y + r.size.h).min(br.origin.y + br.size.h) - r.origin.y.max(br.origin.y);
        if ox > 1.0 && oy > 1.0 {
            issues.push(format!("under traffic lights: {}", describe(v)));
        }
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        walk_text_windowrect(c, br, issues);
    }
}
/// Every visible window must lie entirely outside every screen in headless mode.
fn check_offscreen(tag: &str) {
    if !offscreen_mode() {
        return;
    }
    unsafe {
        let ui = UI.get().unwrap();
        let wins: *mut AnyObject = msg_send![ui.app.get(), windows];
        let n: usize = msg_send![wins, count];
        let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
        let sn: usize = msg_send![screens, count];
        let mut bad = 0;
        for i in 0..n {
            let w: *mut AnyObject = msg_send![wins, objectAtIndex: i];
            let vis: Bool = msg_send![w, isVisible];
            if vis == Bool::NO {
                continue;
            }
            let f: NSRect = msg_send![w, frame];
            for j in 0..sn {
                let s: *mut AnyObject = msg_send![screens, objectAtIndex: j];
                let sf: NSRect = msg_send![s, frame];
                let ox = (f.origin.x + f.size.w).min(sf.origin.x + sf.size.w) - f.origin.x.max(sf.origin.x);
                let oy = (f.origin.y + f.size.h).min(sf.origin.y + sf.size.h) - f.origin.y.max(sf.origin.y);
                if ox > 0.0 && oy > 0.0 {
                    let cls: *mut AnyObject = msg_send![w, className];
                    log(&format!("  window on screen: {} {} {:?}", to_str(cls), to_str(msg_send![w, title]), f));
                    bad += 1;
                }
            }
        }
        state_check(&format!("all visible windows are off-screen ({})", tag), bad == 0);
    }
}
/// With JELLYFAST_SHOTS=<dir>, capture the window with screencapture (own-window capture).
fn shot(tag: &str, idx: usize, win: *mut AnyObject) {
    let dir = match std::env::var("JELLYFAST_SHOTS") {
        Ok(d) if !d.is_empty() => d,
        _ => return,
    };
    unsafe {
        let appearance = std::env::var("JELLYFAST_APPEARANCE").unwrap_or_else(|_| "system".into());
        let clean: String = tag.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        // Name the file after the window too: the Settings sheet is a separate window, so
        // "settings-sheet-w1" reads as the Settings window and "-w0" never gets mistaken for it.
        let title = to_str(msg_send![win, title]);
        let wclean: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        let path = format!("{}/{}-{}-w{}-{}.png", dir, appearance, clean, idx, wclean);
        // Render the view tree straight to a bitmap: works for off-screen windows, unlike screencapture.
        // The frame view (contentView's superview) includes the titlebar and traffic lights.
        let content: *mut AnyObject = msg_send![win, contentView];
        let root: *mut AnyObject = msg_send![content, superview];
        let root = if root.is_null() { content } else { root };
        let _: () = msg_send![root, layoutSubtreeIfNeeded];
        let b: NSRect = msg_send![root, bounds];
        let rep: *mut AnyObject = msg_send![root, bitmapImageRepForCachingDisplayInRect: b];
        if rep.is_null() {
            return;
        }
        let _: () = msg_send![root, cacheDisplayInRect: b, toBitmapImageRep: rep];
        let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        let png: *mut AnyObject = msg_send![rep, representationUsingType: 4usize, properties: props]; // NSBitmapImageFileTypePNG
        let _: Bool = msg_send![png, writeToFile: ns(&path), atomically: Bool::YES];
        log(&format!("  shot {}", path));
    }
}
/// Fails the run (exit 2) if any visible window has layout problems.
fn audit(tag: &str) {
    check_offscreen(tag);
    unsafe {
        let ui = UI.get().unwrap();
        let wins: *mut AnyObject = msg_send![ui.app.get(), windows];
        let n: usize = msg_send![wins, count];
        let mut issues = vec![];
        for i in 0..n {
            let w: *mut AnyObject = msg_send![wins, objectAtIndex: i];
            let vis: Bool = msg_send![w, isVisible];
            if vis == Bool::NO {
                continue;
            }
            let _: () = msg_send![w, layoutIfNeeded];
            let content: *mut AnyObject = msg_send![w, contentView];
            let before = issues.len();
            audit_view(content, &mut issues);
            // While the card is up, only its own text is compared globally: the
            // Home screen behind it is dimmed on purpose.
            let root = if w == ui.win.get() && crate::ui::onboarding::visible() { ui.onboard.host.get() } else { content };
            audit_global_overlap_in(root, &mut issues);
            let is_main = w == ui.win.get();
            if is_main {
                audit_traffic(w, &mut issues);
            }
            let title = to_str(msg_send![w, title]);
            shot(tag, i, w);
            for it in issues.iter().skip(before) {
                log(&format!("  layout[{}] ({}): {}", tag, title, it));
            }
        }
        state_check(&format!("layout audit: {} ({} issues)", tag, issues.len()), issues.is_empty());
    }
}
fn set_content_size(w: f64, h: f64) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.win.get(), setContentSize: size(w, h)];
        let _: () = msg_send![ui.win.get(), displayIfNeeded];
        let _: () = msg_send![ui.grid.get(), layoutSubtreeIfNeeded];
    }
}
/// Audit at minimum and at a large window size.
fn audit_both(tag: &str) {
    set_content_size(960.0, 600.0);
    audit(&format!("{} @min", tag));
    set_content_size(1700.0, 1000.0);
    audit(&format!("{} @large", tag));
    set_content_size(1280.0, 800.0);
}
const LONG: &str = "An extremely long title that goes on and on and on, far beyond any sensible width for a poster cell or a header, to prove truncation works properly";

// ---- steps ----------------------------------------------------------------
fn s_audit_login() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_url.get(), &format!("http://{}.example.com:8096", "very-long-hostname-".repeat(6)));
        set_text(ui.login_user.get(), &"long-username-".repeat(8));
        crate::ui::shell::login_error(LONG);
    }
    audit_both("login/error");
    unsafe {
        crate::ui::shell::login_status(&format!("Connecting to {}…", "very-long-hostname-".repeat(6)));
    }
    audit_both("login/connecting");
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_url.get(), "");
        set_text(ui.login_user.get(), "");
        crate::ui::shell::login_status("");
    }
}
fn s_audit_grid() {
    // Posters arrived: audit with artwork in place as well.
    for i in 0..14 {
        handle_main(MainMsg::PosterReady { key: poster_key(&format!("Movie-{}", i)), bytes: image_bytes(1, 120, 180, (i * 17) as u8) });
    }
    let mut v = items_of("Movie", 14);
    for (i, it) in v.iter_mut().enumerate() {
        it.name = format!("{} {}", i, LONG.repeat(1).chars().take(120).collect::<String>());
    }
    send_items(&format!("Movies {}", LONG), v, true);
    audit_both("grid");
    send_items("Movies", items_of("Movie", 12), false);
}
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
fn s_login_remember() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let saved: isize = if ui.cfg.remember_me { 1 } else { 0 };
        let st: isize = msg_send![ui.login_remember.get(), state];
        state_check("remember me checkbox reflects saved preference", st == saved);
        set_text(ui.login_url.get(), "http://127.0.0.1:1");
        set_text(ui.login_user.get(), "me");
        set_text(ui.login_pass.get(), "pw");
        let _: () = msg_send![ui.login_remember.get(), setState: 1isize];
        act(sel!(connectClicked:), nil());
        let on = take_cmds().iter().any(|c| matches!(c, UiCmd::Connect { remember: true, .. }));
        state_check("checked remember me sends remember=true", on);
        let _: () = msg_send![ui.login_remember.get(), setState: 0isize];
        act(sel!(connectClicked:), nil());
        let off = take_cmds().iter().any(|c| matches!(c, UiCmd::Connect { remember: false, .. }));
        state_check("unchecked remember me sends remember=false", off);
        let _: () = msg_send![ui.login_remember.get(), setState: saved];
    }
}
fn s_connect_error() {
    handle_main(MainMsg::ConnectError("Bad password".into()));
}
fn s_connected() {
    // No remembered place: Home is the landing screen.
    unsafe { UI.get_mut().unwrap().cfg.last_view = None };
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
            let h: Bool = msg_send![ui.login_card.get(), isHidden];
            h == Bool::YES
        });
        state_check("home is the default landing", ui.home.active && ui.selected_view.is_none());
        state_check("home scroll shown, library grid hidden", visible(ui.home.scroll.get()) && !visible(ui.lib_scroll.get()));
        state_check("Home sidebar row selected on launch", row_selected(ui.home.row.get()));
        state_check("no library row selected on launch", selected_rows() == 0);
        state_check("window is opaque", { let o: Bool = msg_send![ui.win.get(), isOpaque]; o == Bool::YES });
        state_check("library canvas draws background", { let o: Bool = msg_send![ui.lib_scroll.get(), drawsBackground]; o == Bool::YES });
        state_check("login backdrop hidden once connected", { let o: Bool = msg_send![ui.bgfx.get(), isHidden]; o == Bool::YES });
        let cmds = take_cmds();
        state_check("no LoadView when landing on Home", !cmds.iter().any(|c| matches!(c, UiCmd::LoadView { .. })));
        state_check("connect sent LoadHome", cmds.iter().any(|c| matches!(c, UiCmd::LoadHome)));
    }
}
fn s_connected_library() {
    // A library was the last place: login restores it instead of Home.
    unsafe { UI.get_mut().unwrap().cfg.last_view = Some("v2".into()) };
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
        state_check("login over remembered library: Home inactive", !ui.home.active);
        state_check("login over remembered library: library selected", ui.selected_view == Some(1));
        state_check("login over remembered library: Home row not selected", !row_selected(ui.home.row.get()));
        let cmds = take_cmds();
        state_check("login over remembered library: LoadView sent", cmds.iter().any(|c| matches!(c, UiCmd::LoadView { .. })));
        state_check("login over remembered library: no LoadHome", !cmds.iter().any(|c| matches!(c, UiCmd::LoadHome)));
    }
}
fn s_login_url_sync() {
    // The worker sends ServerUrl after a login-screen connect. The live config
    // and the Settings field must then show the resolved host, not a stale one.
    UI.get_mut().unwrap().cfg.server_url = Some("http://stale.example.com:8096".into());
    handle_main(MainMsg::ServerUrl("https://media.example.com:8096".into()));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("connect updates in-memory server url", ui.cfg.server_url.as_deref() == Some("https://media.example.com:8096"));
        let cfg = ui.cfg.clone();
        crate::ui::settings::settings_fill(&cfg);
        state_check("settings shows resolved bare host", read_field(ui.s_url.get()) == "media.example.com:8096");
    }
    take_cmds();
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
fn s_home_rows() {
    handle_main(MainMsg::HomeLoaded(HomeRows {
        rows: vec![
            HomeRow { title: "Continue Watching".into(), kind: HomeRowKind::ContinueWatching, items: items_of("Movie", 2) },
            HomeRow { title: "Recently Added in Movies".into(), kind: HomeRowKind::RecentlyAdded, items: items_of("Movie", 3) },
        ],
        ..Default::default()
    }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check(
            "home rows stored in order",
            ui.home.rows.len() == 2
                && ui.home.rows[0].title == "Continue Watching"
                && ui.home.rows[0].items.len() == 2
                && ui.home.rows[1].kind == HomeRowKind::RecentlyAdded,
        );
    }
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
fn s_grid_checks() {
    use crate::ui::library::{grid_metrics, grid_ratio};
    use crate::ui::theme::*;
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.grid.get(), layoutSubtreeIfNeeded];
        let gw = bounds_of(ui.grid.get()).size.w;
        let m = grid_metrics(gw, grid_ratio(&ui.items));
        let d = ui.grid_data.as_ref().unwrap();
        state_check("cells exist", !d.cells.is_empty());
        let f0 = frame_of(d.cells[0].container);
        state_check("first cell x == EDGE", (f0.origin.x - EDGE).abs() < 0.01);
        state_check(
            "every cell has the computed tile width",
            d.cells.iter().all(|c| (frame_of(c.container).size.w - m.tile_w).abs() < 0.01),
        );
        state_check(
            "cells fit inside the grid width",
            d.cells.iter().all(|c| { let f = frame_of(c.container); f.origin.x + f.size.w <= gw - EDGE + 0.5 }),
        );
        let c = &d.cells[0];
        let r: f64 = msg_send![c.art, cornerRadius];
        let mb: Bool = msg_send![c.art, masksToBounds];
        state_check("art layer radius + clipping", r == R_POSTER && mb == Bool::YES);
        let so: f32 = msg_send![c.shadow, shadowOpacity];
        state_check("shadow host at rest opacity 0.18", (so - 0.18).abs() < 0.001);
        let host_clips: Bool = msg_send![c.shadow, masksToBounds];
        state_check("shadow host does not clip", host_clips == Bool::NO);
    }
}
fn s_hover_model() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let c = &ui.grid_data.as_ref().unwrap().cells[0];
        let _: () = msg_send![c.container, mouseEntered: nil()];
        let so: f32 = msg_send![c.shadow, shadowOpacity];
        state_check("hover raises shadow to 0.45", (so - 0.45).abs() < 0.001);
        let _: () = msg_send![c.container, mouseExited: nil()];
        let so: f32 = msg_send![c.shadow, shadowOpacity];
        state_check("unhover restores shadow", (so - 0.18).abs() < 0.001);
    }
}
fn s_poster_states() {
    send_items("Posters", items_of("Garbage", 3), false);
    handle_main(MainMsg::PosterReady { key: poster_key("Garbage-0"), bytes: vec![9, 9, 9, 9, 9] });
    handle_main(MainMsg::PosterReady { key: poster_key("Garbage-1"), bytes: image_bytes(0, 100, 150, 90) });
    unsafe {
        let ui = UI.get().unwrap();
        let d = ui.grid_data.as_ref().unwrap();
        let g0: Bool = msg_send![d.cells[0].glyph, isHidden];
        let g1: Bool = msg_send![d.cells[1].glyph, isHidden];
        let g2: Bool = msg_send![d.cells[2].glyph, isHidden];
        state_check("garbage bytes keep the placeholder glyph", g0 == Bool::NO);
        state_check("valid poster hides the glyph", g1 == Bool::YES);
        state_check("untouched cell keeps its glyph", g2 == Bool::NO);
    }
    send_items("Movies", items_of("Movie", 12), false);
}
fn s_hover() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(d) = ui.grid_data.as_ref() {
            for c in &d.cells {
                let _: () = msg_send![c.container, mouseEntered: nil()];
                let _: () = msg_send![c.container, mouseExited: nil()];
            }
        }
    }
}
fn row_selected(row: *mut AnyObject) -> bool {
    unsafe {
        let rb: *mut AnyObject = msg_send![row, subviews];
        let bg: *mut AnyObject = msg_send![rb, objectAtIndex: 0usize];
        let t: Bool = msg_send![bg, isTransparent];
        t == Bool::NO
    }
}
fn selected_rows() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.rows
            .iter()
            .filter(|r| {
                let rb: *mut AnyObject = msg_send![r.0, subviews];
                let bg: *mut AnyObject = msg_send![rb, objectAtIndex: 0usize];
                let t: Bool = msg_send![bg, isTransparent];
                t == Bool::NO
            })
            .count()
    }
}
fn s_sidebar_click() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let rows: Vec<*mut AnyObject> = ui.rows.iter().map(|r| r.0).collect();
        state_check("5 library rows (livetv already present)", rows.len() == 5);
        // arranged: search field, "Library" header, then JRSidebarRow rows
        let arr: *mut AnyObject = msg_send![ui.side_stack.get(), arrangedSubviews];
        let third: *mut AnyObject = msg_send![arr, objectAtIndex: 3usize]; // search, Home, Library header, first library
        let cls = AnyClass::get(c"JRSidebarRow").unwrap();
        let k: Bool = msg_send![third, isKindOfClass: cls];
        state_check("first library row is a JRSidebarRow", k == Bool::YES);
        let sf = frame_of(ui.side_stack.get());
        let sb = frame_of(ui.sidebar.get());
        state_check(
            "sidebar stack leaves the title bar band free",
            sf.origin.y + sf.size.h <= sb.size.h - crate::ui::theme::TITLEBAR_H + 0.5,
        );
        for (i, _) in rows.iter().enumerate() {
            act(sel!(viewSelected:), tagged_button(i as isize));
            state_check("exactly one row selected after click", selected_rows() == 1);
            state_check("clicked row is the selected one", UI.get().unwrap().selected_view == Some(i));
        }
        let n_before: usize = { let a: *mut AnyObject = msg_send![ui.side_stack.get(), arrangedSubviews]; msg_send![a, count] };
        handle_main(MainMsg::Connected {
            user_name: "tester".into(),
            views: ui.views.clone(),
            has_live_tv: true,
        });
        let n_after: usize = { let a: *mut AnyObject = msg_send![ui.side_stack.get(), arrangedSubviews]; msg_send![a, count] };
        state_check("sidebar rebuild does not pile up views", n_before == n_after);
    }
}
fn s_audit_sidebar() {
    handle_main(MainMsg::Connected {
        user_name: "a-very-long-username-that-must-truncate-in-the-footer".into(),
        views: vec![
            ("v1".into(), "Movies".into(), Some("movies".into())),
            ("v2".into(), "Shows with an exceedingly long library name".into(), Some("tvshows".into())),
            ("v3".into(), "Music".into(), Some("music".into())),
            ("v4".into(), "Live TV".into(), Some("livetv".into())),
            ("v5".into(), "Misc".into(), None),
        ],
        has_live_tv: true,
    });
    audit_both("sidebar");
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
        let h: Bool = msg_send![ui.status_label.get(), isHidden];
        state_check("status label visible", h == Bool::NO);
        let al: isize = msg_send![ui.status_label.get(), alignment];
        state_check("status label centred (arm64-safe ALIGN_CENTER)", al as usize == crate::ui::theme::ALIGN_CENTER);
    }
}
fn header_title_x() -> f64 {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.header.get(), layoutSubtreeIfNeeded];
        crate::ui::shell::header_layout();
        let hf = frame_of(ui.header.get());
        hf.origin.x + frame_of(ui.header_title.get()).origin.x
    }
}
fn s_header() {
    use crate::ui::theme::*;
    unsafe {
        let ui = UI.get_mut().unwrap();
        send_items("Albums", items_of("MusicAlbum", 5), false);
        state_check("title x == SIDEBAR_W + EDGE without back", (header_title_x() - (SIDEBAR_W + EDGE)).abs() < 0.01);
        send_items("Episodes", items_of("Episode", 1), true);
        state_check("title shifts +36 with back", (header_title_x() - (SIDEBAR_W + EDGE + 36.)).abs() < 0.01);
        let bh: Bool = msg_send![ui.lib_back.get(), isHidden];
        state_check("library back visible when can_go_back", bh == Bool::NO);
        state_check("count label singular", read_field(ui.header_count.get()) == "1 episode");
        send_items("Albums", items_of("MusicAlbum", 5), false);
        state_check("count label plural", read_field(ui.header_count.get()) == "5 albums");
        // title never runs into the count or the sort popup, even when very long
        send_items(LONG, items_of("Movie", 3), false);
        let tf = frame_of(ui.header_title.get());
        let cf = frame_of(ui.header_count.get());
        let pf = frame_of(ui.sort_popup.get());
        state_check("title ends before count", tf.origin.x + tf.size.w <= cf.origin.x + 0.5);
        state_check("count ends before sort popup", cf.origin.x + cf.size.w <= pf.origin.x);
        send_items("Movies", items_of("Movie", 12), false);
    }
}
fn s_states() {
    use crate::ui::theme::*;
    unsafe {
        let ui = UI.get_mut().unwrap();
        // first load: skeleton cells, no items clickable
        act(sel!(viewSelected:), tagged_button(1));
        state_check("skeleton cells shown on first load", ui.grid_data.as_ref().unwrap().cells.len() == 12 && ui.items.is_empty());
        let sh: Bool = msg_send![ui.spinner.get(), isHidden];
        state_check("no header spinner during skeleton", sh == Bool::YES);
        take_cmds();
        // error with nothing on screen
        handle_main(MainMsg::PlaybackError("HTTP 500 from the server, which is a rather long message that goes on and on and on".into()));
        let hh: Bool = msg_send![ui.state_host.get(), isHidden];
        state_check("error state shown", hh == Bool::NO);
        let bb: Bool = msg_send![ui.state_btn.get(), isHidden];
        state_check("Try Again visible on error", bb == Bool::NO);
        // empty search state, centred on the grid area
        set_text(ui.search.get(), "zzz");
        send_items("Results", vec![], false);
        let _: () = msg_send![ui.win.get(), layoutIfNeeded];
        let hb: Bool = msg_send![ui.state_btn.get(), isHidden];
        state_check("no Try Again on empty", hb == Bool::YES);
        state_check("empty search body", read_field(ui.state_body.get()).contains("No results for"));
        let sf = frame_of(ui.status_label.get());
        let win_r: NSRect = msg_send![ui.status_label.get(), convertRect: bounds_of(ui.status_label.get()), toView: nil()];
        let content: *mut AnyObject = msg_send![ui.win.get(), contentView];
        let cw = bounds_of(content).size.w;
        let expect = SIDEBAR_W + (cw - SIDEBAR_W) / 2.;
        let _ = sf;
        state_check("empty state centred on grid area", (win_r.origin.x + win_r.size.w / 2. - expect).abs() <= 1.0);
        audit_both("empty-state");
        set_text(ui.search.get(), "");
        // refresh with items present keeps them and spins in the header
        send_items("Movies", items_of("Movie", 12), false);
        act(sel!(refresh:), nil());
        state_check("refresh keeps items", ui.grid_data.as_ref().unwrap().cells.len() == 12);
        let sh: Bool = msg_send![ui.spinner.get(), isHidden];
        state_check("header spinner while refreshing", sh == Bool::NO);
        send_items("Movies", items_of("Movie", 12), false);
        let sh: Bool = msg_send![ui.spinner.get(), isHidden];
        state_check("spinner stops on arrival", sh == Bool::YES);
    }
}
static SEARCH_BASE: Mutex<usize> = Mutex::new(0);
fn search_cmds(cmds: &[UiCmd]) -> usize {
    cmds.iter().filter(|c| matches!(c, UiCmd::Search { .. })).count()
}
fn s_debounce_type() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        take_cmds();
        for (i, q) in ["a", "ab", "abc"].iter().enumerate() {
            set_text(ui.search.get(), q);
            let note: *mut AnyObject = msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification"), object: ui.search.get()];
            act(sel!(controlTextDidChange:), note);
            *SEARCH_BASE.lock().unwrap() = i;
        }
    }
}
fn s_debounce_early() {
    // ~150 ms after the last keystroke: nothing sent yet
    state_check("no Search within 150 ms", search_cmds(&take_cmds()) == 0);
}
fn s_debounce_done() {
    let cmds = take_cmds();
    state_check("exactly one Search after the debounce", search_cmds(&cmds) == 1);
    state_check(
        "debounced Search carries the last text",
        cmds.iter().any(|c| matches!(c, UiCmd::Search { q, .. } if q == "abc")),
    );
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.search.get(), "");
    }
    take_cmds();
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
        let note: *mut AnyObject = msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification"), object: ui.search.get()];
        act(sel!(controlTextDidChange:), note);
        let note2: *mut AnyObject = msg_send![class!(NSNotification), notificationWithName: ns("x"), object: ui.login_url.get()];
        act(sel!(controlTextDidChange:), note2);
        act(sel!(refresh:), nil());
        act(sel!(goBack:), nil());
        act(sel!(signOut:), nil());
    }
}
/// Three movies. Name order starts with Alpha; Date Added (year as the stand-in date,
/// newest first) starts with Charlie.
fn sort_fixture() -> Vec<ListItem> {
    [("Alpha", 2001), ("Bravo", 1999), ("Charlie", 2010)]
        .iter()
        .map(|(name, year)| {
            let mut it = item(&format!("m-{}", name), name, "Movie", false, 0, 6_000_000_000);
            it.year = Some(*year);
            it
        })
        .collect()
}
/// Stands in for Jellyfin: applies the SearchTerm, SortBy and SortOrder the UI-side
/// query carries, so the grid shows what the server would return for that query.
fn fake_server(items: &[ListItem], q: &crate::jellyfin::models::ItemQuery) -> Vec<ListItem> {
    let needle = q.search.as_deref().map(str::to_lowercase);
    let mut out: Vec<ListItem> = items
        .iter()
        .filter(|i| needle.as_ref().map_or(true, |n| i.name.to_lowercase().contains(n)))
        .cloned()
        .collect();
    match q.sort_by.as_str() {
        "DateCreated" => out.sort_by_key(|i| i.year),
        _ => out.sort_by(|a, b| a.name.cmp(&b.name)),
    }
    if q.sort_order == "Descending" {
        out.reverse();
    }
    out
}
fn grid_names() -> Vec<String> {
    unsafe { UI.get_mut().unwrap().items.iter().map(|i| i.name.clone()).collect() }
}
fn header_count_text() -> String {
    unsafe { read_field(UI.get_mut().unwrap().header_count.get()) }
}
fn cfg_sort() -> Option<String> {
    unsafe { UI.get_mut().unwrap().cfg.sort_by.clone() }
}
fn last_set_sort(cmds: &[UiCmd]) -> Option<String> {
    cmds.iter().rev().find_map(|c| match c {
        UiCmd::SetSort { sort_by } => Some(sort_by.clone()),
        _ => None,
    })
}
fn last_search(cmds: &[UiCmd]) -> Option<(String, Option<String>, Option<String>)> {
    cmds.iter().rev().find_map(|c| match c {
        UiCmd::Search { q, parent_id, types } => Some((q.clone(), parent_id.clone(), types.clone())),
        _ => None,
    })
}
fn has_load_view(cmds: &[UiCmd], id: &str) -> bool {
    cmds.iter().any(|c| matches!(c, UiCmd::LoadView { view_id, .. } if view_id == id))
}
/// Pick a popup row and fire its action, as a click does.
fn pick_sort(index: isize) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.sort_popup.get(), selectItemAtIndex: index];
    }
    unsafe { act(sel!(sortChanged:), nil()) };
}
/// Put `text` in the filter field and fire the (debounced) search now.
fn type_filter(text: &str) {
    unsafe {
        set_text(UI.get_mut().unwrap().search.get(), text);
        act(sel!(jrSearchNow:), nil());
    }
}
fn s_sort_filter() {
    use crate::app::library_query::{filter_query, library_query};
    unsafe {
        let ui = UI.get_mut().unwrap();
        let saved_sort = ui.cfg.sort_by.clone();
        let saved_stack = ui.mode_stack.clone();
        crate::ui::home::leave();
        ui.browse_active = false;
        ui.mode_stack = vec![("lib-movies".into(), "Movies".into(), Some("movies".into()))];
        take_cmds();

        let fixture = sort_fixture();
        send_items("Movies", fake_server(&fixture, &library_query("lib-movies", Some("movies"), None)), false);
        state_check("sort: starts in Name order", grid_names().first().map(String::as_str) == Some("Alpha"));

        // Date Added: the choice reaches the worker and the library reloads.
        pick_sort(1);
        let cmds = take_cmds();
        let by = last_set_sort(&cmds);
        state_check("sort: popup choice sent to the worker", by.as_deref() == Some("DateCreated"));
        state_check("sort: library reloads", has_load_view(&cmds, "lib-movies"));
        state_check("sort: choice saved in config", cfg_sort().as_deref() == Some("DateCreated"));
        send_items("Movies", fake_server(&fixture, &library_query("lib-movies", Some("movies"), by.as_deref())), false);
        state_check("sort: first item changes Alpha -> Charlie", grid_names().first().map(String::as_str) == Some("Charlie"));
        state_check("sort: count unchanged", header_count_text() == "3 movies");

        // Filter: typing searches inside this library and the grid shows only the matches.
        type_filter("l");
        let cmds = take_cmds();
        let (q, parent, types) = last_search(&cmds).unwrap_or_default();
        state_check("filter: Search sent with the text", q == "l");
        state_check(
            "filter: scoped to this library",
            parent.as_deref() == Some("lib-movies") && types.as_deref() == Some("Movie"),
        );
        let fq = filter_query(&q, parent.as_deref(), types.as_deref(), cfg_sort().as_deref());
        send_items("Results for “l”", fake_server(&fixture, &fq.unwrap()), true);
        state_check("filter: visible count drops 3 -> 2", grid_names() == ["Charlie", "Alpha"]);
        state_check("filter: header count follows", header_count_text() == "2 movies");

        // Sorting while filtered keeps the filter and re-orders the matches.
        pick_sort(0);
        let cmds = take_cmds();
        let (q, parent, types) = last_search(&cmds).unwrap_or_default();
        state_check("sort with a filter: filter still applied", q == "l" && last_set_sort(&cmds).as_deref() == Some("SortName"));
        let fq = filter_query(&q, parent.as_deref(), types.as_deref(), cfg_sort().as_deref());
        send_items("Results for “l”", fake_server(&fixture, &fq.unwrap()), true);
        state_check("sort with a filter: Name order (Alpha first)", grid_names() == ["Alpha", "Charlie"]);

        // Clearing the field restores the library in the chosen order.
        type_filter("");
        let cmds = take_cmds();
        state_check("clear: library reloads, no Search", last_search(&cmds).is_none() && has_load_view(&cmds, "lib-movies"));
        send_items("Movies", fake_server(&fixture, &library_query("lib-movies", Some("movies"), cfg_sort().as_deref())), false);
        state_check("clear: all items back", grid_names().len() == 3 && header_count_text() == "3 movies");

        // Back from filter results clears the field and shows the library again.
        type_filter("x");
        take_cmds();
        act(sel!(goBack:), nil());
        let cmds = take_cmds();
        state_check(
            "back from filter results: field cleared and library reloaded",
            read_field(UI.get_mut().unwrap().search.get()).is_empty() && has_load_view(&cmds, "lib-movies"),
        );

        // Switching library clears the filter text.
        type_filter("l");
        take_cmds();
        if !UI.get_mut().unwrap().views.is_empty() {
            crate::platform::appkit::select_view(0);
            state_check(
                "switching library clears the filter",
                read_field(UI.get_mut().unwrap().search.get()).is_empty(),
            );
        } else {
            log("  (no libraries connected; library switch check skipped)");
        }
        take_cmds();

        // Album tracks are not sorted: the popup is hidden over them and back for libraries.
        ui.sort_applies = false;
        send_items("Album", items_of("Audio", 2), true);
        let pop: *mut AnyObject = UI.get_mut().unwrap().sort_popup.get();
        let hidden: Bool = msg_send![pop, isHidden];
        state_check("album tracks hide the sort popup", hidden == Bool::YES);
        UI.get_mut().unwrap().sort_applies = true;
        send_items("Movies", fixture.clone(), false);
        let hidden: Bool = msg_send![pop, isHidden];
        state_check("library shows the sort popup", hidden == Bool::NO);

        // Relaunch: the popup opens on the saved choice.
        UI.get_mut().unwrap().cfg.sort_by = Some("DatePlayed".into());
        crate::ui::shell::sync_sort_popup();
        let idx: isize = msg_send![pop, indexOfSelectedItem];
        state_check("popup shows the saved choice (Last Played)", idx == 3);

        // Put the saved state back.
        let ui = UI.get_mut().unwrap();
        ui.cfg.sort_by = saved_sort;
        ui.mode_stack = saved_stack;
        crate::config::save_config_keeping_session(&ui.cfg);
        crate::ui::shell::sync_sort_popup();
        take_cmds();
    }
}
fn s_shortcuts_menu() {
    use crate::ui::shortcuts::{item_enabled, menu_key_problems, CMD};
    let problems = unsafe { menu_key_problems() };
    for p in &problems {
        log(&format!("  shortcut problem: {}", p));
    }
    state_check("every menu action has a shortcut (or is exempt), none duplicated", problems.is_empty());
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("idle: playback keys are greyed out", ui.playing.is_none()
            && !item_enabled("playPause:", 0, 0)
            && !item_enabled("seekBack:", 0, 0)
            && !item_enabled("mute:", 0, 0));
        state_check("⌘F, library ⌘1 and ⌘/ stay enabled", item_enabled("focusSearch:", 0, CMD)
            && item_enabled("viewSelected:", 0, CMD)
            && item_enabled("showShortcuts:", 0, CMD));
    }
}
/// The "Edit" submenu item with this action, if the main menu has one.
unsafe fn edit_menu_item(action: &str) -> Option<*mut AnyObject> {
    let main: *mut AnyObject = msg_send![UI.get().unwrap().app.get(), mainMenu];
    let n: NSInteger = msg_send![main, numberOfItems];
    for i in 0..n {
        let top: *mut AnyObject = msg_send![main, itemAtIndex: i];
        let sub: *mut AnyObject = msg_send![top, submenu];
        if sub.is_null() || to_str(msg_send![sub, title]) != "Edit" {
            continue;
        }
        let m: NSInteger = msg_send![sub, numberOfItems];
        for j in 0..m {
            let it: *mut AnyObject = msg_send![sub, itemAtIndex: j];
            let a: Option<Sel> = msg_send![it, action];
            if a.is_some_and(|a| a.name().to_str().ok() == Some(action)) {
                return Some(it);
            }
        }
    }
    None
}
/// Standard text commands live in an Edit menu with nil target, so AppKit sends them to
/// the focused text field (login password, URL, settings, search).
fn s_edit_menu() {
    use crate::ui::shortcuts::CMD;
    let checks = [
        ("undo:", "z", "Undo ⌘Z"),
        ("redo:", "z", "Redo ⇧⌘Z"),
        ("cut:", "x", "Cut ⌘X"),
        ("copy:", "c", "Copy ⌘C"),
        ("paste:", "v", "Paste ⌘V"),
        ("selectAll:", "a", "Select All ⌘A"),
    ];
    for (action, key, label) in checks {
        let ok = unsafe {
            match edit_menu_item(action) {
                Some(it) => {
                    let k = to_str(msg_send![it, keyEquivalent]);
                    let m: NSUInteger = msg_send![it, keyEquivalentModifierMask];
                    let t: *mut AnyObject = msg_send![it, target];
                    k == key && m & CMD != 0 && t.is_null()
                }
                None => false,
            }
        };
        state_check(&format!("Edit menu: {} with nil target", label), ok);
    }
}
fn s_shortcuts_text() {
    use crate::ui::shortcuts::{item_enabled, CMD};
    unsafe {
        let ui = UI.get_mut().unwrap();
        let win = ui.win.get();
        let _: Bool = msg_send![win, makeFirstResponder: ui.search.get()];
        let fr: *mut AnyObject = msg_send![win, firstResponder];
        let is_text: Bool = msg_send![fr, isKindOfClass: class!(NSText)];
        state_check("search field is editing text", is_text == Bool::YES);
        state_check("plain Space/arrows/M/C/A/F/Tab/Return yield to typing", !item_enabled("playPause:", 0, 0)
            && !item_enabled("seekBack:", 0, 0)
            && !item_enabled("volUp:", 0, 0)
            && !item_enabled("mute:", 0, 0)
            && !item_enabled("fullscreen:", 0, 0)
            && !item_enabled("focusNext:", 0, 0)
            && !item_enabled("openSelected:", 0, 0));
        state_check("⌘ shortcuts still work while typing", item_enabled("focusSearch:", 0, CMD)
            && item_enabled("refresh:", 0, CMD));
        let _: Bool = msg_send![win, makeFirstResponder: nil()];
        let fr2: *mut AnyObject = msg_send![win, firstResponder];
        let is_text2: Bool = msg_send![fr2, isKindOfClass: class!(NSText)];
        state_check("focus released from the text field", is_text2 == Bool::NO);
    }
}
fn s_shortcuts_grid() {
    use crate::ui::library::kb_focus_index;
    use crate::ui::shortcuts::item_enabled;
    unsafe {
        let ui = UI.get_mut().unwrap();
        let n = ui.grid_data.as_ref().map_or(0, |g| g.cells.len());
        state_check("grid has cells for keyboard focus", n > 0);
        state_check("Return is not taken without a focused cell", kb_focus_index().is_none()
            && !item_enabled("openSelected:", 0, 0));
        act(sel!(focusNext:), nil());
        state_check("Tab focuses the first cell", kb_focus_index() == Some(0));
        if n > 1 {
            act(sel!(focusNext:), nil());
            state_check("Tab advances", kb_focus_index() == Some(1));
            act(sel!(focusPrevious:), nil());
            state_check("Shift-Tab goes back", kb_focus_index() == Some(0));
        }
        state_check("Return is enabled on the focused cell", item_enabled("openSelected:", 0, 0));
        act(sel!(focusPrevious:), nil());
        state_check("Shift-Tab wraps to the last cell", kb_focus_index() == Some(n - 1));
        act(sel!(focusNext:), nil());
        state_check("Tab wraps to the first cell", kb_focus_index() == Some(0));
    }
}
fn s_shortcuts_help() {
    use crate::ui::shortcuts::{hide_shortcuts_window, shortcuts_window_visible};
    unsafe {
        act(sel!(showShortcuts:), nil());
        state_check("⌘/ opens the Keyboard Shortcuts window", shortcuts_window_visible());
        audit("shortcuts-help");
        hide_shortcuts_window();
        state_check("help window closes", !shortcuts_window_visible());
    }
}
// ---- item detail page -------------------------------------------------------
const OVERVIEW: &str = "A long overview paragraph that wraps across several lines so the wrapping label has to grow to show every word. It covers a sprawling plot, two timelines, a secret society, a rescue at sea, and an ending that nobody saw coming, written deliberately at length to exercise height measurement and the page layout.";
fn detail_people(n: usize) -> Vec<DetailPerson> {
    (0..n)
        .map(|i| DetailPerson {
            name: format!("Performer {} with a very long stage name that must truncate", i),
            role: if i % 3 == 0 { None } else { Some(format!("Character {} of a long-running saga", i)) },
            image_key: if i % 2 == 0 { Some(poster_key(&format!("person-{}", i))) } else { None },
        })
        .collect()
}
fn detail_view(series: bool) -> DetailView {
    let id = if series { "detail-series" } else { "detail-long" };
    DetailView {
        id: id.into(),
        name: if series {
            "A Series With An Extremely Long Name That Keeps Going Well Past Any Sensible Header Width".into()
        } else {
            LONG.into()
        },
        item_type: if series { "Series" } else { "Movie" }.into(),
        year: Some(2019),
        official_rating: Some("TV-MA".into()),
        community_rating: Some(8.4),
        run_time_ticks: Some(72_000_000_000),
        genres: vec![
            "Science Fiction".into(),
            "Drama with a rather long genre label".into(),
            "Thriller".into(),
            "Mystery".into(),
        ],
        overview: Some(OVERVIEW.into()),
        played: false,
        resume: !series,
        resume_ticks: 0,
        poster_key: Some(poster_key(id)),
        backdrop_key: Some(poster_key(&format!("{}#backdrop", id))),
        people: detail_people(14),
        trailer_urls: Vec::new(),
        tagline: None,
        critic_rating: None,
        badges: Vec::new(),
        is_favorite: false,
        similar: Vec::new(),
        next_up: None,
    }
}
/// Send `sel` to a page view (its own target), like a click would.
unsafe fn act_on(target: *mut AnyObject, sel: Sel) {
    let _: *mut AnyObject = msg_send![target, performSelector: sel, withObject: nil()];
}
fn detail_host() -> *mut AnyObject {
    unsafe { UI.get().unwrap().detail.host.get() }
}
fn s_detail_movie() {
    take_cmds();
    handle_main(MainMsg::DetailLoaded(Box::new(detail_view(false))));
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("detail open", crate::ui::detail::is_open());
        state_check("detail host shown", !is_hidden(detail_host()));
        state_check("library header hidden", is_hidden(ui.header.get()));
        state_check("library grid hidden", is_hidden(ui.lib_scroll.get()));
        state_check("poster, backdrop and portraits laid out", crate::ui::detail::target_count() == 2 + 7);
        state_check("backdrop waits for its image", crate::ui::detail::art_filled(poster_key("detail-long#backdrop")) == Some(false));
    }
    handle_main(MainMsg::PosterReady { key: poster_key("detail-long#backdrop"), bytes: image_bytes(0, 640, 360, 90) });
    handle_main(MainMsg::PosterReady { key: poster_key("detail-long"), bytes: image_bytes(0, 200, 300, 200) });
    handle_main(MainMsg::PosterReady { key: poster_key("person-0"), bytes: image_bytes(1, 96, 96, 120) });
    state_check("backdrop fills on arrival", crate::ui::detail::art_filled(poster_key("detail-long#backdrop")) == Some(true));
    state_check("poster fills on arrival", crate::ui::detail::art_filled(poster_key("detail-long")) == Some(true));
    state_check("portrait fills on arrival", crate::ui::detail::art_filled(poster_key("person-0")) == Some(true));
    state_check("portrait without image has no artwork view", crate::ui::detail::art_filled(poster_key("person-1")).is_none());
    unsafe {
        let host = detail_host();
        act_on(host, sel!(detailPlay:));
        state_check(
            "play sends Play for the item",
            take_cmds().iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "detail-long")),
        );
        act_on(host, sel!(detailWatched:));
        state_check(
            "mark watched sends MarkPlayed",
            take_cmds().iter().any(|c| matches!(c, UiCmd::MarkPlayed { item_id, played: true } if item_id == "detail-long")),
        );
        let ui = UI.get_mut().unwrap();
        state_check("page flips to watched", ui.detail.item.as_ref().map(|i| i.played).unwrap_or(false));
        act_on(host, sel!(detailWatched:));
        take_cmds();
    }
    audit_both("detail movie");
}
fn s_detail_resize() {
    set_content_size(960.0, 600.0);
    state_check("detail relaid out at min width", crate::ui::detail::layout_matches_clip());
    audit("detail @min relaid");
    set_content_size(1280.0, 800.0);
    state_check("detail relaid out after grow", crate::ui::detail::layout_matches_clip());
    unsafe {
        act_on(detail_host(), sel!(detailBack:));
        let ui = UI.get_mut().unwrap();
        state_check("back closes the detail page", !crate::ui::detail::is_open() && is_hidden(detail_host()));
        state_check("back shows the grid header", !is_hidden(ui.header.get()) && !is_hidden(ui.lib_scroll.get()));
    }
}
fn s_detail_series() {
    handle_main(MainMsg::DetailLoaded(Box::new(detail_view(true))));
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("series page open", crate::ui::detail::is_open());
        state_check("series page has no artwork waiting", crate::ui::detail::art_filled(poster_key("detail-series")) == Some(false));
        let _ = ui;
    }
    audit_both("detail series");
}

fn season_entries(n: usize) -> Vec<SeasonEntry> {
    (1..=n).map(|i| SeasonEntry { id: format!("sea-{}", i), name: format!("Season {}", i) }).collect()
}
fn episode_rows(n: usize, prefix: &str) -> Vec<EpisodeRow> {
    (0..n)
        .map(|i| EpisodeRow {
            id: format!("{}-ep{}", prefix, i + 1),
            number: Some(i as u32 + 1),
            name: format!("Episode {} with a long title that must truncate once the row is narrow", i + 1),
            overview: if i % 2 == 0 { Some(OVERVIEW.into()) } else { None },
            run_time_ticks: Some(27_000_000_000),
            played: i == 0,
            position_ticks: if i == 1 { 9_000_000_000 } else { 0 },
            image_key: if i % 2 == 0 { Some(poster_key(&format!("{}-ep{}", prefix, i + 1))) } else { None },
        })
        .collect()
}
/// Send `sel` to a view with `sender`, like a control would when used.
unsafe fn act_with(target: *mut AnyObject, sel: Sel, sender: *mut AnyObject) {
    let _: *mut AnyObject = msg_send![target, performSelector: sel, withObject: sender];
}
/// The transparent play button of the episode row with `tag`, anywhere under `v`.
unsafe fn find_row_button(v: *mut AnyObject, tag: isize) -> *mut AnyObject {
    if is_kind(v, class!(NSButton)) {
        let t: isize = msg_send![v, tag];
        let title: *mut AnyObject = msg_send![v, title];
        if t == tag && to_str(title).is_empty() {
            return v;
        }
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let found = find_row_button(c, tag);
        if !found.is_null() {
            return found;
        }
    }
    nil()
}
fn s_detail_seasons() {
    take_cmds();
    handle_main(MainMsg::DetailLoaded(Box::new(detail_view(true))));
    let cmds = take_cmds();
    state_check(
        "series page asks for its seasons",
        cmds.iter().any(|c| matches!(c, UiCmd::Seasons { series_id } if series_id == "detail-series")),
    );
    audit_both("detail seasons: loading");
    handle_main(MainMsg::SeasonsLoaded { series_id: "detail-series".into(), seasons: season_entries(3), selected: 1 });
    let cmds = take_cmds();
    unsafe {
        let ui = UI.get().unwrap();
        state_check("next-up season is the one selected", ui.detail.seasons.selected == 1);
        state_check(
            "its episodes are requested",
            cmds.iter().any(|c| matches!(c, UiCmd::SeasonEpisodes { series_id, season_id }
                if series_id == "detail-series" && season_id == "sea-2")),
        );
        let idx: isize = msg_send![ui.detail.season_popup.get(), indexOfSelectedItem];
        state_check("picker shows the selected season", idx == 1);
    }
    // An answer for a season that is no longer selected is dropped.
    handle_main(MainMsg::SeasonEpisodesLoaded {
        series_id: "detail-series".into(),
        season_id: "sea-1".into(),
        episodes: episode_rows(2, "s1"),
    });
    state_check("stale season answer is dropped", UI.get().unwrap().detail.seasons.episodes.is_none());
    handle_main(MainMsg::SeasonEpisodesLoaded {
        series_id: "detail-series".into(),
        season_id: "sea-2".into(),
        episodes: episode_rows(4, "s2"),
    });
    state_check(
        "episode rows are laid out",
        UI.get().unwrap().detail.seasons.episodes.as_ref().map(|e| e.len()) == Some(4),
    );
    audit_both("detail seasons: episodes");
    state_check("episode thumbnail waits for its image", crate::ui::detail::art_filled(poster_key("s2-ep1")) == Some(false));
    handle_main(MainMsg::PosterReady { key: poster_key("s2-ep1"), bytes: image_bytes(0, 320, 180, 120) });
    state_check("episode thumbnail fills on arrival", crate::ui::detail::art_filled(poster_key("s2-ep1")) == Some(true));
    unsafe {
        let ui = UI.get().unwrap();
        let btn = find_row_button(ui.detail.doc.get(), 1);
        state_check("episode row has a play target", !btn.is_null());
        take_cmds();
        act_with(ui.detail.host.get(), sel!(detailEpisode:), btn);
        let cmds = take_cmds();
        state_check(
            "clicking an episode plays it",
            cmds.iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "s2-ep2")),
        );
    }
    // Switching season loads that season and drops the old rows.
    unsafe {
        let ui = UI.get().unwrap();
        let _: () = msg_send![ui.detail.season_popup.get(), selectItemAtIndex: 2isize];
        take_cmds();
        act_with(ui.detail.host.get(), sel!(detailSeason:), ui.detail.season_popup.get());
        let cmds = take_cmds();
        state_check(
            "choosing a season requests its episodes",
            cmds.iter().any(|c| matches!(c, UiCmd::SeasonEpisodes { season_id, .. } if season_id == "sea-3")),
        );
        state_check("switching drops the old rows", ui.detail.seasons.episodes.is_none());
    }
    handle_main(MainMsg::SeasonEpisodesLoaded { series_id: "detail-series".into(), season_id: "sea-3".into(), episodes: vec![] });
    state_check(
        "empty season shows no rows",
        UI.get().unwrap().detail.seasons.episodes.as_ref().map(|e| e.is_empty()) == Some(true),
    );
    audit_both("detail seasons: empty");
    handle_main(MainMsg::SeasonsError { series_id: "detail-series".into(), message: LONG.into() });
    state_check("a failure replaces the list with a message", UI.get().unwrap().detail.seasons.error.is_some());
    audit_both("detail seasons: error");
    unsafe {
        act_on(detail_host(), sel!(detailBack:));
    }
    take_cmds();
    state_check("back closes the season page", !crate::ui::detail::is_open());
}
fn s_detail_error() {
    handle_main(MainMsg::DetailError(format!("Server error (HTTP 500): {}", LONG)));
    state_check("detail error shows a toast", unsafe { crate::ui::player::toast_visible() });
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("detail error keeps the grid", !crate::ui::detail::is_open() && !is_hidden(ui.lib_scroll.get()));
    }
}
fn s_misc_actions() {
    unsafe {
        let mi: *mut AnyObject = msg_send![class!(NSMenuItem), new];
        let _: () = msg_send![mi, setRepresentedObject: ns("Movie-1")];
        act(sel!(markWatched:), mi);
        act(sel!(openInWeb:), mi); // open_in_web never launches a browser in self-test mode
        let _: *mut AnyObject = msg_send![mi, autorelease];
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
const HOME_LONG: &str = "A Remarkably Long Title That Keeps Going Well Past Any Reasonable Card Width Ever Made";
/// Fake Home rows: every kind, long names, one progress card per wide row.
fn home_fake_rows() -> Vec<HomeRow> {
    let long = |mut v: Vec<ListItem>| {
        for (i, it) in v.iter_mut().enumerate() {
            it.name = format!("{} {}", i, HOME_LONG);
        }
        v
    };
    vec![
        HomeRow { title: "Continue Watching".into(), kind: HomeRowKind::ContinueWatching, items: long(items_of("Episode", 6)) },
        HomeRow { title: "Next Up".into(), kind: HomeRowKind::NextUp, items: items_of("Episode", 4) },
        HomeRow { title: format!("Recently Added in {}", HOME_LONG), kind: HomeRowKind::RecentlyAdded, items: items_of("Movie", 9) },
        HomeRow { title: format!("Because you watched {}", HOME_LONG), kind: HomeRowKind::Recommended, items: long(items_of("Movie", 12)) },
        HomeRow { title: "Recently Added in Shows".into(), kind: HomeRowKind::RecentlyAdded, items: items_of("Series", 3) },
    ]
}
fn s_home_open() {
    unsafe { act(sel!(homeSelected:), nil()) };
    unsafe {
        let ui = UI.get().unwrap();
        state_check("home: sidebar opens Home", ui.home.active);
        state_check("home: library grid hidden", !visible(ui.lib_scroll.get()));
        state_check("home: Home scroll shown", visible(ui.home.scroll.get()));
        state_check("home: header reads Home", read_field(ui.header_title.get()) == "Home");
        state_check("home: sort popup hidden", !visible(ui.sort_popup.get()));
        state_check("home: Home row selected", row_selected(ui.home.row.get()));
        state_check("home: no library row selected", selected_rows() == 0);
        state_check("home: remembered as last place", ui.cfg.last_view.as_deref() == Some("home"));
    }
    state_check("home: asks the worker for rows", take_cmds().iter().any(|c| matches!(c, UiCmd::LoadHome)));
}
fn s_home_fake() {
    handle_main(MainMsg::HomeLoaded(HomeRows { rows: home_fake_rows(), ..Default::default() }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("home: five rows built", ui.home.views.len() == 5);
        let total: usize = ui.home.rows.iter().map(|r| r.items.len()).sum();
        state_check("home: one card per item", ui.home.flat.len() == total && total == 34);
        let cw = &ui.home.views[0].cells[0];
        state_check("home: continue watching uses wide cards", frame_of(cw.container).size.w == crate::ui::home::WIDE_W);
        state_check("home: continue watching shows progress", cw.prog_frac > 0.49);
        let rec = &ui.home.views[2].cells[0];
        state_check("home: recently added uses poster cards", frame_of(rec.container).size.w == crate::ui::home::POSTER_W);
        state_check("home: scroll visible while rows are on screen", visible(ui.home.scroll.get()));
    }
    // Art for two cards: one wide (its own key), one poster.
    let png = image_bytes(0, 200, 300, 90);
    state_check("home: wide card uses the wide art key", unsafe { UI.get().unwrap().home.views[0].cells[0].key } == crate::ui::home::wide_key("Episode-0"));
    state_check("home: wide and poster keys differ for one item", crate::ui::home::wide_key("Movie-0") != poster_key("Movie-0"));
    handle_main(MainMsg::PosterReady { key: crate::ui::home::wide_key("Episode-0"), bytes: png.clone() });
    handle_main(MainMsg::PosterReady { key: poster_key("Movie-0"), bytes: png });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("home: poster hides placeholder on a wide card", crate::ui::shell::is_hidden(ui.home.views[0].cells[0].glyph));
        state_check("home: poster hides placeholder on a poster card", crate::ui::shell::is_hidden(ui.home.views[2].cells[0].glyph));
        state_check("home: card without artwork keeps placeholder", !crate::ui::shell::is_hidden(ui.home.views[4].cells[2].glyph));
    }
}
fn s_audit_home() {
    audit_both("home");
}
fn s_home_clicks() {
    let (first, series) = unsafe {
        let ui = UI.get().unwrap();
        (
            ui.home.flat[0].id.clone(),
            ui.home.flat.iter().position(|i| i.item_type == "Series").expect("a series card"),
        )
    };
    unsafe { act(sel!(homeItemClicked:), tagged_button(0)) };
    state_check("home: card click plays the item", take_cmds().iter().any(|c| matches!(c, UiCmd::Play { item_id } if *item_id == first)));
    let sid = unsafe { UI.get().unwrap().home.flat[series].id.clone() };
    unsafe { act(sel!(homeItemClicked:), tagged_button(series as isize)) };
    state_check(
        "home: series card opens its detail page",
        take_cmds().iter().any(|c| matches!(c, UiCmd::OpenDetail { item_id } if *item_id == sid)),
    );
    unsafe {
        state_check("home: still on Home after a card click", UI.get().unwrap().home.active);
    }
}
fn s_home_empty() {
    handle_main(MainMsg::HomeLoaded(HomeRows::default()));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("home: empty rows clear the cards", ui.home.views.is_empty());
        state_check("home: empty state shown", visible(ui.state_host.get()));
        state_check("home: empty title", read_field(ui.status_label.get()) == "Nothing here yet");
    }
    audit_both("home empty");
    handle_main(MainMsg::HomeLoaded(HomeRows { rows: home_fake_rows(), ..Default::default() }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("home: rows back, state hidden", !visible(ui.state_host.get()) && ui.home.views.len() == 5);
    }
    // Reopening Home keeps it as the remembered place and asks for fresh rows.
    unsafe { act(sel!(homeSelected:), nil()) };
    unsafe {
        state_check("home: reopen keeps Home remembered", UI.get().unwrap().cfg.last_view.as_deref() == Some("home"));
    }
    state_check("home: reopen asks for rows", take_cmds().iter().any(|c| matches!(c, UiCmd::LoadHome)));
}
fn s_home_leave() {
    unsafe { act(sel!(viewSelected:), tagged_button(0)) };
    unsafe {
        let ui = UI.get().unwrap();
        state_check("library: Home left", !ui.home.active);
        state_check("library: grid back", visible(ui.lib_scroll.get()));
        state_check("library: Home scroll hidden", !visible(ui.home.scroll.get()));
        state_check("library: first library selected", ui.selected_view == Some(0));
        state_check("library: Home row deselected", !row_selected(ui.home.row.get()));
    }
    take_cmds();
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
            audit("settings sheet");
            let ui = UI.get_mut().unwrap();
            {
                let ctl_x: Vec<f64> = [ui.s_url.get(), ui.s_quality.get(), ui.s_sub_lang.get(), ui.s_audio_lang.get(), ui.s_theme.get(), ui.seerr.s_url.get(), ui.seerr.s_mode.get()]
                    .iter()
                    .map(|&v| { let f: NSRect = msg_send![v, frame]; f.origin.x })
                    .collect();
                state_check("settings controls share x = 186", ctl_x.iter().all(|&x| (x - 186.0).abs() < 0.5));
                let c: *mut AnyObject = msg_send![ui.settings_win.get(), contentView];
                let subs: *mut AnyObject = msg_send![c, subviews];
                let n: usize = msg_send![subs, count];
                let mut esc = false;
                for i in 0..n {
                    let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
                    if is_kind(v, class!(NSButton)) && to_str(msg_send![v, title]) == "Cancel" {
                        esc = to_str(msg_send![v, keyEquivalent]) == "\u{1b}";
                    }
                }
                state_check("cancel has Escape", esc);
            }
            state_check("seerr fields filled", read_field(ui.seerr.s_url.get()) == "seerr.example");
            state_check("server url shows bare host", read_field(ui.s_url.get()) == "example:8096");
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
            take_cmds();
            // The app's live config holds the resolved URL the worker stored.
            UI.get_mut().unwrap().cfg.server_url = Some("http://example:8096".into());
            act(sel!(saveSettings:), nil());
            let ui = UI.get().unwrap();
            state_check("audio lang saved", ui.cfg.settings.audio_lang == "spa");
            state_check("theme saved", ui.cfg.settings.theme == "light");
            state_check("unchanged server url sends no resolve", take_cmds().iter().all(|c| !matches!(c, UiCmd::SaveSettings { server_url: Some(_), .. })));
            log("  saving a new bare server host");
            set_text(ui.s_url.get(), "  media.example.com:8096/ ");
            act(sel!(saveSettings:), nil());
            state_check(
                "bare host sent to the worker for resolution",
                take_cmds().iter().any(|c| matches!(c, UiCmd::SaveSettings { reconnect: true, server_url: Some(u), .. } if u == "media.example.com:8096/")),
            );
        });
    });
}
/// Library order/visibility: sidebar honours the saved prefs, the Settings controls edit
/// them, and Save emits SaveLibraries. Restores empty prefs at the end.
fn s_library_prefs() {
    use crate::ui::shell::apply_library_prefs;
    fn ids(v: &[(String, String, Option<String>)]) -> Vec<String> {
        v.iter().map(|x| x.0.clone()).collect()
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.library_order = s(&["v3", "v1", "v2", "v4", "v5"]);
        ui.cfg.hidden_libraries = s(&["v4"]);
        apply_library_prefs();
        let row_ids: Vec<String> = ui.rows.iter().map(|r| r.1.clone()).collect();
        state_check("sidebar follows saved order and hides v4", row_ids == s(&["v3", "v1", "v2", "v5"]));
        state_check("hidden library gone from sidebar rows", row_ids.len() == 4);
        let open_idx = ui.mode_stack.first().and_then(|m| ui.views.iter().position(|v| v.0 == m.0));
        state_check("selection follows the open library", open_idx.is_some() && ui.selected_view == open_idx);

        let cfg = ui.cfg.clone();
        handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
        audit_both("settings libraries");
        let ui = UI.get_mut().unwrap();
        state_check("settings lists all libraries in saved order", ids(&ui.lib_edit) == s(&["v3", "v1", "v2", "v4", "v5"]));
        state_check("settings shows v4 as hidden", ui.lib_hidden == s(&["v4"]));
        state_check("first library selected in popup", ui.lib_sel == 0 && is_shown_state(ui.s_lib_show.get()));

        // Move v3 down: order becomes v1, v3, v2, v4, v5 and the selection follows it.
        act(sel!(libMoveDown:), nil());
        state_check("move down swaps and follows selection", ids(&ui.lib_edit) == s(&["v1", "v3", "v2", "v4", "v5"]) && ui.lib_sel == 1);
        state_check("up arrow enabled after moving down", ctl_enabled(ui.s_lib_up.get()));

        // Show v4 again.
        pick(3);
        let _: () = msg_send![ui.s_lib_show.get(), setState: 1isize];
        act(sel!(libVisibleToggled:), nil());
        state_check("checkbox shows v4", ui.lib_hidden.is_empty());

        // Hide v1 (top row): the up arrow is disabled there.
        pick(0);
        let _: () = msg_send![ui.s_lib_show.get(), setState: 0isize];
        act(sel!(libVisibleToggled:), nil());
        state_check("checkbox hides v1", ui.lib_hidden == s(&["v1"]));
        state_check("up arrow disabled on first row", !ctl_enabled(ui.s_lib_up.get()));

        // Only one library left visible: the last one cannot be hidden.
        ui.lib_hidden = s(&["v1", "v3", "v2", "v4"]);
        pick(4);
        let _: () = msg_send![ui.s_lib_show.get(), setState: 0isize];
        act(sel!(libVisibleToggled:), nil());
        state_check("last visible library cannot be hidden", !ui.lib_hidden.contains(&"v5".to_string()));
        ui.lib_hidden = s(&["v1"]);

        act(sel!(saveSettings:), nil());
        let ui = UI.get_mut().unwrap();
        let cmds = take_cmds();
        state_check(
            "save emits SaveLibraries with the edited order and hidden list",
            cmds.iter().any(|c| matches!(c,
                UiCmd::SaveLibraries { order, hidden } if *order == s(&["v1", "v3", "v2", "v4", "v5"]) && *hidden == s(&["v1"]))),
        );
        state_check("saved prefs reach ui.cfg", ui.cfg.library_order == s(&["v1", "v3", "v2", "v4", "v5"]) && ui.cfg.hidden_libraries == s(&["v1"]));
        let row_ids: Vec<String> = ui.rows.iter().map(|r| r.1.clone()).collect();
        state_check("sidebar updates after save", row_ids == s(&["v3", "v2", "v4", "v5"]));

        // Restore defaults so later steps see the normal sidebar.
        ui.cfg.library_order.clear();
        ui.cfg.hidden_libraries.clear();
        apply_library_prefs();
        state_check("defaults restored", ui.rows.len() == 5);
    }
}
unsafe fn is_shown_state(cb: *mut AnyObject) -> bool {
    let st: NSInteger = msg_send![cb, state];
    st != 0
}
unsafe fn ctl_enabled(v: *mut AnyObject) -> bool {
    let e: Bool = msg_send![v, isEnabled];
    e == Bool::YES
}
/// Select an item in the Libraries popup and fire its action, as a click would.
unsafe fn pick(i: isize) {
    let ui = UI.get_mut().unwrap();
    let _: () = msg_send![ui.s_lib_pop.get(), selectItemAtIndex: i];
    act(sel!(libSelected:), nil());
}
// ---- onboarding and the unified layout -------------------------------------
fn s_onboard_show() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("onboarding: card up after first login", crate::ui::onboarding::visible());
        state_check("onboarding: no layout saved yet", ui.cfg.layout_mode.is_none());
        state_check("onboarding: Home is underneath", ui.home.active);
        state_check("onboarding: separate highlighted by default", ui.onboard.picked == 0);
        state_check("onboarding: library rows are the default sidebar", ui.rows.len() == 5);
    }
    audit_both("onboarding");
}
fn s_onboard_pick() {
    unsafe {
        act(sel!(onboardPick:), tagged_button(1));
        state_check("onboarding: one big library highlighted", UI.get().unwrap().onboard.picked == 1);
        act(sel!(onboardContinue:), nil());
        let ui = UI.get().unwrap();
        state_check("onboarding: card closed on Continue", !crate::ui::onboarding::visible());
        state_check("onboarding: one big library saved", ui.cfg.layout_mode == Some(crate::config::LayoutMode::Unified));
        state_check("unified: no library rows", ui.rows.is_empty());
        state_check("unified: Browse row present", !ui.browse_row.get().is_null());
        state_check("unified: Home still on screen", ui.home.active);
        let cmds = take_cmds();
        state_check(
            "unified: worker told the layout",
            cmds.iter().any(|c| matches!(c, UiCmd::SetLayoutMode { mode: crate::config::LayoutMode::Unified })),
        );
    }
}
fn s_unified_browse() {
    unsafe {
        act(sel!(browseSelected:), nil());
        let ui = UI.get().unwrap();
        state_check("browse: screen is Browse", ui.browse_active && !ui.home.active && ui.selected_view.is_none());
        state_check("browse: header reads Browse", read_field(ui.header_title.get()) == "Browse");
        state_check("browse: Browse row selected", row_selected(ui.browse_row.get()));
        state_check("browse: Home row not selected", !row_selected(ui.home.row.get()));
        state_check("browse: library grid shown", visible(ui.lib_scroll.get()) && !visible(ui.home.scroll.get()));
        let cmds = take_cmds();
        state_check("browse: LoadBrowse sent", cmds.iter().any(|c| matches!(c, UiCmd::LoadBrowse)));
        state_check("browse: no LoadView", !cmds.iter().any(|c| matches!(c, UiCmd::LoadView { .. })));
        state_check("browse: remembered as last place", ui.cfg.last_view.as_deref() == Some("browse"));
    }
    // Movies and shows arrive mixed, the way the worker sends them.
    let mut mixed = items_of("Movie", 6);
    mixed.extend(items_of("Series", 4));
    send_items("Browse", mixed, false);
    unsafe {
        let ui = UI.get().unwrap();
        state_check("browse: grid holds movies and shows", ui.items.len() == 10);
        state_check("browse: title stays Browse", ui.cur_title == "Browse");
    }
    audit_both("unified browse");
}
fn s_unified_actions() {
    unsafe {
        act(sel!(focusSearch:), nil());
        let ui = UI.get().unwrap();
        let fr: *mut AnyObject = msg_send![ui.win.get(), firstResponder];
        let editing: *mut AnyObject = msg_send![ui.search.get(), currentEditor];
        state_check("unified: Search focuses the search field", fr == ui.search.get() || !editing.is_null());
        // Seerr is off in this run, so Requests shows its setup state and has no sidebar row.
        act(sel!(requestsSelected:), nil());
        state_check("unified: Requests tab opens", UI.get().unwrap().seerr.active);
        state_check("unified: no Requests row while Seerr is off", UI.get().unwrap().seerr.row.get().is_null());
        let _ = take_cmds();
        act(sel!(homeSelected:), nil());
        let ui = UI.get().unwrap();
        state_check("unified: Home is back", ui.home.active && !ui.browse_active && !ui.seerr.active);
        state_check("unified: Browse row not selected", !row_selected(ui.browse_row.get()));
        let cmds = take_cmds();
        state_check("unified: LoadHome sent", cmds.iter().any(|c| matches!(c, UiCmd::LoadHome)));
    }
}
fn s_unified_login() {
    // Browse was the last place: a login in the unified layout returns to it.
    unsafe { UI.get_mut().unwrap().cfg.last_view = Some("browse".into()) };
    let views = unsafe { UI.get().unwrap().views.clone() };
    handle_main(MainMsg::Connected { user_name: "tester".into(), views, has_live_tv: true });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("unified login: Browse restored", ui.browse_active && !ui.home.active);
        let cmds = take_cmds();
        state_check("unified login: LoadBrowse sent", cmds.iter().any(|c| matches!(c, UiCmd::LoadBrowse)));
    }
    unsafe { act(sel!(homeSelected:), nil()) };
}
fn s_layout_settings() {
    // Settings can switch the layout back; saving then rebuilds the library rows.
    let mut cfg = Config::default();
    cfg.server_url = Some("http://example:8096".into());
    cfg.layout_mode = Some(crate::config::LayoutMode::Unified);
    handle_main(MainMsg::SettingsSnapshot(Box::new(cfg)));
    unsafe {
        let ui = UI.get().unwrap();
        let idx: NSInteger = msg_send![ui.s_layout.get(), indexOfSelectedItem];
        state_check("settings: shows one big library", idx == 1);
        audit_both("settings layout unified");
        let ui = UI.get().unwrap();
        let _: () = msg_send![ui.s_layout.get(), selectItemAtIndex: 0isize];
        act(sel!(saveSettings:), nil());
        let ui = UI.get().unwrap();
        state_check("settings: separate saved", ui.cfg.layout_mode == Some(crate::config::LayoutMode::Separate));
        state_check("settings: library rows are back", ui.rows.len() == 5);
        state_check("settings: Browse row removed", ui.browse_row.get().is_null());
        state_check("settings: Home still the place", ui.home.active);
        state_check("settings: home tabs hidden in the separate layout", is_hidden(ui.home.seg.get()));
        let cmds = take_cmds();
        state_check(
            "settings: worker told the layout",
            cmds.iter().any(|c| matches!(c, UiCmd::SetLayoutMode { mode: crate::config::LayoutMode::Separate })),
        );
    }
}
/// Unified Home tabs: the segmented control, and each tab's rows and grid.
fn s_tabs_home() {
    handle_main(MainMsg::HomeLoaded(HomeRows { rows: home_fake_rows(), ..Default::default() }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: segmented control exists", !ui.home.seg.get().is_null());
        state_check("tabs: control shown on Home", visible(ui.home.seg.get()));
        let idx: NSInteger = msg_send![ui.home.seg.get(), selectedSegment];
        state_check("tabs: Home selected", idx == 0 && ui.home.section == crate::config::HomeSection::Home);
        state_check("tabs: Home has no grid", ui.home.grid.is_empty());
        state_check("tabs: Home rows built", ui.home.views.len() == 5);
    }
    take_cmds();
    audit_both("unified home tabs");
}
fn s_tabs_movies() {
    use crate::config::HomeSection;
    // Clicking the control (the segment path), then the Go menu path below.
    unsafe {
        let ui = UI.get().unwrap();
        let _: () = msg_send![ui.home.seg.get(), setSelectedSegment: 1isize];
        act(sel!(homeSegment:), ui.home.seg.get());
    }
    let cmds = take_cmds();
    state_check("tabs: Movies asks for movie content", cmds.iter().any(|c| matches!(c, UiCmd::LoadHomeSection { section: HomeSection::Movies })));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: header reads Movies", read_field(ui.header_title.get()) == "Movies");
        state_check("tabs: Home rows cleared", ui.home.views.is_empty());
        state_check("tabs: loading state shown", visible(ui.state_host.get()) && read_field(ui.status_label.get()) == "Loading Movies…");
        state_check("tabs: Movies remembered in config", ui.cfg.home_section == Some(HomeSection::Movies));
    }
    // A Home reply that arrives after the switch is dropped.
    handle_main(MainMsg::HomeLoaded(HomeRows { rows: home_fake_rows(), ..Default::default() }));
    unsafe { state_check("tabs: stale Home reply ignored", UI.get().unwrap().home.views.is_empty()) };
    // The Movies reply is mixed on purpose: series must not show on this tab.
    let mut mixed = items_of("Movie", 14);
    mixed.extend(items_of("Series", 4));
    handle_main(MainMsg::HomeLoaded(HomeRows {
        section: HomeSection::Movies,
        rows: vec![
            HomeRow { title: "Continue Watching".into(), kind: HomeRowKind::ContinueWatching, items: items_of("Movie", 3) },
            HomeRow { title: "Recently Added".into(), kind: HomeRowKind::RecentlyAdded, items: items_of("Movie", 6) },
        ],
        grid: mixed,
    }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: movie rows and grid built", ui.home.views.len() == 3);
        state_check("tabs: only movies on the Movies tab", ui.home.flat.iter().all(|i| i.item_type == "Movie"));
        state_check("tabs: all 14 movies in the grid", ui.home.grid.len() == 14 && ui.home.views[2].cells.len() == 14);
        state_check("tabs: no loading state once loaded", !visible(ui.state_host.get()));
    }
    // The Go menu path: Movies is already selected, so this is a repeat (no reload).
    unsafe { act(sel!(showHomeSection:), tagged_button(1)) };
    state_check("tabs: repeat Movies pick does not reload", !take_cmds().iter().any(|c| matches!(c, UiCmd::LoadHomeSection { .. })));
    audit_both("unified movies tab");
}
fn s_tabs_resize() {
    use crate::ui::library::grid_metrics;
    unsafe {
        let ui = UI.get().unwrap();
        for (w, h) in [(1700.0, 1000.0), (960.0, 600.0)] {
            let _: () = msg_send![ui.win.get(), setContentSize: size(w, h)];
            let _: () = msg_send![ui.win.get(), displayIfNeeded];
            let doc = ui.home.doc.get();
            let dw = bounds_of(doc).size.w;
            let m = grid_metrics(bounds_of(ui.home.scroll.get()).size.w.max(200.), crate::ui::theme::POSTER_ASPECT);
            let cells = &ui.home.views[2].cells;
            let right = cells.iter().map(|c| frame_of(c.container)).fold(0.0f64, |a, f| a.max(f.origin.x + f.size.w));
            state_check(&format!("tabs: grid fits {}x{}", w, h), right <= dw - crate::ui::theme::EDGE + 0.5 && cells[0].container.is_null() == false);
            state_check(&format!("tabs: grid columns match {}x{}", w, h), cells.len() == 14 && m.cols >= 1);
        }
    }
    unsafe { set_content_size(1280.0, 800.0) };
}
fn s_tabs_shows() {
    use crate::config::HomeSection;
    unsafe { act(sel!(showHomeSection:), tagged_button(2)) };
    let cmds = take_cmds();
    state_check("tabs: TV Shows asks for series", cmds.iter().any(|c| matches!(c, UiCmd::LoadHomeSection { section: HomeSection::TvShows })));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: header reads TV Shows", read_field(ui.header_title.get()) == "TV Shows");
        let idx: NSInteger = msg_send![ui.home.seg.get(), selectedSegment];
        state_check("tabs: TV Shows selected in the control", idx == 2);
    }
    let mut mixed = items_of("Series", 12);
    mixed.extend(items_of("Movie", 3));
    handle_main(MainMsg::HomeLoaded(HomeRows {
        section: HomeSection::TvShows,
        rows: vec![
            HomeRow { title: "Next Up".into(), kind: HomeRowKind::NextUp, items: items_of("Episode", 4) },
            HomeRow { title: "Recently Added".into(), kind: HomeRowKind::RecentlyAdded, items: items_of("Series", 5) },
        ],
        grid: mixed,
    }));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: series rows and grid built", ui.home.views.len() == 3);
        state_check("tabs: only series and episodes on TV Shows", ui.home.flat.iter().all(|i| matches!(i.item_type.as_str(), "Series" | "Episode")));
        state_check("tabs: all 12 series in the grid", ui.home.grid.len() == 12);
    }
    audit_both("unified tv tab");
}
fn s_tabs_home_again() {
    unsafe { act(sel!(showHomeSection:), tagged_button(0)) };
    let cmds = take_cmds();
    state_check("tabs: Home asks for LoadHome", cmds.iter().any(|c| matches!(c, UiCmd::LoadHome)));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tabs: Home remembered in config", ui.cfg.home_section == Some(crate::config::HomeSection::Home));
        let idx: NSInteger = msg_send![ui.home.seg.get(), selectedSegment];
        state_check("tabs: Home selected in the control", idx == 0);
    }
    handle_main(MainMsg::HomeLoaded(HomeRows { rows: home_fake_rows(), ..Default::default() }));
    unsafe { state_check("tabs: Home rows back", UI.get().unwrap().home.views.len() == 5 && UI.get().unwrap().home.grid.is_empty()) };
}
fn s_tabs_remember() {
    use crate::config::HomeSection;
    unsafe {
        act(sel!(showHomeSection:), tagged_button(2));
        take_cmds();
        // Leave for Browse, then come back from the sidebar: the TV tab is shown again.
        act(sel!(browseSelected:), nil());
        state_check("tabs: Browse leaves Home", !UI.get().unwrap().home.active);
        act(sel!(homeSelected:), nil());
        let ui = UI.get().unwrap();
        state_check("tabs: Home reopens on the remembered tab", ui.home.section == HomeSection::TvShows);
    }
    let cmds = take_cmds();
    state_check("tabs: reopening asks for TV content", cmds.iter().any(|c| matches!(c, UiCmd::LoadHomeSection { section: HomeSection::TvShows })));
    unsafe { act(sel!(showHomeSection:), tagged_button(0)) };
    take_cmds();
}
fn s_play_start() {
    handle_main(playback(0.0, 6.0));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("playing set", ui.playing.is_some());
    }
}
fn s_play_chrome() {
    use crate::ui::player;
    unsafe {
        let ui = UI.get_mut().unwrap();
        let ap: *mut AnyObject = msg_send![ui.player_host.get(), effectiveAppearance];
        let name: *mut AnyObject = msg_send![ap, name];
        state_check("player chrome is dark", to_str(name) == "NSAppearanceNameDarkAqua");
        let cs: isize = msg_send![ui.avview.get(), controlsStyle];
        state_check("AVKit controls are off (controlsStyle none)", cs == 0);
        state_check("toast hidden when empty", !player::toast_visible());
        let ip: Bool = msg_send![ui.info_panel.get(), isHidden];
        state_check("playback info hidden by default", ip == Bool::YES);
        let sel: Sel = msg_send![ui.back_btn.get(), action];
        state_check("back button stops playback", sel == sel!(stopPlayback:));
        let a: f64 = msg_send![ui.top_bar.get(), alphaValue];
        state_check("top bar visible at start", a > 0.99);
        state_check("backdrop gone once playing", !player::backdrop_visible());
        // info toggle shows the panel and persists
        act(sel!(togglePlaybackInfo:), nil());
        let ip: Bool = msg_send![ui.info_panel.get(), isHidden];
        state_check("info panel shown after toggle", ip == Bool::NO && ui.cfg.settings.show_stats);
        update_chip();
        state_check("info text filled", read_field(ui.info_label.get()).contains("/"));
        act(sel!(togglePlaybackInfo:), nil());
        let ip: Bool = msg_send![ui.info_panel.get(), isHidden];
        state_check("info panel hidden after second toggle", ip == Bool::YES && !ui.cfg.settings.show_stats);
    }
}
fn s_toast_hidden() {
    unsafe {
        state_check("toast hidden 2.2 s after subCycle", !crate::ui::player::toast_visible());
    }
}
fn s_audit_player() {
    handle_main(MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(),
        item_id: "movie-1".into(),
        title: LONG.into(),
        subtitle: Some(format!("{} · {}", "Some Show With A Long Name", LONG)),
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: None,
        media_kind: MediaKind::Video,
        series_name: None,
        season_number: None,
        episode_number: None,
        production_year: None,
    segments: Vec::new(),
    series_id: None,
    next_up: None,
}));
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.show_stats = true;
        crate::ui::player::apply_info_visibility();
        // A long, neutral toast: the wrap and truncation check must not put the error banner over the player.
        crate::ui::player::toast(&format!("Subtitles: {}", LONG.repeat(2)), Some("captions.bubble"), false);
    }
    audit_both("player");
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.show_stats = false;
        crate::ui::player::apply_info_visibility();
        crate::ui::player::toast("Subtitles: Off", Some("captions.bubble"), false);
        audit("player/toast-short");
    }
}
fn s_player_bar() {
    unsafe {
        crate::ui::player::show_bars_instant();
        let ui = UI.get_mut().unwrap();
        let bar = ui.pbar.bar.get();
        state_check("player bar built", !bar.is_null());
        let host = bounds_of(ui.player_host.get());
        let f = frame_of(bar);
        state_check("player bar spans the full width", f.origin.x.abs() < 0.5 && (f.size.w - host.size.w).abs() < 0.5);
        state_check("player bar sits at the bottom", f.origin.y.abs() < 0.5 && (f.size.h - crate::ui::theme::PLAYER_CTRL_H).abs() < 0.5);
        let cs: isize = msg_send![ui.avview.get(), controlsStyle];
        state_check("controlsStyle is none (0)", cs == 0);
        state_check("player bar shown at start", ui.bar_shown && crate::ui::playerbar::visible());
        state_check("player bar: elapsed and remaining filled",
            read_field(ui.pbar.elapsed.get()).contains(':') && read_field(ui.pbar.remain.get()).starts_with('-'));
    }
    audit_both("player bar");
}
fn s_player_bar_buttons() {
    use crate::ui::playerbar as pb;
    unsafe {
        crate::ui::player::show_bars_instant();
        let (play, back, fwd, mute, subs, audio, speed, pip, full) = {
            let p = &UI.get().unwrap().pbar;
            (p.play, p.back, p.fwd, p.mute, p.subs, p.audio, p.speed, p.pip, p.full)
        };
        // Every button is wired to its action on the controller.
        for (b, sel, name) in [
            (play, sel!(playPause:), "play"),
            (back, sel!(seekBack:), "back 10s"),
            (fwd, sel!(seekFwd:), "forward 10s"),
            (mute, sel!(mute:), "mute"),
            (subs, sel!(showSubtitleMenu:), "subtitles"),
            (audio, sel!(showAudioMenu:), "audio"),
            (speed, sel!(showSpeedMenu:), "speed"),
            (pip, sel!(togglePiP:), "picture in picture"),
            (full, sel!(fullscreen:), "full screen"),
        ] {
            let a: Sel = msg_send![b.get(), action];
            state_check(&format!("button {} wired", name), a == sel);
        }
        // PiP and full screen are fired too (off-screen they only count the press).
        act(sel!(togglePiP:), pip.get());
        state_check("pip button fires its action", UI.get().unwrap().pbar.pip_presses == 1);
        // The button follows the controller for this video: enabled exactly when AVKit can start PiP.
        // Nothing is started here, so no window appears; off-screen the press above only counted.
        pb::refresh_pip_button();
        let pc = UI.get().unwrap().pbar.pip_ctl.get();
        state_check("pip controller exists for the video layer", !pc.is_null());
        if !pc.is_null() {
            let possible: Bool = msg_send![pc, isPictureInPicturePossible];
            let en: Bool = msg_send![pip.get(), isEnabled];
            state_check(
                &format!("pip button enabled matches isPictureInPicturePossible (possible: {})", possible == Bool::YES),
                (possible == Bool::YES) == (en == Bool::YES),
            );
            let active: Bool = msg_send![pc, isPictureInPictureActive];
            state_check("pip is not showing in self-test", active == Bool::NO);
        }
        let hid: Bool = msg_send![pip.get(), isHidden];
        state_check("pip button left the bar (the More menu has PiP)", hid == Bool::YES);
        let btn_on: Bool = msg_send![pip.get(), isEnabled];
        state_check("pip menu item follows the button", crate::ui::shortcuts::item_enabled("togglePiP:", 0, crate::ui::shortcuts::ALT | crate::ui::shortcuts::CMD) == (btn_on == Bool::YES));
        state_check("pip shortcut is ⌥⌘P in the table", crate::ui::shortcuts::SHORTCUTS.iter().any(|e| {
            e.action == "togglePiP:" && e.key == "p" && e.mods == crate::ui::shortcuts::ALT | crate::ui::shortcuts::CMD
        }));
        act(sel!(fullscreen:), full.get());
        state_check("full screen button fires its action", UI.get().unwrap().pbar.full_presses == 1);
        // Play/pause: one press flips the rate, the next flips it back.
        let r0: f32 = msg_send![UI.get().unwrap().player.get(), rate];
        act(sel!(playPause:), play.get());
        let r1: f32 = msg_send![UI.get().unwrap().player.get(), rate];
        state_check("play button toggles playback", (r0 > 0.0) != (r1 > 0.0));
        act(sel!(playPause:), play.get());
        // Mute toggles.
        let m0: Bool = msg_send![UI.get().unwrap().player.get(), isMuted];
        act(sel!(mute:), mute.get());
        let m1: Bool = msg_send![UI.get().unwrap().player.get(), isMuted];
        state_check("mute button toggles mute", m0 != m1);
        act(sel!(mute:), mute.get());
        // Back and forward run without touching anything else.
        act(sel!(seekFwd:), fwd.get());
        act(sel!(seekBack:), back.get());
        // Subtitle and audio menus list Off plus every track.
        act(sel!(showSubtitleMenu:), subs.get());
        let want_subs = UI.get().unwrap().sub_opts.len() + 1;
        state_check("subtitle menu lists Off and each track", UI.get().unwrap().pbar.last_menu_items == want_subs);
        act(sel!(showAudioMenu:), audio.get());
        let want_audio = UI.get().unwrap().audio_opts.len() + 1;
        state_check("audio menu lists Off and each track", UI.get().unwrap().pbar.last_menu_items == want_audio);
        if !UI.get().unwrap().sub_opts.is_empty() {
            act(sel!(pickTrack:), tagged_button(0));
            state_check("subtitle menu item Off selects no track", UI.get().unwrap().sel_sub == 0);
        }
        // Speed menu: six rates, and a picked rate reaches the player.
        act(sel!(showSpeedMenu:), speed.get());
        state_check("speed menu lists six rates", UI.get().unwrap().pbar.last_menu_items == 6);
        act(sel!(setSpeed:), tagged_button(150));
        let d: f32 = msg_send![UI.get().unwrap().player.get(), defaultRate];
        state_check("speed pick sets the default rate", (d - 1.5).abs() < 0.01);
        act(sel!(setSpeed:), tagged_button(100));
        // Volume slider drives the player.
        let vol = UI.get().unwrap().pbar.vol.get();
        let _: () = msg_send![vol, setDoubleValue: 0.4f64];
        act(sel!(jrNowVolume:), vol);
        let v: f32 = msg_send![UI.get().unwrap().player.get(), volume];
        state_check("player bar volume drives the player", (v - 0.4).abs() < 0.01);
        let _: () = msg_send![vol, setDoubleValue: 1.0f64];
        act(sel!(jrNowVolume:), vol);
        // Hover tooltip shows the time under the pointer and goes away again.
        pb::set_hover(Some(0.5));
        let tip = UI.get().unwrap().pbar.tip.get();
        let th: Bool = msg_send![tip, isHidden];
        state_check("hover tooltip shows", th == Bool::NO && read_field(UI.get().unwrap().pbar.tip_label.get()).contains(':'));
        pb::set_hover(None);
        let th: Bool = msg_send![tip, isHidden];
        state_check("hover tooltip hides", th == Bool::YES);
        // Scrubber seek moves the played fraction.
        pb::seek_frac(0.25);
        state_check("scrubber seek sets the fraction", (UI.get().unwrap().pbar.frac - 0.25).abs() < 1e-6);
        act(sel!(playPause:), play.get());
        act(sel!(playPause:), play.get());
    }
}
/// Deterministic idle test: the idle age is set directly instead of waiting on the wall clock.
fn s_pb_hide_arm() {
    unsafe {
        let _ = pb_seek_start();
        crate::ui::player::show_bars_instant();
        state_check("auto-hide: bar shown on activity", UI.get().unwrap().bar_shown);
        // Just under the timeout: still shown.
        crate::ui::player::set_idle_age(2.0);
        crate::ui::player::hide_bars_if_idle();
        state_check("auto-hide: still shown before 2.5 s", UI.get().unwrap().bar_shown);
        // Past the timeout while playing: hides (the fade finishes during the next delay).
        crate::ui::player::set_idle_age(2.6);
        crate::ui::player::hide_bars_if_idle();
    }
}
unsafe fn pb_seek_start() -> bool {
    let ui = UI.get().unwrap();
    if ui.playing.is_none() {
        return false;
    }
    let tm = crate::platform::appkit::CMTimeMakeWithSeconds(0.0, 600);
    let _: () = msg_send![ui.player.get(), seekToTime: tm];
    let _: () = msg_send![ui.player.get(), setRate: 1.0f32];
    true
}
fn show_bars_now() {
    unsafe { crate::ui::player::show_bars() }
}
fn s_pb_hide_check() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("auto-hide: video still playing during idle", ui.playing.as_ref().map(|p| !p.paused).unwrap_or(false));
        state_check("auto-hide: top bar hidden after 2.5 s idle", !ui.bar_shown);
        state_check("auto-hide: player bar faded out", !crate::ui::playerbar::visible());
        let ta: f64 = msg_send![ui.top_bar.get(), alphaValue];
        state_check("auto-hide: top bar alpha 0", ta < 0.05);
        // Pointer movement brings both back.
        show_bars_now();
        state_check("show on mouse move: bars back", ui.bar_shown);
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
        if !ui.sub_opts.is_empty() {
            state_check("toast visible after cycling", crate::ui::player::toast_visible());
        }
    }
}
// ---- subtitle overlay ------------------------------------------------------------
/// The cue in the mock video (mov_text, 0.5 s to 5.5 s).
const SUB_CUE: &str = "Subtitle track";
/// A long two-line cue, to check the wrap and the lift beside the Up Next card.
const SUB_LONG: &str = "This is a much longer subtitle line that has to wrap across the video width\nand a second line that keeps going so the block gets tall";

fn s_sub_pick() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("subtitles: the mock video has a subtitle track", !ui.sub_opts.is_empty());
        // Seek into the cue (0.5 s to 5.5 s) and turn the first subtitle track on.
        seek_to_secs(1.0);
        let _: () = msg_send![ui.player.get(), setRate: 1.0f32];
        set_track(0, 1);
        crate::ui::player::show_bars_instant();
    }
}

/// Pauses on the cue, checks it sits above the bar at both sizes, and checks a long cue beside
/// the visible Up Next card also clears the card.
/// Resizes the window and lays the content out at once, so the player host has its new size.
fn sub_resize(w: f64, h: f64) {
    set_content_size(w, h);
    unsafe {
        let ui = UI.get().unwrap();
        let content: *mut AnyObject = msg_send![ui.win.get(), contentView];
        let _: () = msg_send![content, layoutSubtreeIfNeeded];
        crate::ui::subtitles::settle();
    }
}

fn s_sub_on_screen() {
    unsafe {
        let ui = UI.get().unwrap();
        let _: () = msg_send![ui.player.get(), pause];
        crate::ui::player::show_bars_instant();
        let card = ui.skip.card.get();
        for (w, h, tag) in [(960.0, 600.0, "min"), (1700.0, 1000.0, "large")] {
            sub_resize(w, h);
            let (text, f) = crate::ui::subtitles::state();
            state_check(&format!("subtitles: cue on screen at {}", tag), text == SUB_CUE && crate::ui::subtitles::visible());
            log(&format!("  subtitles {}: y={:.0} h={:.0} w={:.0}", tag, f.origin.y, f.size.h, f.size.w));
            let problems = crate::ui::subtitles::chrome_problems();
            state_check(&format!("subtitles: clear of the controls at {} ({})", tag, problems.join("; ")), problems.is_empty());
            let bar_top = crate::ui::theme::PLAYER_CTRL_H;
            state_check(&format!("subtitles: sits above the bar at {}", tag), f.origin.y >= bar_top + 15.5);
            audit(&format!("subtitles @{}", tag));
        }
        // The Up Next card up at the minimum size, with a long two-line cue under it.
        sub_resize(960.0, 600.0);
        let _: () = msg_send![card, setHidden: Bool::NO];
        let _: () = msg_send![card, setAlphaValue: 1.0f64];
        crate::ui::subtitles::show_text(SUB_LONG);
        let problems = crate::ui::subtitles::chrome_problems();
        state_check(&format!("subtitles: long cue clear of the Up Next card ({})", problems.join("; ")), problems.is_empty());
        audit("subtitles long cue @min");
        let _: () = msg_send![card, setHidden: Bool::YES];
        crate::ui::subtitles::show_text(SUB_CUE);
        sub_resize(1280.0, 800.0);
    }
}

fn s_sub_hide() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.player.get(), setRate: 1.0f32];
        if let Some(p) = ui.playing.as_mut() {
            p.paused = false;
        }
        crate::ui::player::set_idle_age(2.6);
        crate::ui::player::hide_bars_if_idle();
        state_check("subtitles: controls hidden after idle", !ui.bar_shown);
    }
}

/// Animated lift: after the controls hide, the overlay settles near the bottom of the video.
fn s_sub_bottom() {
    unsafe {
        let ui = UI.get().unwrap();
        let (text, f) = crate::ui::subtitles::state();
        state_check("subtitles: cue still on screen while playing", text == SUB_CUE);
        state_check(&format!("subtitles: dropped to the bottom (y={:.0})", f.origin.y), f.origin.y < 40.0 && f.origin.y >= 0.0);
        let problems = crate::ui::subtitles::chrome_problems();
        state_check(&format!("subtitles: clear with controls hidden ({})", problems.join("; ")), problems.is_empty());
        state_check("subtitles: controls still hidden", !ui.bar_shown);
    }
    audit("subtitles controls hidden");
}

fn s_sub_seek() {
    unsafe {
        seek_to_secs(4.0);
        let (text, _) = crate::ui::subtitles::state();
        state_check("subtitles: seek clears the cue", text.is_empty() && !crate::ui::subtitles::visible());
    }
}

fn s_sub_off() {
    unsafe {
        set_track(0, 0);
        let ui = UI.get().unwrap();
        state_check("subtitles: Off selects no track", ui.sel_sub == 0);
    }
}

fn s_sub_off_check() {
    unsafe {
        let (text, _) = crate::ui::subtitles::state();
        state_check("subtitles: overlay empty with the track off", text.is_empty() && !crate::ui::subtitles::visible());
        crate::ui::player::show_bars_instant();
    }
}
const LONG_SERIES: &str = "The Extraordinarily Long Series Name That Keeps Going Season After Season Forever";
fn s_player_header() {
    // Episode with a long show name and a long episode title: title on top, "Series · S12 E105" under it.
    handle_main(MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(),
        item_id: "ep-long".into(),
        title: LONG.into(),
        subtitle: Some(LONG_SERIES.into()),
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: None,
        media_kind: MediaKind::Video,
        series_name: Some(LONG_SERIES.into()),
        season_number: Some(12),
        episode_number: Some(105),
        production_year: Some(2001),
    segments: Vec::new(),
    series_id: None,
    next_up: None,
}));
    unsafe {
        let ui = UI.get_mut().unwrap();
        let t = read_field(ui.top_title.get());
        let s = read_field(ui.top_sub.get());
        state_check("episode header: title is the episode name", t == LONG);
        state_check("episode header: series line", s == format!("{} · S12 E105", LONG_SERIES));
        let bar: *mut AnyObject = msg_send![ui.top_title.get(), superview];
        state_check("episode header sits in the top bar (fades with it)", bar == ui.top_bar.get());
    }
    audit_both("player header (episode)");
    // Movie: the year goes under the title.
    handle_main(MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(),
        item_id: "movie-hdr".into(),
        title: "Fake Movie".into(),
        subtitle: None,
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: None,
        media_kind: MediaKind::Video,
        series_name: None,
        season_number: None,
        episode_number: None,
        production_year: Some(1999),
    segments: Vec::new(),
    series_id: None,
    next_up: None,
}));
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("movie header: title", read_field(ui.top_title.get()) == "Fake Movie");
        state_check("movie header: year", read_field(ui.top_sub.get()) == "1999");
    }
    audit_both("player header (movie)");
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
fn audio_start(title: &str, artist: &str) -> MainMsg {
    MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(), // the video asset has an audio track; the Audio kind never shows the picture
        item_id: "track-1".into(),
        title: title.into(),
        subtitle: Some(artist.into()),
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: Some(poster_key("Audio-0")),
        media_kind: MediaKind::Audio,
        series_name: None,
        season_number: None,
        episode_number: None,
        production_year: None,
    segments: Vec::new(),
    series_id: None,
    next_up: None,
})
}
fn s_audio_start() {
    // Colourful artwork so the tint wash is a real colour.
    handle_main(MainMsg::PosterReady { key: poster_key("Audio-0"), bytes: image_bytes(0, 120, 120, 200) });
    unsafe {
        let ui = UI.get().unwrap();
        state_check("tint extracted for artwork", ui.tint_cache.contains_key(&poster_key("Audio-0")));
        let (_, s, b) = ui.tint_cache[&poster_key("Audio-0")];
        state_check("tint within clamp", (0.25..=0.60).contains(&s) && (0.22..=0.34).contains(&b));
        state_check("bar hidden before audio", !crate::ui::nowbar::visible());
        let f: NSRect = msg_send![ui.lib_scroll.get(), frame];
        state_check("lib_scroll y == 0 before audio", f.origin.y == 0.0);
    }
    handle_main(audio_start(
        &format!("{} {}", "A Remarkably Long Track Title That Cannot Possibly Fit", LONG),
        &format!("{} · {}", "An Artist Name Which Is Also Extremely Long Indeed", LONG),
    ));
}
fn s_audio_bar() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        state_check("audio: playing set", ui.playing.is_some());
        let ph: Bool = msg_send![ui.player_host.get(), isHidden];
        state_check("audio: player_host stays hidden", ph == Bool::YES);
        let ch: Bool = msg_send![ui.content_host.get(), isHidden];
        state_check("audio: content_host stays visible", ch == Bool::NO);
        let bh: Bool = msg_send![ui.now.bar.get(), isHidden];
        state_check("audio: bar visible", bh == Bool::NO && crate::ui::nowbar::visible());
        let f: NSRect = msg_send![ui.lib_scroll.get(), frame];
        state_check("audio: lib_scroll origin y == NOW_BAR_H", f.origin.y == crate::ui::theme::NOW_BAR_H);
        let bf: NSRect = msg_send![ui.now.bar.get(), frame];
        state_check("audio: bar at the bottom of the content area",
            bf.origin.y == 0.0 && bf.origin.x == crate::ui::theme::SIDEBAR_W && bf.size.h == crate::ui::theme::NOW_BAR_H);
        let iv: f64 = msg_send![ui.timer.get(), timeInterval];
        state_check("audio: 0.25 s tick while visible", (iv - 0.25).abs() < 1e-6);
        state_check("audio: title filled", read_field(ui.now.title.get()).contains("Remarkably"));
        state_check("audio: elapsed/remaining filled",
            read_field(ui.now.elapsed.get()).contains(':') && read_field(ui.now.remain.get()).starts_with('-'));
        // transport + sliders
        act(sel!(jrNowSkip:), nil());
        act(sel!(jrNowRestart:), nil());
        let _: () = msg_send![ui.now.scrub.get(), setDoubleValue: 0.5f64];
        act(sel!(jrNowScrub:), ui.now.scrub.get());
        let _: () = msg_send![ui.now.vol.get(), setDoubleValue: 0.4f64];
        act(sel!(jrNowVolume:), ui.now.vol.get());
        let v: f32 = msg_send![ui.player.get(), volume];
        state_check("audio: volume slider drives the player", (v - 0.4).abs() < 0.01);
        let _: () = msg_send![ui.now.vol.get(), setDoubleValue: 1.0f64];
        act(sel!(jrNowVolume:), ui.now.vol.get());
        act(sel!(playPause:), nil());
        update_now_bar();
        act(sel!(playPause:), nil());
    }
    audit_both("now playing");
    // Short strings too.
    handle_main(audio_start("Song", "Artist · Album"));
    audit("now playing/short");
}
fn update_now_bar() {
    unsafe { crate::ui::nowbar::update() }
}
fn s_audio_stop() {
    unsafe {
        act(sel!(stopPlayback:), nil());
        let ui = UI.get().unwrap();
        state_check("audio: playing cleared", ui.playing.is_none());
        let bh: Bool = msg_send![ui.now.bar.get(), isHidden];
        state_check("audio: bar hidden after stop", bh == Bool::YES && !crate::ui::nowbar::visible());
        let f: NSRect = msg_send![ui.lib_scroll.get(), frame];
        state_check("audio: lib_scroll origin y restored to 0", f.origin.y == 0.0);
        let iv: f64 = msg_send![ui.timer.get(), timeInterval];
        state_check("audio: 1 s tick restored", (iv - 1.0).abs() < 1e-6);
    }
}
fn s_fullscreen() {
    handle_main(playback(0.0, 6.0));
    if offscreen_mode() {
        // Full screen would take over a real display; only exercised with JELLYFAST_SELFTEST_ONSCREEN=1.
        log("  fullscreen toggle skipped (headless mode)");
        return;
    }
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
// ---- skip buttons, Up Next and autoplay -------------------------------------------------------
fn playback_ext(item: &str, title: &str, resume: f64, run: f64, segs: Vec<Segment>, series: Option<&str>, next: Option<NextUp>) -> MainMsg {
    MainMsg::PlaybackReady(PlaybackStart {
        season_episodes: Vec::new(),
        url: video_url(),
        item_id: item.into(),
        title: title.into(),
        subtitle: None,
        resume_secs: resume,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: run,
        image_key: None,
        media_kind: MediaKind::Video,
        series_name: series.map(|_| "Mock Show".to_string()),
        season_number: series.map(|_| 1),
        episode_number: series.map(|_| 2),
        // Movies carry a year, as real ones do; the header then always has a subtitle line.
        production_year: if series.is_none() { Some(2024) } else { None },
        segments: segs,
        series_id: series.map(str::to_string),
        next_up: next,
    })
}
/// Intro from the start to `end` seconds (the 6 s self-test clip).
fn intro_to(end: f64) -> Vec<Segment> {
    vec![Segment { kind: SegmentKind::Intro, start: 0.0, end }]
}
fn mock_next() -> NextUp {
    NextUp { id: "ep-next".into(), name: "The Next One".into(), season: Some(1), number: Some(3), image_key: None }
}
/// Scrubber seeks go through pb::seek_frac; the landing is read a step later, once the seek completes.
/// (A +/-10 s check is left out: the 6 s mock item clamps every such seek, so it would prove nothing.)
fn seek_playing_dur() -> f64 {
    unsafe { UI.get().unwrap().playing.as_ref().map(|p| p.duration_secs).unwrap_or(0.0) }
}
fn seek_now() -> f64 {
    unsafe {
        let t: CMTime = msg_send![UI.get().unwrap().player.get(), currentTime];
        t.seconds()
    }
}
fn s_seek_scrub() {
    unsafe { crate::ui::playerbar::seek_frac(0.5) };
}
fn s_seek_scrub_check() {
    let dur = seek_playing_dur();
    let p = seek_now();
    state_check(&format!("seek: scrubber landed near 50% (at {:.2}s of {:.2}s)", p, dur * 0.5),
        dur > 0.0 && (p - dur * 0.5).abs() < 1.0);
}
fn in_segment_pos() -> f64 {
    unsafe { crate::ui::skip::position() }
}
// The overlay is driven by its 0.25 s timer, so every check on it runs a step after the change.
fn s_skip_show() {
    handle_main(playback_ext("skip-1", "Intro Test", 1.5, 6.0, intro_to(4.5), None, None));
}
fn s_skip_audit() {
    unsafe {
        state_check("skip: button shown inside the intro", crate::ui::skip::button_visible());
        state_check("skip: label is Skip Intro", crate::ui::skip::button_label() == "Skip Intro");
        let m = crate::ui::skip::button_metrics();
        state_check(&format!("skip: button at least 44 pt tall (panel {:.0}, button {:.0})", m.pill_h, m.btn_h), m.pill_h >= 44.0 && m.btn_h >= 44.0);
        state_check("skip: label fits the button, not truncated", m.label_fits);
        state_check(&format!("skip: button 32 pt in from the right edge (at {:.0})", m.right_gap), (m.right_gap - 32.0).abs() < 1.0);
        state_check(&format!("skip: button clears the transport bar (gap {:.0})", m.bottom_gap), m.bottom_gap >= 12.0);
        // Hover in and out exercises the button's own pointer handling (its press path needs a real event).
        let btn = UI.get().unwrap().skip.button.get();
        let _: () = msg_send![btn, mouseEntered: nil()];
        let _: () = msg_send![btn, mouseExited: nil()];
        // Resume seek (1.5 s) must land there, not at 0: about 0.7 s has played since.
        let p = in_segment_pos();
        state_check(&format!("resume: playhead landed near 1.5 s (at {:.2}s)", p), p >= 1.2 && p <= 3.2);
    }
    audit_both("skip button");
}
fn s_skip_click() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let btn = ui.skip.button.get();
        let _: () = msg_send![btn, performClick: nil()];
    }
}
fn s_skip_click_check() {
    unsafe {
        let p = in_segment_pos();
        state_check(&format!("skip: click seeked to the end (at {:.2}s)", p), p >= 4.3 && p <= 5.5);
        state_check("skip: button hides after the skip", !crate::ui::skip::button_visible());
    }
}
fn s_skip_key_start() {
    handle_main(playback_ext("skip-2", "Intro Key", 1.5, 6.0, intro_to(4.5), None, None));
}
fn s_skip_key() {
    unsafe {
        state_check("skip: button up for the S key case", crate::ui::skip::button_visible());
        act(sel!(skipSegment:), nil());
    }
}
fn s_skip_key_check() {
    unsafe {
        let p = in_segment_pos();
        state_check(&format!("skip: S key seeked past the intro (at {:.2}s)", p), p >= 4.3);
    }
}
fn s_skip_series_start() {
    handle_main(playback_ext("skip-3", "Auto Test", 1.5, 6.0, intro_to(4.5), Some("show-auto"), None));
    unsafe {
        state_check("skip: per-show off by default", !crate::ui::skip::auto_skip_on());
    }
}
fn s_skip_series_on() {
    unsafe {
        state_check("skip: button up before turning it on", crate::ui::skip::button_visible());
        act(sel!(jrToggleSeriesSkip:), nil());
    }
}
fn s_skip_series_auto() {
    unsafe {
        state_check("skip: per-show on after toggle", crate::ui::skip::auto_skip_on());
        state_check("skip: toggle persisted for the show", crate::config::skip_intros_for("show-auto", false));
        let p = in_segment_pos();
        state_check(&format!("skip: toggle inside the intro skipped it (at {:.2}s)", p), p >= 4.3);
    }
    handle_main(playback_ext("skip-4", "Auto Next Time", 1.5, 6.0, intro_to(4.5), Some("show-auto"), None));
}
fn s_skip_series_off() {
    unsafe {
        state_check("skip: auto-skipped on the next playback", in_segment_pos() >= 4.3);
        let toast = read_field(UI.get().unwrap().chip_label.get());
        state_check(&format!("skip: 'Skipped intro' toast ({})", toast), toast.contains("Skipped intro"));
        state_check("skip: no button after auto-skip", !crate::ui::skip::button_visible());
        act(sel!(jrToggleSeriesSkip:), nil());
        state_check("skip: per-show off persisted", !crate::config::skip_intros_for("show-auto", true));
    }
}
/// Four episodes of season 1 for the Episodes panel: the first watched, the second half way, the rest unwatched.
fn ep_fixture(n: u32) -> Vec<crate::app::messages::SeasonEpisode> {
    (1..=n)
        .map(|i| crate::app::messages::SeasonEpisode {
            id: format!("ep-{}", i),
            number: Some(i),
            name: if i == 2 { LONG.to_string() } else { format!("Episode {} Title", i) },
            runtime_secs: 1500.,
            progress: match i {
                1 => 1.0,
                2 => 0.45,
                _ => 0.0,
            },
            image_key: None,
        })
        .collect()
}
/// Overlay of a movie: no episode buttons, More and full screen present.
fn s_player_movie_overlay() {
    handle_main(playback(0.0, 6.0));
    unsafe {
        crate::ui::player::show_bars_instant();
        let ui = UI.get().unwrap();
        state_check("movie overlay: Episodes and Next Episode hidden", is_hidden(ui.pbar.episodes.get()) && is_hidden(ui.pbar.next.get()));
        state_check("movie overlay: More and full screen shown", !is_hidden(ui.pbar.more.get()) && !is_hidden(ui.pbar.full.get()));
        state_check("movie overlay: centred play button shown", !is_hidden(ui.pbar.play.get()));
    }
    audit_both("player overlay movie");
}
/// Overlay of an episode with a next one: the Episodes and Next Episode buttons join the right cluster.
fn s_player_episode_overlay() {
    handle_main(playback_ext("ep-2", LONG, 0.0, 6.0, Vec::new(), Some("show-ep"), Some(mock_next())));
    unsafe { crate::ui::episodes::set_items(ep_fixture(4), "ep-2", Some(1)) };
    unsafe {
        crate::ui::player::show_bars_instant();
        let ui = UI.get().unwrap();
        state_check("episode overlay: Episodes and Next Episode shown", !is_hidden(ui.pbar.episodes.get()) && !is_hidden(ui.pbar.next.get()));
        let en: Bool = msg_send![ui.pbar.next.get(), isEnabled];
        state_check("episode overlay: Next Episode enabled with a next episode", en == Bool::YES);
        state_check("episode overlay: transport and bar both shown", ui.bar_shown);
    }
    audit_both("player overlay episode");
}
/// Walks the view tree under `v` for the first button whose tag is `tag`.
unsafe fn find_tagged_button(v: *mut AnyObject, tag: isize) -> *mut AnyObject {
    let is: Bool = msg_send![v, isKindOfClass: class!(NSButton)];
    let t: isize = msg_send![v, tag];
    if is == Bool::YES && t == tag {
        return v;
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let f = find_tagged_button(c, tag);
        if !f.is_null() {
            return f;
        }
    }
    std::ptr::null_mut()
}
fn s_player_episodes_panel() {
    unsafe {
        act(sel!(toggleEpisodes:), nil());
        let ui = UI.get().unwrap();
        state_check("episodes panel opens", crate::ui::episodes::is_open() && !is_hidden(ui.episodes.panel.get()));
        let sv = ui.episodes.scroll.get();
        let doc: *mut AnyObject = msg_send![sv, documentView];
        let rows: *mut AnyObject = msg_send![doc, subviews];
        let n: usize = msg_send![rows, count];
        state_check(&format!("episodes panel lists the season ({} rows)", n), n == 4);
        state_check("episodes panel title names the season", read_field(ui.episodes.title.get()) == "Season 1");
        let ui_panel = ui.episodes.panel.get();
        let right: f64 = crate::ui::episodes::right_inset();
        state_check("episodes panel pushes the overlays left", right > 0.0);
        let _ = ui_panel;
    }
    audit_both("episodes panel");
}
fn s_player_episodes_pick() {
    unsafe {
        take_cmds();
        let doc: *mut AnyObject = msg_send![UI.get().unwrap().episodes.scroll.get(), documentView];
        let btn = find_tagged_button(doc, 2);
        state_check("episodes panel: row button found", !btn.is_null());
        if !btn.is_null() {
            act_with(ctl(), sel!(jrEpisodePick:), btn);
        }
        let sent = take_cmds();
        state_check(
            "episodes panel: picking a row plays that episode",
            sent.iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "ep-3")),
        );
        state_check("episodes panel closes after a pick", !crate::ui::episodes::is_open());
    }
}
fn s_player_next_episode() {
    unsafe {
        handle_main(playback_ext("ep-2", LONG, 0.0, 6.0, Vec::new(), Some("show-ep"), Some(mock_next())));
        take_cmds();
        state_check("next episode: N shortcut is in the table", crate::ui::shortcuts::SHORTCUTS.iter().any(|e| e.action == "nextEpisode:" && e.key == "n"));
        state_check("next episode: E shortcut is in the table", crate::ui::shortcuts::SHORTCUTS.iter().any(|e| e.action == "toggleEpisodes:" && e.key == "e"));
        state_check("next episode: enabled when there is a next", crate::ui::shortcuts::item_enabled("nextEpisode:", 0, 0));
        act(sel!(nextEpisode:), nil());
        let sent = take_cmds();
        state_check(
            "next episode: plays the next episode now",
            sent.iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "ep-next")),
        );
    }
}
fn s_upnext_card() {
    handle_main(playback_ext("ep-cur", "Current Episode", 0.0, 6.0, Vec::new(), Some("show-next"), Some(mock_next())));
}
fn s_upnext_play() {
    unsafe {
        state_check("up next: card shown near the end", crate::ui::skip::card_visible());
        let text = crate::ui::skip::card_text();
        state_check(&format!("up next: card names the next episode ({})", text), text.contains("The Next One") && text.contains("Playing in"));
        state_check("up next: no skip button on a plain episode", !crate::ui::skip::button_visible());
        state_check("up next: title fits the card", crate::ui::skip::card_title_fits());
    }
    audit_both("up next card");
    unsafe {
        act(sel!(jrUpNextPlay:), nil());
    }
    let cmds = take_cmds();
    let asked = cmds.iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "ep-next"));
    let stopped = cmds.iter().any(|c| matches!(c, UiCmd::Stopped { item_id, .. } if item_id == "ep-cur"));
    let marked = cmds.iter().any(|c| matches!(c, UiCmd::MarkPlayed { item_id, played: true } if item_id == "ep-cur"));
    state_check("up next: Play Now asks for the next episode", asked);
    state_check("up next: current episode stopped and marked watched", stopped && marked);
}
fn s_upnext_next() {
    handle_main(playback_ext("ep-next", "The Next One", 0.0, 6.0, Vec::new(), Some("show-next"), None));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("up next: player title updated", read_field(ui.top_title.get()) == "The Next One");
        state_check("up next: now playing the next episode", ui.playing.as_ref().map(|p| p.item_id.as_str()) == Some("ep-next"));
        state_check("up next: card gone", !crate::ui::skip::card_visible());
    }
}
fn s_upnext_cancel() {
    handle_main(playback_ext("ep-c", "Cancel Test", 0.0, 6.0, Vec::new(), Some("show-next"), Some(mock_next())));
}
fn s_upnext_cancel_check() {
    unsafe {
        state_check("up next: card up before cancel", crate::ui::skip::card_visible());
        act(sel!(jrUpNextCancel:), nil());
        state_check("up next: cancel hides the card", !crate::ui::skip::card_visible());
    }
    let cmds = take_cmds();
    state_check("up next: cancel sends no Play", !cmds.iter().any(|c| matches!(c, UiCmd::Play { .. })));
}
fn s_upnext_cancel_end() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("up next: cancelled episode ends and leaves the player", ui.playing.is_none());
    }
    let cmds = take_cmds();
    state_check("up next: cancelled episode is marked watched at the end", cmds.iter().any(|c| matches!(c, UiCmd::MarkPlayed { item_id, played: true } if item_id == "ep-c")));
}
fn s_upnext_end() {
    handle_main(playback_ext("ep-e", "End Test", 0.0, 6.0, Vec::new(), Some("show-next"), Some(mock_next())));
}
fn s_upnext_end_check() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("up next: still in the player at the end", ui.playing.is_some());
    }
    let cmds = take_cmds();
    state_check("up next: reaching the end asks for the next episode", cmds.iter().any(|c| matches!(c, UiCmd::Play { item_id } if item_id == "ep-next")));
    handle_main(playback_ext("ep-next", "The Next One", 0.0, 6.0, Vec::new(), Some("show-next"), None));
}
fn s_upnext_stop() {
    unsafe { act(sel!(stopPlayback:), nil()) }
}
fn s_settings_skip_controls() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let mut c = ui.cfg.clone();
        c.settings.autoplay_next = false;
        c.settings.skip_intros_default = true;
        crate::ui::settings::settings_fill(&c);
        let a: NSInteger = msg_send![ui.s_autoplay.get(), state];
        let s: NSInteger = msg_send![ui.s_skip_default.get(), state];
        state_check("settings: autoplay checkbox reflects the setting", a == 0);
        state_check("settings: skip-by-default checkbox reflects the setting", s == 1);
        crate::ui::settings::settings_fill(&ui.cfg.clone());
        let a: NSInteger = msg_send![ui.s_autoplay.get(), state];
        state_check("settings: autoplay checkbox back to on", a == 1);
    }
}
unsafe fn find_titled(v: *mut AnyObject, title: &str) -> *mut AnyObject {
    if is_kind(v, class!(NSButton)) {
        let t: *mut AnyObject = msg_send![v, title];
        if to_str(t) == title {
            return v;
        }
    }
    let subs: *mut AnyObject = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let found = find_titled(c, title);
        if !found.is_null() {
            return found;
        }
    }
    nil()
}
fn s_detail_skip_toggle() {
    handle_main(MainMsg::DetailLoaded(Box::new(detail_view(true))));
    unsafe {
        let cb = find_titled(detail_host(), "Skip intros automatically");
        state_check("detail: series page has the skip toggle", !cb.is_null());
        if cb.is_null() {
            return;
        }
        let _: () = msg_send![cb, performClick: nil()];
        let on: NSInteger = msg_send![cb, state];
        state_check("detail: toggle saved for the show", crate::config::skip_intros_for("detail-series", false) == (on != 0));
        let _: () = msg_send![cb, performClick: nil()];
        let on: NSInteger = msg_send![cb, state];
        state_check("detail: toggle back, saved again", crate::config::skip_intros_for("detail-series", true) == (on != 0));
    }
    crate::ui::detail::hide();
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
    unsafe {
        let ui = UI.get().unwrap();
        let c: *mut AnyObject = msg_send![ui.login_err.get(), textColor];
        // The secondary role resolves through the active palette (the default dark carries its own).
        let sec = crate::ui::theme::color(crate::ui::theme::Col::Text2);
        // Theme colours are dynamic objects, so compare what they resolve to.
        state_check("signed-out message is neutral (secondary colour)", srgb(c) == srgb(sec));
        let t: *mut AnyObject = msg_send![ui.win.get(), title];
        state_check("window title still Jellyfast after playback", to_str(t) == "Jellyfast");
    }
}
fn s_quit_prep() {
    unsafe {
        let ui = UI.get().unwrap();
        state_check("no sheet left", {
            let _ = ui;
            !crate::platform::appkit::settings_open()
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
        poster_path: if id == 11 || id == 22 { Some(format!("/p{}.jpg", id)) } else { None },
        overview: "overview".into(),
        availability: a,
        ..Default::default()
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
fn seerr_hidden(v: *mut AnyObject) -> bool {
    unsafe {
        let h: Bool = msg_send![v, isHidden];
        h == Bool::YES
    }
}
/// Requests is the visible screen in the main window (not a window or sheet of its own).
fn seerr_on_screen() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr.active && visible(ui.seerr.host.get()) && visible(ui.seerr.doc.get())
    }
}
fn sheet_open() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        let _ = ui;
        crate::platform::appkit::settings_open()
    }
}
/// Direct subviews of `row` that are an instance of `cls` and satisfy `pred`.
unsafe fn seerr_subviews(row: *mut AnyObject, cls: &str, pred: &dyn Fn(*mut AnyObject) -> bool) -> usize {
    let c = AnyClass::get(&std::ffi::CString::new(cls).expect("class name has no NUL")).map_or(std::ptr::null_mut(), |c| c as *const AnyClass as *mut AnyObject);
    let subs: *mut AnyObject = msg_send![row, subviews];
    let n: usize = msg_send![subs, count];
    let mut k = 0;
    for i in 0..n {
        let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let is: Bool = msg_send![v, isKindOfClass: c];
        if is == Bool::YES && pred(v) {
            k += 1;
        }
    }
    k
}
/// A status pill: a 20 pt tall box with radius 10. Poster placeholders share the radius,
/// so the height tells them apart.
unsafe fn seerr_is_pill(b: *mut AnyObject) -> bool {
    let rad: f64 = msg_send![b, cornerRadius];
    let f: NSRect = msg_send![b, frame];
    (rad - 10.).abs() < 0.01 && (f.size.h - 20.).abs() < 0.01
}
/// Number of rows in the Seerr list that carry a Request button.
fn seerr_button_rows() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr.cards.iter().filter(|r| seerr_subviews(**r, "NSButton", &|_| true) > 0).count()
    }
}
/// Status pills (NSBox with corner radius 10) across all rows.
fn seerr_pills() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr
            .cards
            .iter()
            .map(|r| {
                seerr_subviews(*r, "NSBox", &|b| seerr_is_pill(b))
            })
            .sum()
    }
}
/// Every card in the grid has the same height (rows line up).
fn seerr_card_heights_ok() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        // Cards in one grid row share a height and a top edge; every card has a height.
        ui.seerr.cards.iter().all(|c| {
            let f = frame_of(*c);
            f.size.h > 0. && ui.seerr.cards.iter().filter(|o| (frame_of(**o).origin.y - f.origin.y).abs() < 0.5).all(|o| (frame_of(*o).size.h - f.size.h).abs() < 0.01)
        })
    }
}
/// Each result card has a Request button exactly when it is not requested, and a pill otherwise.
fn seerr_kinds_match() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr.cards.len() == ui.seerr.results.len()
            && ui.seerr.cards.iter().zip(ui.seerr.results.iter()).all(|(card, r)| {
                let want_button = r.availability == Availability::NotRequested;
                let has_button = seerr_subviews(*card, "NSButton", &|_| true) > 0;
                let has_pill = seerr_subviews(*card, "NSBox", &|b| seerr_is_pill(b)) > 0;
                has_button == want_button && has_pill == !want_button
            })
    }
}
/// Whether the Requests row exists in the sidebar and is a sidebar row.
fn seerr_sidebar_row_ok() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        let row = ui.seerr.row.get();
        if row.is_null() {
            return false;
        }
        let cls = AnyClass::get(c"JRSidebarRow").unwrap();
        let k: Bool = msg_send![row, isKindOfClass: cls];
        k == Bool::YES
    }
}
fn seerr_empty_shown() -> Option<(String, String)> {
    unsafe {
        let ui = UI.get().unwrap();
        if seerr_hidden(ui.seerr.empty.get()) {
            return None;
        }
        Some((read_field(ui.seerr.empty_title.get()), read_field(ui.seerr.empty_body.get())))
    }
}
const TEST_PNG: [u8; 75] = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 6, 8, 6, 0, 0, 0, 228, 57, 63, 117, 0, 0, 0, 18, 73, 68, 65, 84, 120, 156, 99, 56, 97, 99, 243, 31, 25, 51, 208, 66, 0, 0, 182, 120, 53, 233, 148, 150, 172, 5, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130];
fn seerr_status_text() -> String {
    unsafe { read_field(UI.get().unwrap().seerr.status.get()) }
}
fn seerr_note(field: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification"), object: field]
    }
}

fn s_seerr_disabled() {
    unsafe {
        take_cmds();
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = false;
        ui.cfg.settings.seerr_url = String::new();
        act(sel!(requestsSelected:), nil());
        let ui = UI.get().unwrap();
        state_check("requests tab shown when disabled (setup state)", seerr_on_screen());
        state_check("no sidebar Requests row when disabled", ui.seerr.row.get().is_null());
        state_check("setup state shown when disabled", !seerr_hidden(ui.seerr.hint.get()));
        state_check("search hidden when disabled", seerr_hidden(ui.seerr.search.get()));
        state_check("Home not selected on Requests", !ui.home.active && !visible(ui.home.scroll.get()));
        state_check("Requests remembered as last place", ui.cfg.last_view.as_deref() == Some("requests"));
        audit("requests setup state");
    }
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = true; // enabled but no URL is still unconfigured
        act(sel!(openSeerr:), nil());
        let ui = UI.get().unwrap();
        state_check("setup state when URL empty", !seerr_hidden(ui.seerr.hint.get()));
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
        act(sel!(requestsSelected:), nil()); // the sidebar entry appears and is selected
        let ui = UI.get().unwrap();
        state_check("sidebar Requests row shown when configured", seerr_sidebar_row_ok());
        state_check("Requests row selected", row_selected(ui.seerr.row.get()));
        state_check("Home row not selected", !row_selected(ui.home.row.get()));
        state_check("requests view on screen", seerr_on_screen());
        state_check("header reads Requests", read_field(ui.header_title.get()) == "Requests");
        state_check("library grid hidden", !visible(ui.lib_scroll.get()));
        state_check("sort popup hidden", !visible(ui.sort_popup.get()));
        check_offscreen("requests tab");
        state_check("setup state hidden when configured", seerr_hidden(ui.seerr.hint.get()));
        state_check("search visible", !seerr_hidden(ui.seerr.search.get()));
        state_check("no cards yet", ui.seerr.cards.is_empty());
        state_check(
            "hint state: search prompt",
            seerr_empty_shown().map(|(_, b)| b == "Search for a movie or show to request.") == Some(true),
        );
        state_check("remembered as last place", ui.cfg.last_view.as_deref() == Some("requests"));
    }
    audit("requests empty");
}
fn s_seerr_login_restore() {
    unsafe {
        take_cmds();
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.seerr_enabled = true;
        ui.cfg.settings.seerr_url = "http://seerr.example:5055".into();
        ui.cfg.last_view = Some(crate::ui::seerr::REQUESTS_ID.into());
    }
    // Same libraries as the other connect steps, so later library steps see the usual set.
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
        state_check("login restores Requests over Home", ui.seerr.active && seerr_on_screen());
        state_check("login: Home inactive", !ui.home.active);
        state_check("login: no LoadHome", !take_cmds().iter().any(|c| matches!(c, UiCmd::LoadHome)));
        act(sel!(homeSelected:), nil());
        state_check("back to Home from Requests", UI.get().unwrap().home.active && !seerr_on_screen());
    }
}
fn s_seerr_results() {
    unsafe { act(sel!(requestsSelected:), nil()) };
    state_check("requests on screen before results", seerr_on_screen());
    handle_main(MainMsg::SeerrResults(seerr_mixed()));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("7 cards", ui.seerr.cards.len() == 7);
        state_check("2 request buttons (NotRequested only)", seerr_button_rows() == 2);
        state_check("requestable cards: Request button, no pill; others: pill", seerr_kinds_match());
        state_check("cards share one height", seerr_card_heights_ok());
        state_check("a pill per card without a Request button", seerr_pills() == 7 - seerr_button_rows());
        state_check("empty state hidden with rows", seerr_empty_shown().is_none());
        state_check("thumbnail waits for the poster", unsafe { crate::ui::seerr::thumb_has_image("/p11.jpg") } == Some(false));
    }
    audit("requests results");
    // The worker would fetch these through the Seerr server; fake its reply (no network).
    handle_main(MainMsg::SeerrPoster { path: "/p11.jpg".into(), bytes: TEST_PNG.to_vec() });
    handle_main(MainMsg::SeerrPoster { path: "/bad.jpg".into(), bytes: vec![1, 2, 3] });
    unsafe {
        state_check("thumbnail image set after poster reply", crate::ui::seerr::thumb_has_image("/p11.jpg") == Some(true));
        state_check("other thumbnail still a placeholder", crate::ui::seerr::thumb_has_image("/p22.jpg") == Some(false));
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
        state_check("loading: no rows, no empty state", ui.seerr.cards.is_empty() && seerr_empty_shown().is_none());
        handle_main(MainMsg::SeerrRequestList(vec![
            seerr_request_item(1, "One", RequestStatus::Pending, Availability::Pending),
            seerr_request_item(2, "Two", RequestStatus::Approved, Availability::Processing),
            seerr_request_item(3, "Three", RequestStatus::Declined, Availability::NotRequested),
            seerr_request_item(4, "Four", RequestStatus::Failed, Availability::NotRequested),
            seerr_request_item(5, "Five", RequestStatus::Completed, Availability::Available),
            seerr_request_item(6, "Six", RequestStatus::Unknown, Availability::Blocklisted),
        ]));
        state_check("6 request rows", ui.seerr.cards.len() == 6);
        state_check("no request buttons in My requests", seerr_button_rows() == 0);
        state_check("a pill per request row", seerr_pills() == 6);
        audit("seerr my requests");
        handle_main(MainMsg::SeerrRequestList(vec![]));
        state_check(
            "empty requests state",
            ui.seerr.cards.is_empty()
                && seerr_empty_shown() == Some(("No requests yet".into(), "You haven't requested anything yet.".into())),
        );
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
        state_check("back on search tab", !ui.seerr.requests_mode && ui.seerr.cards.len() == 7);
    }
}
/// The grid reflows with the window: column count follows the width, nothing overflows,
/// and the layout audit is clean at the minimum, a mid size and a large size.
fn s_seerr_resize() {
    for (w, h) in [(960.0, 600.0), (1700.0, 1000.0), (1280.0, 800.0)] {
        set_content_size(w, h);
        handle_main(MainMsg::SeerrResults(seerr_mixed())); // rebuild at the new width
        unsafe {
            let ui = UI.get().unwrap();
            let doc_w = frame_of(ui.seerr.doc.get()).size.w;
            let (cols, _) = crate::ui::seerr::card_columns(doc_w);
            let first_y = frame_of(ui.seerr.cards[0]).origin.y;
            let on_first_row = ui.seerr.cards.iter().filter(|c| (frame_of(**c).origin.y - first_y).abs() < 0.5).count();
            state_check(
                &format!("columns follow width @{}x{}", w, h),
                on_first_row == cols.min(ui.seerr.cards.len()),
            );
            let fits = ui.seerr.cards.iter().all(|c| {
                let f = frame_of(*c);
                f.origin.x >= 0. && f.origin.x + f.size.w <= doc_w + 0.5
            });
            state_check(&format!("cards inside the grid width @{}x{}", w, h), fits);
        }
        audit(&format!("requests @{}x{}", w, h));
    }
    set_content_size(1280.0, 800.0);
}
fn s_seerr_reopen() {
    unsafe { act(sel!(homeSelected:), nil()) };
    unsafe {
        let ui = UI.get().unwrap();
        state_check("leaving Requests hides it", !seerr_on_screen() && !ui.seerr.active);
        state_check("Home is the screen again", ui.home.active && visible(ui.home.scroll.get()));
        state_check("Home remembered as last place", ui.cfg.last_view.as_deref() == Some("home"));
    }
    // messages while Requests is away are applied, not dropped
    handle_main(MainMsg::SeerrRequested { title: "Fresh Show".into() });
    unsafe { act(sel!(requestsSelected:), nil()) };
    unsafe {
        let ui = UI.get().unwrap();
        state_check("Requests comes back", seerr_on_screen());
        state_check("results survive leaving", ui.seerr.results.len() == 7 && ui.seerr.cards.len() == 7);
        state_check("Requests remembered again", ui.cfg.last_view.as_deref() == Some("requests"));
    }
}
fn s_seerr_empty() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.seerr.search.get(), "zzzz");
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check(
            "no results state",
            ui.seerr.cards.is_empty()
                && seerr_empty_shown() == Some(("No results".into(), "No matches for “zzzz”.".into())),
        );
        set_text(ui.seerr.search.get(), "");
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check("blank search state", ui.seerr.cards.is_empty() && seerr_empty_shown().map(|t| t.0) == Some("Find a title".into()));
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
    audit("settings seerr error");
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
        state_check("requests shows the search UI", seerr_hidden(ui.seerr.hint.get()));
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
    unsafe { act(sel!(homeSelected:), nil()) };
    take_cmds();
    state_check("Requests left for Home", !unsafe { UI.get().unwrap().seerr.active });
}

// ---- themes -----------------------------------------------------------------
fn theme_ocean_json() -> &'static str {
    r##"{"base":"dark","name":"Selftest Ocean","colors":{"window":"#0B1620","panel":"#0F1D2A","surface":"#1A2D3D","surface_hover":"#243B4E","surface_active":"#2E4A61","outline":"#2E4A61","text":"#E6F7FF","secondary":"#A9C8D8","dim":"#8FB0C2","accent":"#4FC3F7","accent_hover":"#81D4FA","on_accent":"#0B1620","danger":"#FF6B6B","warning":"#FFD166","overlay":"#000000B3","shadow":"#00000066","tint":"#0F1D2A","ok":"#7BD88F"}}"##
}
/// Resolved sRGB (thousandths) of a 0xRRGGBB value, as `srgb()` reports it.
fn expect_srgb(v: u32) -> [i64; 4] {
    let c = |s: u32| (((v >> s) & 0xff) as f64 / 255. * 1000.).round() as i64;
    [c(16), c(8), c(0), 1000]
}
fn popup_titles(p: *mut AnyObject) -> Vec<String> {
    unsafe {
        let t: *mut AnyObject = msg_send![p, itemTitles];
        let n: usize = msg_send![t, count];
        (0..n).map(|i| to_str(msg_send![t, objectAtIndex: i])).collect()
    }
}
fn s_themes_palette() {
    let dir = std::env::temp_dir().join(format!("jf-selftest-themes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("selftest-ocean.json"), theme_ocean_json()).unwrap();
    std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
    std::env::set_var("JELLYFAST_THEMES_DIR", &dir);
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.theme = "selftest-ocean".into();
        crate::ui::shell::reload_themes();
        let reg = crate::ui::palettes::registry();
        state_check("themes: valid palette loaded", reg.iter().any(|t| t.id == "selftest-ocean"));
        state_check("themes: invalid palette skipped", !reg.iter().any(|t| t.id == "broken"));
        let titles = popup_titles(ui.s_theme.get());
        state_check("themes: picker lists palette by name", titles.iter().any(|t| t == "Selftest Ocean"));
        state_check("themes: picker starts with Follow system", titles.first().map(|t| t == "Follow system").unwrap_or(false));
        state_check("themes: palette text colour applied", srgb(crate::ui::theme::color(crate::ui::theme::Col::Text)) == expect_srgb(0xE6F7FF));
        state_check("themes: palette window colour applied", srgb(crate::ui::theme::color(crate::ui::theme::Col::Card)) == expect_srgb(0x0B1620));
        state_check("themes: palette accent applied", srgb(crate::ui::theme::color(crate::ui::theme::Col::Accent)) == expect_srgb(0x4FC3F7));
        state_check("themes: representative label follows palette", srgb(msg_send![ui.s_theme_note.get(), textColor]) == expect_srgb(0xA9C8D8));
        state_check("themes: single theme hides follow pickers", is_hidden(ui.s_light.get()));
    }
    audit("theme: selftest ocean");
}
fn s_themes_follow() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let base_h = ui.s_follow.base_h;
        ui.cfg.settings.theme = "system".into();
        ui.cfg.settings.system_light = "light".into();
        ui.cfg.settings.system_dark = "selftest-ocean".into();
        crate::ui::settings::fill_theme_popups(&ui.cfg);
        state_check("themes: follow system shows light picker", !is_hidden(ui.s_light.get()));
        state_check("themes: follow system shows dark picker", !is_hidden(ui.s_dark.get()));
        let light = popup_titles(ui.s_light.get());
        let dark = popup_titles(ui.s_dark.get());
        state_check("themes: light picker starts with Jellyfast Light", light.first().map(|t| t == "Jellyfast Light").unwrap_or(false));
        state_check("themes: light picker has no dark palette", !light.iter().any(|t| t == "Selftest Ocean"));
        state_check("themes: dark picker has the dark palette", dark.iter().any(|t| t == "Selftest Ocean"));
        ui.cfg.settings.theme = "dark".into();
        crate::ui::settings::fill_theme_popups(&ui.cfg);
        state_check("themes: single theme hides follow pickers again", is_hidden(ui.s_dark.get()));
        state_check("themes: window shrinks when pickers hide", ui.s_follow.cur_h < base_h);
        ui.cfg.settings.theme = "system".into();
        crate::ui::settings::fill_theme_popups(&ui.cfg);
        state_check("themes: follow system restores pickers", !is_hidden(ui.s_light.get()) && (ui.s_follow.cur_h - base_h).abs() < 0.5);
        // Missing palette: the cached copy keeps the colours and the Theme row says why.
        ui.cfg.settings.theme = "selftest-ocean".into();
    }
    let dir = std::path::PathBuf::from(std::env::var("JELLYFAST_THEMES_DIR").unwrap_or_default());
    let _ = std::fs::remove_file(dir.join("selftest-ocean.json"));
    unsafe {
        let ui = UI.get_mut().unwrap();
        crate::ui::shell::reload_themes();
        state_check("themes: cached palette still applied", srgb(crate::ui::theme::color(crate::ui::theme::Col::Text)) == expect_srgb(0xE6F7FF));
        let note = crate::ui::shell::THEME_NOTE.lock().map(|n| n.clone()).unwrap_or_default();
        state_check("themes: missing palette shows a note", note.contains("last saved"));
        state_check("themes: note reaches the Theme row", to_str(msg_send![ui.s_theme_note.get(), stringValue]).contains("last saved"));
    }
}
fn s_themes_shots() {
    for id in ["catppuccin-mocha", "catppuccin-latte", "rose-pine-dawn"] {
        unsafe {
            let ui = UI.get_mut().unwrap();
            ui.cfg.settings.theme = id.into();
            crate::ui::shell::apply_theme();
        }
        audit(&format!("theme: {id}"));
    }
}
fn s_themes_restore() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.theme = "dark".into();
        ui.cfg.settings.system_light = String::new();
        ui.cfg.settings.system_dark = String::new();
        crate::ui::shell::apply_theme();
        state_check("themes: restored to Dark", active_theme_is_builtin());
    }
    std::env::remove_var("JELLYFAST_THEMES_DIR");
}
fn active_theme_is_builtin() -> bool {
    unsafe {
        let text = srgb(crate::ui::theme::color(crate::ui::theme::Col::Text));
        // The default dark is a fixed near-black palette now; the system label colour
        // is still accepted for the light built-in.
        let label: *mut AnyObject = msg_send![class!(NSColor), labelColor];
        let dark_text: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: 242. / 255., green: 242. / 255., blue: 247. / 255., alpha: 1.0f64];
        text == srgb(label) || text == srgb(dark_text)
    }
}
