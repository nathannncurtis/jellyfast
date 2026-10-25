//! Custom transport bar for video playback. AVKit's own controls are switched off
//! (`controlsStyle` none), so this bar is the whole transport. Bottom to top it has a
//! gradient wash, a scrubber row (elapsed | track | remaining) with a hover time tooltip,
//! and a button row: play/pause, back 10 s, forward 10 s on the left; mute and volume,
//! subtitles, audio, speed, Picture in Picture and full screen on the right.
//! It fades with the top bar after the idle timeout (see `player::hide_bars_if_idle`).
#![allow(unsafe_code)]
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};
use std::sync::OnceLock;

static BAR_CLASS: OnceLock<&'static Class> = OnceLock::new();
static SCRUB_CLASS: OnceLock<&'static Class> = OnceLock::new();

/// Secondary button hit size, and the larger play/pause target.
const BTN: f64 = 40.;
const BTN_PRIMARY: f64 = 48.;
/// Scrubber row: elapsed label | track | remaining label.
const TIME_W: f64 = 76.;
const SCRUB_Y: f64 = 72.;
const SCRUB_H: f64 = 24.;
/// Vertical centre of the button row.
const CTRL_CY: f64 = 40.;
const TIP_W: f64 = 80.;
const TIP_H: f64 = 22.;
const VOL_W: f64 = 120.;
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
    /// Owned Picture in Picture controller, created on first use.
    pub pip_ctl: Ptr,
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
            pip_ctl: n(),
            frac: 0.0,
            buffered: 0.0,
            hover: None,
            dragging: false,
            shows_pause: true,
            mute_icon: "",
            last_menu_items: 0,
        }
    }
}

pub(crate) unsafe fn register_playerbar_classes() {
    let mut d = ClassDecl::new("JRPlayerBar", class!(NSView)).unwrap();
    d.add_method(sel!(makeBackingLayer), bar_layer as extern "C" fn(&mut Object, Sel) -> *mut Object);
    d.add_method(sel!(layout), bar_layout as extern "C" fn(&mut Object, Sel));
    BAR_CLASS.set(d.register()).ok();

    let mut s = ClassDecl::new("JRScrubber", class!(NSView)).unwrap();
    s.add_method(sel!(drawRect:), scrub_draw as extern "C" fn(&mut Object, Sel, NSRect));
    s.add_method(sel!(acceptsFirstMouse:), yes_obj as extern "C" fn(&mut Object, Sel, *mut Object) -> BOOL);
    s.add_method(sel!(mouseDown:), scrub_down as extern "C" fn(&mut Object, Sel, *mut Object));
    s.add_method(sel!(mouseDragged:), scrub_drag as extern "C" fn(&mut Object, Sel, *mut Object));
    s.add_method(sel!(mouseUp:), scrub_up as extern "C" fn(&mut Object, Sel, *mut Object));
    s.add_method(sel!(mouseMoved:), scrub_moved as extern "C" fn(&mut Object, Sel, *mut Object));
    s.add_method(sel!(mouseExited:), scrub_exited as extern "C" fn(&mut Object, Sel, *mut Object));
    SCRUB_CLASS.set(s.register()).ok();
}

extern "C" fn yes_obj(_t: &mut Object, _c: Sel, _e: *mut Object) -> BOOL {
    YES
}

extern "C" fn bar_layer(_t: &mut Object, _c: Sel) -> *mut Object {
    unsafe {
        // Transparent at the top, darkening toward the bottom edge so white controls read on video.
        let l: *mut Object = msg_send![class!(CAGradientLayer), layer];
        let cols = [cg_black(0.0), cg_black(0.72)];
        let arr: *mut Object = msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr() count: 2usize];
        let _: () = msg_send![l, setColors: arr];
        let _: () = msg_send![l, setStartPoint: NSPoint { x: 0.5, y: 1.0 }];
        let _: () = msg_send![l, setEndPoint: NSPoint { x: 0.5, y: 0.0 }];
        l
    }
}

extern "C" fn bar_layout(this: &mut Object, _c: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
        relayout();
    }
}

/// Builds the bar inside the player host, above AVKit's view. Hidden state is set by playback.
pub(crate) unsafe fn build(ph: *mut Object, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let bcls = *BAR_CLASS.get().unwrap();
    let bar: *mut Object = msg_send![bcls, alloc];
    let bar: *mut Object = msg_send![bar, initWithFrame: rect(0., 0., b.size.w, PLAYER_CTRL_H)];
    let _: () = msg_send![bar, setWantsLayer: YES];
    set_autoresize(bar, RESIZE_W | MARGIN_MAXY);
    add_sub(ph, bar);
    autorel(bar);
    ui.pbar.bar = Ptr(bar);

    // Scrubber: a plain view that draws itself and handles the pointer.
    let scls = *SCRUB_CLASS.get().unwrap();
    let sc: *mut Object = msg_send![scls, alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: rect(0., 0., 10., SCRUB_H)];
    let ta: *mut Object = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut Object = msg_send![ta,
        initWithRect: rect(0., 0., 0., 0.)
        options: (0x01usize | 0x02usize | 0x20usize | 0x200usize) owner: sc userInfo: nil()];
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
    let _: () = msg_send![cl, setMasksToBounds: YES];
    set_hidden(chip, true);
    add_sub(bar, chip);
    ui.pbar.tip = Ptr(chip);
    let tl = label("", T_BODY_EM, C_ON_MEDIA, ALIGN_CENTER);
    set_frame(tl, rect(0., (TIP_H - 16.) / 2., TIP_W, 16.));
    add_sub(chip, tl);
    ui.pbar.tip_label = Ptr(tl);

    let play = ctl_button(bar, "play.fill", 26., "Play / Pause", sel!(playPause:));
    ui.pbar.play = Ptr(play);
    let back = ctl_button(bar, "gobackward.10", 22., "Back 10 seconds", sel!(seekBack:));
    ui.pbar.back = Ptr(back);
    let fwd = ctl_button(bar, "goforward.10", 22., "Forward 10 seconds", sel!(seekFwd:));
    ui.pbar.fwd = Ptr(fwd);

    let mute = ctl_button(bar, "speaker.wave.2.fill", 20., "Mute", sel!(mute:));
    ui.pbar.mute = Ptr(mute);
    let s: *mut Object = msg_send![class!(NSSlider), alloc];
    let s: *mut Object = msg_send![s, initWithFrame: rect(0., 0., VOL_W, 16.)];
    let _: () = msg_send![s, setMinValue: 0.0f64];
    let _: () = msg_send![s, setMaxValue: 1.0f64];
    let _: () = msg_send![s, setControlSize: 0usize]; // regular: easier to hit from the couch
    let _: () = msg_send![s, setContinuous: YES];
    let _: () = msg_send![s, setTarget: ui.ctl.get()];
    let _: () = msg_send![s, setAction: sel!(jrNowVolume:)];
    let _: () = msg_send![s, setToolTip: ns("Volume")];
    autorel(s);
    add_sub(bar, s);
    ui.pbar.vol = Ptr(s);

    let subs = ctl_button(bar, "captions.bubble", 20., "Subtitles", sel!(showSubtitleMenu:));
    ui.pbar.subs = Ptr(subs);
    let audio = ctl_button(bar, "music.note.list", 20., "Audio track", sel!(showAudioMenu:));
    ui.pbar.audio = Ptr(audio);
    let speed = ctl_button(bar, "gauge.with.dots.needle.67percent", 20., "Playback speed", sel!(showSpeedMenu:));
    ui.pbar.speed = Ptr(speed);
    let pip = ctl_button(bar, "pip.enter", 20., "Picture in Picture", sel!(togglePiP:));
    ui.pbar.pip = Ptr(pip);
    let full = ctl_button(bar, "arrow.up.left.and.arrow.down.right", 20., "Full screen", sel!(fullscreen:));
    ui.pbar.full = Ptr(full);

    relayout();
}

unsafe fn ctl_button(bar: *mut Object, name: &str, pt: f64, tip: &str, action: Sel) -> *mut Object {
    let ui = UI.get().unwrap();
    let b: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("") target: ui.ctl.get() action: action];
    let _: () = msg_send![b, setBordered: NO];
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

    // Left cluster: play/pause, then the 10 s skips.
    set_frame(p.play.get(), rect(EDGE, cy - BTN_PRIMARY / 2., BTN_PRIMARY, BTN_PRIMARY));
    let back_x = EDGE + BTN_PRIMARY + SP2;
    set_frame(p.back.get(), rect(back_x, cy - BTN / 2., BTN, BTN));
    set_frame(p.fwd.get(), rect(back_x + BTN + SP1, cy - BTN / 2., BTN, BTN));

    // Right cluster, counted from the right edge: slot 0 is full screen, slot 4 is subtitles.
    let slot = |i: f64| w - EDGE - BTN - i * (BTN + SP1);
    set_frame(p.full.get(), rect(slot(0.), cy - BTN / 2., BTN, BTN));
    set_frame(p.pip.get(), rect(slot(1.), cy - BTN / 2., BTN, BTN));
    set_frame(p.speed.get(), rect(slot(2.), cy - BTN / 2., BTN, BTN));
    set_frame(p.audio.get(), rect(slot(3.), cy - BTN / 2., BTN, BTN));
    set_frame(p.subs.get(), rect(slot(4.), cy - BTN / 2., BTN, BTN));

    // Volume sits just left of the subtitle button: mute, then the slider.
    let vol_x = slot(4.) - SP4 - VOL_W;
    set_frame(p.vol.get(), rect(vol_x, cy - 8., VOL_W, 16.));
    set_frame(p.mute.get(), rect(vol_x - SP1 - BTN, cy - BTN / 2., BTN, BTN));
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
            let _: () = msg_send![ui.pbar.scrub.get(), setNeedsDisplay: YES];
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
    let ranges: *mut Object = msg_send![ui.pitem.get(), loadedTimeRanges];
    let n: NSUInteger = msg_send![ranges, count];
    ui.pbar.buffered = if n > 0 && dur > 0.0 {
        let v: *mut Object = msg_send![ranges, objectAtIndex: n - 1];
        let r: CMTimeRange = msg_send![v, CMTimeRangeValue];
        ((r.start.seconds() + r.duration.seconds()) / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    redraw_scrub();

    let v: f32 = msg_send![ui.player.get(), volume];
    let muted: BOOL = msg_send![ui.player.get(), isMuted];
    let _: () = msg_send![ui.pbar.vol.get(), setDoubleValue: v as f64];
    let icon = if muted == YES || v <= 0.001 {
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

/// Playback started: show the bar and check whether Picture in Picture can be offered here.
pub(crate) unsafe fn on_start() {
    let ui = UI.get_mut().unwrap();
    if ui.pbar.bar.get().is_null() {
        return;
    }
    let root: *mut Object = msg_send![ui.avview.get(), layer];
    let supported: BOOL = msg_send![class!(AVPictureInPictureController), isPictureInPictureSupported];
    let ok = supported == YES && !find_player_layer(root, 0).is_null();
    set_hidden(ui.pbar.pip.get(), !ok);
}

/// Finds the AVPlayerLayer that AVPlayerView draws into (it may sit below the view's root layer).
unsafe fn find_player_layer(l: *mut Object, depth: u32) -> *mut Object {
    if l.is_null() || depth > 4 {
        return nil();
    }
    let is: BOOL = msg_send![l, isKindOfClass: class!(AVPlayerLayer)];
    if is == YES {
        return l;
    }
    let subs: *mut Object = msg_send![l, sublayers];
    if subs.is_null() {
        return nil();
    }
    let n: NSUInteger = msg_send![subs, count];
    for i in 0..n {
        let s: *mut Object = msg_send![subs, objectAtIndex: i];
        let f = find_player_layer(s, depth + 1);
        if !f.is_null() {
            return f;
        }
    }
    nil()
}

// ---- scrubber -----------------------------------------------------------------------------------

unsafe fn fill_rr(r: NSRect, radius: f64, c: *mut Object) {
    let _: () = msg_send![c, set];
    let p: *mut Object = msg_send![class!(NSBezierPath), bezierPathWithRoundedRect: r xRadius: radius yRadius: radius];
    let _: () = msg_send![p, fill];
}
unsafe fn fill_oval(r: NSRect, c: *mut Object) {
    let _: () = msg_send![c, set];
    let p: *mut Object = msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: r];
    let _: () = msg_send![p, fill];
}

extern "C" fn scrub_draw(this: &mut Object, _c: Sel, _dirty: NSRect) {
    unsafe {
        let ui = match UI.get() {
            Some(u) => u,
            None => return,
        };
        let b = bounds_of(this as *mut Object);
        let (w, h) = (b.size.w, b.size.h);
        let p = &ui.pbar;
        let hot = p.hover.is_some() || p.dragging;
        let th = if hot { 8. } else { 5. };
        let cy = h / 2.;
        let x_played = (p.frac * w).clamp(0., w);
        let x_buf = (p.buffered * w).clamp(0., w);
        fill_rr(rect(0., cy - th / 2., w, th), th / 2., color_a(C_ON_MEDIA, 0.28));
        fill_rr(rect(0., cy - th / 2., x_buf, th), th / 2., color_a(C_ON_MEDIA, 0.50));
        fill_rr(rect(0., cy - th / 2., x_played, th), th / 2., color(C_ACCENT));
        let r = if hot { 9. } else { 7. };
        let kx = x_played.clamp(r, (w - r).max(r));
        fill_oval(rect(kx - r, cy - r, 2. * r, 2. * r), color(C_ON_MEDIA));
    }
}

/// Fraction of the track under an event's pointer, 0..1.
unsafe fn frac_at(this: *mut Object, ev: *mut Object) -> f64 {
    let loc: NSPoint = msg_send![ev, locationInWindow];
    let p: NSPoint = msg_send![this, convertPoint: loc fromView: nil()];
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
    let tm = CMTimeMakeWithSeconds(f * dur, 600);
    let _: () = msg_send![ui.player.get(), seekToTime: tm];
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

extern "C" fn scrub_down(this: &mut Object, _c: Sel, ev: *mut Object) {
    unsafe {
        let me = this as *mut Object;
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
extern "C" fn scrub_drag(this: &mut Object, _c: Sel, ev: *mut Object) {
    unsafe {
        let me = this as *mut Object;
        let f = frac_at(me, ev);
        seek_frac(f);
        set_hover(Some(f));
    }
}
extern "C" fn scrub_up(_this: &mut Object, _c: Sel, _ev: *mut Object) {
    if let Some(ui) = UI.get_mut() {
        ui.pbar.dragging = false;
    }
    redraw_scrub();
}
extern "C" fn scrub_moved(this: &mut Object, _c: Sel, ev: *mut Object) {
    unsafe {
        set_hover(Some(frac_at(this as *mut Object, ev)));
    }
}
extern "C" fn scrub_exited(_this: &mut Object, _c: Sel, _ev: *mut Object) {
    unsafe {
        if let Some(ui) = UI.get_mut() {
            if !ui.pbar.dragging {
                set_hover(None);
            }
        }
    }
}

// ---- menus ----------------------------------------------------------------------------------------

unsafe fn new_menu(title: &str) -> *mut Object {
    let m: *mut Object = msg_send![class!(NSMenu), alloc];
    let m: *mut Object = msg_send![m, initWithTitle: ns(title)];
    autorel(m)
}
unsafe fn menu_row(menu: *mut Object, title: &str, action: Sel, tag: isize, on: bool) {
    let it = menu_item(title, action, "");
    let _: () = msg_send![it, setTarget: UI.get().unwrap().ctl.get()];
    let _: () = msg_send![it, setTag: tag];
    let _: () = msg_send![it, setState: if on { 1isize } else { 0isize }];
    let _: () = msg_send![menu, addItem: it];
}
/// Opens a menu under its button. Self-test mode only counts the items.
unsafe fn present(menu: *mut Object, anchor: *mut Object) {
    let n: NSInteger = msg_send![menu, numberOfItems];
    UI.get_mut().unwrap().pbar.last_menu_items = n as usize;
    if offscreen_mode() {
        return;
    }
    let h = bounds_of(anchor).size.h;
    let _: BOOL = msg_send![menu, popUpMenuPositioningItem: nil() atLocation: NSPoint { x: 0.0, y: h } inView: anchor];
}

/// Subtitle (which = 0) or audio (which = 1) menu. Tags are `which * 1000 + index`, where index 0 is Off
/// and index i+1 is option i. Selection itself is `avkit::set_track`, shared with the Controls menu.
unsafe fn track_menu(which: usize, anchor: *mut Object) {
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
pub(crate) extern "C" fn show_sub_menu(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe { track_menu(0, sender) }
}
pub(crate) extern "C" fn show_audio_menu(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe { track_menu(1, sender) }
}
pub(crate) extern "C" fn pick_track(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        crate::platform::avkit::set_track((tag / 1000) as usize, (tag % 1000) as usize);
    }
}
pub(crate) extern "C" fn show_speed_menu(_t: &mut Object, _c: Sel, sender: *mut Object) {
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

// ---- Picture in Picture ------------------------------------------------------------------------------

/// Toggles Picture in Picture on the AVPlayerLayer under AVPlayerView. Not used in self-test mode.
pub(crate) extern "C" fn toggle_pip(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        if offscreen_mode() {
            return;
        }
        let ui = UI.get_mut().unwrap();
        if ui.pbar.pip_ctl.get().is_null() {
            let root: *mut Object = msg_send![ui.avview.get(), layer];
            let layer = find_player_layer(root, 0);
            if layer.is_null() {
                return;
            }
            let c: *mut Object = msg_send![class!(AVPictureInPictureController), alloc];
            let c: *mut Object = autorel(msg_send![c, initWithPlayerLayer: layer]);
            if c.is_null() {
                return;
            }
            ui.pbar.pip_ctl = Ptr::retained(c);
        }
        let c = ui.pbar.pip_ctl.get();
        let active: BOOL = msg_send![c, isPictureInPictureActive];
        if active == YES {
            let _: () = msg_send![c, stopPictureInPicture];
        } else {
            let _: () = msg_send![c, startPictureInPicture];
        }
    }
}

// ---- visibility -------------------------------------------------------------------------------------

/// Whether the bar is fully shown. Used by the self-test.
pub(crate) unsafe fn visible() -> bool {
    let ui = UI.get().unwrap();
    let a: f64 = msg_send![ui.pbar.bar.get(), alphaValue];
    a > 0.99
}
