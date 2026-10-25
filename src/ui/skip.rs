//! Skip buttons and the Up Next card, drawn over the video above the transport bar.
//!
//! `update` runs on every playback tick. It tracks which media segment playback is in,
//! shows "Skip Intro" (or Recap, Preview, Commercial) while inside one, and shows the
//! Up Next card at its trigger. When the countdown ends, or the episode does, `advance`
//! starts the next episode in the same player: the handover writes what the app already
//! writes at the end of an episode (stopped, marked watched) and then asks for the next one.
//! Per-playback state lives in `PlayingNow.skip`; the views live in `Ui.skip`.
#![allow(unsafe_code)]
use crate::app::messages::{MediaKind, PlaybackStart, UiCmd};
use crate::config::{set_skip_intros, skip_intros_for};
use crate::jellyfin::segments::{self, NextUp, Segment, SegmentKind, UpNextWindow};
use crate::jellyfin::urls::ticks_from_secs;
use crate::platform::appkit::*;
use crate::ui::player::{fade, toast};
use crate::ui::shell::is_hidden;
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, Sel, NO, YES};
use objc::{sel, sel_impl};
use std::time::Instant;

/// The skip button stays up this long after segment entry or the last pointer movement.
const BUTTON_IDLE: f64 = 8.0;
/// A skip seek is in flight for this long; the old position may still read back meanwhile.
const SEEK_SETTLE: f64 = 1.0;
const BUTTON_H: f64 = 36.;
const CARD_W: f64 = 340.;
const CARD_H: f64 = 112.;
const THUMB_W: f64 = 120.;
const THUMB_H: f64 = 68.;
/// Left edge of the card's text column: thumbnail, gap, padding.
const TEXT_X: f64 = SP3 + THUMB_W + SP3;
const BTN_PLAY_W: f64 = 92.;
const BTN_CANCEL_W: f64 = 72.;

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
}

impl SkipUi {
    pub fn new() -> SkipUi {
        let n = || Ptr(std::ptr::null_mut());
        SkipUi { pill: n(), button: n(), timer: n(), series_item: n(), card: n(), thumb: n(), title: n(), count: n(), play: n(), cancel: n() }
    }
}

/// Builds the skip button and the Up Next card inside the player host, above the transport bar.
pub(crate) unsafe fn build(ph: *mut Object, b: NSRect) {
    let ui = UI.get_mut().unwrap();

    // Up Next card: bottom right, just above the transport bar.
    let card = vev(MAT_HUD, BLEND_WITHIN);
    set_frame(card, rect(b.size.w - SP5 - CARD_W, PLAYER_CTRL_H + SP3, CARD_W, CARD_H));
    set_autoresize(card, MARGIN_MINX | MARGIN_MAXY);
    let cl = layer_of(card);
    set_corner(cl, R_CARD);
    let _: () = msg_send![cl, setMasksToBounds: YES];
    set_hidden(card, true);
    add_sub(ph, card);
    ui.skip.card = Ptr(card);

    let thumb: *mut Object = msg_send![class!(NSImageView), alloc];
    let thumb: *mut Object = autorel(msg_send![thumb, initWithFrame: rect(SP3, (CARD_H - THUMB_H) / 2., THUMB_W, THUMB_H)]);
    let _: () = msg_send![thumb, setImageScaling: 3isize];
    set_corner(layer_of(thumb), R_THUMB);
    add_sub(card, thumb);
    ui.skip.thumb = Ptr(thumb);

    let text_w = CARD_W - TEXT_X - SP3;
    let kicker = label("UP NEXT", T_CAPTION_EM, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(kicker, rect(TEXT_X, CARD_H - SP3 - 14., text_w, 14.));
    add_sub(card, kicker);
    let title = label("", T_BODY_EM, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(title, rect(TEXT_X, CARD_H - SP3 - 14. - SP1 - 18., text_w, 18.));
    add_sub(card, title);
    ui.skip.title = Ptr(title);
    let count = label("", T_CAPTION, C_ON_MEDIA2, ALIGN_LEFT);
    // Under the buttons (which are 26 pt tall, from SP3), and clear of the title above.
    set_frame(count, rect(TEXT_X, SP3 + 26. + SP2, text_w, 14.));
    add_sub(card, count);
    ui.skip.count = Ptr(count);

    let play: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Play Now")
        target: ui.ctl.get() action: sel!(jrUpNextPlay:)];
    let _: () = msg_send![play, setBezelStyle: 1isize];
    let _: () = msg_send![play, setKeyEquivalent: ns("\r")];
    set_frame(play, rect(TEXT_X, SP3, BTN_PLAY_W, 26.));
    add_sub(card, play);
    ui.skip.play = Ptr(play);
    let cancel: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel")
        target: ui.ctl.get() action: sel!(jrUpNextCancel:)];
    let _: () = msg_send![cancel, setBezelStyle: 1isize];
    set_frame(cancel, rect(TEXT_X + BTN_PLAY_W + SP2, SP3, BTN_CANCEL_W, 26.));
    add_sub(card, cancel);
    ui.skip.cancel = Ptr(cancel);

    // Skip button: a HUD pill holding a borderless button, like the toast.
    let pill = vev(MAT_HUD, BLEND_WITHIN);
    set_frame(pill, rect(b.size.w - SP5 - 160., PLAYER_CTRL_H + SP3, 160., BUTTON_H));
    set_autoresize(pill, MARGIN_MINX | MARGIN_MAXY);
    let pl = layer_of(pill);
    set_corner(pl, BUTTON_H / 2.);
    let _: () = msg_send![pl, setMasksToBounds: YES];
    set_hidden(pill, true);
    add_sub(ph, pill);
    ui.skip.pill = Ptr(pill);

    let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Skip Intro")
        target: ui.ctl.get() action: sel!(jrSkipSegment:)];
    let _: () = msg_send![btn, setBordered: NO];
    let img = symbol("forward.end.fill", 12., Weight::Semibold);
    if !img.is_null() {
        let _: () = msg_send![btn, setImage: img];
    }
    let _: () = msg_send![btn, setImagePosition: 7isize];
    let _: () = msg_send![btn, setContentTintColor: color(C_ON_MEDIA)];
    let _: () = msg_send![btn, setFont: font(T_BODY_EM)];
    let _: () = msg_send![btn, setToolTip: ns("Skip this part (S). Right-click for more.")];
    set_frame(btn, rect(0., 0., 160., BUTTON_H));
    set_autoresize(btn, RESIZE_W | RESIZE_H);
    add_sub(pill, btn);
    ui.skip.button = Ptr(btn);

    // Right-click menu on the button: the per-show intro setting.
    let menu: *mut Object = autorel(msg_send![class!(NSMenu), new]);
    let item: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let item: *mut Object = autorel(msg_send![item, initWithTitle: ns("Skip Intros Automatically for This Show")
        action: sel!(jrToggleSeriesSkip:) keyEquivalent: ns("")]);
    let _: () = msg_send![item, setTarget: ui.ctl.get()];
    let _: () = msg_send![menu, addItem: item];
    let _: () = msg_send![btn, setMenu: menu];
    ui.skip.series_item = Ptr(item);
    reset_views();
}

/// Called when an item starts. Loads its segments and next episode and hides the overlay until `update` shows it.
pub(crate) unsafe fn start(ps: &PlaybackStart) {
    let ui = UI.get_mut().unwrap();
    let video = ps.media_kind == MediaKind::Video;
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
        let t: *mut Object = msg_send![class!(NSTimer),
            scheduledTimerWithTimeInterval: 0.25f64 target: ui.ctl.get()
            selector: sel!(jrSkipTick:) userInfo: nil() repeats: YES];
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
    set_hidden(ui.skip.pill.get(), true);
    set_hidden(ui.skip.card.get(), true);
}

/// Show or hide a view. Showing fades in; hiding removes it at once so it cannot take clicks.
unsafe fn set_shown(v: *mut Object, on: bool) {
    if on {
        if is_hidden(v) {
            let _: () = msg_send![v, setAlphaValue: 0.0f64];
            set_hidden(v, false);
            fade(v, 1.0, M_FAST);
        }
    } else if !is_hidden(v) {
        set_hidden(v, true);
    }
}

/// Stacks the skip button above the card while the card is up, otherwise at the bar's top edge.
unsafe fn place_pill(pill_w: Option<f64>) {
    let ui = UI.get_mut().unwrap();
    let f = frame_of(ui.skip.pill.get());
    let w = pill_w.unwrap_or(f.size.w);
    let lift = if is_hidden(ui.skip.card.get()) { 0. } else { CARD_H + SP2 };
    let x = bounds_of(ui.player_host.get()).size.w - SP5 - w;
    set_frame(ui.skip.pill.get(), rect(x, PLAYER_CTRL_H + SP3 + lift, w, BUTTON_H));
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
    let enabled = if pn.skip.series_id.is_some() { YES } else { NO };
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
    if let Some(label) = skip_label {
        if label != st.button_label {
            st.button_label = label;
            let btn = ui.skip.button.get();
            let _: () = msg_send![btn, setTitle: ns(label)];
            // Size from the cell's own content (icon and title), then fit the button to the pill.
            let cell: *mut Object = msg_send![btn, cell];
            let content: NSSize = msg_send![cell, cellSize];
            let w = content.w.ceil() + 2. * SP4;
            place_pill(Some(w));
            set_frame(btn, rect(0., 0., w, BUTTON_H));
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
    let was_card = !is_hidden(ui.skip.card.get());
    set_shown(ui.skip.card.get(), card_on);
    if card_changed || was_card != card_on {
        place_pill(None);
    }
    set_shown(ui.skip.pill.get(), button_on);

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
    let ui = UI.get_mut().unwrap();
    let Some(pn) = ui.playing.as_mut() else { return false };
    if pn.skip.advancing {
        return true;
    }
    let Some(next) = pn.skip.next.clone() else { return false };
    if !pn.skip.autoplay || pn.skip.cancelled {
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

pub(crate) extern "C" fn skip_tick(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { update() }
}

pub(crate) extern "C" fn skip_segment(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { skip_current() }
}

pub(crate) extern "C" fn up_next_play(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        advance();
    }
}

pub(crate) extern "C" fn up_next_cancel(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(pn) = ui.playing.as_mut() {
            pn.skip.cancelled = true;
            pn.skip.card_shown = false;
        }
        set_shown(ui.skip.card.get(), false);
        place_pill(None);
    }
}

/// The per-show "skip intros" setting, from the button's menu. Saved at once; on turns the skip happens now if inside an intro.
pub(crate) extern "C" fn toggle_series_skip(_t: &mut Object, _c: Sel, _s: *mut Object) {
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
    !is_hidden(ui.skip.pill.get()) && !ui.skip.pill.get().is_null()
}
pub(crate) unsafe fn button_label() -> String {
    let ui = UI.get().unwrap();
    to_str(msg_send![ui.skip.button.get(), title])
}
pub(crate) unsafe fn card_visible() -> bool {
    let ui = UI.get().unwrap();
    !is_hidden(ui.skip.card.get())
}
pub(crate) unsafe fn card_text() -> String {
    let ui = UI.get().unwrap();
    format!("{} | {}", read_field(ui.skip.title.get()), read_field(ui.skip.count.get()))
}
pub(crate) unsafe fn auto_skip_on() -> bool {
    UI.get().and_then(|u| u.playing.as_ref()).map(|p| p.skip.auto_skip).unwrap_or(false)
}
