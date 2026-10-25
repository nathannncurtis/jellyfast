//! Now-playing bar for audio: artwork with a tint wash, title and artist, transport,
//! scrubber with elapsed/remaining times, and volume. It sits at the bottom of the
//! content area (library stays visible) and shrinks `lib_scroll` by `NOW_BAR_H`.
//! Audio plays through the same AVPlayer as video; `player_host` is never shown.
#![allow(unsafe_code)]
use crate::app::messages::PlaybackStart;
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Class, Object, Sel, NO, YES};
use objc::{sel, sel_impl};
use std::sync::OnceLock;

static BAR_CLASS: OnceLock<&'static Class> = OnceLock::new();

const ART: f64 = 48.;
const TEXT_X: f64 = 76.;
const TIME_W: f64 = 52.;
const CIRCLE: f64 = 36.;
const SIDE_BTN: f64 = 28.;
const CLOSE: f64 = 20.;
/// Narrowest the title/artist column is allowed to get when the centre block squeezes it.
const TEXT_MIN_W: f64 = 120.;

pub struct NowBar {
    pub bar: Ptr,
    wash: Ptr,
    art_layer: Ptr,
    art_glyph: Ptr,
    pub title: Ptr,
    pub artist: Ptr,
    play_bg: Ptr,
    pub play: Ptr,
    prev: Ptr,
    next: Ptr,
    pub scrub: Ptr,
    pub elapsed: Ptr,
    pub remain: Ptr,
    vol_icon: Ptr,
    pub vol: Ptr,
    close: Ptr,
    /// The bar is on screen (an audio item is playing).
    pub active: bool,
    inset_on: bool,
    /// 0.25 s timer while visible instead of 1 Hz.
    fast: bool,
    last_slow: Option<std::time::Instant>,
    art_key: Option<u64>,
    shows_pause: bool,
}
impl NowBar {
    pub fn new() -> NowBar {
        let n = || Ptr(std::ptr::null_mut());
        NowBar {
            bar: n(),
            wash: n(),
            art_layer: n(),
            art_glyph: n(),
            title: n(),
            artist: n(),
            play_bg: n(),
            play: n(),
            prev: n(),
            next: n(),
            scrub: n(),
            elapsed: n(),
            remain: n(),
            vol_icon: n(),
            vol: n(),
            close: n(),
            active: false,
            inset_on: false,
            fast: false,
            last_slow: None,
            art_key: None,
            shows_pause: true,
        }
    }
}

pub(crate) unsafe fn register_nowbar_class() {
    let mut d = ClassDecl::new("JRNowBar", class!(NSView)).unwrap();
    d.add_method(sel!(layout), bar_layout as extern "C" fn(&mut Object, Sel));
    BAR_CLASS.set(d.register()).ok();
}
extern "C" fn bar_layout(this: &mut Object, _c: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
        relayout();
    }
}

unsafe fn icon_button(name: &str, pt: f64, tip: &str, action: Sel) -> *mut Object {
    let ui = UI.get_mut().unwrap();
    let b: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
        target: ui.ctl.get() action: action];
    let _: () = msg_send![b, setBordered: NO];
    let img = symbol(name, pt, Weight::Medium);
    if !img.is_null() {
        let _: () = msg_send![b, setImage: img];
    }
    let _: () = msg_send![b, setImagePosition: 1isize]; // image only
    let _: () = msg_send![b, setContentTintColor: color(C_TEXT)];
    let _: () = msg_send![b, setToolTip: ns(tip)];
    b
}
unsafe fn slider(action: Sel, continuous: bool) -> *mut Object {
    let ui = UI.get_mut().unwrap();
    let s: *mut Object = msg_send![class!(NSSlider), alloc];
    let s: *mut Object = msg_send![s, initWithFrame: rect(0., 0., 100., 16.)];
    let _: () = msg_send![s, setMinValue: 0.0f64];
    let _: () = msg_send![s, setMaxValue: 1.0f64];
    let _: () = msg_send![s, setControlSize: 1usize]; // small
    let _: () = msg_send![s, setContinuous: if continuous { YES } else { NO }];
    let _: () = msg_send![s, setTarget: ui.ctl.get()];
    let _: () = msg_send![s, setAction: action];
    autorel(s)
}

/// Build the bar (hidden) at the bottom of `content_host`.
pub(crate) unsafe fn build(content_host: *mut Object, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let w = b.size.w - SIDEBAR_W;
    let cls = *BAR_CLASS.get().unwrap();
    let bar: *mut Object = msg_send![cls, alloc];
    let bar: *mut Object = msg_send![bar, initWithFrame: rect(SIDEBAR_W, 0., w, NOW_BAR_H)];
    set_autoresize(bar, RESIZE_W | MARGIN_MAXY);
    set_hidden(bar, true);
    add_sub(content_host, bar);
    autorel(bar);
    let full = rect(0., 0., w, NOW_BAR_H);

    // tint wash behind the material
    let wash: *mut Object = msg_send![class!(NSView), alloc];
    let wash: *mut Object = autorel(msg_send![wash, initWithFrame: full]);
    let _: () = msg_send![wash, setWantsLayer: YES];
    set_autoresize(wash, RESIZE_W | RESIZE_H);
    add_sub(bar, wash);
    let blur = vev(MAT_HEADER, BLEND_WITHIN);
    set_frame(blur, full);
    set_autoresize(blur, RESIZE_W | RESIZE_H);
    add_sub(bar, blur);
    let hl = surface(rect(0., NOW_BAR_H - 1., w, 1.), C_SEPARATOR, 0.);
    set_autoresize(hl, RESIZE_W | MARGIN_MAXY);
    add_sub(bar, hl);

    // artwork: a standalone layer (aspect fill, rounded) plus a placeholder glyph
    let art: *mut Object = msg_send![class!(NSView), alloc];
    let art: *mut Object = autorel(msg_send![art, initWithFrame: rect(SP4, (NOW_BAR_H - ART) / 2., ART, ART)]);
    let _: () = msg_send![art, setWantsLayer: YES];
    let al: *mut Object = msg_send![class!(CALayer), layer];
    let _: () = msg_send![al, setFrame: rect(0., 0., ART, ART)];
    set_corner(al, R_THUMB);
    let _: () = msg_send![al, setMasksToBounds: YES];
    let _: () = msg_send![al, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![al, setBackgroundColor: cg_white(0.10)];
    let root = layer_of(art);
    let _: () = msg_send![root, addSublayer: al];
    add_sub(bar, art);
    let glyph: *mut Object = msg_send![class!(NSImageView), alloc];
    let glyph: *mut Object = autorel(msg_send![glyph, initWithFrame: rect(10., 10., 28., 28.)]);
    let gi = symbol("music.note", 20., Weight::Medium);
    if !gi.is_null() {
        let _: () = msg_send![glyph, setImage: gi];
    }
    let _: () = msg_send![glyph, setContentTintColor: color(C_TEXT3)];
    add_sub(art, glyph);

    let title = label("", T_HEADLINE, C_TEXT, ALIGN_LEFT);
    add_sub(bar, title);
    let artist = label("", T_CAPTION, C_TEXT2, ALIGN_LEFT);
    add_sub(bar, artist);

    // transport
    let play_bg = surface(rect(0., 0., CIRCLE, CIRCLE), C_FILL_SEL, CIRCLE / 2.);
    add_sub(bar, play_bg);
    let prev = icon_button("backward.fill", 15., "Restart", sel!(jrNowRestart:));
    add_sub(bar, prev);
    let play = icon_button("pause.fill", 22., "Play / Pause", sel!(playPause:));
    add_sub(bar, play);
    let next = icon_button("forward.fill", 15., "Skip forward 10 seconds", sel!(jrNowSkip:));
    add_sub(bar, next);

    let scrub = slider(sel!(jrNowScrub:), false);
    add_sub(bar, scrub);
    let elapsed = label("0:00", T_MONO, C_TEXT2, ALIGN_RIGHT);
    add_sub(bar, elapsed);
    let remain = label("-0:00", T_MONO, C_TEXT2, ALIGN_LEFT);
    add_sub(bar, remain);

    // volume + close
    let vi: *mut Object = msg_send![class!(NSImageView), alloc];
    let vi: *mut Object = autorel(msg_send![vi, initWithFrame: rect(0., 0., 16., 16.)]);
    let vimg = symbol("speaker.wave.2", 13., Weight::Medium);
    if !vimg.is_null() {
        let _: () = msg_send![vi, setImage: vimg];
    }
    let _: () = msg_send![vi, setContentTintColor: color(C_TEXT2)];
    add_sub(bar, vi);
    let vol = slider(sel!(jrNowVolume:), true);
    add_sub(bar, vol);
    let close = icon_button("xmark", 11., "Stop", sel!(stopPlayback:));
    let _: () = msg_send![close, setContentTintColor: color(C_TEXT2)];
    add_sub(bar, close);

    ui.now.bar = Ptr(bar);
    ui.now.wash = Ptr(wash);
    ui.now.art_layer = Ptr(al);
    ui.now.art_glyph = Ptr(glyph);
    ui.now.title = Ptr(title);
    ui.now.artist = Ptr(artist);
    ui.now.play_bg = Ptr(play_bg);
    ui.now.play = Ptr(play);
    ui.now.prev = Ptr(prev);
    ui.now.next = Ptr(next);
    ui.now.scrub = Ptr(scrub);
    ui.now.elapsed = Ptr(elapsed);
    ui.now.remain = Ptr(remain);
    ui.now.vol_icon = Ptr(vi);
    ui.now.vol = Ptr(vol);
    ui.now.close = Ptr(close);
    relayout();
}

unsafe fn cg_white(alpha: f64) -> *mut Object {
    let c: *mut Object = msg_send![class!(NSColor), colorWithWhite: 1.0f64 alpha: alpha];
    msg_send![c, CGColor]
}

/// Pure layout of the bar's controls for a bar `w` wide. Returns, in order:
/// (text_w, scrub_x, scrub_w, centre_x, vol_x, vol_w).
pub(crate) fn plan(w: f64) -> (f64, f64, f64, f64, f64, f64) {
    let ctrl_gap = SP2;
    let times = 2. * (TIME_W + ctrl_gap);
    // Spec: scrubber is min(480, 0.4 w); also leave the title column TEXT_MIN_W at narrow widths.
    let squeeze = w - 2. * (TEXT_X + TEXT_MIN_W + SP3) - times;
    let scrub_w = (w * 0.4).min(480.).min(squeeze.max(120.)).max(80.);
    let block = TIME_W + ctrl_gap + scrub_w + ctrl_gap + TIME_W;
    let block_x = ((w - block) / 2.).round();
    let scrub_x = block_x + TIME_W + ctrl_gap;
    let centre_x = block_x + block / 2.;
    let text_w = (w * 0.3).min(block_x - SP3 - TEXT_X).max(40.);
    // Right group: [speaker 16][gap][volume][gap][close]; volume shrinks if the centre block needs room.
    let fixed = 16. + ctrl_gap + ctrl_gap + CLOSE + SP4;
    let avail = w - (block_x + block + SP3) - fixed;
    let vol_w = avail.clamp(40., 96.);
    let vol_x = w - SP4 - CLOSE - ctrl_gap - vol_w;
    (text_w, scrub_x, scrub_w, centre_x, vol_x, vol_w)
}

pub(crate) unsafe fn relayout() {
    let ui = match UI.get() {
        Some(u) => u,
        None => return,
    };
    let nb = &ui.now;
    if nb.bar.get().is_null() {
        return;
    }
    let w = bounds_of(nb.bar.get()).size.w;
    let (text_w, scrub_x, scrub_w, cx, vol_x, vol_w) = plan(w);
    tx_begin_instant();
    set_frame(nb.title.get(), rect(TEXT_X, 34., text_w, 18.));
    set_frame(nb.artist.get(), rect(TEXT_X, 16., text_w, 14.));
    set_frame(nb.play_bg.get(), rect(cx - CIRCLE / 2., 26., CIRCLE, CIRCLE));
    set_frame(nb.play.get(), rect(cx - CIRCLE / 2., 26., CIRCLE, CIRCLE));
    set_frame(nb.prev.get(), rect(cx - CIRCLE / 2. - SP4 - SIDE_BTN, 30., SIDE_BTN, SIDE_BTN));
    set_frame(nb.next.get(), rect(cx + CIRCLE / 2. + SP4, 30., SIDE_BTN, SIDE_BTN));
    set_frame(nb.scrub.get(), rect(scrub_x, 8., scrub_w, 16.));
    set_frame(nb.elapsed.get(), rect(scrub_x - SP2 - TIME_W, 9., TIME_W, 14.));
    set_frame(nb.remain.get(), rect(scrub_x + scrub_w + SP2, 9., TIME_W, 14.));
    set_frame(nb.vol_icon.get(), rect(vol_x - SP2 - 16., 28., 16., 16.));
    set_frame(nb.vol.get(), rect(vol_x, 28., vol_w, 16.));
    set_frame(nb.close.get(), rect(w - SP4 - CLOSE, (NOW_BAR_H - CLOSE) / 2., CLOSE, CLOSE));
    tx_end();
}

// ---- show / hide ----------------------------------------------------------------------------
unsafe fn apply_inset(on: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.now.inset_on == on {
        return;
    }
    ui.now.inset_on = on;
    let d = if on { NOW_BAR_H } else { -NOW_BAR_H };
    for v in [ui.lib_scroll.get(), ui.state_host.get(), ui.home.scroll.get()] {
        let f = frame_of(v);
        let y = if on { NOW_BAR_H } else { 0. };
        set_frame(v, rect(f.origin.x, y, f.size.w, (f.size.h - d).max(0.)));
    }
}

unsafe fn set_fast(on: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.now.fast == on {
        return;
    }
    ui.now.fast = on;
    ui.now.last_slow = None;
    if !ui.timer.get().is_null() {
        let _: () = msg_send![ui.timer.get(), invalidate];
    }
    let t: *mut Object = msg_send![class!(NSTimer),
        scheduledTimerWithTimeInterval: if on { 0.25f64 } else { 1.0f64 } target: ui.ctl.get()
        selector: sel!(jrTick:) userInfo: nil() repeats: YES];
    ui.timer = Ptr(t);
}

/// True when the 1 Hz part of `tick` should run (always, unless the 0.25 s timer is active).
pub(crate) unsafe fn slow_due() -> bool {
    let ui = UI.get_mut().unwrap();
    if !ui.now.fast {
        return true;
    }
    let due = ui.now.last_slow.map(|t| t.elapsed().as_secs_f64() >= 0.9).unwrap_or(true);
    if due {
        ui.now.last_slow = Some(std::time::Instant::now());
    }
    due
}

/// Show the bar for an audio item (or update it when one is already showing).
pub(crate) unsafe fn show(ps: &PlaybackStart) {
    let ui = UI.get_mut().unwrap();
    if ui.now.bar.get().is_null() {
        return;
    }
    if !ui.now.active {
        crossfade(ui.content_host.get(), M_STD);
        set_hidden(ui.now.bar.get(), false);
        apply_inset(true);
        ui.now.active = true;
        set_fast(true);
    }
    set_text(ui.now.title.get(), &ps.title);
    set_text(ui.now.artist.get(), ps.subtitle.as_deref().unwrap_or(""));
    ui.now.art_key = ps.image_key;
    ui.now.shows_pause = false; // force the glyph refresh in update()
    apply_art();
    let _: () = msg_send![ui.now.scrub.get(), setDoubleValue: 0.0f64];
    relayout();
    update();
}

pub(crate) unsafe fn hide() {
    let ui = UI.get_mut().unwrap();
    if !ui.now.active {
        return;
    }
    crossfade(ui.content_host.get(), M_STD);
    set_hidden(ui.now.bar.get(), true);
    apply_inset(false);
    ui.now.active = false;
    ui.now.art_key = None;
    set_fast(false);
}

pub(crate) unsafe fn visible() -> bool {
    UI.get().map(|u| u.now.active).unwrap_or(false)
}

/// Re-apply artwork and tint for the current track.
unsafe fn apply_art() {
    let ui = UI.get_mut().unwrap();
    let key = ui.now.art_key;
    let img = key.and_then(|k| ui.poster_cache.get(&k).copied()).unwrap_or(nil());
    let (h, s, b) = key.and_then(|k| ui.tint_cache.get(&k).copied()).unwrap_or((0., 0., 0.20));
    tx_begin(M_STD);
    let c: *mut Object = msg_send![class!(NSColor), colorWithHue: h saturation: s brightness: b alpha: 0.35f64];
    let cgc: *mut Object = msg_send![c, CGColor];
    let wl = layer_of(ui.now.wash.get());
    let _: () = msg_send![wl, setBackgroundColor: cgc];
    let _: () = msg_send![ui.now.art_layer.get(), setContents: img];
    tx_end();
    set_hidden(ui.now.art_glyph.get(), !img.is_null());
}
/// A poster finished loading: refresh the bar if it is the current track's artwork.
pub(crate) fn artwork_arrived(key: u64) {
    unsafe {
        if let Some(ui) = UI.get() {
            if ui.now.active && ui.now.art_key == Some(key) {
                apply_art();
            }
        }
    }
}

// ---- per-tick refresh ------------------------------------------------------------------------
pub(crate) unsafe fn update() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    if !ui.now.active || ui.pitem.get().is_null() {
        return;
    }
    let (paused, dur_known) = match ui.playing.as_ref() {
        Some(p) => (p.paused, p.duration_secs),
        None => return,
    };
    let t: CMTime = msg_send![ui.player.get(), currentTime];
    let d: CMTime = msg_send![ui.pitem.get(), duration];
    let dur = if d.seconds() > 0.0 { d.seconds() } else { dur_known };
    let cur = t.seconds().max(0.0);
    set_text(ui.now.elapsed.get(), &crate::jellyfin::urls::fmt_clock(cur));
    let left = if dur > 0.0 { (dur - cur).max(0.0) } else { 0.0 };
    set_text(ui.now.remain.get(), &format!("-{}", crate::jellyfin::urls::fmt_clock(left)));
    let frac = if dur > 0.0 { (cur / dur).clamp(0.0, 1.0) } else { 0.0 };
    let _: () = msg_send![ui.now.scrub.get(), setDoubleValue: frac];
    let v: f32 = msg_send![ui.player.get(), volume];
    let _: () = msg_send![ui.now.vol.get(), setDoubleValue: v as f64];
    if ui.now.shows_pause != !paused {
        ui.now.shows_pause = !paused;
        let img = symbol(if paused { "play.fill" } else { "pause.fill" }, 22., Weight::Medium);
        if !img.is_null() {
            let _: () = msg_send![ui.now.play.get(), setImage: img];
        }
    }
}

// ---- actions --------------------------------------------------------------------------------------
pub(crate) extern "C" fn restart(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.playing.is_none() {
            return;
        }
        let tm = CMTimeMakeWithSeconds(0.0, 600);
        let _: () = msg_send![ui.player.get(), seekToTime: tm];
        update();
    }
}
pub(crate) extern "C" fn skip(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        crate::platform::avkit::seek_by(10.0);
        update();
    }
}
pub(crate) extern "C" fn scrub(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let dur = match ui.playing.as_ref() {
            Some(p) => p.duration_secs,
            None => return,
        };
        if dur <= 0.0 {
            return;
        }
        let v: f64 = msg_send![sender, doubleValue];
        let tm = CMTimeMakeWithSeconds((v * dur).clamp(0.0, dur), 600);
        let _: () = msg_send![ui.player.get(), seekToTime: tm];
    }
}
pub(crate) extern "C" fn volume(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let v: f64 = msg_send![sender, doubleValue];
        let v = v.clamp(0.0, 1.0);
        let _: () = msg_send![ui.player.get(), setVolume: v as f32];
        // Persist on mouse-up only (not on every drag event).
        let ev: *mut Object = msg_send![ui.app.get(), currentEvent];
        let ty: NSUInteger = if ev.is_null() { 2 } else { msg_send![ev, type] };
        if ty != 6 {
            ui.cfg.volume = v;
            crate::config::save_config(&ui.cfg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_never_collides() {
        for w in [720., 800., 1040., 1460., 2000.] {
            let (text_w, scrub_x, scrub_w, cx, vol_x, vol_w) = plan(w);
            let block_x = scrub_x - SP2 - TIME_W;
            let block_end = scrub_x + scrub_w + SP2 + TIME_W;
            assert!(TEXT_X + text_w <= block_x - SP3 + 0.5, "{w}: text {text_w} vs block {block_x}");
            assert!(text_w >= 100., "{w}: text too narrow {text_w}");
            let right_start = vol_x - SP2 - 16.;
            assert!(block_end + SP3 <= right_start + 0.5, "{w}: block end {block_end} vs right {right_start}");
            assert!(vol_x + vol_w + SP2 + CLOSE + SP4 <= w + 0.5);
            assert!((cx - w / 2.).abs() < 1.0);
            assert!(scrub_w <= 480.);
        }
    }
}
