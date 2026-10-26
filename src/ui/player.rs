//! Player chrome: gradient top bar (auto-hides), toast, playback-info panel and
//! the buffering backdrop. Everything lives inside `player_host`, which is
//! forced to the dark appearance so white-on-video text is always legible.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::platform::appkit::*;
use crate::ui::shell::{fix_height, is_hidden};
use crate::ui::theme::*;
use std::sync::OnceLock;

pub(crate) static PLAYER_HOST_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
pub(crate) static GRADIENT_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

/// Standard window buttons occupy the first ~72 pt of the title band; chrome starts after them.
pub const TL_RESERVE: f64 = 84.;
/// Toast row sits just below the top bar; the info panel one row lower, so neither collides.
const TOAST_TOP: f64 = PLAYER_BAR_H + SP2;
/// Toast height: a 13 pt line with room to breathe, matching the skip button's panel.
const TOAST_H: f64 = 40.;
const INFO_TOP: f64 = TOAST_TOP + TOAST_H + SP2;

pub(crate) unsafe fn register_player_classes() {
    let mut d = ClassBuilder::new(c"JRPlayerHost", class!(NSView)).unwrap();
    d.add_method(sel!(mouseMoved:), host_mouse_moved as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    crate::ui::subtitles::register_classes();
    PLAYER_HOST_CLASS.set(d.register()).ok();
    let mut g = ClassBuilder::new(c"JRGradientView", class!(NSView)).unwrap();
    g.add_method(sel!(makeBackingLayer), gradient_layer as extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject);
    GRADIENT_CLASS.set(g.register()).ok();
}
extern "C" fn host_mouse_moved(_t: *mut AnyObject, _c: Sel, _e: *mut AnyObject) {
    unsafe { show_bars() }
}
extern "C" fn gradient_layer(_t: *mut AnyObject, _c: Sel) -> *mut AnyObject {
    unsafe {
        let l: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
        let arr: *mut AnyObject = {
            let cols = [cg_black(0.55) as *mut AnyObject, cg_black(0.0) as *mut AnyObject];
            msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr(), count: 2usize]
        };
        let _: () = msg_send![l, setColors: arr];
        let _: () = msg_send![l, setStartPoint: NSPoint { x: 0.5, y: 1.0 }];
        let _: () = msg_send![l, setEndPoint: NSPoint { x: 0.5, y: 0.0 }];
        l
    }
}

pub(crate) unsafe fn dark_appearance() -> *mut AnyObject {
    msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameDarkAqua")]
}

/// AirPlay state of the player: while the video plays on another screen, the host shows this
/// card instead of AVKit's black frame.
pub struct ExtUi {
    pub view: Ptr,
    pub label: Ptr,
    pub active: bool,
}

impl ExtUi {
    pub fn new() -> ExtUi {
        ExtUi { view: Ptr(std::ptr::null_mut()), label: Ptr(std::ptr::null_mut()), active: false }
    }
}

/// The card's headline. `device` is the AirPlay output's name when it is known.
pub fn external_title(device: Option<&str>) -> String {
    match device.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => format!("Playing on {}", d),
        None => "Playing on AirPlay".to_string(),
    }
}

/// Whether the video view is on screen (the player host is up).
pub(crate) unsafe fn video_on_screen() -> bool {
    match UI.get() {
        Some(ui) => !ui.player_host.get().is_null() && !crate::ui::shell::is_hidden(ui.player_host.get()),
        None => false,
    }
}

/// AVPlayer's `externalPlaybackActive` changed: show or hide the AirPlay card.
pub(crate) unsafe fn set_external(active: bool) {
    let Some(ui) = UI.get_mut() else { return };
    if ui.ext.view.get().is_null() {
        return;
    }
    if active {
        let device = crate::platform::airplay::airplay_output_name();
        set_text(ui.ext.label.get(), &external_title(device.as_deref()));
    }
    ui.ext.active = active;
    set_hidden(ui.ext.view.get(), !active);
}

/// Builds the AirPlay card inside `ph`, above AVKit's view and below the bars.
unsafe fn build_external(ph: *mut AnyObject, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let v: *mut AnyObject = msg_send![class!(NSView), alloc];
    let v: *mut AnyObject = autorel(msg_send![v, initWithFrame: b]);
    set_autoresize(v, RESIZE_W | RESIZE_H);
    let _: () = msg_send![v, setWantsLayer: Bool::YES];
    let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
    let cg: *mut CGColor = msg_send![black, CGColor];
    let _: () = msg_send![layer_of(v), setBackgroundColor: cg];
    set_hidden(v, true);
    add_sub(ph, v);
    ui.ext.view = Ptr(v);

    let cx = b.size.w / 2.;
    let cy = b.size.h / 2.;
    let flex = MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY;
    let iv: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let iv: *mut AnyObject = autorel(msg_send![iv, initWithFrame: rect(cx - 36., cy + 12., 72., 60.)]);
    let img = symbol("airplayvideo", 56., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![iv, setImage: img];
    }
    let _: () = msg_send![iv, setContentTintColor: color(C_ON_MEDIA)];
    set_autoresize(iv, flex);
    add_sub(v, iv);

    let head = label(&external_title(None), T_TITLE, C_ON_MEDIA, ALIGN_CENTER);
    set_frame(head, rect(cx - 260., cy - 26., 520., 26.));
    set_autoresize(head, flex);
    add_sub(v, head);
    ui.ext.label = Ptr(head);

    let sub = label("The video is on your TV. Pause, seek and volume still work here.", T_CALLOUT, C_ON_MEDIA2, ALIGN_CENTER);
    set_frame(sub, rect(cx - 260., cy - 54., 520., 18.));
    set_autoresize(sub, flex);
    add_sub(v, sub);
}

/// Builds bar, toast, info panel and backdrop inside `ph` (the player host) around `avv`.
pub(crate) unsafe fn build_player_chrome(ph: *mut AnyObject, avv: *mut AnyObject, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let _: () = msg_send![ph, setAppearance: dark_appearance()];
    let ta: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut AnyObject = msg_send![ta,
        initWithRect: rect(0., 0., 10., 10.), options: (0x02usize | 0x01usize | 0x20usize | 0x200usize), owner: ph, userInfo: nil()];
    let _: () = msg_send![ph, addTrackingArea: ta];
    autorel(ta);

    // AVKit's own controls are off: the custom transport bar (ui::playerbar) is the transport.
    // Media keys / Control Centre still work through now-playing.
    let _: () = msg_send![avv, setControlsStyle: 0isize];
    let _: () = msg_send![avv, setUpdatesNowPlayingInfoCenter: Bool::YES];

    // Subtitle overlay first, so the AirPlay card, the bars and the skip stack stay above it.
    crate::ui::subtitles::build(ph);
    build_external(ph, b);

    // ---- top bar (gradient) ----
    let gcls = *GRADIENT_CLASS.get().unwrap();
    let bar: *mut AnyObject = msg_send![gcls, alloc];
    let bar: *mut AnyObject = msg_send![bar, initWithFrame: rect(0., b.size.h - PLAYER_BAR_H, b.size.w, PLAYER_BAR_H)];
    let _: () = msg_send![bar, setWantsLayer: Bool::YES];
    set_autoresize(bar, RESIZE_W | MARGIN_MINY);
    add_sub(ph, bar);
    autorel(bar);
    ui.top_bar = Ptr(bar);

    let back: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: sel!(stopPlayback:)];
    let _: () = msg_send![back, setBordered: Bool::NO];
    let bi = symbol("chevron.left", 17., Weight::Medium);
    if !bi.is_null() {
        let _: () = msg_send![back, setImage: bi];
    }
    let _: () = msg_send![back, setImagePosition: 1isize];
    let _: () = msg_send![back, setContentTintColor: color(C_ON_MEDIA)];
    let _: () = msg_send![back, setToolTip: ns("Back to library")];
    set_frame(back, rect(TL_RESERVE, (PLAYER_BAR_H - 28.) / 2., 28., 28.));
    add_sub(bar, back);
    ui.back_btn = Ptr(back);

    let tx = TL_RESERVE + 28. + SP2;
    let tt = label("", T_HEADLINE, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(tt, rect(tx, 26., b.size.w - tx - SP4, 18.));
    set_autoresize(tt, RESIZE_W);
    add_sub(bar, tt);
    ui.top_title = Ptr(tt);
    let ts = label("", T_CAPTION, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(ts, rect(tx, 9., b.size.w - tx - SP4, 14.));
    set_autoresize(ts, RESIZE_W);
    add_sub(bar, ts);
    ui.top_sub = Ptr(ts);

    // ---- bottom transport bar (scrubber, buttons, menus) ----
    crate::ui::playerbar::build_transport(ph, b);
    crate::ui::playerbar::build(ph, b);
    crate::ui::skip::build(ph, b);
    crate::ui::episodes::build(ph);

    // ---- toast ----
    // Same panel as the skip button: frosted HUD, continuous corners, light border.
    let chip = crate::ui::skip::hud_panel(12.);
    set_frame(chip, rect(b.size.w / 2. - 100., b.size.h - TOAST_TOP - TOAST_H, 200., TOAST_H));
    set_autoresize(chip, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY);
    let _: () = msg_send![chip, setAlphaValue: 0.0f64];
    add_sub(ph, chip);
    ui.chip = Ptr(chip);
    let ti: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let ti: *mut AnyObject = autorel(msg_send![ti, initWithFrame: rect(SP3, 8., 16., 16.)]);
    let _: () = msg_send![ti, setContentTintColor: color(C_ON_MEDIA)];
    set_hidden(ti, true);
    add_sub(chip, ti);
    ui.toast_icon = Ptr(ti);
    let tl = label("", T_BODY_EM, C_ON_MEDIA, ALIGN_CENTER);
    set_frame(tl, rect(SP4, (TOAST_H - 17.) / 2., 168., 17.));
    add_sub(chip, tl);
    ui.chip_label = Ptr(tl);

    // ---- playback info panel (off by default) ----
    let ip = vev(MAT_HUD, BLEND_WITHIN);
    set_frame(ip, rect(SP4, b.size.h - INFO_TOP - 26., 440., 26.));
    set_autoresize(ip, MARGIN_MAXX | MARGIN_MINY);
    let il = layer_of(ip);
    set_corner(il, 8.);
    let _: () = msg_send![il, setMasksToBounds: Bool::YES];
    add_sub(ph, ip);
    set_hidden(ip, !ui.cfg.settings.show_stats);
    ui.info_panel = Ptr(ip);
    let ilab = label("", T_MONO, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(ilab, rect(SP2, 5., 424., 16.));
    set_autoresize(ilab, RESIZE_W);
    add_sub(ip, ilab);
    ui.info_label = Ptr(ilab);

    // ---- buffering backdrop (inside AVKit's overlay view, under its controls) ----
    let overlay: *mut AnyObject = msg_send![avv, contentOverlayView];
    let parent = if overlay.is_null() { ph } else { overlay };
    let ob = bounds_of(parent);
    let bd: *mut AnyObject = msg_send![class!(NSView), alloc];
    let bd: *mut AnyObject = msg_send![bd, initWithFrame: ob];
    let _: () = msg_send![bd, setWantsLayer: Bool::YES];
    set_autoresize(bd, RESIZE_W | RESIZE_H);
    set_hidden(bd, true);
    add_sub(parent, bd);
    autorel(bd);
    ui.backdrop = Ptr(bd);
    let (pw, ph_) = (180., 270.);
    let pv: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let pv: *mut AnyObject = autorel(msg_send![pv, initWithFrame: rect((ob.size.w - pw) / 2., (ob.size.h - ph_) / 2. + 20., pw, ph_)]);
    let _: () = msg_send![pv, setImageScaling: 3isize]; // axes independently: we size it to the aspect ourselves
    let pl = layer_of(pv);
    set_corner(pl, R_POSTER);
    let _: () = msg_send![pl, setMasksToBounds: Bool::YES];
    set_autoresize(pv, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(bd, pv);
    ui.backdrop_poster = Ptr(pv);
    let sp: *mut AnyObject = msg_send![class!(NSProgressIndicator), alloc];
    let sp: *mut AnyObject = autorel(msg_send![sp, initWithFrame: rect(ob.size.w / 2. - 14., (ob.size.h - ph_) / 2. - 36., 28., 28.)]);
    let _: () = msg_send![sp, setStyle: 1isize];
    let _: () = msg_send![sp, setIndeterminate: Bool::YES];
    let _: () = msg_send![sp, setUsesThreadedAnimation: Bool::YES];
    set_autoresize(sp, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(bd, sp);
    ui.backdrop_spin = Ptr(sp);
    let _ = (fix_height as unsafe fn(*mut AnyObject, f64), is_hidden as unsafe fn(*mut AnyObject) -> bool);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_title_names_the_device_or_falls_back() {
        assert_eq!(external_title(Some("Living Room Apple TV")), "Playing on Living Room Apple TV");
        assert_eq!(external_title(Some("  ")), "Playing on AirPlay");
        assert_eq!(external_title(None), "Playing on AirPlay");
    }
}

// ---- bar visibility --------------------------------------------------------------------------
pub(crate) unsafe fn show_bars() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    ui.last_mouse = Some(std::time::Instant::now());
    if !ui.bar_shown {
        ui.bar_shown = true;
        fade(ui.top_bar.get(), 1.0, M_FAST);
        fade_transport(1.0, M_FAST);
        crate::ui::subtitles::update();
    }
}
/// The bottom transport bar fades together with the top bar.
unsafe fn fade_transport(to: f64, d: f64) {
    let ui = UI.get().unwrap();
    if !ui.pbar.bar.get().is_null() {
        fade(ui.pbar.bar.get(), to, d);
    }
    if !ui.pbar.transport.get().is_null() {
        fade(ui.pbar.transport.get(), to, d);
    }
}
/// Self-test hook: pretends the pointer last moved `secs` ago.
pub(crate) unsafe fn set_idle_age(secs: f64) {
    let ui = UI.get_mut().unwrap();
    ui.last_mouse = std::time::Instant::now().checked_sub(std::time::Duration::from_secs_f64(secs));
}
/// Self-test hook: shows both bars at once, with no fade, and restarts the idle clock.
pub(crate) unsafe fn show_bars_instant() {
    let ui = UI.get_mut().unwrap();
    ui.last_mouse = Some(std::time::Instant::now());
    ui.bar_shown = true;
    let _: () = msg_send![ui.top_bar.get(), setAlphaValue: 1.0f64];
    if !ui.pbar.bar.get().is_null() {
        let _: () = msg_send![ui.pbar.bar.get(), setAlphaValue: 1.0f64];
    }
    if !ui.pbar.transport.get().is_null() {
        let _: () = msg_send![ui.pbar.transport.get(), setAlphaValue: 1.0f64];
    }
    crate::ui::subtitles::update();
}
pub(crate) unsafe fn hide_bars_if_idle() {
    let ui = UI.get_mut().unwrap();
    let paused = ui.playing.as_ref().map(|p| p.paused).unwrap_or(true);
    if paused {
        if !ui.bar_shown {
            show_bars();
        }
        return;
    }
    if ui.bar_shown {
        // No pointer movement yet this playback counts as idle from now.
        let t = *ui.last_mouse.get_or_insert_with(std::time::Instant::now);
        if t.elapsed().as_secs_f64() >= M_IDLE_HIDE {
            ui.bar_shown = false;
            fade(ui.top_bar.get(), 0.0, M_STD);
            fade_transport(0.0, M_STD);
            crate::ui::subtitles::update();
            let _: () = msg_send![class!(NSCursor), setHiddenUntilMouseMoves: Bool::YES];
        }
    }
}
pub(crate) unsafe fn fade(v: *mut AnyObject, to: f64, d: f64) {
    anim_begin(d, if to > 0.5 { timing_ease_out() } else { timing_ease_in_out() });
    let a: *mut AnyObject = msg_send![v, animator];
    let _: () = msg_send![a, setAlphaValue: to];
    anim_end();
}

// ---- toast -------------------------------------------------------------------------------------------
/// Transient pill at the top centre. Empty text hides it (never an empty pill).
pub(crate) unsafe fn toast(text: &str, symbol_name: Option<&str>, is_err: bool) {
    let ui = UI.get_mut().unwrap();
    let ctl = ui.ctl.get();
    let _: () = msg_send![class!(NSObject), cancelPreviousPerformRequestsWithTarget: ctl, selector: sel!(jrHideToast:), object: nil()];
    if text.is_empty() {
        let _: () = msg_send![ui.chip.get(), setAlphaValue: 0.0f64];
        return;
    }
    set_text(ui.chip_label.get(), text);
    let icon = symbol_name.map(|n| symbol(n, 13., Weight::Medium)).unwrap_or(nil());
    let has_icon = !icon.is_null();
    if has_icon {
        let _: () = msg_send![ui.toast_icon.get(), setImage: icon];
        let tint = if is_err { color(C_DANGER) } else { color(C_ON_MEDIA) };
        let _: () = msg_send![ui.toast_icon.get(), setContentTintColor: tint];
    }
    set_hidden(ui.toast_icon.get(), !has_icon);
    let cell: *mut AnyObject = msg_send![ui.chip_label.get(), cell];
    let tsz: NSSize = msg_send![cell, cellSize];
    let icon_w = if has_icon { 16. + SP2 } else { 0. };
    let w = (tsz.w.ceil() + 2. * SP4 + icon_w).clamp(96., 420.);
    let ph_b = bounds_of(ui.player_host.get());
    set_frame(ui.chip.get(), rect((ph_b.size.w - w) / 2., ph_b.size.h - TOAST_TOP - TOAST_H, w, TOAST_H));
    set_frame(ui.toast_icon.get(), rect(SP3, (TOAST_H - 16.) / 2., 16., 16.));
    set_frame(ui.chip_label.get(), rect(SP4 + icon_w, (TOAST_H - 17.) / 2., w - 2. * SP4 - icon_w, 17.));
    fade(ui.chip.get(), 1.0, M_STD);
    let delay = if is_err { 6.0f64 } else { 1.8f64 };
    let _: () = msg_send![ctl, performSelector: sel!(jrHideToast:), withObject: nil(), afterDelay: delay];
}
pub(crate) extern "C" fn hide_toast(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        fade(ui.chip.get(), 0.0, M_STD);
    }
}
pub(crate) unsafe fn toast_visible() -> bool {
    let ui = UI.get().unwrap();
    let a: f64 = msg_send![ui.chip.get(), alphaValue];
    a > 0.01
}

// ---- playback info -------------------------------------------------------------------------------------
pub(crate) extern "C" fn toggle_info(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.cfg.settings.show_stats = !ui.cfg.settings.show_stats;
        crate::config::save_config_keeping_session(&ui.cfg);
        apply_info_visibility();
    }
}
pub(crate) unsafe fn apply_info_visibility() {
    let ui = UI.get_mut().unwrap();
    let on = ui.cfg.settings.show_stats;
    set_hidden(ui.info_panel.get(), !on);
    if !ui.info_item.get().is_null() {
        let _: () = msg_send![ui.info_item.get(), setState: on as isize];
    }
    if on {
        crate::platform::appkit::update_chip();
    }
}

// ---- backdrop ---------------------------------------------------------------------------------------------
/// Show the buffering backdrop: a tinted fill, the poster when we have one, and a spinner.
pub(crate) unsafe fn backdrop_show(tint: *mut AnyObject, poster: *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    let ctl = ui.ctl.get();
    let _: () = msg_send![class!(NSObject), cancelPreviousPerformRequestsWithTarget: ctl, selector: sel!(jrBackdropOff:), object: nil()];
    let l = layer_of(ui.backdrop.get());
    let c: *mut CGColor = msg_send![tint, CGColor];
    let _: () = msg_send![l, setBackgroundColor: c];
    if poster.is_null() {
        set_hidden(ui.backdrop_poster.get(), true);
    } else {
        let sz: NSSize = msg_send![poster, size];
        let ratio = if sz.w > 0. { (sz.h / sz.w).clamp(0.5, 1.8) } else { 1.5 };
        let ob = bounds_of(ui.backdrop.get());
        let (pw, phh) = (180., (180. * ratio).round());
        set_frame(ui.backdrop_poster.get(), rect((ob.size.w - pw) / 2., (ob.size.h - phh) / 2. + 20., pw, phh));
        set_frame(ui.backdrop_spin.get(), rect(ob.size.w / 2. - 14., (ob.size.h - phh) / 2. - 36., 28., 28.));
        let _: () = msg_send![ui.backdrop_poster.get(), setImage: poster];
        set_hidden(ui.backdrop_poster.get(), false);
    }
    let _: () = msg_send![ui.backdrop.get(), setAlphaValue: 1.0f64];
    set_hidden(ui.backdrop.get(), false);
    let _: () = msg_send![ui.backdrop_spin.get(), startAnimation: nil()];
}
/// Fade the backdrop out; it is hidden for good (so it cannot eat clicks) once the fade is done.
pub(crate) unsafe fn backdrop_hide() {
    let ui = UI.get_mut().unwrap();
    if is_hidden(ui.backdrop.get()) {
        return;
    }
    let _: () = msg_send![ui.backdrop_spin.get(), stopAnimation: nil()];
    fade(ui.backdrop.get(), 0.0, M_STD);
    let ctl = ui.ctl.get();
    let _: () = msg_send![ctl, performSelector: sel!(jrBackdropOff:), withObject: nil(), afterDelay: M_STD + 0.05];
}
pub(crate) extern "C" fn backdrop_off(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_hidden(ui.backdrop.get(), true);
    }
}
pub(crate) unsafe fn backdrop_visible() -> bool {
    let ui = UI.get().unwrap();
    !is_hidden(ui.backdrop.get())
}
pub const _KEEP: Bool = Bool::NO;
