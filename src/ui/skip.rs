//! Skip buttons and the Up Next card, drawn over the video above the transport bar.
//!
//! `update` runs on every playback tick. It tracks which media segment playback is in,
//! shows "Skip Intro" (or Recap, Preview, Commercial) while inside one, and shows the
//! Up Next card at its trigger. When the countdown ends, or the episode does, `advance`
//! starts the next episode in the same player: the handover writes what the app already
//! writes at the end of an episode (stopped, marked watched) and then asks for the next one.
//! Per-playback state lives in `PlayingNow.skip`; the views live in `Ui.skip`.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::sync::atomic::{AtomicU64, Ordering};
use crate::app::messages::{MediaKind, PlaybackStart, UiCmd};
use crate::config::{set_skip_intros, skip_intros_for};
use crate::jellyfin::segments::{self, NextUp, Segment, SegmentKind, UpNextWindow};
use crate::jellyfin::urls::ticks_from_secs;
use crate::platform::appkit::*;
use crate::ui::player::{fade, toast};
use crate::ui::shell::is_hidden;
use crate::ui::theme::*;
use std::sync::OnceLock;
use std::time::Instant;

/// The skip button stays up this long after segment entry or the last pointer movement.
const BUTTON_IDLE: f64 = 8.0;
/// A skip seek is in flight for this long; the old position may still read back meanwhile.
const SEEK_SETTLE: f64 = 1.0;
/// Skip button height: above the 44 pt minimum for a pointer target.
const BUTTON_H: f64 = 48.;
/// Horizontal padding inside the skip button, either side of its label and icon.
const BTN_PAD: f64 = SP6;
/// Corner radius shared by the skip button, the Up Next card and the toast.
const PANEL_R: f64 = 12.;
/// Gap between the skip button and the right edge of the video.
const EDGE: f64 = SP8;
/// Gap between the skip button and the top of the transport bar.
const RISE: f64 = SP4;
const CARD_W: f64 = 360.;
/// 16:9 artwork across the card's top, then the title, the countdown and the buttons.
const ART_H: f64 = CARD_W * 9. / 16.;
const CARD_H: f64 = ART_H + SP3 + 20. + SP1 + 15. + SP3 + PILL_H + SP4;
const CARD_PAD: f64 = SP4;
/// Play Now pill: a white pill with a countdown ring at its left.
const PILL_H: f64 = 36.;
const RING_D: f64 = 22.;
const RING_X: f64 = SP3;
/// Left edge of the card's text column: thumbnail, gap, padding.
const BTN_PLAY_W: f64 = 140.;
const BTN_CANCEL_W: f64 = 72.;
/// Border and wash alphas (white over the HUD material): rest, hover, and pressed darkening.
const BORDER_REST: f64 = 0.20;
const BORDER_HOVER: f64 = 0.36;
const WASH_HOVER: f64 = 0.10;
const PRESS_DARKEN: f64 = 0.24;
/// Skip button label: a step above body text, so it reads as the primary control.
const T_SKIP: TypeStyle = TypeStyle { size: 17., weight: Weight::Semibold, mono: false };
static SKIP_BTN_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
static RING_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
/// Share of the Up Next countdown still to run, 0..1, as f64 bits. The ring view reads it when it draws.
static RING_FRAC: AtomicU64 = AtomicU64::new(0);

/// Playback-scoped state. Reset by `start` for every item.
#[derive(Default)]
pub struct SkipState {
    pub segments: Vec<Segment>,
    pub series_id: Option<String>,
    pub next: Option<NextUp>,
    /// Settings at the start of this item: play the next episode when this one ends.
    pub autoplay: bool,
    /// Skip intros as soon as playback enters one (per-show setting, or the global default).
    pub auto_skip: bool,
    /// Index into `segments` of the segment playback is inside, as of the last update.
    pub current: Option<usize>,
    /// When `current` was entered; the button shows for `BUTTON_IDLE` after this.
    pub entered: Option<Instant>,
    /// Segment changes are ignored until this instant (a skip seek is in flight).
    pub settle_until: Option<Instant>,
    /// Up Next timing, worked out once the duration is known.
    pub window: Option<UpNextWindow>,
    pub card_shown: bool,
    /// "Cancel" was pressed: no Up Next for this episode.
    pub cancelled: bool,
    /// The handover to the next episode has started.
    pub advancing: bool,
    /// Whole seconds last drawn in the countdown label (-1: none yet).
    pub countdown_shown: i64,
    /// The card's thumbnail has been set.
    pub image_applied: bool,
    /// Label last set on the skip button.
    pub button_label: &'static str,
}

/// Views of the overlay, built once with the player chrome.
pub struct SkipUi {
    pub pill: Ptr,
    pub button: Ptr,
    /// 0.25 s timer that drives `update` while an item has segments or a next episode.
    pub timer: Ptr,
    pub series_item: Ptr,
    pub card: Ptr,
    pub thumb: Ptr,
    pub title: Ptr,
    pub count: Ptr,
    pub play: Ptr,
    pub cancel: Ptr,
    /// The countdown ring on the Play Now pill.
    pub ring: Ptr,
}

impl SkipUi {
    pub fn new() -> SkipUi {
        let n = || Ptr(std::ptr::null_mut());
        SkipUi { pill: n(), button: n(), timer: n(), series_item: n(), card: n(), thumb: n(), title: n(), count: n(), play: n(), cancel: n(), ring: n() }
    }
}

/// A frosted HUD panel: dark vibrancy, active state, continuous rounded corners, 1 px light border.
/// Player overlays stay light-on-dark in both appearances, so the panel pins a dark appearance.
pub(crate) unsafe fn hud_panel(r: f64) -> *mut AnyObject {
    let v = vev(MAT_HUD, BLEND_WITHIN);
    let _: () = msg_send![v, setState: 1i64]; // active: the panel stays frosted when the window is inactive
    let dark: *mut AnyObject = msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameDarkAqua")];
    let _: () = msg_send![v, setAppearance: dark];
    let l = layer_of(v);
    set_corner(l, r);
    let _: () = msg_send![l, setMasksToBounds: Bool::YES];
    let _: () = msg_send![l, setBorderWidth: 1.0f64];
    let _: () = msg_send![l, setBorderColor: cg_white(BORDER_REST)];
    v
}

/// White, or black, at `a` alpha, as a CGColor for a layer.
unsafe fn cg_rgba(r: f64, a: f64) -> *mut CGColor {
    let c: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: r, green: r, blue: r, alpha: a];
    msg_send![c, CGColor]
}
unsafe fn cg_white(a: f64) -> *mut CGColor {
    cg_rgba(1.0, a)
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Rest,
    Hover,
    Down,
}

/// Paints the skip button for a pointer state: its own layer carries the wash, the panel carries the border.
unsafe fn paint_button(btn: *mut AnyObject, state: Press) {
    let (wash, border) = match state {
        Press::Rest => (cg_white(0.0), BORDER_REST),
        Press::Hover => (cg_white(WASH_HOVER), BORDER_HOVER),
        Press::Down => (cg_rgba(0.0, PRESS_DARKEN), BORDER_REST),
    };
    let _: () = msg_send![layer_of(btn), setBackgroundColor: wash];
    let pill: *mut AnyObject = msg_send![btn, superview];
    if !pill.is_null() {
        let _: () = msg_send![layer_of(pill), setBorderColor: cg_white(border)];
    }
}

/// The skip button: a borderless button that does its own hover, press and pointing-hand cursor.
pub(crate) unsafe fn register_skip_classes() {
    let mut d = ClassBuilder::new(c"JRSkipButton", class!(NSButton)).unwrap();
    d.add_method(sel!(mouseEntered:), btn_enter as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    d.add_method(sel!(mouseExited:), btn_exit as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    d.add_method(sel!(mouseDown:), btn_down as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    d.add_method(sel!(resetCursorRects), btn_cursor as extern "C" fn(*mut AnyObject, Sel));
    SKIP_BTN_CLASS.set(d.register()).ok();

    // Countdown ring on the Play Now pill: an empty track with the time left as an arc.
    let mut r = ClassBuilder::new(c"JRCountdownRing", class!(NSView)).unwrap();
    r.add_method(sel!(drawRect:), ring_draw as extern "C" fn(*mut AnyObject, Sel, NSRect));
    r.add_method(sel!(acceptsFirstMouse:), ring_pass as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool);
    RING_CLASS.set(r.register()).ok();
}
extern "C" fn btn_enter(this: *mut AnyObject, _c: Sel, _ev: *mut AnyObject) {
    unsafe { paint_button(this, Press::Hover) }
}
extern "C" fn btn_exit(this: *mut AnyObject, _c: Sel, _ev: *mut AnyObject) {
    unsafe { paint_button(this, Press::Rest) }
}
extern "C" fn btn_down(this: *mut AnyObject, _c: Sel, ev: *mut AnyObject) {
    unsafe {
        paint_button(this, Press::Down);
        let _: () = msg_send![super(this, class!(NSButton)), mouseDown: ev];
        // After the click: hover if the pointer is still over the button, otherwise rest.
        let win: *mut AnyObject = msg_send![this, window];
        let mut inside = false;
        if !win.is_null() {
            let m: NSPoint = msg_send![win, mouseLocationOutsideOfEventStream];
            let p: NSPoint = msg_send![this, convertPoint: m fromView: nil()];
            let b: NSRect = msg_send![this, bounds];
            inside = p.x >= b.origin.x && p.x <= b.origin.x + b.size.w && p.y >= b.origin.y && p.y <= b.origin.y + b.size.h;
        }
        paint_button(this, if inside { Press::Hover } else { Press::Rest });
    }
}
extern "C" fn btn_cursor(this: *mut AnyObject, _c: Sel) {
    unsafe {
        let cur: *mut AnyObject = msg_send![class!(NSCursor), pointingHandCursor];
        let b: NSRect = msg_send![this, bounds];
        let _: () = msg_send![this, addCursorRect: b cursor: cur];
    }
}

/// Builds the skip button and the Up Next card inside the player host, above the transport bar.
pub(crate) unsafe fn build(ph: *mut AnyObject, b: NSRect) {
    let ui = UI.get_mut().unwrap();

    // Up Next card: bottom right, inset like the skip button, just above the transport bar.
    // Top to bottom: 16:9 artwork, the episode title, the countdown, then Play Now and Cancel.
    let card = hud_panel(PANEL_R);
    set_frame(card, rect(b.size.w - EDGE - CARD_W, PLAYER_CTRL_H + RISE, CARD_W, CARD_H));
    set_autoresize(card, MARGIN_MINX | MARGIN_MAXY);
    set_hidden(card, true);
    add_sub(ph, card);
    ui.skip.card = Ptr(card);

    let thumb: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let thumb: *mut AnyObject = autorel(msg_send![thumb, initWithFrame: rect(0., CARD_H - ART_H, CARD_W, ART_H)]);
    let _: () = msg_send![thumb, setImageScaling: 3isize];
    let tl = layer_of(thumb);
    set_corner(tl, PANEL_R);
    let _: () = msg_send![tl, setMasksToBounds: Bool::YES];
    let _: () = msg_send![tl, setMaskedCorners: 12usize]; // top corners only: the card's frame rounds the rest
    add_sub(card, thumb);
    ui.skip.thumb = Ptr(thumb);

    let text_w = CARD_W - 2. * CARD_PAD;
    let title_y = CARD_H - ART_H - SP3 - 20.;
    let title = label("", T_HEADLINE, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(title, rect(CARD_PAD, title_y, text_w, 20.));
    add_sub(card, title);
    ui.skip.title = Ptr(title);
    let count = label("", T_CALLOUT, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(count, rect(CARD_PAD, title_y - SP1 - 15., text_w, 15.));
    add_sub(card, count);
    ui.skip.count = Ptr(count);

    // Play Now: a white pill with the countdown ring at its left. A clear button over the pill takes the click.
    let pill_y = CARD_PAD;
    let pill: *mut AnyObject = msg_send![class!(NSView), alloc];
    let pill: *mut AnyObject = autorel(msg_send![pill, initWithFrame: rect(CARD_PAD, pill_y, BTN_PLAY_W, PILL_H)]);
    let _: () = msg_send![pill, setWantsLayer: Bool::YES];
    let pl = layer_of(pill);
    set_corner(pl, PILL_H / 2.);
    let _: () = msg_send![pl, setBackgroundColor: cg_a(C_ON_MEDIA, 1.0)];
    add_sub(card, pill);
    let ring: *mut AnyObject = msg_send![*RING_CLASS.get().unwrap(), alloc];
    let ring: *mut AnyObject = autorel(msg_send![ring, initWithFrame: rect(RING_X, (PILL_H - RING_D) / 2., RING_D, RING_D)]);
    let _: () = msg_send![ring, setWantsLayer: Bool::YES];
    add_sub(pill, ring);
    ui.skip.ring = Ptr(ring);
    let glyph = symbol("play.fill", 8., Weight::Semibold);
    let gv: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let gv: *mut AnyObject = autorel(msg_send![gv, initWithFrame: rect(RING_X + (RING_D - 9.) / 2. + 1., (PILL_H - 9.) / 2., 9., 9.)]);
    let _: () = msg_send![gv, setImage: glyph];
    let _: () = msg_send![gv, setContentTintColor: black_color()];
    add_sub(pill, gv);
    let pill_label = label("Play Now", T_BODY_EM, C_ON_MEDIA, ALIGN_CENTER);
    let _: () = msg_send![pill_label, setTextColor: black_color()];
    let lx = RING_X + RING_D + SP2;
    set_frame(pill_label, rect(lx, (PILL_H - 18.) / 2., BTN_PLAY_W - lx - SP3, 18.));
    add_sub(pill, pill_label);
    let play: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: sel!(jrUpNextPlay:)];
    let _: () = msg_send![play, setBordered: Bool::NO];
    let _: () = msg_send![play, setKeyEquivalent: ns("\r")];
    let _: () = msg_send![play, setFocusRingType: 1usize];
    let _: () = msg_send![play, setToolTip: ns("Play now (Return)")];
    set_frame(play, rect(0., 0., BTN_PLAY_W, PILL_H));
    add_sub(pill, play);
    ui.skip.play = Ptr(play);
    // Cancel: text only, beside the pill.
    let cancel: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel"), target: ui.ctl.get(), action: sel!(jrUpNextCancel:)];
    let _: () = msg_send![cancel, setBordered: Bool::NO];
    let _: () = msg_send![cancel, setFont: font(T_BODY_EM)];
    let _: () = msg_send![cancel, setContentTintColor: color(C_ON_MEDIA2)];
    let _: () = msg_send![cancel, setFocusRingType: 1usize];
    set_frame(cancel, rect(CARD_PAD + BTN_PLAY_W + SP3, pill_y + (PILL_H - 22.) / 2., BTN_CANCEL_W, 22.));
    add_sub(card, cancel);
    ui.skip.cancel = Ptr(cancel);

    // Skip button: a HUD panel holding the button, inset 1 pt so the panel's border shows around it.
    let pill = hud_panel(PANEL_R);
    set_frame(pill, rect(b.size.w - EDGE - 160., PLAYER_CTRL_H + RISE, 160., BUTTON_H));
    set_autoresize(pill, MARGIN_MINX | MARGIN_MAXY);
    set_hidden(pill, true);
    add_sub(ph, pill);
    ui.skip.pill = Ptr(pill);

    let cls = *SKIP_BTN_CLASS.get().unwrap();
    let btn: *mut AnyObject = msg_send![cls, alloc];
    let btn: *mut AnyObject = autorel(msg_send![btn, initWithFrame: rect(1., 1., 158., BUTTON_H - 2.)]);
    let _: () = msg_send![btn, setTitle: ns("Skip Intro")];
    let _: () = msg_send![btn, setTarget: ui.ctl.get()];
    let _: () = msg_send![btn, setAction: sel!(jrSkipSegment:)];
    let _: () = msg_send![btn, setBordered: Bool::NO];
    let _: () = msg_send![btn, setFocusRingType: 1isize]; // none: the press and hover states already show it
    let _: () = msg_send![btn, setFont: font(T_SKIP)];
    let _: () = msg_send![btn, setImageHugsTitle: Bool::YES];
    // Trailing symbol, at the label's weight.
    let img = symbol("forward.end.fill", 15., Weight::Semibold);
    if !img.is_null() {
        let _: () = msg_send![btn, setImage: img];
    }
    let _: () = msg_send![btn, setImagePosition: 3isize]; // NSImageRight: the symbol trails the label
    let _: () = msg_send![btn, setContentTintColor: color(C_ON_MEDIA)];
    let _: () = msg_send![btn, setToolTip: ns("Skip this part (S). Right-click for more.")];
    let bl = layer_of(btn);
    set_corner(bl, PANEL_R - 1.);
    let _: () = msg_send![bl, setMasksToBounds: Bool::YES];
    set_autoresize(btn, RESIZE_W | RESIZE_H);
    // Hover is tracked over the whole button; the cursor comes from resetCursorRects.
    let ta: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut AnyObject = msg_send![ta,
        initWithRect: rect(0., 0., 0., 0.)
        options: (0x01usize | 0x80usize | 0x200usize) owner: btn userInfo: nil()];
    let _: () = msg_send![btn, addTrackingArea: ta];
    autorel(ta);
    add_sub(pill, btn);
    ui.skip.button = Ptr(btn);

    // Right-click menu on the button: the per-show intro setting.
    let menu: *mut AnyObject = autorel(msg_send![class!(NSMenu), new]);
    let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
    let item: *mut AnyObject = autorel(msg_send![item, initWithTitle: ns("Skip Intros Automatically for This Show"), action: sel!(jrToggleSeriesSkip:), keyEquivalent: ns("")]);
    let _: () = msg_send![item, setTarget: ui.ctl.get()];
    let _: () = msg_send![menu, addItem: item];
    let _: () = msg_send![btn, setMenu: menu];
    ui.skip.series_item = Ptr(item);
    reset_views();
}

/// Called when an item starts. Loads its segments and next episode and hides the overlay until `update` shows it.
pub(crate) unsafe fn start(ps: &PlaybackStart) {
    let video = ps.media_kind == MediaKind::Video;
    // The Episodes panel takes this item's season (and closes, as the new item starts).
    let season = if video { ps.season_episodes.clone() } else { Vec::new() };
    crate::ui::episodes::set_items(season, &ps.item_id, ps.season_number);
    let ui = UI.get_mut().unwrap();
    let series_id = if video { ps.series_id.clone() } else { None };
    let auto_skip = series_id
        .as_deref()
        .map(|id| skip_intros_for(id, ui.cfg.settings.skip_intros_default))
        .unwrap_or(false);
    let st = SkipState {
        segments: if video { ps.segments.clone() } else { Vec::new() },
        series_id,
        next: if video { ps.next_up.clone() } else { None },
        autoplay: ui.cfg.settings.autoplay_next,
        auto_skip,
        countdown_shown: -1,
        ..Default::default()
    };
    let wants_timer = !st.segments.is_empty() || st.next.is_some();
    if let Some(pn) = ui.playing.as_mut() {
        pn.skip = st;
    }
    reset_views();
    sync_series_item();
    set_timer(wants_timer);
    // The episode buttons follow the new item's series.
    crate::ui::playerbar::relayout();
}

/// Runs `update` four times a second while the overlay may need to change. The playback
/// tick itself only runs once a second for video.
unsafe fn set_timer(on: bool) {
    let ui = UI.get_mut().unwrap();
    if !ui.skip.timer.get().is_null() {
        let _: () = msg_send![ui.skip.timer.get(), invalidate];
        ui.skip.timer = Ptr(nil());
    }
    if on {
        let t: *mut AnyObject = msg_send![class!(NSTimer),
            scheduledTimerWithTimeInterval: 0.25f64, target: ui.ctl.get(), selector: sel!(jrSkipTick:), userInfo: nil(), repeats: Bool::YES];
        ui.skip.timer = Ptr(t);
    }
}

/// Hides the overlay and forgets its state (playback stopped).
pub(crate) unsafe fn stop() {
    if let Some(pn) = UI.get_mut().and_then(|u| u.playing.as_mut()) {
        pn.skip = SkipState::default();
    }
    reset_views();
    set_timer(false);
}

/// Hide both views at once, without animation.
unsafe fn reset_views() {
    let ui = UI.get_mut().unwrap();
    for v in [ui.skip.pill.get(), ui.skip.card.get()] {
        cancel_hide(v);
        set_hidden(v, true);
    }
}

/// A view that is visible to the pointer: unhidden and not fading out.
unsafe fn shown(v: *mut AnyObject) -> bool {
    if v.is_null() || is_hidden(v) {
        return false;
    }
    let a: f64 = msg_send![v, alphaValue];
    a > 0.01
}

unsafe fn cancel_hide(v: *mut AnyObject) {
    let ctl = UI.get().unwrap().ctl.get();
    let _: () = msg_send![class!(NSObject), cancelPreviousPerformRequestsWithTarget: ctl
        selector: sel!(jrSkipHideDone:) object: v];
}

/// Show or hide a view. Both fade over M_STD. A hidden view is then removed, so it cannot take clicks.
unsafe fn set_shown(v: *mut AnyObject, on: bool) {
    if v.is_null() {
        return;
    }
    cancel_hide(v);
    if on {
        if is_hidden(v) {
            let _: () = msg_send![v, setAlphaValue: 0.0f64];
            set_hidden(v, false);
        }
        let a: f64 = msg_send![v, alphaValue];
        if a < 1.0 {
            fade(v, 1.0, M_STD);
        }
    } else if !is_hidden(v) {
        let a: f64 = msg_send![v, alphaValue];
        if a > 0.0 {
            fade(v, 0.0, M_STD);
            let ctl = UI.get().unwrap().ctl.get();
            let _: () = msg_send![ctl, performSelector: sel!(jrSkipHideDone:) withObject: v afterDelay: M_STD];
        } else {
            set_hidden(v, true);
        }
    }
}

/// Runs once a fade-out has finished: removes the view unless it was shown again meanwhile.
pub(crate) extern "C" fn skip_hide_done(_t: *mut AnyObject, _c: Sel, v: *mut AnyObject) {
    unsafe {
        let a: f64 = msg_send![v, alphaValue];
        if a < 0.01 {
            set_hidden(v, true);
        }
    }
}

/// Places the skip button at the bottom right, EDGE in from the right and RISE above the transport bar.
/// While the Up Next card is up, the button lifts above it.
unsafe fn place_pill(pill_w: Option<f64>, card_on: bool) {
    let ui = UI.get_mut().unwrap();
    let f = frame_of(ui.skip.pill.get());
    let w = pill_w.unwrap_or(f.size.w);
    let lift = if card_on { CARD_H + SP2 } else { 0. };
    let x = bounds_of(ui.player_host.get()).size.w - EDGE - w - crate::ui::episodes::right_inset();
    set_frame(ui.skip.pill.get(), rect(x, PLAYER_CTRL_H + RISE + lift, w, BUTTON_H));
}

/// Moves the Up Next card and the skip button clear of the Episodes panel (or back), at the current state.
pub(crate) unsafe fn place_now() {
    let ui = UI.get_mut().unwrap();
    let card = ui.skip.card.get();
    let x = bounds_of(ui.player_host.get()).size.w - EDGE - CARD_W - crate::ui::episodes::right_inset();
    set_frame(card, rect(x, PLAYER_CTRL_H + RISE, CARD_W, CARD_H));
    place_pill(None, card_visible());
}

/// Fills the countdown ring: `frac` of the countdown still to run.
unsafe fn set_ring(frac: f64) {
    let f = frac.clamp(0., 1.);
    let prev = f64::from_bits(RING_FRAC.swap(f.to_bits(), Ordering::Relaxed));
    if (prev - f).abs() > 0.001 {
        let ui = UI.get().unwrap();
        let _: () = msg_send![ui.skip.ring.get(), setNeedsDisplay: Bool::YES];
    }
}

/// Draws the countdown ring: a faint full track and the time left as a dark arc, clockwise from the top.
extern "C" fn ring_draw(this: *mut AnyObject, _c: Sel, _dirty: NSRect) {
    unsafe {
        let b = bounds_of(this);
        let frac = f64::from_bits(RING_FRAC.load(Ordering::Relaxed));
        let r = (b.size.w.min(b.size.h) / 2.) - 1.5;
        let c = NSPoint { x: b.size.w / 2., y: b.size.h / 2. };
        let ink: *mut AnyObject = msg_send![class!(NSColor), blackColor];
        let track: *mut AnyObject = msg_send![ink, colorWithAlphaComponent: 0.18f64];
        let _: () = msg_send![track, setStroke];
        let p: *mut AnyObject = msg_send![class!(NSBezierPath), bezierPathWithOvalInRect: rect(c.x - r, c.y - r, 2. * r, 2. * r)];
        let _: () = msg_send![p, setLineWidth: 2.5f64];
        let _: () = msg_send![p, stroke];
        if frac > 0.001 {
            let _: () = msg_send![ink, setStroke];
            let arc: *mut AnyObject = msg_send![class!(NSBezierPath), bezierPath];
            let _: () = msg_send![arc, setLineWidth: 2.5f64];
            let _: () = msg_send![arc, setLineCapStyle: 1isize];
            let end = 90.0 - 360.0 * frac;
            let _: () = msg_send![arc, appendBezierPathWithArcWithCenter: c, radius: r, startAngle: 90.0f64, endAngle: end, clockwise: Bool::YES];
            let _: () = msg_send![arc, stroke];
        }
    }
}

extern "C" fn ring_pass(_t: *mut AnyObject, _c: Sel, _e: *mut AnyObject) -> Bool {
    Bool::YES
}

/// Black, for the text and glyph on the white Play Now pill.
unsafe fn black_color() -> *mut AnyObject {
    msg_send![class!(NSColor), blackColor]
}

/// Keeps the context-menu item in step with the current item's show.
unsafe fn sync_series_item() {
    let ui = UI.get_mut().unwrap();
    let Some(pn) = ui.playing.as_ref() else { return };
    let item = ui.skip.series_item.get();
    if item.is_null() {
        return;
    }
    let on: NSInteger = pn.skip.auto_skip as NSInteger;
    let _: () = msg_send![item, setState: on];
    let enabled = if pn.skip.series_id.is_some() { Bool::YES } else { Bool::NO };
    let _: () = msg_send![item, setEnabled: enabled];
}

/// Seek the current item to `secs`.
unsafe fn seek(secs: f64) {
    crate::platform::avkit::seek_to_secs(secs);
}

pub(crate) unsafe fn position() -> f64 {
    let ui = UI.get_mut().unwrap();
    let t: CMTime = msg_send![ui.player.get(), currentTime];
    t.seconds()
}

/// Per-tick work: segment entry and exit, the skip button, auto-skip, and the Up Next card.
/// Cheap when there is nothing to show.
pub(crate) unsafe fn update() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    if ui.playing.is_none() || ui.pitem.get().is_null() {
        return;
    }
    let pos = position();
    let d: CMTime = msg_send![ui.pitem.get(), duration];
    let dur = if d.seconds() > 0.0 { d.seconds() } else { ui.playing.as_ref().map(|p| p.duration_secs).unwrap_or(0.) };
    let mouse_age = ui.last_mouse.map(|t| t.elapsed().as_secs_f64()).unwrap_or(f64::MAX);
    let pn = ui.playing.as_mut().unwrap();
    let st = &mut pn.skip;
    if st.segments.is_empty() && st.next.is_none() {
        return;
    }

    // Segment entry and exit. Auto-skip seeks to the end of an intro entered from outside.
    let now = Instant::now();
    let settling = st.settle_until.map(|u| now < u).unwrap_or(false);
    let cur = if settling { None } else { segments::current(&st.segments, pos) };
    let mut auto_skipped = false;
    if cur != st.current && !settling {
        st.current = cur;
        st.entered = cur.map(|_| now);
        if let Some(i) = cur {
            let seg = st.segments[i];
            if seg.kind == SegmentKind::Intro && st.auto_skip {
                auto_skipped = true;
                st.current = None;
                st.settle_until = Some(now + std::time::Duration::from_secs_f64(SEEK_SETTLE));
                seek(seg.end);
            }
        }
    }
    let cur_seg: Option<Segment> = st.current.map(|i| st.segments[i]);
    let skip_label = cur_seg.and_then(|s| s.kind.skip_label());
    let entered_age = st.entered.map(|t| t.elapsed().as_secs_f64()).unwrap_or(f64::MAX);
    let button_on = skip_label.is_some() && !auto_skipped && (entered_age < BUTTON_IDLE || mouse_age < BUTTON_IDLE);
    let card_before = st.card_shown && !st.cancelled && !st.advancing;
    if let Some(label) = skip_label {
        if label != st.button_label {
            st.button_label = label;
            let btn = ui.skip.button.get();
            let _: () = msg_send![btn, setTitle: ns(label)];
            // Size from the cell's own content (icon and title) plus the padding, then fit the panel to it.
            let cell: *mut AnyObject = msg_send![btn, cell];
            let content: NSSize = msg_send![cell, cellSize];
            let w = (content.w.ceil() + 2. * BTN_PAD + 2.).max(BUTTON_H);
            place_pill(Some(w), card_before);
            set_frame(btn, rect(1., 1., w - 2., BUTTON_H - 2.));
        }
    }

    // Up Next: the card at its trigger, the handover at its play time or the end.
    let mut want_advance = false;
    let mut card_changed = false;
    if st.next.is_some() && st.autoplay && !st.cancelled && !st.advancing && dur > 0.0 {
        if st.window.is_none() {
            st.window = segments::up_next_window(dur, segments::outro(&st.segments));
        }
        if let Some(w) = st.window {
            if !st.card_shown && pos >= w.trigger {
                st.card_shown = true;
                st.countdown_shown = -1;
                card_changed = true;
            }
            if st.card_shown {
                let left = (w.play_at - pos).ceil().max(0.) as i64;
                let span = (w.play_at - w.trigger).max(0.001);
                set_ring((w.play_at - pos) / span);
                if left != st.countdown_shown {
                    st.countdown_shown = left;
                    let text = if left == 1 { "Playing in 1 second".to_string() } else { format!("Playing in {} seconds", left) };
                    set_text(ui.skip.count.get(), &text);
                }
                if pos >= w.play_at {
                    want_advance = true;
                }
            }
        }
        if let Some(n) = st.next.as_ref() {
            if st.card_shown && !st.image_applied {
                let key = n.image_key;
                let img = key.and_then(|k| ui.poster_cache.get(&k).copied());
                if let Some(img) = img {
                    let _: () = msg_send![ui.skip.thumb.get(), setImage: img];
                    st.image_applied = true;
                }
            }
            if card_changed {
                set_text(ui.skip.title.get(), &card_title(n));
            }
        }
    }

    let card_on = st.card_shown && !st.cancelled && !st.advancing;
    set_shown(ui.skip.card.get(), card_on);
    set_shown(ui.skip.pill.get(), button_on);
    place_pill(None, card_on);

    if auto_skipped {
        toast("Skipped intro", Some("forward.end"), false);
    }
    if want_advance {
        advance();
    }
}

fn card_title(n: &NextUp) -> String {
    let code = n.code();
    if code.is_empty() {
        n.name.clone()
    } else {
        format!("{} · {}", code, n.name)
    }
}

/// Skip the segment playback is inside (button, or S). Seeks to its end.
pub(crate) unsafe fn skip_current() {
    let ui = UI.get_mut().unwrap();
    if ui.playing.is_none() {
        return;
    }
    let pos = position();
    let pn = ui.playing.as_mut().unwrap();
    let target = pn
        .skip
        .segments
        .iter()
        .find(|s| pos >= s.start && pos < s.end && s.kind.skip_label().is_some())
        .map(|s| s.end);
    match target {
        Some(end) => {
            pn.skip.current = None;
            pn.skip.settle_until = Some(Instant::now() + std::time::Duration::from_secs_f64(SEEK_SETTLE));
            set_shown(ui.skip.pill.get(), false);
            seek(end);
        }
        None => toast("Nothing to skip here", Some("forward"), false),
    }
}

/// Hands over to the next episode: the end-of-episode writes, then the request for the next one.
/// Returns false when there is nothing to hand over to (no next episode, autoplay off, or cancelled).
pub(crate) unsafe fn advance() -> bool {
    hand_over(false)
}

/// The Next Episode button and N: plays the next episode now, whatever Autoplay and Cancel say.
pub(crate) extern "C" fn next_episode(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        hand_over(true);
    }
}

/// Hands playback to the next episode. `force` skips the Autoplay and Cancel checks that the automatic
/// handover makes; the manual Next Episode action is always allowed when there is a next episode.
unsafe fn hand_over(force: bool) -> bool {
    let ui = UI.get_mut().unwrap();
    let Some(pn) = ui.playing.as_mut() else { return false };
    if pn.skip.advancing {
        return true;
    }
    let Some(next) = pn.skip.next.clone() else { return false };
    if !force && (!pn.skip.autoplay || pn.skip.cancelled) {
        return false;
    }
    pn.skip.advancing = true;
    pn.skip.card_shown = false;
    let item_id = pn.item_id.clone();
    let end_ticks = ticks_from_secs(pn.duration_secs);
    let _ = ui.ui_tx.send(UiCmd::Stopped { item_id: item_id.clone(), ticks: end_ticks });
    let _ = ui.ui_tx.send(UiCmd::MarkPlayed { item_id, played: true });
    let _ = ui.ui_tx.send(UiCmd::Play { item_id: next.id });
    set_shown(ui.skip.card.get(), false);
    set_shown(ui.skip.pill.get(), false);
    true
}

// ---- actions ------------------------------------------------------------------------------

pub(crate) extern "C" fn skip_tick(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { update() }
}

pub(crate) extern "C" fn skip_segment(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { skip_current() }
}

pub(crate) extern "C" fn up_next_play(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        advance();
    }
}

pub(crate) extern "C" fn up_next_cancel(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(pn) = ui.playing.as_mut() {
            pn.skip.cancelled = true;
            pn.skip.card_shown = false;
        }
        set_shown(ui.skip.card.get(), false);
        place_pill(None, false);
    }
}

/// The per-show "skip intros" setting, from the button's menu. Saved at once; on turns the skip happens now if inside an intro.
pub(crate) extern "C" fn toggle_series_skip(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let Some(pn) = ui.playing.as_mut() else { return };
        let Some(id) = pn.skip.series_id.clone() else { return };
        let on = !pn.skip.auto_skip;
        if set_skip_intros(&id, on).is_err() {
            toast("Couldn't save that setting", Some("exclamationmark.triangle"), true);
            return;
        }
        pn.skip.auto_skip = on;
        sync_series_item();
        if on {
            let pos = position();
            let inside_intro = UI
                .get()
                .and_then(|u| u.playing.as_ref())
                .map(|p| p.skip.segments.iter().any(|s| s.kind == SegmentKind::Intro && pos >= s.start && pos < s.end))
                .unwrap_or(false);
            toast("Skipping intros for this show", Some("forward.end"), false);
            if inside_intro {
                skip_current();
            }
        } else {
            toast("Intros won't be skipped for this show", Some("forward.end"), false);
        }
    }
}

/// Test hooks (self-test): what the overlay shows right now.
pub(crate) unsafe fn button_visible() -> bool {
    let ui = UI.get().unwrap();
    shown(ui.skip.pill.get())
}
pub(crate) unsafe fn button_label() -> String {
    let ui = UI.get().unwrap();
    to_str(msg_send![ui.skip.button.get(), title])
}
/// Geometry of the skip button, for the self-test: sizes in pt, and the gaps to the video's edges.
pub(crate) struct ButtonMetrics {
    pub pill_h: f64,
    pub btn_h: f64,
    pub label_fits: bool,
    pub right_gap: f64,
    pub bottom_gap: f64,
}
pub(crate) unsafe fn button_metrics() -> ButtonMetrics {
    let ui = UI.get().unwrap();
    let pill = frame_of(ui.skip.pill.get());
    let btn: *mut AnyObject = ui.skip.button.get();
    let bf: NSRect = msg_send![btn, frame];
    let cell: *mut AnyObject = msg_send![btn, cell];
    let content: NSSize = msg_send![cell, cellSize];
    let host = bounds_of(ui.player_host.get());
    ButtonMetrics {
        pill_h: pill.size.h,
        btn_h: bf.size.h,
        // The label fits when its natural width leaves the padding on both sides.
        label_fits: content.w + 2. * BTN_PAD <= bf.size.w + 0.5,
        right_gap: host.size.w - (pill.origin.x + pill.size.w),
        bottom_gap: pill.origin.y - PLAYER_CTRL_H,
    }
}
pub(crate) unsafe fn card_visible() -> bool {
    let ui = UI.get().unwrap();
    shown(ui.skip.card.get())
}
/// The Up Next title's natural width fits its label, so the name is not cut off.
pub(crate) unsafe fn card_title_fits() -> bool {
    let ui = UI.get().unwrap();
    let t = ui.skip.title.get();
    let cell: *mut AnyObject = msg_send![t, cell];
    let content: NSSize = msg_send![cell, cellSize];
    let f: NSRect = msg_send![t, frame];
    content.w <= f.size.w + 0.5
}
pub(crate) unsafe fn card_text() -> String {
    let ui = UI.get().unwrap();
    format!("{} | {}", read_field(ui.skip.title.get()), read_field(ui.skip.count.get()))
}
pub(crate) unsafe fn auto_skip_on() -> bool {
    UI.get().and_then(|u| u.playing.as_ref()).map(|p| p.skip.auto_skip).unwrap_or(false)
}
