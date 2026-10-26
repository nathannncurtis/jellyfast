//! Custom transport bar for video playback. AVKit's own controls are switched off
//! (`controlsStyle` none), so this bar is the whole transport. Bottom to top it has a
//! gradient wash, a scrubber row (elapsed | track | remaining) with a hover time tooltip,
//! and a button row: play/pause, back 10 s, forward 10 s on the left; mute and volume,
//! subtitles, audio, speed, Picture in Picture and full screen on the right.
//! It fades with the top bar after the idle timeout (see `player::hide_bars_if_idle`).
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::platform::appkit::*;
use crate::ui::theme::*;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static BAR_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
static SCRUB_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
static PIP_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
static TRANSPORT_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
/// Set when the app itself asks AVKit to stop PiP, so the resulting stop callback is not read as the user closing it.
static PIP_APP_STOP: AtomicBool = AtomicBool::new(false);
/// Set when the user presses PiP's restore button, which ends PiP without stopping playback.
static PIP_RESTORING: AtomicBool = AtomicBool::new(false);

/// Secondary button hit size, and the larger play/pause target.
const BTN: f64 = 40.;
/// Scrubber row: elapsed label | track | remaining label.
const TIME_W: f64 = 76.;
const SCRUB_Y: f64 = 72.;
const SCRUB_H: f64 = 24.;
/// Vertical centre of the button row.
const CTRL_CY: f64 = 40.;
const TIP_W: f64 = 80.;
const TIP_H: f64 = 22.;
const VOL_W: f64 = 120.;
/// Centred transport over the picture: the big play/pause and the two 10 s skips either side of it.
const CENTER_PLAY: f64 = 88.;
const CENTER_PLAY_SYMBOL: f64 = 72.;
const CENTER_SKIP: f64 = 64.;
const CENTER_SKIP_SYMBOL: f64 = 40.;
const CENTER_GAP: f64 = SP6;
/// Width of the More menu's AirPlay row (the route picker is the menu item's view).
const ROUTE_ROW_W: f64 = 220.;
const ROUTE_ROW_H: f64 = 28.;
/// Speed menu rows (label, tag = rate x 100). Same set as the Controls menu.
const SPEEDS: [(&str, isize); 6] = [("0.5×", 50), ("0.75×", 75), ("1×", 100), ("1.25×", 125), ("1.5×", 150), ("2×", 200)];

pub struct PlayerBar {
    pub bar: Ptr,
    pub scrub: Ptr,
    pub elapsed: Ptr,
    pub remain: Ptr,
    pub tip: Ptr,
    pub tip_label: Ptr,
    pub play: Ptr,
    pub back: Ptr,
    pub fwd: Ptr,
    pub mute: Ptr,
    pub vol: Ptr,
    pub subs: Ptr,
    pub audio: Ptr,
    pub speed: Ptr,
    pub pip: Ptr,
    pub full: Ptr,
    /// AirPlay route picker (AVKit) and the Play On cast menu button.
    pub route: Ptr,
    pub cast: Ptr,
    /// Centred transport (play/pause and the 10 s skips), the More menu, and the episode buttons.
    pub transport: Ptr,
    pub more: Ptr,
    pub episodes: Ptr,
    pub next: Ptr,
    /// Owned Picture in Picture controller for the player layer, created on first use.
    pub pip_ctl: Ptr,
    /// The AVPlayerLayer the controller was built for (not owned; AVPlayerView owns the layer).
    pub pip_layer: Ptr,
    /// Owned delegate for the controller (AVKit keeps only a weak reference to it).
    pub pip_delegate: Ptr,
    /// Whether the PiP button currently shows the exit glyph.
    pip_shows_exit: bool,
    /// Played fraction and buffered-end fraction, both 0..1, as last drawn.
    pub frac: f64,
    pub buffered: f64,
    /// Fraction under the pointer while it is over the track.
    pub hover: Option<f64>,
    pub dragging: bool,
    shows_pause: bool,
    mute_icon: &'static str,
    /// Items in the last menu opened. Menus are not shown in self-test mode, so tests read this.
    pub last_menu_items: usize,
    /// Presses seen by the PiP / full screen actions while off-screen (self-test only).
    pub pip_presses: usize,
    pub full_presses: usize,
}

impl PlayerBar {
    pub fn new() -> PlayerBar {
        let n = || Ptr(std::ptr::null_mut());
        PlayerBar {
            bar: n(),
            scrub: n(),
            elapsed: n(),
            remain: n(),
            tip: n(),
            tip_label: n(),
            play: n(),
            back: n(),
            fwd: n(),
            mute: n(),
            vol: n(),
            subs: n(),
            audio: n(),
            speed: n(),
            pip: n(),
            full: n(),
            route: n(),
            cast: n(),
            transport: n(),
            more: n(),
            episodes: n(),
            next: n(),
            pip_ctl: n(),
            pip_layer: n(),
            pip_delegate: n(),
            pip_shows_exit: false,
            frac: 0.0,
            buffered: 0.0,
            hover: None,
            dragging: false,
            shows_pause: true,
            mute_icon: "",
            last_menu_items: 0,
            pip_presses: 0,
            full_presses: 0,
        }
    }
}

pub(crate) unsafe fn register_playerbar_classes() {
    let mut d = ClassBuilder::new(c"JRPlayerBar", class!(NSView)).unwrap();
    d.add_method(sel!(makeBackingLayer), bar_layer as extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject);
    d.add_method(sel!(layout), bar_layout as extern "C" fn(*mut AnyObject, Sel));
    BAR_CLASS.set(d.register()).ok();

    // Centred transport: draws the vignette and passes clicks through to the video everywhere but its buttons.
    let mut t = ClassBuilder::new(c"JRTransport", class!(NSView)).unwrap();
    t.add_method(sel!(makeBackingLayer), transport_layer as extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject);
    t.add_method(sel!(layout), transport_layout as extern "C" fn(*mut AnyObject, Sel));
    t.add_method(sel!(hitTest:), transport_hit as extern "C" fn(*mut AnyObject, Sel, NSPoint) -> *mut AnyObject);
    TRANSPORT_CLASS.set(t.register()).ok();

    let mut s = ClassBuilder::new(c"JRScrubber", class!(NSView)).unwrap();
    s.add_method(sel!(drawRect:), scrub_draw as extern "C" fn(*mut AnyObject, Sel, NSRect));
    s.add_method(sel!(acceptsFirstMouse:), yes_obj as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool);
    s.add_method(sel!(mouseDown:), scrub_down as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    s.add_method(sel!(mouseDragged:), scrub_drag as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    s.add_method(sel!(mouseUp:), scrub_up as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    s.add_method(sel!(mouseMoved:), scrub_moved as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    s.add_method(sel!(mouseExited:), scrub_exited as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    SCRUB_CLASS.set(s.register()).ok();

    // Picture in Picture delegate: AVKit calls it on the main thread for start, stop, failure and restore.
    let mut p = ClassBuilder::new(c"JRPiPDelegate", class!(NSObject)).unwrap();
    if let Some(proto) = objc2::runtime::AnyProtocol::get(c"AVPictureInPictureControllerDelegate") {
        p.add_protocol(proto);
    }
    p.add_method(
        sel!(pictureInPictureController:restoreUserInterfaceForPictureInPictureStopWithCompletionHandler:),
        pip_restore as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
    );
    p.add_method(
        sel!(pictureInPictureControllerDidStartPictureInPicture:),
        pip_started as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
    );
    p.add_method(
        sel!(pictureInPictureControllerDidStopPictureInPicture:),
        pip_stopped as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
    );
    p.add_method(
        sel!(pictureInPictureController:failedToStartPictureInPictureWithError:),
        pip_failed as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
    );
    PIP_CLASS.set(p.register()).ok();
}

extern "C" fn yes_obj(_t: *mut AnyObject, _c: Sel, _e: *mut AnyObject) -> Bool {
    Bool::YES
}

extern "C" fn bar_layer(_t: *mut AnyObject, _c: Sel) -> *mut AnyObject {
    unsafe {
        // Transparent at the top, darkening toward the bottom edge so white controls read on video.
        let l: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
        let cols = [cg_black(0.0) as *mut AnyObject, cg_black(0.72) as *mut AnyObject];
        let arr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr(), count: 2usize];
        let _: () = msg_send![l, setColors: arr];
        let _: () = msg_send![l, setStartPoint: NSPoint { x: 0.5, y: 1.0 }];
        let _: () = msg_send![l, setEndPoint: NSPoint { x: 0.5, y: 0.0 }];
        l
    }
}

extern "C" fn bar_layout(this: *mut AnyObject, _c: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
        relayout();
    }
}

/// Builds the bar inside the player host, above AVKit's view. Hidden state is set by playback.
pub(crate) unsafe fn build(ph: *mut AnyObject, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let bcls = *BAR_CLASS.get().unwrap();
    let bar: *mut AnyObject = msg_send![bcls, alloc];
    let bar: *mut AnyObject = msg_send![bar, initWithFrame: rect(0., 0., b.size.w, PLAYER_CTRL_H)];
    let _: () = msg_send![bar, setWantsLayer: Bool::YES];
    set_autoresize(bar, RESIZE_W | MARGIN_MAXY);
    add_sub(ph, bar);
    autorel(bar);
    ui.pbar.bar = Ptr(bar);

    // Scrubber: a plain view that draws itself and handles the pointer.
    let scls = *SCRUB_CLASS.get().unwrap();
    let sc: *mut AnyObject = msg_send![scls, alloc];
    let sc: *mut AnyObject = msg_send![sc, initWithFrame: rect(0., 0., 10., SCRUB_H)];
    let ta: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut AnyObject = msg_send![ta,
        initWithRect: rect(0., 0., 0., 0.), options: (0x01usize | 0x02usize | 0x20usize | 0x200usize), owner: sc, userInfo: nil()];
    let _: () = msg_send![sc, addTrackingArea: ta];
    autorel(ta);
    add_sub(bar, sc);
    autorel(sc);
    ui.pbar.scrub = Ptr(sc);

    let el = label("0:00", T_PLAYER_TIME, C_ON_MEDIA, ALIGN_LEFT);
    add_sub(bar, el);
    ui.pbar.elapsed = Ptr(el);
    let rl = label("-0:00", T_PLAYER_TIME, C_ON_MEDIA, ALIGN_RIGHT);
    add_sub(bar, rl);
    ui.pbar.remain = Ptr(rl);

    // Hover tooltip: a small HUD pill with the time under the pointer.
    let chip = vev(MAT_HUD, BLEND_WITHIN);
    let cl = layer_of(chip);
    set_corner(cl, TIP_H / 2.);
    let _: () = msg_send![cl, setMasksToBounds: Bool::YES];
    set_hidden(chip, true);
    add_sub(bar, chip);
    ui.pbar.tip = Ptr(chip);
    let tl = label("", T_BODY_EM, C_ON_MEDIA, ALIGN_CENTER);
    set_frame(tl, rect(0., (TIP_H - 16.) / 2., TIP_W, 16.));
    add_sub(chip, tl);
    ui.pbar.tip_label = Ptr(tl);

    let mute = ctl_button(bar, "speaker.wave.2.fill", 20., "Mute", sel!(mute:));
    ui.pbar.mute = Ptr(mute);
    let s: *mut AnyObject = msg_send![class!(NSSlider), alloc];
    let s: *mut AnyObject = msg_send![s, initWithFrame: rect(0., 0., VOL_W, 16.)];
    let _: () = msg_send![s, setMinValue: 0.0f64];
    let _: () = msg_send![s, setMaxValue: 1.0f64];
    let _: () = msg_send![s, setControlSize: 0usize]; // regular: easier to hit from the couch
    let _: () = msg_send![s, setContinuous: Bool::YES];
    let _: () = msg_send![s, setTarget: ui.ctl.get()];
    let _: () = msg_send![s, setAction: sel!(jrNowVolume:)];
    let _: () = msg_send![s, setToolTip: ns("Volume")];
    autorel(s);
    add_sub(bar, s);
    ui.pbar.vol = Ptr(s);

    // Right cluster, from the left: Next Episode, Episodes, Subtitles, Audio, More, Full screen.
    let next = ctl_button(bar, "forward.end.fill", 20., "Next episode (N)", sel!(nextEpisode:));
    ui.pbar.next = Ptr(next);
    let episodes = ctl_button(bar, "list.bullet.rectangle", 20., "Episodes (E)", sel!(toggleEpisodes:));
    ui.pbar.episodes = Ptr(episodes);
    let subs = ctl_button(bar, "captions.bubble", 20., "Subtitles", sel!(showSubtitleMenu:));
    ui.pbar.subs = Ptr(subs);
    let audio = ctl_button(bar, "music.note.list", 20., "Audio track", sel!(showAudioMenu:));
    ui.pbar.audio = Ptr(audio);
    let more = ctl_button(bar, "ellipsis.circle", 20., "More: speed, AirPlay, Picture in Picture", sel!(showMoreMenu:));
    ui.pbar.more = Ptr(more);
    let full = ctl_button(bar, "arrow.up.left.and.arrow.down.right", 20., "Full screen", sel!(fullscreen:));
    ui.pbar.full = Ptr(full);

    // Speed, Picture in Picture and Cast live in the More menu now. The buttons stay as hidden
    // targets: the PiP button carries the exit glyph, and the Cast menu anchors to a view.
    let speed = ctl_button(bar, "gauge.with.dots.needle.67percent", 20., "Playback speed", sel!(showSpeedMenu:));
    set_hidden(speed, true);
    ui.pbar.speed = Ptr(speed);
    let pip = ctl_button(bar, "pip.enter", 20., "Picture in Picture", sel!(togglePiP:));
    set_hidden(pip, true);
    ui.pbar.pip = Ptr(pip);
    let cast = ctl_button(bar, "tv", 20., "Cast to a device", sel!(showCastMenu:));
    set_hidden(cast, true);
    ui.pbar.cast = Ptr(cast);

    relayout();
}

/// AirPlay route picker: AVKit's own control, so the system lists AirPlay receivers.
/// The More menu hosts one as its AirPlay row; it is made fresh each time the menu opens.
unsafe fn new_route_picker() -> *mut AnyObject {
    let v: *mut AnyObject = msg_send![class!(AVRoutePickerView), alloc];
    let v: *mut AnyObject = msg_send![v, initWithFrame: rect(0., 0., ROUTE_ROW_W, ROUTE_ROW_H)];
    // The icon follows the player's dark appearance; there is no colour setter on macOS.
    let _: () = msg_send![v, setToolTip: ns("AirPlay")];
    autorel(v)
}

pub(crate) unsafe fn ctl_button(bar: *mut AnyObject, name: &str, pt: f64, tip: &str, action: Sel) -> *mut AnyObject {
    let ui = UI.get().unwrap();
    let b: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: action];
    let _: () = msg_send![b, setBordered: Bool::NO];
    let img = symbol(name, pt, Weight::Medium);
    if !img.is_null() {
        let _: () = msg_send![b, setImage: img];
    }
    let _: () = msg_send![b, setImagePosition: 1isize]; // image only
    let _: () = msg_send![b, setContentTintColor: color(C_ON_MEDIA)];
    let _: () = msg_send![b, setFocusRingType: 1usize]; // none: the bar is a control surface, not a form
    let _: () = msg_send![b, setToolTip: ns(tip)];
    add_sub(bar, b);
    b
}

/// Positions every child from the bar's current width. Runs on build and on every layout pass.
pub(crate) unsafe fn relayout() {
    let ui = match UI.get() {
        Some(u) => u,
        None => return,
    };
    let p = &ui.pbar;
    if p.bar.get().is_null() {
        return;
    }
    let w = bounds_of(p.bar.get()).size.w;
    let cy = CTRL_CY;
    set_frame(p.elapsed.get(), rect(EDGE, SCRUB_Y, TIME_W, SCRUB_H));
    set_frame(p.remain.get(), rect(w - EDGE - TIME_W, SCRUB_Y, TIME_W, SCRUB_H));
    set_frame(
        p.scrub.get(),
        rect(EDGE + TIME_W + SP3, SCRUB_Y, (w - 2. * EDGE - 2. * TIME_W - 2. * SP3).max(60.), SCRUB_H),
    );

    // Left cluster: mute, then the volume slider.
    set_frame(p.mute.get(), rect(EDGE, cy - BTN / 2., BTN, BTN));
    set_frame(p.vol.get(), rect(EDGE + BTN + SP1, cy - 8., VOL_W, 16.));

    // Right cluster, counted from the right edge. The episode buttons only show for episodes.
    let episode = ui.playing.as_ref().map(|pl| pl.skip.series_id.is_some()).unwrap_or(false);
    let mut x = w - EDGE;
    for (b, episode_only) in [
        (p.full.get(), false),
        (p.more.get(), false),
        (p.audio.get(), false),
        (p.subs.get(), false),
        (p.episodes.get(), true),
        (p.next.get(), true),
    ] {
        if episode_only && !episode {
            set_hidden(b, true);
            continue;
        }
        set_hidden(b, false);
        set_frame(b, rect(x - BTN, cy - BTN / 2., BTN, BTN));
        x -= BTN + SP1;
    }
    // Next Episode is only live while there is a next episode to play.
    let has_next = ui.playing.as_ref().map(|pl| pl.skip.next.is_some()).unwrap_or(false);
    let _: () = msg_send![p.next.get(), setEnabled: if has_next { Bool::YES } else { Bool::NO }];
}

/// Places the centred transport over the picture, midway between the bottom bar and the top bar.
pub(crate) unsafe fn relayout_transport() {
    let ui = match UI.get() {
        Some(u) => u,
        None => return,
    };
    let t = ui.pbar.transport.get();
    if t.is_null() {
        return;
    }
    let b = bounds_of(t).size;
    let cx = b.w / 2.;
    let cy = (PLAYER_CTRL_H + (b.h - PLAYER_BAR_H)) / 2.;
    // The Episodes panel is sized from the picture too; this pass runs on every resize of the player.
    crate::ui::episodes::layout();
    let play = ui.pbar.play.get();
    set_frame(play, rect(cx - CENTER_PLAY / 2., cy - CENTER_PLAY / 2., CENTER_PLAY, CENTER_PLAY));
    let back_x = cx - CENTER_PLAY / 2. - CENTER_GAP - CENTER_SKIP;
    set_frame(ui.pbar.back.get(), rect(back_x, cy - CENTER_SKIP / 2., CENTER_SKIP, CENTER_SKIP));
    let fwd_x = cx + CENTER_PLAY / 2. + CENTER_GAP;
    set_frame(ui.pbar.fwd.get(), rect(fwd_x, cy - CENTER_SKIP / 2., CENTER_SKIP, CENTER_SKIP));
}

/// Builds the centred transport over the picture. It sits below the bottom bar and the skip overlays.
pub(crate) unsafe fn build_transport(ph: *mut AnyObject, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let tcls = *TRANSPORT_CLASS.get().unwrap();
    let t: *mut AnyObject = msg_send![tcls, alloc];
    let t: *mut AnyObject = msg_send![t, initWithFrame: rect(0., 0., b.size.w, b.size.h)];
    let _: () = msg_send![t, setWantsLayer: Bool::YES];
    set_autoresize(t, RESIZE_W | RESIZE_H);
    add_sub(ph, t);
    autorel(t);
    ui.pbar.transport = Ptr(t);

    let play = ctl_button(t, "play.fill", CENTER_PLAY_SYMBOL, "Play / Pause", sel!(playPause:));
    ui.pbar.play = Ptr(play);
    let back = ctl_button(t, "gobackward.10", CENTER_SKIP_SYMBOL, "Back 10 seconds", sel!(seekBack:));
    ui.pbar.back = Ptr(back);
    let fwd = ctl_button(t, "goforward.10", CENTER_SKIP_SYMBOL, "Forward 10 seconds", sel!(seekFwd:));
    ui.pbar.fwd = Ptr(fwd);
    relayout_transport();
}

/// The vignette: clear over the middle of the picture, darkening toward the edges so the centred controls
/// and the subtitles above the bar read on any frame.
extern "C" fn transport_layer(_t: *mut AnyObject, _c: Sel) -> *mut AnyObject {
    unsafe {
        let l: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
        let _: () = msg_send![l, setType: ns("radial")];
        let cols = [cg_black(0.0) as *mut AnyObject, cg_black(0.0) as *mut AnyObject, cg_black(0.6) as *mut AnyObject];
        let arr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr(), count: 3usize];
        let _: () = msg_send![l, setColors: arr];
        let mut locs: Vec<*mut AnyObject> = Vec::new();
        for v in [0.0f64, 0.5, 1.0] {
            let n: *mut AnyObject = msg_send![class!(NSNumber), numberWithDouble: v];
            locs.push(n);
        }
        let larr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: locs.as_ptr(), count: 3usize];
        let _: () = msg_send![l, setLocations: larr];
        let _: () = msg_send![l, setStartPoint: NSPoint { x: 0.5, y: 0.5 }];
        let _: () = msg_send![l, setEndPoint: NSPoint { x: 1.0, y: 1.0 }];
        l
    }
}

extern "C" fn transport_layout(this: *mut AnyObject, _c: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
        relayout_transport();
    }
}

/// Clicks land on the transport's buttons; anywhere else they pass through to the video below.
extern "C" fn transport_hit(this: *mut AnyObject, _c: Sel, p: NSPoint) -> *mut AnyObject {
    unsafe {
        let hit: *mut AnyObject = msg_send![super(this, class!(NSView)), hitTest: p];
        if hit == this {
            nil()
        } else {
            hit
        }
    }
}

/// Duration in seconds: the item's when it is known, else the one the worker sent.
unsafe fn duration_of(ui: &Ui) -> f64 {
    let mut secs = ui.playing.as_ref().map(|p| p.duration_secs).unwrap_or(0.0);
    if !ui.pitem.get().is_null() {
        let d: CMTime = msg_send![ui.pitem.get(), duration];
        if d.seconds() > 0.0 {
            secs = d.seconds();
        }
    }
    secs
}

fn redraw_scrub() {
    unsafe {
        if let Some(ui) = UI.get() {
            let _: () = msg_send![ui.pbar.scrub.get(), setNeedsDisplay: Bool::YES];
        }
    }
}

/// Called on every playback tick while the bar is up: clocks, progress, buffered range, icons.
pub(crate) unsafe fn update() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    if ui.pbar.bar.get().is_null() || ui.playing.is_none() || ui.pitem.get().is_null() {
        return;
    }
    sync_pip_controls();
    let ui = UI.get_mut().unwrap();
    let paused = ui.playing.as_ref().map(|p| p.paused).unwrap_or(true);
    let t: CMTime = msg_send![ui.player.get(), currentTime];
    let dur = duration_of(ui);
    let cur = t.seconds().max(0.0);
    set_text(ui.pbar.elapsed.get(), &crate::jellyfin::urls::fmt_clock(cur));
    let left = if dur > 0.0 { (dur - cur).max(0.0) } else { 0.0 };
    set_text(ui.pbar.remain.get(), &format!("-{}", crate::jellyfin::urls::fmt_clock(left)));
    if !ui.pbar.dragging {
        ui.pbar.frac = if dur > 0.0 { (cur / dur).clamp(0.0, 1.0) } else { 0.0 };
    }
    // Buffered range: the end of the last loaded range, as a fraction of the duration.
    let ranges: *mut AnyObject = msg_send![ui.pitem.get(), loadedTimeRanges];
    let n: NSUInteger = msg_send![ranges, count];
    ui.pbar.buffered = if n > 0 && dur > 0.0 {
        let v: *mut AnyObject = msg_send![ranges, objectAtIndex: n - 1];
        let r: CMTimeRange = msg_send![v, CMTimeRangeValue];
        ((r.start.seconds() + r.duration.seconds()) / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    redraw_scrub();

    let v: f32 = msg_send![ui.player.get(), volume];
    let muted: Bool = msg_send![ui.player.get(), isMuted];
    let _: () = msg_send![ui.pbar.vol.get(), setDoubleValue: v as f64];
    let icon = if muted == Bool::YES || v <= 0.001 {
        "speaker.slash.fill"
    } else if v < 0.5 {
        "speaker.wave.1.fill"
    } else {
        "speaker.wave.2.fill"
    };
    if icon != ui.pbar.mute_icon {
        ui.pbar.mute_icon = icon;
        let img = symbol(icon, 20., Weight::Medium);
        let _: () = msg_send![ui.pbar.mute.get(), setImage: img];
    }
    if ui.pbar.shows_pause == paused {
        ui.pbar.shows_pause = !paused;
        let img = symbol(if paused { "play.fill" } else { "pause.fill" }, 26., Weight::Medium);
        if !img.is_null() {
            let _: () = msg_send![ui.pbar.play.get(), setImage: img];
        }
    }
}

/// Playback started: show the bar and set up Picture in Picture for this player layer, if the Mac has it.
pub(crate) unsafe fn on_start() {
    let ui = UI.get_mut().unwrap();
    if ui.pbar.bar.get().is_null() {
        return;
    }
    sync_pip_controls();
    relayout();
}

/// Makes the PiP controller once AVPlayerView has its layer (it appears lazily, so this may take a few ticks),
/// then hides the button when there is no controller and syncs its state. Cheap; runs on start and each tick.
unsafe fn sync_pip_controls() {
    let ui = UI.get_mut().unwrap();
    let supported: Bool = msg_send![class!(AVPictureInPictureController), isPictureInPictureSupported];
    if supported == Bool::YES && ui.pbar.pip_ctl.get().is_null() {
        ensure_pip_controller();
    }
    let ui = UI.get_mut().unwrap();
    // The PiP button is no longer in the bar: Picture in Picture lives in the More menu.
    let _ = supported;
    set_hidden(ui.pbar.pip.get(), true);
    refresh_pip_button();
}

/// Finds the AVPlayerLayer that AVPlayerView draws into (it may sit below the view's root layer).
unsafe fn find_player_layer(l: *mut AnyObject, depth: u32) -> *mut AnyObject {
    // AVPlayerView nests its AVPlayerLayer about six levels down, so the search goes that deep.
    if l.is_null() || depth > 8 {
        return nil();
    }
    let is: Bool = msg_send![l, isKindOfClass: class!(AVPlayerLayer)];
    if is == Bool::YES {
        return l;
    }
    let subs: *mut AnyObject = msg_send![l, sublayers];
    if subs.is_null() {
        return nil();
    }
    let n: NSUInteger = msg_send![subs, count];
    for i in 0..n {
        let s: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let f = find_player_layer(s, depth + 1);
        if !f.is_null() {
            return f;
        }
    }
    nil()
}

// ---- scrubber -----------------------------------------------------------------------------------

unsafe fn fill_rr(r: NSRect, radius: f64, c: *mut AnyObject) {
    let _: () = msg_send![c, set];
    let p: *mut AnyObject = msg_send![class!(NSBezierPath), bezierPathWithRoundedRect: r, xRadius: radius, yRadius: radius];
    let _: () = msg_send![p, fill];
}
unsafe fn fill_oval(r: NSRect, c: *mut AnyObject) {
    let _: () = msg_send![c, set];
    let p: *mut AnyObject = msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: r];
    let _: () = msg_send![p, fill];
}

extern "C" fn scrub_draw(this: *mut AnyObject, _c: Sel, _dirty: NSRect) {
    unsafe {
        let ui = match UI.get() {
            Some(u) => u,
            None => return,
        };
        let b = bounds_of(this);
        let (w, h) = (b.size.w, b.size.h);
        let p = &ui.pbar;
        let hot = p.hover.is_some() || p.dragging;
        let th = if hot { 8. } else { 5. };
        let cy = h / 2.;
        let x_played = (p.frac * w).clamp(0., w);
        let x_buf = (p.buffered * w).clamp(0., w);
        fill_rr(rect(0., cy - th / 2., w, th), th / 2., color_a(C_ON_MEDIA, 0.28));
        fill_rr(rect(0., cy - th / 2., x_buf, th), th / 2., color_a(C_ON_MEDIA, 0.50));
        fill_rr(rect(0., cy - th / 2., x_played, th), th / 2., color(C_ON_MEDIA));
        let r = if hot { 9. } else { 7. };
        let kx = x_played.clamp(r, (w - r).max(r));
        fill_oval(rect(kx - r, cy - r, 2. * r, 2. * r), color(C_ON_MEDIA));
    }
}

/// Fraction of the track under an event's pointer, 0..1.
unsafe fn frac_at(this: *mut AnyObject, ev: *mut AnyObject) -> f64 {
    let loc: NSPoint = msg_send![ev, locationInWindow];
    let p: NSPoint = msg_send![this, convertPoint: loc, fromView: nil()];
    let w = bounds_of(this).size.w;
    if w <= 0.0 {
        0.0
    } else {
        (p.x / w).clamp(0.0, 1.0)
    }
}

/// Seeks to a fraction of the item and moves the track there at once.
pub(crate) unsafe fn seek_frac(f: f64) {
    let ui = UI.get_mut().unwrap();
    if ui.playing.is_none() {
        return;
    }
    let dur = duration_of(ui);
    if dur <= 0.0 {
        return;
    }
    let f = f.clamp(0.0, 1.0);
    ui.pbar.frac = f;
    crate::platform::avkit::seek_to_secs(f * dur);
    redraw_scrub();
}

/// Shows the time under the pointer above the track, or hides it (`None`).
pub(crate) unsafe fn set_hover(f: Option<f64>) {
    let ui = UI.get_mut().unwrap();
    if ui.pbar.bar.get().is_null() {
        return;
    }
    let dur = duration_of(ui);
    let p = &mut ui.pbar;
    p.hover = f;
    match f {
        Some(x) => {
            let sf = frame_of(p.scrub.get());
            set_text(p.tip_label.get(), &crate::jellyfin::urls::fmt_clock(x.clamp(0.0, 1.0) * dur));
            let cx = sf.origin.x + x.clamp(0.0, 1.0) * sf.size.w;
            let bw = bounds_of(p.bar.get()).size.w;
            let tx = (cx - TIP_W / 2.).clamp(EDGE, (bw - EDGE - TIP_W).max(EDGE));
            set_frame(p.tip.get(), rect(tx, sf.origin.y + sf.size.h + SP1, TIP_W, TIP_H));
            set_hidden(p.tip.get(), false);
        }
        None => set_hidden(p.tip.get(), true),
    }
    redraw_scrub();
}

extern "C" fn scrub_down(this: *mut AnyObject, _c: Sel, ev: *mut AnyObject) {
    unsafe {
        let me = this;
        let ui = UI.get_mut().unwrap();
        // A click on the faded-out bar only brings it back; it does not seek.
        let a: f64 = msg_send![ui.pbar.bar.get(), alphaValue];
        if a < 0.5 {
            crate::ui::player::show_bars();
            return;
        }
        ui.pbar.dragging = true;
        seek_frac(frac_at(me, ev));
    }
}
extern "C" fn scrub_drag(this: *mut AnyObject, _c: Sel, ev: *mut AnyObject) {
    unsafe {
        let me = this;
        let f = frac_at(me, ev);
        seek_frac(f);
        set_hover(Some(f));
    }
}
extern "C" fn scrub_up(_this: *mut AnyObject, _c: Sel, _ev: *mut AnyObject) {
    if let Some(ui) = UI.get_mut() {
        ui.pbar.dragging = false;
    }
    redraw_scrub();
}
extern "C" fn scrub_moved(this: *mut AnyObject, _c: Sel, ev: *mut AnyObject) {
    unsafe {
        set_hover(Some(frac_at(this, ev)));
    }
}
extern "C" fn scrub_exited(_this: *mut AnyObject, _c: Sel, _ev: *mut AnyObject) {
    unsafe {
        if let Some(ui) = UI.get_mut() {
            if !ui.pbar.dragging {
                set_hover(None);
            }
        }
    }
}

// ---- menus ----------------------------------------------------------------------------------------

unsafe fn new_menu(title: &str) -> *mut AnyObject {
    let m: *mut AnyObject = msg_send![class!(NSMenu), alloc];
    let m: *mut AnyObject = msg_send![m, initWithTitle: ns(title)];
    autorel(m)
}
unsafe fn menu_row(menu: *mut AnyObject, title: &str, action: Sel, tag: isize, on: bool) {
    let it = menu_item(title, action, "");
    let _: () = msg_send![it, setTarget: UI.get().unwrap().ctl.get()];
    let _: () = msg_send![it, setTag: tag];
    let _: () = msg_send![it, setState: if on { 1isize } else { 0isize }];
    let _: () = msg_send![menu, addItem: it];
}
/// Opens a menu under its button. Self-test mode only counts the items.
unsafe fn present(menu: *mut AnyObject, anchor: *mut AnyObject) {
    let n: NSInteger = msg_send![menu, numberOfItems];
    UI.get_mut().unwrap().pbar.last_menu_items = n as usize;
    if offscreen_mode() {
        return;
    }
    let h = bounds_of(anchor).size.h;
    let _: Bool = msg_send![menu, popUpMenuPositioningItem: nil(), atLocation: NSPoint { x: 0.0, y: h }, inView: anchor];
}

/// Subtitle (which = 0) or audio (which = 1) menu. Tags are `which * 1000 + index`, where index 0 is Off
/// and index i+1 is option i. Selection itself is `avkit::set_track`, shared with the Controls menu.
unsafe fn track_menu(which: usize, anchor: *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    let (opts, cur) = if which == 0 {
        (ui.sub_opts.clone(), ui.sel_sub)
    } else {
        (ui.audio_opts.clone(), ui.sel_audio)
    };
    let menu = new_menu(if which == 0 { "Subtitles" } else { "Audio" });
    let base = (which * 1000) as isize;
    menu_row(menu, "Off", sel!(pickTrack:), base, cur == 0);
    for (i, o) in opts.iter().enumerate() {
        let name = to_str(msg_send![*o, displayName]);
        menu_row(menu, &name, sel!(pickTrack:), base + i as isize + 1, cur == i + 1);
    }
    present(menu, anchor);
}
pub(crate) extern "C" fn show_sub_menu(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe { track_menu(0, sender) }
}
pub(crate) extern "C" fn show_audio_menu(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe { track_menu(1, sender) }
}
pub(crate) extern "C" fn pick_track(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        crate::platform::avkit::set_track((tag / 1000) as usize, (tag % 1000) as usize);
    }
}
pub(crate) extern "C" fn show_speed_menu(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let ui = UI.get().unwrap();
        let rate: f32 = msg_send![ui.player.get(), defaultRate];
        let menu = new_menu("Playback Speed");
        for (label, tag) in SPEEDS {
            let on = ((tag as f32) / 100.0 - rate).abs() < 0.01;
            menu_row(menu, label, sel!(setSpeed:), tag, on);
        }
        present(menu, sender);
    }
}

/// The More menu: playback speed, AirPlay, Picture in Picture and Cast. Built fresh each time it opens,
/// so the PiP title and the speed tick always match the player.
pub(crate) extern "C" fn show_more_menu(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let menu = new_menu("More");

        let rate: f32 = msg_send![ui.player.get(), defaultRate];
        let speeds = new_menu("Playback Speed");
        for (label, tag) in SPEEDS {
            let on = ((tag as f32) / 100.0 - rate).abs() < 0.01;
            menu_row(speeds, label, sel!(setSpeed:), tag, on);
        }
        let speed_item = menu_item("Playback Speed", sel!(showSpeedMenu:), "");
        let _: () = msg_send![speed_item, setSubmenu: speeds];
        let _: () = msg_send![menu, addItem: speed_item];

        // AirPlay: AVKit's route picker is the row itself, so clicking it opens the system receiver list.
        let route = new_route_picker();
        UI.get_mut().unwrap().pbar.route = Ptr(route);
        let air = menu_item("AirPlay", sel!(showMoreMenu:), "");
        let _: () = msg_send![air, setView: route];
        let _: () = msg_send![menu, addItem: air];

        let pip_title = if pip_active() { "Exit Picture in Picture" } else { "Picture in Picture" };
        let pip = menu_item(pip_title, sel!(togglePiP:), "");
        let _: () = msg_send![pip, setTarget: UI.get().unwrap().ctl.get()];
        let _: () = msg_send![menu, addItem: pip];

        let cast = menu_item("Cast to a Device…", sel!(castFromMore:), "");
        let _: () = msg_send![cast, setTarget: UI.get().unwrap().ctl.get()];
        let _: () = msg_send![menu, addItem: cast];

        present(menu, sender);
    }
}

/// Cast from the More menu: the cast menu opens under the More button.
pub(crate) extern "C" fn cast_from_more(t: *mut AnyObject, c: Sel, _s: *mut AnyObject) {
    let more = UI.get().map(|u| u.pbar.more.get()).unwrap_or(std::ptr::null_mut());
    crate::ui::cast::show_menu(t, c, more);
}

// ---- Picture in Picture ------------------------------------------------------------------------------

/// The controller for the player layer under AVPlayerView. One is kept per layer and reused for every item
/// played in it; if the layer is ever replaced, the old controller is stopped and released first.
/// Returns null when no player layer exists yet.
unsafe fn ensure_pip_controller() -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let root: *mut AnyObject = msg_send![ui.avview.get(), layer];
    let layer = find_player_layer(root, 0);
    if layer.is_null() {
        return nil();
    }
    if !ui.pbar.pip_ctl.get().is_null() {
        if ui.pbar.pip_layer.get() == layer {
            return ui.pbar.pip_ctl.get();
        }
        release_pip_controller();
    }
    let ui = UI.get_mut().unwrap();
    if ui.pbar.pip_delegate.get().is_null() {
        let dcls = *PIP_CLASS.get().unwrap();
        let d: *mut AnyObject = msg_send![dcls, new];
        ui.pbar.pip_delegate = Ptr(d); // +1 from new, kept for the app's life
    }
    let c: *mut AnyObject = msg_send![class!(AVPictureInPictureController), alloc];
    let c: *mut AnyObject = msg_send![c, initWithPlayerLayer: layer];
    if c.is_null() {
        return nil();
    }
    let _: () = msg_send![c, setDelegate: ui.pbar.pip_delegate.get()];
    for kp in ["pictureInPicturePossible", "pictureInPictureActive"] {
        let _: () = msg_send![c, addObserver: ui.ctl.get(), forKeyPath: ns(kp), options: 0usize, context: std::ptr::null_mut::<std::ffi::c_void>()];
    }
    ui.pbar.pip_ctl = Ptr(c); // +1 from alloc/init, owned by the bar
    ui.pbar.pip_layer = Ptr(layer);
    c
}

/// Stops the controller, drops its observers and delegate, and releases it.
unsafe fn release_pip_controller() {
    let ui = UI.get_mut().unwrap();
    let c = ui.pbar.pip_ctl.get();
    if c.is_null() {
        return;
    }
    let active: Bool = msg_send![c, isPictureInPictureActive];
    if active == Bool::YES {
        PIP_APP_STOP.store(true, Ordering::SeqCst);
        let _: () = msg_send![c, stopPictureInPicture];
    }
    for kp in ["pictureInPicturePossible", "pictureInPictureActive"] {
        let _: () = msg_send![c, removeObserver: ui.ctl.get(), forKeyPath: ns(kp)];
    }
    let _: () = msg_send![c, setDelegate: nil()];
    release_obj(c);
    ui.pbar.pip_ctl = Ptr(nil());
    ui.pbar.pip_layer = Ptr(nil());
}

/// Whether Picture in Picture can be started or is already showing. Drives the button and the ⌥⌘P menu item.
pub(crate) unsafe fn pip_available() -> bool {
    let c = match UI.get() {
        Some(u) => u.pbar.pip_ctl.get(),
        None => return false,
    };
    if c.is_null() {
        return false;
    }
    let possible: Bool = msg_send![c, isPictureInPicturePossible];
    let active: Bool = msg_send![c, isPictureInPictureActive];
    possible == Bool::YES || active == Bool::YES
}

/// Whether Picture in Picture is showing right now.
pub(crate) unsafe fn pip_active() -> bool {
    match UI.get() {
        Some(u) if !u.pbar.pip_ctl.get().is_null() => {
            let active: Bool = msg_send![u.pbar.pip_ctl.get(), isPictureInPictureActive];
            active == Bool::YES
        }
        _ => false,
    }
}

/// Syncs the Picture in Picture button with the controller. Disabled while AVKit cannot start PiP
/// (for example before the item is ready); the glyph switches to the exit icon while PiP is showing.
pub(crate) unsafe fn refresh_pip_button() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    let btn = ui.pbar.pip.get();
    if btn.is_null() {
        return;
    }
    let on = pip_available();
    let active = pip_active();
    let _: () = msg_send![btn, setEnabled: if on { Bool::YES } else { Bool::NO }];
    let _: () = msg_send![btn, setAlphaValue: if on { 1.0f64 } else { 0.35f64 }];
    if active != ui.pbar.pip_shows_exit {
        ui.pbar.pip_shows_exit = active;
        let name = if active { "pip.exit" } else { "pip.enter" };
        let tip = if active { "Exit Picture in Picture" } else { "Picture in Picture" };
        let img = symbol(name, 20., Weight::Medium);
        if !img.is_null() {
            let _: () = msg_send![btn, setImage: img];
        }
        let _: () = msg_send![btn, setToolTip: ns(tip)];
    }
}

/// Stops Picture in Picture if it is showing (playback stopped, the item ended, or the player went to audio).
pub(crate) unsafe fn pip_stop() {
    let c = match UI.get() {
        Some(u) => u.pbar.pip_ctl.get(),
        None => return,
    };
    if !c.is_null() && pip_active() {
        PIP_APP_STOP.store(true, Ordering::SeqCst);
        let _: () = msg_send![c, stopPictureInPicture];
    }
    refresh_pip_button();
}

/// Toggles Picture in Picture on the AVPlayerLayer under AVPlayerView. Off-screen (self-test) it only counts the press.
pub(crate) extern "C" fn toggle_pip(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        if offscreen_mode() {
            UI.get_mut().unwrap().pbar.pip_presses += 1;
            return;
        }
        let c = ensure_pip_controller();
        if c.is_null() {
            return;
        }
        if pip_active() {
            PIP_APP_STOP.store(true, Ordering::SeqCst);
            let _: () = msg_send![c, stopPictureInPicture];
        } else if pip_available() {
            let _: () = msg_send![c, startPictureInPicture];
        }
        refresh_pip_button();
    }
}

/// `keepsRunningAfterLastWindowClosed` stand-in: closing the window quits the app as usual, unless Picture in
/// Picture is showing, in which case the app keeps running so the PiP window can bring the player back.
pub(crate) extern "C" fn terminate_after_last_window(_t: *mut AnyObject, _c: Sel, _o: *mut AnyObject) -> Bool {
    if unsafe { pip_active() } { Bool::NO } else { Bool::YES }
}

/// Calls an Objective-C block that takes one BOOL. A block literal keeps its invoke pointer at offset 16
/// (after isa, flags and reserved), and the invoke function takes the block itself as its first argument.
unsafe fn call_block_bool(block: *mut AnyObject, v: Bool) {
    let invoke = *((block as *const u8).add(16) as *const *const c_void);
    let f: extern "C" fn(*mut AnyObject, Bool) = std::mem::transmute(invoke);
    f(block, v);
}

/// Restore from the Picture in Picture window: bring the main player window back, then tell AVKit it may
/// finish the transition by calling the completion handler with YES.
extern "C" fn pip_restore(_t: *mut AnyObject, _c: Sel, _ctl: *mut AnyObject, done: *mut AnyObject) {
    unsafe {
        let ui = UI.get().unwrap();
        let w = ui.win.get();
        let mini: Bool = msg_send![w, isMiniaturized];
        if mini == Bool::YES {
            let _: () = msg_send![w, deminiaturize: nil()];
        }
        let _: () = msg_send![ui.app.get(), activateIgnoringOtherApps: Bool::YES];
        let _: () = msg_send![w, makeKeyAndOrderFront: nil()];
        crate::ui::player::show_bars();
        PIP_RESTORING.store(true, Ordering::SeqCst);
        if !done.is_null() {
            call_block_bool(done, Bool::YES);
        }
    }
}

extern "C" fn pip_started(_t: *mut AnyObject, _c: Sel, _ctl: *mut AnyObject) {
    unsafe { refresh_pip_button() }
}

/// PiP ended. Three ways it can end: the app stopped it (nothing more to do), the restore button was pressed
/// (the player window is back, playback goes on), or PiP's close button was pressed. The last one stops
/// playback, as Apple's TV app does. The app keeps running either way, even with no window open.
extern "C" fn pip_stopped(_t: *mut AnyObject, _c: Sel, _ctl: *mut AnyObject) {
    unsafe {
        refresh_pip_button();
        let app_stop = PIP_APP_STOP.swap(false, Ordering::SeqCst);
        let restoring = PIP_RESTORING.swap(false, Ordering::SeqCst);
        if app_stop || restoring {
            return;
        }
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        if ui.playing.is_some() {
            if let Some(pn) = ui.playing.as_ref() {
                let _ = ui.ui_tx.send(crate::app::messages::UiCmd::Stopped {
                    item_id: pn.item_id.clone(),
                    ticks: crate::jellyfin::urls::ticks_from_secs(pn.last_secs),
                });
            }
            crate::platform::avkit::stop_playback_ui();
        }
    }
}

extern "C" fn pip_failed(_t: *mut AnyObject, _c: Sel, _ctl: *mut AnyObject, _err: *mut AnyObject) {
    unsafe {
        refresh_pip_button();
        crate::ui::player::toast("Picture in Picture could not start.", Some("pip.exit"), true);
    }
}

// ---- visibility -------------------------------------------------------------------------------------

/// Whether the bar is fully shown. Used by the self-test.
pub(crate) unsafe fn visible() -> bool {
    let ui = UI.get().unwrap();
    let a: f64 = msg_send![ui.pbar.bar.get(), alphaValue];
    a > 0.99
}
