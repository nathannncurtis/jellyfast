//! Subtitle overlay. AVPlayer's own legible rendering is switched off for every item
//! (`suppressesPlayerRendering`), and the cue text arrives through an
//! `AVPlayerItemLegibleOutput` instead. The text is drawn in a label above the video, so
//! it can sit above the transport bar and the skip button rather than under them.
//! Track choice stays with AVPlayer: `avkit::set_track` still selects the option, and the
//! output then delivers the cues of that option only.
#![allow(unsafe_code)]
use crate::platform::appkit::*;
use crate::ui::shell::is_hidden;
use crate::ui::theme::*;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::os::raw::c_char;
use std::sync::OnceLock;

static DELEGATE_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

/// Gap between the overlay and whatever sits above it (the transport bar or the skip stack).
const GAP: f64 = SP4;
/// Bottom edge of the overlay while the controls are hidden.
const BOTTOM_IDLE: f64 = SP6;
/// The text block may use this share of the video width.
const MAX_W_FRAC: f64 = 0.8;
const PAD_X: f64 = SP3;
const PAD_Y: f64 = SP2;
/// Slack the label cell keeps around its text, so a line that just fits does not wrap.
const CELL_SLACK: f64 = 6.;

pub struct SubtitleUi {
    /// The overlay label (non-editable NSTextField), a subview of the player host.
    pub label: Ptr,
    /// Delegate of the legible outputs (process lifetime, owned).
    pub delegate: Ptr,
    /// Serial queue the outputs call the delegate on (process lifetime, owned).
    pub queue: Ptr,
    /// Legible output of the current item (retained; the item owns it as well).
    pub output: Ptr,
    /// Cue text on screen; empty when none.
    pub text: String,
    /// The text the label was last styled with, and the point size it used.
    styled_text: String,
    font_pt: f64,
    /// Frame last applied as (x, y, w, h); None while hidden or before the first placement.
    placed: Option<[f64; 4]>,
}

impl SubtitleUi {
    pub fn new() -> SubtitleUi {
        SubtitleUi {
            label: Ptr(std::ptr::null_mut()),
            delegate: Ptr(std::ptr::null_mut()),
            queue: Ptr(std::ptr::null_mut()),
            output: Ptr(std::ptr::null_mut()),
            text: String::new(),
            styled_text: String::new(),
            font_pt: 0.,
            placed: None,
        }
    }
}

extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut AnyObject) -> *mut AnyObject;
}

pub(crate) unsafe fn register_classes() {
    let mut d = ClassBuilder::new(c"JRLegibleDelegate", class!(NSObject)).unwrap();
    if let Some(p) = AnyProtocol::get(c"AVPlayerItemLegibleOutputPushDelegate") {
        d.add_protocol(p);
    }
    d.add_method(
        sel!(legibleOutput:didOutputAttributedStrings:nativeSampleBuffers:forItemTime:),
        legible_output as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, CMTime),
    );
    d.add_method(sel!(jrShowCues:), show_cues as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    DELEGATE_CLASS.set(d.register()).ok();
}

/// Builds the overlay label inside the player host. Called before the AirPlay card and the
/// bars are added, so those stay above it.
pub(crate) unsafe fn build(ph: *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    let lab: *mut AnyObject = msg_send![class!(NSTextField), labelWithString: ns("")];
    let _: () = msg_send![lab, setAlignment: ALIGN_CENTER];
    let _: () = msg_send![lab, setLineBreakMode: 0usize]; // word wrap
    let _: () = msg_send![lab, setUsesSingleLineMode: Bool::NO];
    let _: () = msg_send![lab, setMaximumNumberOfLines: 0isize];
    let _: () = msg_send![lab, setSelectable: Bool::NO];
    let _: () = msg_send![lab, setEditable: Bool::NO];
    let _: () = msg_send![lab, setBezeled: Bool::NO];
    let _: () = msg_send![lab, setBordered: Bool::NO];
    let _: () = msg_send![lab, setDrawsBackground: Bool::NO];
    set_hidden(lab, true);
    add_sub(ph, lab);
    ui.subs.label = Ptr(lab);
}

/// Gives a new item its legible output. Called right after the item is made, before it plays.
pub(crate) unsafe fn attach(item: *mut AnyObject) {
    ensure_delegate();
    let ui = UI.get_mut().unwrap();
    let none: *mut AnyObject = msg_send![class!(NSArray), array];
    let out: *mut AnyObject = msg_send![class!(AVPlayerItemLegibleOutput), alloc];
    let out: *mut AnyObject = autorel(msg_send![out, initWithMediaSubtypesForNativeRepresentation: none]);
    let _: () = msg_send![out, setSuppressesPlayerRendering: Bool::YES];
    let _: () = msg_send![out, setDelegate: ui.subs.delegate.get(), queue: ui.subs.queue.get()];
    let _: () = msg_send![item, addOutput: out];
    release_obj(ui.subs.output.get());
    ui.subs.output = Ptr::retained(out);
    clear();
}

/// The item is gone: drop the text and our reference to its output.
pub(crate) unsafe fn detach() {
    clear();
    let ui = UI.get_mut().unwrap();
    release_obj(ui.subs.output.get());
    ui.subs.output = Ptr(nil());
}

unsafe fn ensure_delegate() {
    let ui = UI.get_mut().unwrap();
    if !ui.subs.delegate.get().is_null() {
        return;
    }
    let cls = *DELEGATE_CLASS.get().unwrap();
    let d: *mut AnyObject = msg_send![cls, alloc];
    let d: *mut AnyObject = msg_send![d, init];
    ui.subs.delegate = Ptr(d);
    ui.subs.queue = Ptr(dispatch_queue_create(b"jellyfast.legible\0".as_ptr() as *const c_char, nil()));
}

/// AVKit's queue: the strings are handed to the main thread together with their output.
extern "C" fn legible_output(this: *mut AnyObject, _c: Sel, out: *mut AnyObject, strings: *mut AnyObject, _bufs: *mut AnyObject, _at: CMTime) {
    unsafe {
        let objs = [out, strings];
        let pair: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: objs.as_ptr(), count: 2usize];
        let me = this;
        let _: () = msg_send![me, performSelectorOnMainThread: sel!(jrShowCues:), withObject: pair, waitUntilDone: Bool::NO];
    }
}

/// Main thread: joins the cue's strings into the overlay text. Cues from an output that is no
/// longer the current item's are dropped.
extern "C" fn show_cues(_this: *mut AnyObject, _c: Sel, pair: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let out: *mut AnyObject = msg_send![pair, objectAtIndex: 0usize];
        if out != ui.subs.output.get() {
            return;
        }
        let strings: *mut AnyObject = msg_send![pair, objectAtIndex: 1usize];
        let n: NSUInteger = msg_send![strings, count];
        let mut parts: Vec<String> = Vec::new();
        for i in 0..n {
            let s: *mut AnyObject = msg_send![strings, objectAtIndex: i];
            let text_obj: *mut AnyObject = msg_send![s, string];
            let t = to_str(text_obj);
            if !t.trim().is_empty() {
                parts.push(t.trim().to_string());
            }
        }
        show(&parts.join("\n"));
    }
}

/// Hides the overlay and forgets the cue. Used on item change, track off, seek and stop.
pub(crate) unsafe fn clear() {
    show("");
}

/// Sets the cue on screen (empty hides it) and places it.
unsafe fn show(text: &str) {
    let ui = UI.get_mut().unwrap();
    ui.subs.text = text.to_string();
    update();
}

/// Self-test hook: puts `text` on screen as if a cue had arrived.
pub(crate) unsafe fn show_text(text: &str) {
    show(text);
    settle();
}

/// Per-tick and on every show/hide of the controls: keeps the overlay placed, animating a lift.
pub(crate) unsafe fn update() {
    sync(true);
}

/// Places the overlay at once, with no animation (self-test: layout is read right after a resize).
pub(crate) unsafe fn settle() {
    sync(false);
}

unsafe fn sync(animate: bool) {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    let lab = ui.subs.label.get();
    if lab.is_null() {
        return;
    }
    let showing = !ui.subs.text.is_empty()
        && crate::ui::player::video_on_screen()
        && !ui.ext.active
        && !crate::ui::player::backdrop_visible();
    if !showing {
        set_hidden(lab, true);
        ui.subs.placed = None;
        return;
    }
    place(animate);
}

/// Styles the label for the current text and video height, then moves it to its place.
unsafe fn place(animate: bool) {
    let ui = UI.get_mut().unwrap();
    let lab = ui.subs.label.get();
    let host = bounds_of(ui.player_host.get());
    // Size scales with the video height: 4.5 % of it, between 14 and 40 pt.
    let pt = (host.size.h * 0.045).clamp(14., 40.);
    if ui.subs.styled_text != ui.subs.text || (pt - ui.subs.font_pt).abs() > 0.5 {
        let s = styled(&ui.subs.text, pt);
        let _: () = msg_send![lab, setAttributedStringValue: s];
        ui.subs.styled_text = ui.subs.text.clone();
        ui.subs.font_pt = pt;
    }

    let maxw = host.size.w * MAX_W_FRAC;
    let attr: *mut AnyObject = msg_send![lab, attributedStringValue];
    let inner = maxw - 2. * PAD_X - CELL_SLACK;
    let r: NSRect = msg_send![attr, boundingRectWithSize: size(inner, 10000.), options: 1usize, context: nil()];
    let w = (r.size.w.ceil() + 2. * PAD_X + CELL_SLACK).min(maxw);
    let h = r.size.h.ceil() + 2. * PAD_Y;
    let x = ((host.size.w - w) / 2.).max(0.);

    // Normally near the bottom of the video; with the controls up, just above the transport bar.
    let mut bottom = if ui.bar_shown { PLAYER_CTRL_H + GAP } else { BOTTOM_IDLE };
    // The skip button and the Up Next card sit above the bar and can be up without it, so the
    // overlay goes above whichever of them it overlaps horizontally.
    for v in [ui.skip.card.get(), ui.skip.pill.get()] {
        if v.is_null() || is_hidden(v) {
            continue;
        }
        let f = frame_of(v);
        if x < f.origin.x + f.size.w && f.origin.x < x + w {
            bottom = bottom.max(f.origin.y + f.size.h + GAP);
        }
    }

    let target = [x, bottom, w, h];
    set_hidden(lab, false);
    let unchanged = ui.subs.placed == Some(target);
    let first = ui.subs.placed.is_none();
    ui.subs.placed = Some(target);
    // A lift already under way is left to finish; a settle always sets the frame itself.
    if unchanged && animate {
        return;
    }
    let fr = rect(x, bottom, w, h);
    if animate && !first {
        anim_begin(M_STD, timing_ease_out());
        let a: *mut AnyObject = msg_send![lab, animator];
        let _: () = msg_send![a, setFrame: fr];
        anim_end();
    } else {
        set_frame(lab, fr);
    }
}

/// Attributed text for the overlay: white, centred, soft shadow, AVKit's own styling dropped.
unsafe fn styled(text: &str, pt: f64) -> *mut AnyObject {
    let font: *mut AnyObject = msg_send![class!(NSFont), systemFontOfSize: pt];
    let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
    let shadow: *mut AnyObject = autorel(msg_send![class!(NSShadow), new]);
    let _: () = msg_send![shadow, setShadowOffset: size(0., -1.)];
    let _: () = msg_send![shadow, setShadowBlurRadius: 4.0f64];
    let sc: *mut AnyObject = msg_send![class!(NSColor), colorWithCalibratedWhite: 0.0f64, alpha: 0.85f64];
    let _: () = msg_send![shadow, setShadowColor: sc];
    let ps: *mut AnyObject = autorel(msg_send![class!(NSMutableParagraphStyle), new]);
    let _: () = msg_send![ps, setAlignment: ALIGN_CENTER];
    let _: () = msg_send![ps, setLineBreakMode: 0usize];
    let vals: [*mut AnyObject; 4] = [font, white, shadow, ps];
    let keys: [*mut AnyObject; 4] = [ns("NSFont"), ns("NSColor"), ns("NSShadow"), ns("NSParagraphStyle")];
    let attrs: *mut AnyObject = msg_send![class!(NSDictionary),
        dictionaryWithObjects: vals.as_ptr(), forKeys: keys.as_ptr(), count: 4usize];
    let s: *mut AnyObject = msg_send![class!(NSAttributedString), alloc];
    autorel(msg_send![s, initWithString: ns(text), attributes: attrs])
}

// ---- self-test views ----------------------------------------------------------------------

/// The cue on screen ("" when none) and the overlay frame in player-host coordinates.
pub(crate) unsafe fn state() -> (String, NSRect) {
    let ui = UI.get().unwrap();
    (ui.subs.text.clone(), frame_of(ui.subs.label.get()))
}

/// Whether the overlay is shown.
pub(crate) unsafe fn visible() -> bool {
    let ui = UI.get().unwrap();
    !ui.subs.label.get().is_null() && !is_hidden(ui.subs.label.get())
}

/// Layout problems of the overlay: outside the video, or over the transport bar or scrubber
/// (while the controls are up) or over the skip button or Up Next card. Empty when clear.
pub(crate) unsafe fn chrome_problems() -> Vec<String> {
    let ui = UI.get().unwrap();
    let mut out = Vec::new();
    if !visible() {
        out.push("overlay hidden".to_string());
        return out;
    }
    let f = frame_of(ui.subs.label.get());
    let host = bounds_of(ui.player_host.get());
    if f.origin.x < 0. || f.origin.y < 0. || f.origin.x + f.size.w > host.size.w || f.origin.y + f.size.h > host.size.h {
        out.push(format!("overlay outside the video at {:?}", (f.origin.x, f.origin.y, f.size.w, f.size.h)));
    }
    if ui.bar_shown {
        let bar = frame_of(ui.pbar.bar.get());
        if overlaps(f, bar) {
            out.push("overlaps the transport bar".to_string());
        }
        // The scrubber is laid out in the bar's own coordinates.
        let mut sc = frame_of(ui.pbar.scrub.get());
        sc.origin.x += bar.origin.x;
        sc.origin.y += bar.origin.y;
        if overlaps(f, sc) {
            out.push("overlaps the scrubber".to_string());
        }
    }
    for (name, v) in [("the Up Next card", ui.skip.card.get()), ("the skip button", ui.skip.pill.get())] {
        if !v.is_null() && !is_hidden(v) && overlaps(f, frame_of(v)) {
            out.push(format!("overlaps {}", name));
        }
    }
    out
}

fn overlaps(a: NSRect, b: NSRect) -> bool {
    a.origin.x < b.origin.x + b.size.w
        && b.origin.x < a.origin.x + a.size.w
        && a.origin.y < b.origin.y + b.size.h
        && b.origin.y < a.origin.y + a.size.h
}
