//! Casting. Two ways to send video to a TV:
//! - AirPlay: AVKit's route picker sits in the player bar (`playerbar::route_button`).
//! - Play On: start the item on another Jellyfin device (a session the user controls).
//!   The cast menu lists both. Choosing a Jellyfin device stops local playback and
//!   shows a small remote bar (title, play/pause, stop, scrub) that polls the session.
#![allow(unsafe_code)]
use crate::app::messages::UiCmd;
use crate::jellyfin::sessions::{RemoteAction, RemoteSession};
use crate::jellyfin::urls::fmt_clock;
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};
use std::time::{Duration, Instant};

const BAR_W: f64 = 640.;
const BAR_H: f64 = 60.;
/// The remote bar re-reads the session this often while it is up.
const POLL_EVERY: Duration = Duration::from_millis(2000);

/// What a cast sends: a movie or episode, and where to start it.
#[derive(Clone, Debug, PartialEq)]
pub struct CastItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub start_secs: f64,
    pub run_secs: f64,
}

/// The session the remote bar controls, as last reported by the server.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub session_id: String,
    pub device: String,
    /// `None`: nothing is playing on the device.
    pub title: Option<String>,
    pub paused: bool,
    pub pos_secs: f64,
    pub run_secs: f64,
    /// False once the session is missing from the server's list.
    pub online: bool,
}

pub struct CastUi {
    pub bar: Ptr,
    pub title: Ptr,
    pub device: Ptr,
    pub play: Ptr,
    pub stop: Ptr,
    pub scrub: Ptr,
    pub time: Ptr,
    pub close: Ptr,
    /// The cast menu is waiting for the session list; it opens under `anchor` at `at`.
    pending: bool,
    anchor: Ptr,
    at: NSPoint,
    /// The item the open cast menu would send.
    pub item: Option<CastItem>,
    /// Sessions behind the open menu's rows (a row's tag is its index here).
    menu: Vec<RemoteSession>,
    /// Titles of the last menu built, and its item count. Menus are not shown in self-test mode, so tests read these.
    pub menu_titles: Vec<String>,
    pub menu_items: usize,
    pub remote: Option<Remote>,
    last_poll: Option<Instant>,
    /// Self-test only: presses seen by the AirPlay row while off-screen.
    pub airplay_presses: usize,
}

impl CastUi {
    pub fn new() -> CastUi {
        let n = || Ptr(std::ptr::null_mut());
        CastUi {
            bar: n(),
            title: n(),
            device: n(),
            play: n(),
            stop: n(),
            scrub: n(),
            time: n(),
            close: n(),
            pending: false,
            anchor: n(),
            at: NSPoint { x: 0., y: 0. },
            item: None,
            menu: Vec::new(),
            menu_titles: Vec::new(),
            menu_items: 0,
            remote: None,
            last_poll: None,
            airplay_presses: 0,
        }
    }
}

/// Seconds from Jellyfin ticks, for the remote bar's clock and scrubber.
fn secs(ticks: i64) -> f64 {
    crate::jellyfin::urls::secs_from_ticks(ticks)
}

// ---- the cast menu ------------------------------------------------------------------------------

/// Opens the cast menu for `item` under `anchor`, after the session list arrives.
pub(crate) unsafe fn open_menu(anchor: *mut Object, item: CastItem) {
    let Some(ui) = UI.get_mut() else { return };
    let h = if anchor.is_null() { 0. } else { bounds_of(anchor).size.h };
    ui.cast.anchor = Ptr(anchor);
    ui.cast.at = NSPoint { x: 0., y: h };
    ui.cast.item = Some(item);
    ui.cast.pending = true;
    let _ = ui.ui_tx.send(UiCmd::LoadSessions { silent: false });
}

/// Shortcut (⇧⌘C) and the player bar's cast button. Playing video casts what is playing;
/// otherwise the open movie or episode; otherwise there is nothing to cast.
pub(crate) extern "C" fn show_menu(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if let Some(pn) = ui.playing.as_ref() {
            let now: crate::platform::appkit::CMTime = msg_send![ui.player.get(), currentTime];
            let subtitle = read_field(ui.top_sub.get());
            let item = CastItem {
                id: pn.item_id.clone(),
                title: pn.title.clone(),
                subtitle,
                start_secs: now.seconds().max(0.0),
                run_secs: pn.duration_secs,
            };
            let anchor = if sender.is_null() { ui.pbar.cast.get() } else { sender };
            open_menu(anchor, item);
            return;
        }
        if let Some(d) = ui.detail.item.as_ref() {
            if d.item_type != "Series" && ui.detail.open {
                let item = CastItem {
                    id: d.id.clone(),
                    title: d.name.clone(),
                    subtitle: String::new(),
                    start_secs: secs(d.resume_ticks),
                    run_secs: secs(d.run_time_ticks.unwrap_or(0)),
                };
                open_menu(ui.detail.host.get(), item);
                return;
            }
        }
        crate::ui::shell::set_count("Open a movie or episode, or play one, to cast it.", false);
    }
}

/// Session list arrived: refresh the remote bar, and open the menu if one is waiting.
pub fn on_sessions(list: Vec<RemoteSession>) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if let Some(r) = ui.cast.remote.clone() {
            ui.cast.remote = Some(update_remote(&r, &list));
            apply_remote();
        }
        if !ui.cast.pending {
            return;
        }
        ui.cast.pending = false;
        build_and_present(list);
    }
}

/// The remote bar's view of its session after a server list: found (name, state, clock) or offline.
fn update_remote(r: &Remote, list: &[RemoteSession]) -> Remote {
    match list.iter().find(|s| s.id == r.session_id) {
        Some(s) => Remote {
            session_id: r.session_id.clone(),
            device: s.label(),
            title: s.now_playing.as_ref().map(|n| n.title.clone()),
            paused: s.paused,
            pos_secs: secs(s.position_ticks),
            run_secs: s.now_playing.as_ref().map(|n| secs(n.run_time_ticks)).filter(|v| *v > 0.).unwrap_or(r.run_secs),
            online: true,
        },
        None => Remote { online: false, title: None, ..r.clone() },
    }
}

unsafe fn build_and_present(list: Vec<RemoteSession>) {
    let ui = UI.get_mut().unwrap();
    let menu = new_menu("Cast");
    let mut titles: Vec<String> = Vec::new();
    if crate::ui::player::video_on_screen() {
        let it = menu_item("AirPlay", sel!(castAirPlay:), "");
        add_to_menu(menu, it, None);
        titles.push("AirPlay".into());
        add_to_menu(menu, msg_send![class!(NSMenuItem), separatorItem], None);
    }
    let head: *mut Object = msg_send![class!(NSMenuItem), sectionHeaderWithTitle: ns("Play on")];
    add_to_menu(menu, head, None);
    titles.push("Play on".into());
    if list.is_empty() {
        let it = menu_item("No other Jellyfin devices are active", sel!(castPick:), "");
        let _: () = msg_send![it, setEnabled: NO];
        add_to_menu(menu, it, None);
        titles.push("No other Jellyfin devices are active".into());
    }
    for (i, s) in list.iter().enumerate() {
        let label = s.label();
        let it = menu_item(&label, sel!(castPick:), "");
        add_to_menu(menu, it, Some(i as isize));
        titles.push(label);
    }
    ui.cast.menu = list;
    ui.cast.menu_titles = titles;
    let anchor = if ui.cast.anchor.get().is_null() || !is_on_screen(ui.cast.anchor.get()) {
        ui.win.get()
    } else {
        ui.cast.anchor.get()
    };
    let content: *mut Object = if anchor == ui.win.get() { msg_send![ui.win.get(), contentView] } else { anchor };
    let at = if anchor == ui.win.get() { NSPoint { x: EDGE, y: bounds_of(content).size.h - TITLEBAR_H } } else { ui.cast.at };
    let n: NSInteger = msg_send![menu, numberOfItems];
    ui.cast.menu_items = n as usize;
    if offscreen_mode() {
        return;
    }
    let _: BOOL = msg_send![menu, popUpMenuPositioningItem: nil() atLocation: at inView: content];
}

unsafe fn is_on_screen(v: *mut Object) -> bool {
    let w: *mut Object = msg_send![v, window];
    !w.is_null()
}

unsafe fn new_menu(title: &str) -> *mut Object {
    let m: *mut Object = msg_send![class!(NSMenu), alloc];
    let m: *mut Object = msg_send![m, initWithTitle: ns(title)];
    autorel(m)
}

/// Adds `it` to `menu`. Rows that pick a session get the ctl as target and their index as tag.
unsafe fn add_to_menu(menu: *mut Object, it: *mut Object, tag: Option<isize>) {
    if let Some(t) = tag {
        let _: () = msg_send![it, setTarget: UI.get().unwrap().ctl.get()];
        let _: () = msg_send![it, setTag: t];
    } else if !is_separator(it) {
        let _: () = msg_send![it, setTarget: UI.get().unwrap().ctl.get()];
    }
    let _: () = msg_send![menu, addItem: it];
}

unsafe fn is_separator(it: *mut Object) -> bool {
    let s: BOOL = msg_send![it, isSeparatorItem];
    s == YES
}

/// A session row was chosen: start the item there and show the remote bar.
pub(crate) extern "C" fn pick(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let tag: NSInteger = msg_send![sender, tag];
        let Some(session) = ui.cast.menu.get(tag as usize).cloned() else { return };
        let Some(item) = ui.cast.item.take() else { return };
        // One screen at a time: local playback stops, the TV takes over.
        if ui.playing.is_some() {
            crate::platform::avkit::stop_for_cast();
        }
        let _ = ui.ui_tx.send(UiCmd::CastPlay {
            session_id: session.id.clone(),
            item_id: item.id.clone(),
            start_secs: item.start_secs,
        });
        ui.cast.remote = Some(Remote {
            session_id: session.id.clone(),
            device: session.label(),
            title: Some(item.title.clone()),
            paused: false,
            pos_secs: item.start_secs,
            run_secs: item.run_secs,
            online: true,
        });
        // The first refresh comes soon after the start command, so the title is right.
        ui.cast.last_poll = Some(Instant::now().checked_sub(POLL_EVERY * 3 / 4).unwrap_or_else(Instant::now));
        apply_remote();
    }
}

/// AirPlay row: press AVKit's route button in the player bar so its picker opens.
pub(crate) extern "C" fn airplay_pick(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if offscreen_mode() {
            ui.cast.airplay_presses += 1;
            return;
        }
        let btn = find_button(ui.pbar.route.get(), 0);
        if btn.is_null() {
            crate::ui::player::toast("Use the AirPlay button in the player bar.", Some("airplayvideo"), false);
        } else {
            let _: () = msg_send![btn, performClick: nil()];
        }
    }
}

/// The first NSButton inside AVKit's route picker (its own control).
unsafe fn find_button(v: *mut Object, depth: u32) -> *mut Object {
    if v.is_null() || depth > 4 {
        return nil();
    }
    let is: BOOL = msg_send![v, isKindOfClass: class!(NSButton)];
    if is == YES {
        return v;
    }
    let subs: *mut Object = msg_send![v, subviews];
    let n: NSUInteger = msg_send![subs, count];
    for i in 0..n {
        let s: *mut Object = msg_send![subs, objectAtIndex: i];
        let b = find_button(s, depth + 1);
        if !b.is_null() {
            return b;
        }
    }
    nil()
}

// ---- the remote bar -------------------------------------------------------------------------------

/// Builds the remote bar on top of the window content (hidden until a cast starts).
pub(crate) unsafe fn build(content: *mut Object, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let w = BAR_W.min(b.size.w - 2. * EDGE);
    let bar = vev(MAT_HUD, BLEND_WITHIN);
    let _: () = msg_send![bar, setFrame: rect((b.size.w - w) / 2., 16., w, BAR_H)];
    set_autoresize(bar, MARGIN_MINX | MARGIN_MAXX | MARGIN_MAXY);
    let _: () = msg_send![bar, setAppearance: crate::ui::player::dark_appearance()];
    set_corner(layer_of(bar), BAR_H / 2.);
    let _: () = msg_send![layer_of(bar), setMasksToBounds: YES];
    set_hidden(bar, true);
    add_sub(content, bar);
    ui.cast.bar = Ptr(bar);

    let title = label("", T_BODY_EM, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(title, rect(SP4, 30., 212., 18.));
    add_sub(bar, title);
    ui.cast.title = Ptr(title);
    let device = label("", T_CAPTION, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(device, rect(SP4, 12., 212., 14.));
    add_sub(bar, device);
    ui.cast.device = Ptr(device);

    let play = crate::ui::playerbar::ctl_button(bar, "play.fill", 18., "Play / Pause", sel!(jrCastPlayPause:));
    set_frame(play, rect(236., 14., 32., 32.));
    ui.cast.play = Ptr(play);
    let stop = crate::ui::playerbar::ctl_button(bar, "stop.fill", 14., "Stop on the device", sel!(jrCastStop:));
    set_frame(stop, rect(272., 14., 32., 32.));
    ui.cast.stop = Ptr(stop);

    let s: *mut Object = msg_send![class!(NSSlider), alloc];
    let s: *mut Object = msg_send![s, initWithFrame: rect(316., 25., 170., 16.)];
    let _: () = msg_send![s, setMinValue: 0.0f64];
    let _: () = msg_send![s, setMaxValue: 1.0f64];
    let _: () = msg_send![s, setContinuous: NO];
    let _: () = msg_send![s, setTarget: ui.ctl.get()];
    let _: () = msg_send![s, setAction: sel!(jrCastScrub:)];
    let _: () = msg_send![s, setToolTip: ns("Seek on the device")];
    autorel(s);
    add_sub(bar, s);
    ui.cast.scrub = Ptr(s);

    let time = label("", T_MONO, C_ON_MEDIA2, ALIGN_RIGHT);
    set_frame(time, rect(490., 21., 84., 16.));
    add_sub(bar, time);
    ui.cast.time = Ptr(time);

    let close = crate::ui::playerbar::ctl_button(bar, "xmark", 11., "Hide", sel!(jrCastClose:));
    set_frame(close, rect(w - 40., 18., 24., 24.));
    ui.cast.close = Ptr(close);
}

/// Pushes `ui.cast.remote` into the bar: title, device, play/pause icon, scrubber and clock.
unsafe fn apply_remote() {
    let ui = UI.get_mut().unwrap();
    let Some(r) = ui.cast.remote.clone() else {
        set_hidden(ui.cast.bar.get(), true);
        return;
    };
    set_hidden(ui.cast.bar.get(), false);
    let title = match (&r.title, r.online) {
        (Some(t), _) => t.clone(),
        (None, true) => "Nothing playing".to_string(),
        (None, false) => "Not available right now".to_string(),
    };
    set_text(ui.cast.title.get(), &title);
    set_text(ui.cast.device.get(), &r.device);
    let playing = r.title.is_some() && !r.paused;
    let img = symbol(if playing { "pause.fill" } else { "play.fill" }, 18., Weight::Medium);
    if !img.is_null() {
        let _: () = msg_send![ui.cast.play.get(), setImage: img];
    }
    let _: () = msg_send![ui.cast.play.get(), setEnabled: if r.title.is_some() && r.online { YES } else { NO }];
    let seekable = r.title.is_some() && r.run_secs > 0. && r.online;
    let _: () = msg_send![ui.cast.scrub.get(), setEnabled: if seekable { YES } else { NO }];
    let frac = if r.run_secs > 0. { (r.pos_secs / r.run_secs).clamp(0., 1.) } else { 0. };
    let _: () = msg_send![ui.cast.scrub.get(), setDoubleValue: frac];
    let clock = if r.title.is_some() {
        format!("{} / {}", fmt_clock(r.pos_secs), fmt_clock(r.run_secs))
    } else {
        String::new()
    };
    set_text(ui.cast.time.get(), &clock);
}

pub(crate) extern "C" fn play_pause(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(r) = ui.cast.remote.as_mut() else { return };
        if r.title.is_none() {
            return;
        }
        let action = if r.paused { RemoteAction::Unpause } else { RemoteAction::Pause };
        r.paused = !r.paused;
        let _ = ui.ui_tx.send(UiCmd::CastControl { session_id: r.session_id.clone(), action });
        apply_remote();
    }
}

pub(crate) extern "C" fn stop(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(r) = ui.cast.remote.as_mut() else { return };
        let _ = ui.ui_tx.send(UiCmd::CastControl { session_id: r.session_id.clone(), action: RemoteAction::Stop });
        r.title = None;
        r.paused = false;
        r.pos_secs = 0.;
        apply_remote();
    }
}

/// Hides the remote bar. The device keeps playing.
pub(crate) extern "C" fn close(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        ui.cast.remote = None;
        ui.cast.last_poll = None;
        apply_remote();
    }
}

/// Scrubber released: seek the device to that fraction of the item.
pub(crate) extern "C" fn scrub(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let frac: f64 = msg_send![sender, doubleValue];
        let Some(r) = ui.cast.remote.as_mut() else { return };
        if r.title.is_none() || r.run_secs <= 0. {
            return;
        }
        let to = frac.clamp(0., 1.) * r.run_secs;
        r.pos_secs = to;
        let _ = ui.ui_tx.send(UiCmd::CastControl { session_id: r.session_id.clone(), action: RemoteAction::SeekSecs(to) });
        apply_remote();
    }
}

/// Called from the playback tick (about once a second): polls the session while the bar is up.
pub(crate) unsafe fn poll_tick() {
    let Some(ui) = UI.get_mut() else { return };
    if ui.cast.remote.is_none() {
        return;
    }
    let due = ui.cast.last_poll.map(|t| t.elapsed() >= POLL_EVERY).unwrap_or(true);
    if due {
        ui.cast.last_poll = Some(Instant::now());
        let _ = ui.ui_tx.send(UiCmd::LoadSessions { silent: true });
    }
}

/// A cast request or remote command failed. The menu is not left waiting.
pub fn on_error(message: String) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        ui.cast.pending = false;
        if crate::ui::player::video_on_screen() {
            crate::ui::player::toast(&message, Some("exclamationmark.triangle"), true);
        } else {
            crate::ui::shell::set_count(&crate::ui::shell::clip_msg(&message, 80), true);
        }
    }
}

// ---- self-test hooks -------------------------------------------------------------------------------

/// Whether the remote bar is shown.
pub fn remote_visible() -> bool {
    unsafe { UI.get().map(|ui| !crate::ui::shell::is_hidden(ui.cast.bar.get())).unwrap_or(false) }
}

/// The remote bar's view, for layout audits.
pub fn remote_bar_view() -> *mut Object {
    UI.get().map(|ui| ui.cast.bar.get()).unwrap_or(std::ptr::null_mut())
}

/// The remote bar's current state (self-test reads it).
pub fn remote_state() -> Option<Remote> {
    UI.get().and_then(|ui| ui.cast.remote.clone())
}

/// Titles of the last cast menu built, and how many items it had.
pub fn menu_titles() -> (Vec<String>, usize) {
    UI.get().map(|ui| (ui.cast.menu_titles.clone(), ui.cast.menu_items)).unwrap_or_default()
}

/// Chooses row `i` of the menu as a press would (self-test). Returns the session it picks.
pub fn choose_row_for_test(i: usize) -> Option<RemoteSession> {
    unsafe {
        let ui = UI.get_mut()?;
        let s = ui.cast.menu.get(i).cloned()?;
        let mi: *mut Object = msg_send![class!(NSMenuItem), new];
        let _: () = msg_send![mi, setTag: i as isize];
        pick(&mut *ui.ctl.get(), sel!(castPick:), mi);
        let _: () = msg_send![mi, release];
        Some(s)
    }
}
