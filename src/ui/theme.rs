//! Design tokens and small view helpers shared by every screen: spacing,
//! type scale, semantic colours, radii, shadows, materials and motion.
//! Nothing here is stateful; call sites pick tokens instead of raw numbers.
#![allow(unsafe_code)]
#![allow(dead_code)]
use crate::platform::appkit::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, BOOL, NO, YES};
use objc::{sel, sel_impl};

// ---- Spacing (4-pt grid) -----------------------------------------------
pub const SP1: f64 = 4.;
pub const SP2: f64 = 8.;
pub const SP3: f64 = 12.;
pub const SP4: f64 = 16.;
pub const SP5: f64 = 20.;
pub const SP6: f64 = 24.;
pub const SP8: f64 = 32.;
pub const SP10: f64 = 40.;

// ---- Layout constants ----------------------------------------------------
pub const SIDEBAR_W: f64 = 240.;
/// Reserved band at the top of the window (the traffic lights live here).
pub const TITLEBAR_H: f64 = 52.;
/// Content left/right gutter.
pub const EDGE: f64 = SP6;
pub const GRID_COL_GAP: f64 = SP5;
pub const GRID_ROW_GAP: f64 = SP6;
pub const TILE_MIN_W: f64 = 160.;
pub const POSTER_ASPECT: f64 = 1.5;
/// 8 gap + 17 title + 1 + 14 subtitle + 4.
pub const CELL_TEXT_H: f64 = 44.;
pub const SIDEBAR_ROW_H: f64 = 28.;
pub const NOW_BAR_H: f64 = 72.;
pub const PLAYER_BAR_H: f64 = 52.;
pub const HEADER_H: f64 = 44.;

// ---- Radii -----------------------------------------------------------------
pub const R_POSTER: f64 = 8.;
pub const R_THUMB: f64 = 6.;
pub const R_ROW: f64 = 6.;
pub const R_CARD: f64 = 16.;

// ---- Type scale --------------------------------------------------------------
#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSFontWeightRegular: f64;
    static NSFontWeightMedium: f64;
    static NSFontWeightSemibold: f64;
    static NSFontWeightBold: f64;
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
}
impl Weight {
    pub fn value(self) -> f64 {
        unsafe {
            match self {
                Weight::Regular => NSFontWeightRegular,
                Weight::Medium => NSFontWeightMedium,
                Weight::Semibold => NSFontWeightSemibold,
                Weight::Bold => NSFontWeightBold,
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TypeStyle {
    pub size: f64,
    pub weight: Weight,
    pub mono: bool,
}
pub const T_DISPLAY: TypeStyle = TypeStyle { size: 26., weight: Weight::Bold, mono: false };
pub const T_TITLE: TypeStyle = TypeStyle { size: 20., weight: Weight::Semibold, mono: false };
pub const T_HEADLINE: TypeStyle = TypeStyle { size: 15., weight: Weight::Semibold, mono: false };
pub const T_BODY: TypeStyle = TypeStyle { size: 13., weight: Weight::Regular, mono: false };
pub const T_BODY_EM: TypeStyle = TypeStyle { size: 13., weight: Weight::Medium, mono: false };
pub const T_CALLOUT: TypeStyle = TypeStyle { size: 12., weight: Weight::Regular, mono: false };
pub const T_CAPTION: TypeStyle = TypeStyle { size: 11., weight: Weight::Regular, mono: false };
pub const T_CAPTION_EM: TypeStyle = TypeStyle { size: 11., weight: Weight::Semibold, mono: false };
pub const T_MONO: TypeStyle = TypeStyle { size: 11., weight: Weight::Medium, mono: true };

/// NSTextAlignment values differ by architecture (NSText.h): Intel keeps the
/// classic Right = 1 / Center = 2, Apple silicon uses the iOS values.
pub const ALIGN_LEFT: usize = 0;
#[cfg(target_arch = "x86_64")]
pub const ALIGN_RIGHT: usize = 1;
#[cfg(target_arch = "x86_64")]
pub const ALIGN_CENTER: usize = 2;
#[cfg(not(target_arch = "x86_64"))]
pub const ALIGN_CENTER: usize = 1;
#[cfg(not(target_arch = "x86_64"))]
pub const ALIGN_RIGHT: usize = 2;
pub const ALIGN_NATURAL: usize = 4;

pub unsafe fn font(t: TypeStyle) -> *mut Object {
    if t.mono {
        msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: t.size weight: t.weight.value()]
    } else {
        msg_send![class!(NSFont), systemFontOfSize: t.size weight: t.weight.value()]
    }
}

// ---- Colour roles ------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Col {
    Text,
    Text2,
    Text3,
    Canvas,
    Card,
    Separator,
    FillSel,
    FillHover,
    Accent,
    Danger,
    Ok,
    Warn,
    OnMedia,
    OnMedia2,
    Clear,
}
pub const C_TEXT: Col = Col::Text;
pub const C_TEXT2: Col = Col::Text2;
pub const C_TEXT3: Col = Col::Text3;
pub const C_CANVAS: Col = Col::Canvas;
pub const C_CARD: Col = Col::Card;
pub const C_SEPARATOR: Col = Col::Separator;
pub const C_FILL_SEL: Col = Col::FillSel;
pub const C_FILL_HOVER: Col = Col::FillHover;
pub const C_ACCENT: Col = Col::Accent;
pub const C_DANGER: Col = Col::Danger;
pub const C_OK: Col = Col::Ok;
pub const C_WARN: Col = Col::Warn;
pub const C_ON_MEDIA: Col = Col::OnMedia;
pub const C_ON_MEDIA2: Col = Col::OnMedia2;

/// Semantic NSColor for a role (autoreleased / shared, never freed by us).
pub unsafe fn color(c: Col) -> *mut Object {
    let cls = class!(NSColor);
    match c {
        Col::Text => msg_send![cls, labelColor],
        Col::Text2 => msg_send![cls, secondaryLabelColor],
        Col::Text3 => msg_send![cls, tertiaryLabelColor],
        Col::Canvas => msg_send![cls, controlBackgroundColor],
        Col::Card => msg_send![cls, windowBackgroundColor],
        Col::Separator => msg_send![cls, separatorColor],
        Col::FillSel => {
            let ok: BOOL = msg_send![cls, respondsToSelector: sel!(tertiarySystemFillColor)];
            if ok == YES {
                msg_send![cls, tertiarySystemFillColor]
            } else {
                msg_send![cls, quaternaryLabelColor]
            }
        }
        Col::FillHover => {
            let ok: BOOL = msg_send![cls, respondsToSelector: sel!(quaternarySystemFillColor)];
            if ok == YES {
                msg_send![cls, quaternarySystemFillColor]
            } else {
                msg_send![cls, quaternaryLabelColor]
            }
        }
        Col::Accent => msg_send![cls, controlAccentColor],
        Col::Danger => msg_send![cls, systemRedColor],
        Col::Ok => msg_send![cls, systemGreenColor],
        Col::Warn => msg_send![cls, systemOrangeColor],
        Col::OnMedia => msg_send![cls, whiteColor],
        Col::OnMedia2 => {
            let w: *mut Object = msg_send![cls, whiteColor];
            msg_send![w, colorWithAlphaComponent: 0.72f64]
        }
        Col::Clear => msg_send![cls, clearColor],
    }
}
/// Role colour with alpha (autoreleased).
pub unsafe fn color_a(c: Col, a: f64) -> *mut Object {
    let base = color(c);
    msg_send![base, colorWithAlphaComponent: a]
}
/// Resolved CGColor for a role under the *current* appearance. Layers that
/// must follow light/dark need this re-applied from `apply_theme`.
pub unsafe fn cg(c: Col) -> *mut Object {
    msg_send![color(c), CGColor]
}
pub unsafe fn cg_black(alpha: f64) -> *mut Object {
    let b: *mut Object = msg_send![class!(NSColor), blackColor];
    let c: *mut Object = msg_send![b, colorWithAlphaComponent: alpha];
    msg_send![c, CGColor]
}

// ---- Shadows ---------------------------------------------------------------------
#[derive(Clone, Copy)]
pub struct Shadow {
    pub opacity: f32,
    pub radius: f64,
    pub dy: f64,
}
pub const SH_REST: Shadow = Shadow { opacity: 0.18, radius: 6., dy: -2. };
pub const SH_HOVER: Shadow = Shadow { opacity: 0.32, radius: 14., dy: -6. };
pub const SH_CARD: Shadow = Shadow { opacity: 0.25, radius: 24., dy: -8. };

// ---- Materials (NSVisualEffectView.h) --------------------------------------------
pub const MAT_SIDEBAR: i64 = 7;
pub const MAT_HEADER: i64 = 10;
pub const MAT_UNDER_WINDOW: i64 = 21;
pub const MAT_HUD: i64 = 13;
pub const MAT_POPOVER: i64 = 6;

// ---- Motion ---------------------------------------------------------------------------
pub const M_FAST: f64 = 0.12;
pub const M_STD: f64 = 0.20;
pub const M_EMPH: f64 = 0.30;
pub const M_IDLE_HIDE: f64 = 2.5;

pub fn reduce_motion() -> bool {
    unsafe {
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let b: BOOL = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        b == YES
    }
}
pub fn reduce_transparency() -> bool {
    unsafe {
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let b: BOOL = msg_send![ws, accessibilityDisplayShouldReduceTransparency];
        b == YES
    }
}
/// Duration after honouring Reduce Motion (fades up to M_FAST remain).
pub fn dur(d: f64) -> f64 {
    if reduce_motion() {
        d.min(M_FAST) * 0.0
    } else {
        d
    }
}
pub unsafe fn timing_ease_out() -> *mut Object {
    msg_send![class!(CAMediaTimingFunction), functionWithName: ns("easeOut")]
}
pub unsafe fn timing_ease_in_out() -> *mut Object {
    msg_send![class!(CAMediaTimingFunction), functionWithName: ns("easeInEaseOut")]
}
pub unsafe fn timing_emph() -> *mut Object {
    // Unnamed selector parts are not expressible in msg_send!, so go through the runtime directly.
    objc::__send_message(
        class!(CAMediaTimingFunction),
        objc::runtime::Sel::register("functionWithControlPoints::::"),
        (0.2f32, 0.8f32, 0.2f32, 1.0f32),
    )
    .unwrap_or(std::ptr::null_mut())
}

/// Begin an implicit-animation group for view properties set via `animator`.
pub unsafe fn anim_begin(d: f64, timing: *mut Object) {
    let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
    let ctx: *mut Object = msg_send![class!(NSAnimationContext), currentContext];
    let _: () = msg_send![ctx, setDuration: dur(d)];
    let _: () = msg_send![ctx, setTimingFunction: timing];
    let _: () = msg_send![ctx, setAllowsImplicitAnimation: YES];
}
pub unsafe fn anim_end() {
    let _: () = msg_send![class!(NSAnimationContext), endGrouping];
}
/// Fade any hidden/shown swap inside `container` (made layer-backed here).
/// Call it in the same run-loop turn, *before* the swap.
pub unsafe fn crossfade(container: *mut Object, d: f64) {
    if dur(d) <= 0.0 {
        return;
    }
    let t: *mut Object = msg_send![class!(CATransition), animation];
    let _: () = msg_send![t, setType: ns("fade")];
    let _: () = msg_send![t, setDuration: dur(d)];
    let l = layer_of(container);
    let _: () = msg_send![l, addAnimation: t forKey: ns("jrFade")];
}
/// CATransaction with a duration (0 disables animation). Pair with `tx_end`.
pub unsafe fn tx_begin(d: f64) {
    let _: () = msg_send![class!(CATransaction), begin];
    let d = dur(d);
    if d <= 0.0 {
        let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
    } else {
        let _: () = msg_send![class!(CATransaction), setAnimationDuration: d];
    }
}
pub unsafe fn tx_begin_instant() {
    let _: () = msg_send![class!(CATransaction), begin];
    let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
}
pub unsafe fn tx_end() {
    let _: () = msg_send![class!(CATransaction), commit];
}

// ---- Layer helpers --------------------------------------------------------------------
/// Radius with the continuous ("squircle") corner curve.
pub unsafe fn set_corner(layer: *mut Object, r: f64) {
    let _: () = msg_send![layer, setCornerRadius: r];
    let _: () = msg_send![layer, setCornerCurve: ns("continuous")];
}

// ---- Labels ---------------------------------------------------------------------------------
/// Single-line label that truncates with an ellipsis (autoreleased).
pub unsafe fn label(text: &str, t: TypeStyle, c: Col, align: usize) -> *mut Object {
    let v: *mut Object = msg_send![class!(NSTextField), labelWithString: ns(text)];
    let _: () = msg_send![v, setFont: font(t)];
    let _: () = msg_send![v, setTextColor: color(c)];
    let _: () = msg_send![v, setAlignment: align as isize];
    let cell: *mut Object = msg_send![v, cell];
    let _: () = msg_send![cell, setLineBreakMode: 4usize]; // truncating tail
    let _: () = msg_send![v, setMaximumNumberOfLines: 1isize];
    v
}
/// Multi-line label that wraps (autoreleased).
pub unsafe fn wrapping_label(text: &str, t: TypeStyle, c: Col, align: usize) -> *mut Object {
    let v: *mut Object = msg_send![class!(NSTextField), wrappingLabelWithString: ns(text)];
    let _: () = msg_send![v, setFont: font(t)];
    let _: () = msg_send![v, setTextColor: color(c)];
    let _: () = msg_send![v, setAlignment: align as isize];
    v
}
pub unsafe fn set_label_color(v: *mut Object, c: Col) {
    let _: () = msg_send![v, setTextColor: color(c)];
}

// ---- Symbols ----------------------------------------------------------------------------------
/// SF Symbol at a size and weight (autoreleased; nil when the symbol is missing).
pub unsafe fn symbol(name: &str, point_size: f64, weight: Weight) -> *mut Object {
    let img: *mut Object =
        msg_send![class!(NSImage), imageWithSystemSymbolName: ns(name) accessibilityDescription: nil()];
    if img.is_null() {
        return img;
    }
    let cfg: *mut Object = msg_send![class!(NSImageSymbolConfiguration),
        configurationWithPointSize: point_size weight: weight.value()];
    let scaled: *mut Object = msg_send![img, imageWithSymbolConfiguration: cfg];
    if scaled.is_null() {
        img
    } else {
        scaled
    }
}

// ---- NSBox "custom" surface (redraws dynamically with the appearance) ---------------------------
pub unsafe fn surface(frame: NSRect, fill: Col, radius: f64) -> *mut Object {
    let b: *mut Object = msg_send![class!(NSBox), alloc];
    let b: *mut Object = msg_send![b, initWithFrame: frame];
    let _: () = msg_send![b, setBoxType: 4usize]; // NSBoxCustom
    let _: () = msg_send![b, setBorderWidth: 0.0f64];
    let _: () = msg_send![b, setTitlePosition: 0usize]; // NSNoTitle
    let _: () = msg_send![b, setContentViewMargins: size(0., 0.)];
    let _: () = msg_send![b, setCornerRadius: radius];
    let _: () = msg_send![b, setFillColor: color(fill)];
    autorel(b)
}
/// Hairline separator (NSBox separator type, 1 pt).
pub unsafe fn hairline(frame: NSRect) -> *mut Object {
    let b: *mut Object = msg_send![class!(NSBox), alloc];
    let b: *mut Object = msg_send![b, initWithFrame: frame];
    let _: () = msg_send![b, setBoxType: 2usize]; // NSBoxSeparator
    autorel(b)
}

pub const _UNUSED_NO: BOOL = NO;
