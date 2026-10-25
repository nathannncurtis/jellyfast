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
    std::env::var("JELLYFAST_SELFTEST").map(|v| v == "1").unwrap_or(false)
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
        // Launch state: no library chrome, no spinner, branded window.
        let h: BOOL = msg_send![ui.content_host.get(), isHidden];
        state_check("launch: content_host hidden", h == YES);
        let h: BOOL = msg_send![ui.spinner.get(), isHidden];
        state_check("launch: spinner hidden", h == YES);
        if !crate::ui::theme::reduce_transparency() {
            let bh: BOOL = msg_send![ui.bgfx.get(), isHidden];
            state_check("launch: login backdrop visible", bh == NO);
        }
        let t: *mut Object = msg_send![ui.win.get(), title];
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
        ("items: movies", s_items_movies, 100),
        ("worker->UI path (WORKER_TX)", s_worker_path, 400),
        ("worker->UI path check", s_worker_check, 50),
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
        ("playback: chrome", s_play_chrome, 100),
        ("playback: controls", s_play_controls, 300),
        ("playback: subtitle/audio cycle", s_play_cycle, 2300),
        ("playback: toast hides itself", s_toast_hidden, 50),
        ("audit: player (long title, toast, info)", s_audit_player, 200),
        ("playback: speed", s_play_speed, 300),
        ("playback: server down/up/error", s_play_net, 500),
        ("playback: replace item", s_play_start, 1500),
        ("playback: stop", s_play_stop, 200),
        ("audio: start (now-playing bar)", s_audio_start, 1500),
        ("audio: bar layout + audit", s_audio_bar, 100),
        ("audio: stop", s_audio_stop, 200),
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
    let path = std::env::var("JELLYFAST_SELFTEST_VIDEO").unwrap_or_else(|_| {
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
        image_key: Some(poster_key("Movie-0")),
        media_kind: MediaKind::Video,
    })
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
unsafe fn visible(v: *mut Object) -> bool {
    let h: BOOL = msg_send![v, isHidden];
    let a: f64 = msg_send![v, alphaValue];
    h == NO && a > 0.01
}
unsafe fn is_kind(v: *mut Object, cls: &objc::runtime::Class) -> bool {
    let b: BOOL = msg_send![v, isKindOfClass: cls];
    b == YES
}
unsafe fn describe(v: *mut Object) -> String {
    let cls: *mut Object = msg_send![v, className];
    let txt = if is_kind(v, class!(NSControl)) {
        let sv: *mut Object = msg_send![v, stringValue];
        to_str(sv)
    } else {
        String::new()
    };
    let t: String = txt.chars().take(24).collect();
    let f: NSRect = msg_send![v, frame];
    format!("{}[\"{}\" {:.0},{:.0} {:.0}x{:.0}]", to_str(cls), t, f.origin.x, f.origin.y, f.size.w, f.size.h)
}
unsafe fn is_text_item(v: *mut Object) -> bool {
    is_kind(v, class!(NSTextField)) || is_kind(v, class!(NSButton))
}
unsafe fn is_hit_target(v: *mut Object) -> bool {
    if !is_kind(v, class!(NSButton)) {
        return false;
    }
    let t: *mut Object = msg_send![v, title];
    let img: *mut Object = msg_send![v, image];
    to_str(t).is_empty() && img.is_null()
}
unsafe fn audit_view(v: *mut Object, issues: &mut Vec<String>) {
    if !visible(v) {
        return;
    }
    let subs: *mut Object = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    let mut items: Vec<*mut Object> = vec![];
    for i in 0..n {
        let c: *mut Object = msg_send![subs, objectAtIndex: i];
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
            let editable: BOOL = msg_send![c, isEditable];
            if editable == YES {
                continue; // inputs scroll their text
            }
            let f: NSRect = msg_send![c, frame];
            let cell: *mut Object = msg_send![c, cell];
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
unsafe fn collect_text_windowrects(v: *mut Object, out: &mut Vec<(*mut Object, NSRect)>) {
    if !visible(v) {
        return;
    }
    if is_text_item(v) && !is_hit_target(v) {
        let txt = if is_kind(v, class!(NSButton)) { to_str(msg_send![v, title]) } else { to_str(msg_send![v, stringValue]) };
        let vr: NSRect = msg_send![v, visibleRect];
        if !txt.is_empty() && vr.size.w > 1.0 && vr.size.h > 1.0 {
            let bb: NSRect = msg_send![v, bounds];
            let r: NSRect = msg_send![v, convertRect: bb toView: nil()];
            out.push((v, r));
        }
    }
    if is_kind(v, class!(NSControl)) {
        return; // a control's internal subviews (e.g. NSButtonTextField) are part of it
    }
    let subs: *mut Object = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut Object = msg_send![subs, objectAtIndex: i];
        collect_text_windowrects(c, out);
    }
}
unsafe fn audit_global_overlap(win: *mut Object, issues: &mut Vec<String>) {
    let content: *mut Object = msg_send![win, contentView];
    let mut items = vec![];
    collect_text_windowrects(content, &mut items);
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let ((va, a), (vb, b)) = (items[i], items[j]);
            let anc_a: BOOL = msg_send![va, isDescendantOf: vb];
            let anc_b: BOOL = msg_send![vb, isDescendantOf: va];
            let sa: *mut Object = msg_send![va, superview];
            let sb: *mut Object = msg_send![vb, superview];
            if anc_a == YES || anc_b == YES || sa == sb {
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
unsafe fn audit_traffic(win: *mut Object, issues: &mut Vec<String>) {
    let content: *mut Object = msg_send![win, contentView];
    for kind in 0..3isize {
        let b: *mut Object = msg_send![win, standardWindowButton: kind];
        if b.is_null() || !visible(b) {
            continue;
        }
        let bb: NSRect = msg_send![b, bounds];
        let br: NSRect = msg_send![b, convertRect: bb toView: nil()];
        walk_text_windowrect(content, br, issues);
    }
}
unsafe fn walk_text_windowrect(v: *mut Object, br: NSRect, issues: &mut Vec<String>) {
    if !visible(v) {
        return;
    }
    if is_kind(v, class!(NSTextField)) && !to_str(msg_send![v, stringValue]).is_empty() {
        let bb: NSRect = msg_send![v, bounds];
        let r: NSRect = msg_send![v, convertRect: bb toView: nil()];
        let ox = (r.origin.x + r.size.w).min(br.origin.x + br.size.w) - r.origin.x.max(br.origin.x);
        let oy = (r.origin.y + r.size.h).min(br.origin.y + br.size.h) - r.origin.y.max(br.origin.y);
        if ox > 1.0 && oy > 1.0 {
            issues.push(format!("under traffic lights: {}", describe(v)));
        }
    }
    let subs: *mut Object = msg_send![v, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let c: *mut Object = msg_send![subs, objectAtIndex: i];
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
        let wins: *mut Object = msg_send![ui.app.get(), windows];
        let n: usize = msg_send![wins, count];
        let screens: *mut Object = msg_send![class!(NSScreen), screens];
        let sn: usize = msg_send![screens, count];
        let mut bad = 0;
        for i in 0..n {
            let w: *mut Object = msg_send![wins, objectAtIndex: i];
            let vis: BOOL = msg_send![w, isVisible];
            if vis == NO {
                continue;
            }
            let f: NSRect = msg_send![w, frame];
            for j in 0..sn {
                let s: *mut Object = msg_send![screens, objectAtIndex: j];
                let sf: NSRect = msg_send![s, frame];
                let ox = (f.origin.x + f.size.w).min(sf.origin.x + sf.size.w) - f.origin.x.max(sf.origin.x);
                let oy = (f.origin.y + f.size.h).min(sf.origin.y + sf.size.h) - f.origin.y.max(sf.origin.y);
                if ox > 0.0 && oy > 0.0 {
                    let cls: *mut Object = msg_send![w, className];
                    log(&format!("  window on screen: {} {} {:?}", to_str(cls), to_str(msg_send![w, title]), f));
                    bad += 1;
                }
            }
        }
        state_check(&format!("all visible windows are off-screen ({})", tag), bad == 0);
    }
}
/// With JELLYFAST_SHOTS=<dir>, capture the window with screencapture (own-window capture).
fn shot(tag: &str, idx: usize, win: *mut Object) {
    let dir = match std::env::var("JELLYFAST_SHOTS") {
        Ok(d) if !d.is_empty() => d,
        _ => return,
    };
    unsafe {
        let appearance = std::env::var("JELLYFAST_APPEARANCE").unwrap_or_else(|_| "system".into());
        let clean: String = tag.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        let path = format!("{}/{}-{}-w{}.png", dir, appearance, clean, idx);
        // Render the view tree straight to a bitmap: works for off-screen windows, unlike screencapture.
        // The frame view (contentView's superview) includes the titlebar and traffic lights.
        let content: *mut Object = msg_send![win, contentView];
        let root: *mut Object = msg_send![content, superview];
        let root = if root.is_null() { content } else { root };
        let _: () = msg_send![root, layoutSubtreeIfNeeded];
        let b: NSRect = msg_send![root, bounds];
        let rep: *mut Object = msg_send![root, bitmapImageRepForCachingDisplayInRect: b];
        if rep.is_null() {
            return;
        }
        let _: () = msg_send![root, cacheDisplayInRect: b toBitmapImageRep: rep];
        let props: *mut Object = msg_send![class!(NSDictionary), dictionary];
        let png: *mut Object = msg_send![rep, representationUsingType: 4usize properties: props]; // NSBitmapImageFileTypePNG
        let _: BOOL = msg_send![png, writeToFile: ns(&path) atomically: YES];
        log(&format!("  shot {}", path));
    }
}
/// Fails the run (exit 2) if any visible window has layout problems.
fn audit(tag: &str) {
    check_offscreen(tag);
    unsafe {
        let ui = UI.get().unwrap();
        let wins: *mut Object = msg_send![ui.app.get(), windows];
        let n: usize = msg_send![wins, count];
        let mut issues = vec![];
        for i in 0..n {
            let w: *mut Object = msg_send![wins, objectAtIndex: i];
            let vis: BOOL = msg_send![w, isVisible];
            if vis == NO {
                continue;
            }
            let _: () = msg_send![w, layoutIfNeeded];
            let content: *mut Object = msg_send![w, contentView];
            let before = issues.len();
            audit_view(content, &mut issues);
            audit_global_overlap(w, &mut issues);
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
        state_check("auto-selected first library", ui.mode_stack.len() == 1 && ui.mode_stack[0].0 == "v1");
        state_check("selected_view set", ui.selected_view == Some(0));
        state_check("window is opaque", { let o: BOOL = msg_send![ui.win.get(), isOpaque]; o == YES });
        state_check("library canvas draws background", { let o: BOOL = msg_send![ui.lib_scroll.get(), drawsBackground]; o == YES });
        state_check("login backdrop hidden once connected", { let o: BOOL = msg_send![ui.bgfx.get(), isHidden]; o == YES });
        state_check("auto-select sent LoadView", take_cmds().iter().any(|c| matches!(c, UiCmd::LoadView { view_id, .. } if view_id == "v1")));
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
        let mb: BOOL = msg_send![c.art, masksToBounds];
        state_check("art layer radius + clipping", r == R_POSTER && mb == YES);
        let so: f32 = msg_send![c.shadow, shadowOpacity];
        state_check("shadow host at rest opacity 0.18", (so - 0.18).abs() < 0.001);
        let host_clips: BOOL = msg_send![c.shadow, masksToBounds];
        state_check("shadow host does not clip", host_clips == NO);
    }
}
fn s_hover_model() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let c = &ui.grid_data.as_ref().unwrap().cells[0];
        let _: *mut Object = msg_send![c.container, mouseEntered: nil()];
        let so: f32 = msg_send![c.shadow, shadowOpacity];
        state_check("hover raises shadow to 0.32", (so - 0.32).abs() < 0.001);
        let _: *mut Object = msg_send![c.container, mouseExited: nil()];
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
        let g0: BOOL = msg_send![d.cells[0].glyph, isHidden];
        let g1: BOOL = msg_send![d.cells[1].glyph, isHidden];
        let g2: BOOL = msg_send![d.cells[2].glyph, isHidden];
        state_check("garbage bytes keep the placeholder glyph", g0 == NO);
        state_check("valid poster hides the glyph", g1 == YES);
        state_check("untouched cell keeps its glyph", g2 == NO);
    }
    send_items("Movies", items_of("Movie", 12), false);
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
fn selected_rows() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.rows
            .iter()
            .filter(|r| {
                let rb: *mut Object = msg_send![r.0, subviews];
                let bg: *mut Object = msg_send![rb, objectAtIndex: 0usize];
                let t: BOOL = msg_send![bg, isTransparent];
                t == NO
            })
            .count()
    }
}
fn s_sidebar_click() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let rows: Vec<*mut Object> = ui.rows.iter().map(|r| r.0).collect();
        state_check("5 library rows (livetv already present)", rows.len() == 5);
        // arranged: search field, "Library" header, then JRSidebarRow rows
        let arr: *mut Object = msg_send![ui.side_stack.get(), arrangedSubviews];
        let third: *mut Object = msg_send![arr, objectAtIndex: 2usize];
        let cls = objc::runtime::Class::get("JRSidebarRow").unwrap();
        let k: BOOL = msg_send![third, isKindOfClass: cls];
        state_check("first library row is a JRSidebarRow", k == YES);
        let sf = frame_of(ui.side_stack.get());
        let sb = frame_of(ui.sidebar.get());
        state_check(
            "sidebar stack leaves the title bar band free",
            sf.origin.y + sf.size.h <= sb.size.h - crate::ui::theme::TITLEBAR_H + 0.5,
        );
        state_check("one row selected after auto-select", selected_rows() == 1);
        for (i, _) in rows.iter().enumerate() {
            act(sel!(viewSelected:), tagged_button(i as isize));
            state_check("exactly one row selected after click", selected_rows() == 1);
            state_check("clicked row is the selected one", UI.get().unwrap().selected_view == Some(i));
        }
        let n_before: usize = { let a: *mut Object = msg_send![ui.side_stack.get(), arrangedSubviews]; msg_send![a, count] };
        handle_main(MainMsg::Connected {
            user_name: "tester".into(),
            views: ui.views.clone(),
            has_live_tv: true,
        });
        let n_after: usize = { let a: *mut Object = msg_send![ui.side_stack.get(), arrangedSubviews]; msg_send![a, count] };
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
        let h: BOOL = msg_send![ui.status_label.get(), isHidden];
        state_check("status label visible", h == NO);
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
        let bh: BOOL = msg_send![ui.lib_back.get(), isHidden];
        state_check("library back visible when can_go_back", bh == NO);
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
        let sh: BOOL = msg_send![ui.spinner.get(), isHidden];
        state_check("no header spinner during skeleton", sh == YES);
        take_cmds();
        // error with nothing on screen
        handle_main(MainMsg::PlaybackError("HTTP 500 from the server, which is a rather long message that goes on and on and on".into()));
        let hh: BOOL = msg_send![ui.state_host.get(), isHidden];
        state_check("error state shown", hh == NO);
        let bb: BOOL = msg_send![ui.state_btn.get(), isHidden];
        state_check("Try Again visible on error", bb == NO);
        // empty search state, centred on the grid area
        set_text(ui.search.get(), "zzz");
        send_items("Results", vec![], false);
        let _: () = msg_send![ui.win.get(), layoutIfNeeded];
        let hb: BOOL = msg_send![ui.state_btn.get(), isHidden];
        state_check("no Try Again on empty", hb == YES);
        state_check("empty search body", read_field(ui.state_body.get()).contains("No results for"));
        let sf = frame_of(ui.status_label.get());
        let win_r: NSRect = msg_send![ui.status_label.get(), convertRect: bounds_of(ui.status_label.get()) toView: nil()];
        let content: *mut Object = msg_send![ui.win.get(), contentView];
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
        let sh: BOOL = msg_send![ui.spinner.get(), isHidden];
        state_check("header spinner while refreshing", sh == NO);
        send_items("Movies", items_of("Movie", 12), false);
        let sh: BOOL = msg_send![ui.spinner.get(), isHidden];
        state_check("spinner stops on arrival", sh == YES);
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
            let note: *mut Object = msg_send![class!(NSNotification), notificationWithName: ns("NSControlTextDidChangeNotification") object: ui.search.get()];
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
        cmds.iter().any(|c| matches!(c, UiCmd::Search { q } if q == "abc")),
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
            audit("settings sheet");
            let ui = UI.get_mut().unwrap();
            {
                let ctl_x: Vec<f64> = [ui.s_url.get(), ui.s_quality.get(), ui.s_sub_lang.get(), ui.s_audio_lang.get(), ui.s_theme.get(), ui.seerr.s_url.get(), ui.seerr.s_mode.get()]
                    .iter()
                    .map(|&v| { let f: NSRect = msg_send![v, frame]; f.origin.x })
                    .collect();
                state_check("settings controls share x = 186", ctl_x.iter().all(|&x| (x - 186.0).abs() < 0.5));
                let c: *mut Object = msg_send![ui.settings_win.get(), contentView];
                let subs: *mut Object = msg_send![c, subviews];
                let n: usize = msg_send![subs, count];
                let mut esc = false;
                for i in 0..n {
                    let v: *mut Object = msg_send![subs, objectAtIndex: i];
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
        let ap: *mut Object = msg_send![ui.player_host.get(), effectiveAppearance];
        let name: *mut Object = msg_send![ap, name];
        state_check("player chrome is dark", to_str(name) == "NSAppearanceNameDarkAqua");
        let cs: isize = msg_send![ui.avview.get(), controlsStyle];
        state_check("AVKit controls are floating", cs == 2);
        state_check("toast hidden when empty", !player::toast_visible());
        let ip: BOOL = msg_send![ui.info_panel.get(), isHidden];
        state_check("playback info hidden by default", ip == YES);
        let sel: objc::runtime::Sel = msg_send![ui.back_btn.get(), action];
        state_check("back button stops playback", sel == sel!(stopPlayback:));
        let a: f64 = msg_send![ui.top_bar.get(), alphaValue];
        state_check("top bar visible at start", a > 0.99);
        state_check("backdrop gone once playing", !player::backdrop_visible());
        // info toggle shows the panel and persists
        act(sel!(togglePlaybackInfo:), nil());
        let ip: BOOL = msg_send![ui.info_panel.get(), isHidden];
        state_check("info panel shown after toggle", ip == NO && ui.cfg.settings.show_stats);
        update_chip();
        state_check("info text filled", read_field(ui.info_label.get()).contains("/"));
        act(sel!(togglePlaybackInfo:), nil());
        let ip: BOOL = msg_send![ui.info_panel.get(), isHidden];
        state_check("info panel hidden after second toggle", ip == YES && !ui.cfg.settings.show_stats);
    }
}
fn s_toast_hidden() {
    unsafe {
        state_check("toast hidden 2.2 s after subCycle", !crate::ui::player::toast_visible());
    }
}
fn s_audit_player() {
    handle_main(MainMsg::PlaybackReady(PlaybackStart {
        url: video_url(),
        item_id: "movie-1".into(),
        title: LONG.into(),
        subtitle: Some(format!("{} · {}", "Some Show With A Long Name", LONG)),
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: None,
        media_kind: MediaKind::Video,
    }));
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.show_stats = true;
        crate::ui::player::apply_info_visibility();
        crate::ui::player::toast(&"Can't reach your server. Press ⌘R to retry. ".repeat(4), Some("wifi.exclamationmark"), true);
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
        url: video_url(), // the video asset has an audio track; the Audio kind never shows the picture
        item_id: "track-1".into(),
        title: title.into(),
        subtitle: Some(artist.into()),
        resume_secs: 0.0,
        method: PlaybackMethod::DirectPlay,
        run_time_secs: 6.0,
        image_key: Some(poster_key("Audio-0")),
        media_kind: MediaKind::Audio,
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
        let ph: BOOL = msg_send![ui.player_host.get(), isHidden];
        state_check("audio: player_host stays hidden", ph == YES);
        let ch: BOOL = msg_send![ui.content_host.get(), isHidden];
        state_check("audio: content_host stays visible", ch == NO);
        let bh: BOOL = msg_send![ui.now.bar.get(), isHidden];
        state_check("audio: bar visible", bh == NO && crate::ui::nowbar::visible());
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
        let bh: BOOL = msg_send![ui.now.bar.get(), isHidden];
        state_check("audio: bar hidden after stop", bh == YES && !crate::ui::nowbar::visible());
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
        let c: *mut Object = msg_send![ui.login_err.get(), textColor];
        let sec: *mut Object = msg_send![class!(NSColor), secondaryLabelColor];
        let eq: BOOL = msg_send![c, isEqual: sec];
        state_check("signed-out message is neutral (secondary colour)", eq == YES);
        let t: *mut Object = msg_send![ui.win.get(), title];
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
        let _ = ui;
        crate::platform::appkit::settings_open()
    }
}
/// Direct subviews of `row` that are an instance of `cls` and satisfy `pred`.
unsafe fn seerr_subviews(row: *mut Object, cls: &str, pred: &dyn Fn(*mut Object) -> bool) -> usize {
    let c = objc::runtime::objc_getClass(format!("{}\0", cls).as_ptr() as *const std::os::raw::c_char) as *mut Object;
    let subs: *mut Object = msg_send![row, subviews];
    let n: usize = msg_send![subs, count];
    let mut k = 0;
    for i in 0..n {
        let v: *mut Object = msg_send![subs, objectAtIndex: i];
        let is: BOOL = msg_send![v, isKindOfClass: c];
        if is == YES && pred(v) {
            k += 1;
        }
    }
    k
}
/// Number of rows in the Seerr list that carry a Request button.
fn seerr_button_rows() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr.rows.iter().filter(|r| seerr_subviews(**r, "NSButton", &|_| true) > 0).count()
    }
}
/// Status pills (NSBox with corner radius 10) across all rows.
fn seerr_pills() -> usize {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr
            .rows
            .iter()
            .map(|r| {
                seerr_subviews(*r, "NSBox", &|b| {
                    let rad: f64 = msg_send![b, cornerRadius];
                    (rad - 10.).abs() < 0.01
                })
            })
            .sum()
    }
}
fn seerr_row_heights_ok() -> bool {
    unsafe {
        let ui = UI.get().unwrap();
        ui.seerr.rows.iter().all(|r| (frame_of(*r).size.h - 76.).abs() < 0.01)
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
        check_offscreen("seerr window");
        state_check("hint hidden when configured", seerr_hidden(ui.seerr.hint.get()));
        state_check("search visible", !seerr_hidden(ui.seerr.search.get()));
        state_check("no rows", ui.seerr.rows.is_empty());
        state_check(
            "hint state: search prompt",
            seerr_empty_shown().map(|(_, b)| b == "Search for a movie or show to request.") == Some(true),
        );
    }
}
fn s_seerr_results() {
    handle_main(MainMsg::SeerrResults(seerr_mixed()));
    unsafe {
        let ui = UI.get().unwrap();
        state_check("7 rows", ui.seerr.rows.len() == 7);
        state_check("2 request buttons (NotRequested only)", seerr_button_rows() == 2);
        state_check("rows are 76 tall", seerr_row_heights_ok());
        state_check("a pill per row without a Request button", seerr_pills() == 7 - seerr_button_rows());
        state_check("empty state hidden with rows", seerr_empty_shown().is_none());
        state_check("thumbnail waits for the poster", unsafe { crate::ui::seerr::thumb_has_image("/p11.jpg") } == Some(false));
    }
    // The worker would fetch these through the Seerr server; fake its reply (no network).
    handle_main(MainMsg::SeerrPoster { path: "/p11.jpg".into(), bytes: TEST_PNG.to_vec() });
    handle_main(MainMsg::SeerrPoster { path: "/bad.jpg".into(), bytes: vec![1, 2, 3] });
    unsafe {
        state_check("thumbnail image set after poster reply", crate::ui::seerr::thumb_has_image("/p11.jpg") == Some(true));
        state_check("other thumbnail still a placeholder", crate::ui::seerr::thumb_has_image("/p22.jpg") == Some(false));
    }
    audit("seerr results");
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
        state_check("loading: no rows, no empty state", ui.seerr.rows.is_empty() && seerr_empty_shown().is_none());
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
        state_check("a pill per request row", seerr_pills() == 6);
        audit("seerr my requests");
        handle_main(MainMsg::SeerrRequestList(vec![]));
        state_check(
            "empty requests state",
            ui.seerr.rows.is_empty()
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
        state_check("back on search tab", !ui.seerr.requests_mode && ui.seerr.rows.len() == 7);
    }
}
fn s_seerr_resize() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        for (w, h) in [(520.0, 420.0), (900.0, 800.0), (680.0, 680.0)] {
            let _: () = msg_send![ui.seerr.win.get(), setContentSize: size(w, h)];
            let _: () = msg_send![ui.seerr.win.get(), displayIfNeeded];
            let _: () = msg_send![ui.seerr.doc.get(), layoutSubtreeIfNeeded];
            handle_main(MainMsg::SeerrResults(seerr_mixed())); // rebuild at the new width
            audit(&format!("seerr @{}x{}", w, h));
        }
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
        state_check(
            "no results state",
            ui.seerr.rows.is_empty()
                && seerr_empty_shown() == Some(("No results".into(), "No matches for “zzzz”.".into())),
        );
        set_text(ui.seerr.search.get(), "");
        handle_main(MainMsg::SeerrResults(vec![]));
        state_check("blank search state", ui.seerr.rows.is_empty() && seerr_empty_shown().map(|t| t.0) == Some("Find a title".into()));
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
