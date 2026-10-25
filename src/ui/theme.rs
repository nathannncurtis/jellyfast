//! Design tokens and small view helpers shared by every screen: spacing,
//! type scale, semantic colours, radii, shadows, materials and motion.
//! Nothing here is stateful; call sites pick tokens instead of raw numbers.
#![allow(unsafe_code)]
#![allow(dead_code)]
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use objc2::encode::{Encode, Encoding};
use crate::platform::appkit::*;

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
/// Custom transport bar at the bottom of the video player (scrubber row + button row).
pub const PLAYER_CTRL_H: f64 = 128.;
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
/// Elapsed / remaining clock on the video transport bar (10-foot sizing).
pub const T_PLAYER_TIME: TypeStyle = TypeStyle { size: 14., weight: Weight::Medium, mono: true };

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

pub unsafe fn font(t: TypeStyle) -> *mut AnyObject {
    if t.mono {
        msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: t.size, weight: t.weight.value()]
    } else {
        msg_send![class!(NSFont), systemFontOfSize: t.size, weight: t.weight.value()]
    }
}

// ---- Colour roles ------------------------------------------------------------
//
// Every role resolves through a dynamic NSColor whose provider reads the active
// palette (set by `set_active`, applied by shell::apply_theme). Views keep the colour
// object they were built with, so switching theme repaints them without a rebuild;
// layer-backed colours re-resolve through `refresh_appearance_tree`.
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
    FillActive,
    Accent,
    AccentHover,
    OnAccent,
    Danger,
    Ok,
    Warn,
    OnMedia,
    OnMedia2,
    Clear,
    /// Black scrims behind artwork and video chrome (palette `overlay`).
    Scrim,
    /// Drop-shadow colour (palette `shadow`).
    Shadow,
    /// Backdrop behind the video while a title loads (palette `tint`).
    PlayerBackdrop,
}
pub const C_TEXT: Col = Col::Text;
pub const C_TEXT2: Col = Col::Text2;
pub const C_TEXT3: Col = Col::Text3;
pub const C_CANVAS: Col = Col::Canvas;
pub const C_CARD: Col = Col::Card;
pub const C_SEPARATOR: Col = Col::Separator;
pub const C_FILL_SEL: Col = Col::FillSel;
pub const C_FILL_HOVER: Col = Col::FillHover;
pub const C_FILL_ACTIVE: Col = Col::FillActive;
pub const C_ACCENT: Col = Col::Accent;
pub const C_DANGER: Col = Col::Danger;
pub const C_OK: Col = Col::Ok;
pub const C_WARN: Col = Col::Warn;
pub const C_ON_MEDIA: Col = Col::OnMedia;
pub const C_ON_MEDIA2: Col = Col::OnMedia2;

/// Every role, in the order the provider blocks are numbered.
const ALL_ROLES: [Col; 21] = [
    Col::Text,
    Col::Text2,
    Col::Text3,
    Col::Canvas,
    Col::Card,
    Col::Separator,
    Col::FillSel,
    Col::FillHover,
    Col::FillActive,
    Col::Accent,
    Col::AccentHover,
    Col::OnAccent,
    Col::Danger,
    Col::Ok,
    Col::Warn,
    Col::OnMedia,
    Col::OnMedia2,
    Col::Clear,
    Col::Scrim,
    Col::Shadow,
    Col::PlayerBackdrop,
];

/// Palette slot (index into `palettes::KEYS`) a role reads, if it is palette-driven.
fn slot_of(c: Col) -> Option<usize> {
    Some(match c {
        Col::Card => 0,          // window
        Col::Canvas => 1,        // panel
        Col::FillSel => 2,       // surface
        Col::FillHover => 3,     // surface_hover
        Col::FillActive => 4,    // surface_active
        Col::Separator => 5,     // outline
        Col::Text => 6,
        Col::Text2 => 7,         // secondary
        Col::Text3 => 8,         // dim
        Col::Accent => 9,
        Col::AccentHover => 10,
        Col::OnAccent => 11,
        Col::Danger => 12,
        Col::Warn => 13,         // warning
        Col::Scrim => 14,        // overlay
        Col::Shadow => 15,
        Col::PlayerBackdrop => 16, // tint
        Col::Ok => 17,
        Col::OnMedia | Col::OnMedia2 | Col::Clear => return None,
    })
}

/// Colours of the active palette, indexed by `palettes::KEYS`. `None` means the
/// Jellyfast built-in (system colours, following the appearance).
static ACTIVE: std::sync::RwLock<Option<[crate::ui::palettes::Rgba; crate::ui::palettes::SLOTS]>> =
    std::sync::RwLock::new(None);

/// Make `theme` the active palette (`None` = Jellyfast Light/Dark). Views repaint on
/// their next draw; call `refresh_appearance_tree` to re-resolve layers now.
pub fn set_active(theme: Option<&crate::ui::palettes::Theme>) {
    if let Ok(mut a) = ACTIVE.write() {
        *a = theme.map(|t| t.colors);
    }
}

unsafe fn rgb_color(v: u32, alpha: f64) -> *mut AnyObject {
    let r = ((v >> 16) & 0xff) as f64 / 255.;
    let g = ((v >> 8) & 0xff) as f64 / 255.;
    let b = (v & 0xff) as f64 / 255.;
    msg_send![class!(NSColor), colorWithSRGBRed: r, green: g, blue: b, alpha: alpha]
}

/// System colour for a role under the Jellyfast built-ins (adapts to the appearance).
unsafe fn system_color(c: Col) -> *mut AnyObject {
    let cls = class!(NSColor);
    match c {
        Col::Text => msg_send![cls, labelColor],
        Col::Text2 => msg_send![cls, secondaryLabelColor],
        Col::Text3 => msg_send![cls, tertiaryLabelColor],
        Col::Canvas => msg_send![cls, controlBackgroundColor],
        Col::Card => msg_send![cls, windowBackgroundColor],
        Col::Separator => msg_send![cls, separatorColor],
        Col::FillSel | Col::FillActive => {
            let ok: Bool = msg_send![cls, respondsToSelector: sel!(tertiarySystemFillColor)];
            if ok == Bool::YES {
                msg_send![cls, tertiarySystemFillColor]
            } else {
                msg_send![cls, quaternaryLabelColor]
            }
        }
        Col::FillHover => {
            let ok: Bool = msg_send![cls, respondsToSelector: sel!(quaternarySystemFillColor)];
            if ok == Bool::YES {
                msg_send![cls, quaternarySystemFillColor]
            } else {
                msg_send![cls, quaternaryLabelColor]
            }
        }
        Col::Accent | Col::AccentHover => msg_send![cls, controlAccentColor],
        Col::OnAccent | Col::OnMedia => msg_send![cls, whiteColor],
        Col::OnMedia2 => {
            let w: *mut AnyObject = msg_send![cls, whiteColor];
            msg_send![w, colorWithAlphaComponent: 0.72f64]
        }
        Col::Danger => msg_send![cls, systemRedColor],
        Col::Ok => msg_send![cls, systemGreenColor],
        Col::Warn => msg_send![cls, systemOrangeColor],
        Col::Clear => msg_send![cls, clearColor],
        Col::Scrim | Col::Shadow => msg_send![cls, blackColor],
        Col::PlayerBackdrop => rgb_color(0x212121, 1.0),
    }
}

/// Resolve one role under the active palette, or the system colour for the built-ins.
unsafe fn resolve_role(c: Col) -> *mut AnyObject {
    let slot = slot_of(c);
    let palette = ACTIVE.read().ok().and_then(|a| *a);
    match (palette, slot) {
        (Some(p), Some(s)) => {
            let (v, a) = p[s];
            rgb_color(v, a as f64 / 255.)
        }
        _ => system_color(c),
    }
}

/// Block passed to `colorWithName:dynamicProvider:`. A hand-built block with no
/// captures (never copied, leaked once per role); the role index sits after the
/// descriptor.
#[repr(C)]
struct RoleBlock {
    isa: *const std::os::raw::c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*mut RoleBlock, *mut AnyObject) -> *mut AnyObject,
    desc: *const BlockDesc,
    role: usize,
}
#[repr(C)]
struct BlockDesc {
    reserved: u64,
    size: u64,
}
static BLOCK_DESC: BlockDesc = BlockDesc { reserved: 0, size: std::mem::size_of::<RoleBlock>() as u64 };
#[link(name = "System", kind = "dylib")]
extern "C" {
    static _NSConcreteGlobalBlock: *const std::os::raw::c_void;
}

unsafe extern "C" fn role_provider(b: *mut RoleBlock, _appearance: *mut AnyObject) -> *mut AnyObject {
    resolve_role(ALL_ROLES[(*b).role])
}

thread_local! {
    static ROLE_COLORS: std::cell::RefCell<[usize; 21]> = const { std::cell::RefCell::new([0; 21]) };
}

/// Semantic colour for a role (autoreleased; the dynamic object itself is kept alive).
pub unsafe fn color(c: Col) -> *mut AnyObject {
    let idx = ALL_ROLES.iter().position(|r| *r == c).unwrap_or(0);
    let cached = ROLE_COLORS.with(|v| v.borrow()[idx]);
    if cached != 0 {
        return cached as *mut AnyObject;
    }
    let blk: *mut RoleBlock = Box::leak(Box::new(RoleBlock {
        isa: _NSConcreteGlobalBlock,
        flags: 1 << 28, // BLOCK_IS_GLOBAL: the runtime never copies or frees it
        reserved: 0,
        invoke: role_provider,
        desc: &BLOCK_DESC,
        role: idx,
    }));
    let name = ns(&format!("jf.role.{idx}"));
    let obj: *mut AnyObject = msg_send![class!(NSColor), colorWithName: name, dynamicProvider: blk as *mut AnyObject];
    let _: *mut AnyObject = msg_send![obj, retain];
    ROLE_COLORS.with(|v| v.borrow_mut()[idx] = obj as usize);
    obj
}
/// Role colour with alpha (autoreleased).
pub unsafe fn color_a(c: Col, a: f64) -> *mut AnyObject {
    let base = color(c);
    msg_send![base, colorWithAlphaComponent: a]
}
/// Resolved CGColor for a role under the active palette. Layers that must follow the
/// theme need this re-applied from `apply_theme` (see `refresh_appearance_tree`).
pub unsafe fn cg(c: Col) -> *mut CGColor {
    msg_send![color(c), CGColor]
}
pub unsafe fn cg_a(c: Col, alpha: f64) -> *mut CGColor {
    msg_send![color_a(c, alpha), CGColor]
}
pub unsafe fn cg_black(alpha: f64) -> *mut CGColor {
    cg_a(Col::Scrim, alpha)
}

/// Re-resolve layer-backed colours and redraw every view under `root`. Needed when the
/// palette changes without an appearance flip, since AppKit only notifies on flips.
pub unsafe fn refresh_appearance_tree(root: *mut AnyObject) {
    if root.is_null() {
        return;
    }
    let _: () = msg_send![root, viewDidChangeEffectiveAppearance];
    let _: () = msg_send![root, setNeedsDisplay: Bool::YES];
    let subs: *mut AnyObject = msg_send![root, subviews];
    let n: usize = msg_send![subs, count];
    for i in 0..n {
        let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        refresh_appearance_tree(v);
    }
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
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let b: Bool = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        b == Bool::YES
    }
}
pub fn reduce_transparency() -> bool {
    unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let b: Bool = msg_send![ws, accessibilityDisplayShouldReduceTransparency];
        b == Bool::YES
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
pub unsafe fn timing_ease_out() -> *mut AnyObject {
    msg_send![class!(CAMediaTimingFunction), functionWithName: ns("easeOut")]
}
pub unsafe fn timing_ease_in_out() -> *mut AnyObject {
    msg_send![class!(CAMediaTimingFunction), functionWithName: ns("easeInEaseOut")]
}
pub unsafe fn timing_emph() -> *mut AnyObject {
    // Unnamed selector parts are not expressible in msg_send!, so go through the runtime directly.
    let f: *mut AnyObject = objc2::runtime::MessageReceiver::send_message(
        class!(CAMediaTimingFunction),
        Sel::register(c"functionWithControlPoints::::"),
        (0.2f32, 0.8f32, 0.2f32, 1.0f32),
    );
    f
}

/// Begin an implicit-animation group for view properties set via `animator`.
pub unsafe fn anim_begin(d: f64, timing: *mut AnyObject) {
    let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
    let ctx: *mut AnyObject = msg_send![class!(NSAnimationContext), currentContext];
    let _: () = msg_send![ctx, setDuration: dur(d)];
    let _: () = msg_send![ctx, setTimingFunction: timing];
    let _: () = msg_send![ctx, setAllowsImplicitAnimation: Bool::YES];
}
pub unsafe fn anim_end() {
    let _: () = msg_send![class!(NSAnimationContext), endGrouping];
}
/// Fade any hidden/shown swap inside `container` (made layer-backed here).
/// Call it in the same run-loop turn, *before* the swap.
pub unsafe fn crossfade(container: *mut AnyObject, d: f64) {
    if dur(d) <= 0.0 {
        return;
    }
    let t: *mut AnyObject = msg_send![class!(CATransition), animation];
    let _: () = msg_send![t, setType: ns("fade")];
    let _: () = msg_send![t, setDuration: dur(d)];
    let l = layer_of(container);
    let _: () = msg_send![l, addAnimation: t, forKey: ns("jrFade")];
}
/// CATransaction with a duration (0 disables animation). Pair with `tx_end`.
pub unsafe fn tx_begin(d: f64) {
    let _: () = msg_send![class!(CATransaction), begin];
    let d = dur(d);
    if d <= 0.0 {
        let _: () = msg_send![class!(CATransaction), setDisableActions: Bool::YES];
    } else {
        let _: () = msg_send![class!(CATransaction), setAnimationDuration: d];
    }
}
pub unsafe fn tx_begin_instant() {
    let _: () = msg_send![class!(CATransaction), begin];
    let _: () = msg_send![class!(CATransaction), setDisableActions: Bool::YES];
}
pub unsafe fn tx_end() {
    let _: () = msg_send![class!(CATransaction), commit];
}

// ---- Layer helpers --------------------------------------------------------------------
/// Radius with the continuous ("squircle") corner curve.
pub unsafe fn set_corner(layer: *mut AnyObject, r: f64) {
    let _: () = msg_send![layer, setCornerRadius: r];
    let _: () = msg_send![layer, setCornerCurve: ns("continuous")];
}

// ---- Labels ---------------------------------------------------------------------------------
/// Single-line label that truncates with an ellipsis (autoreleased).
pub unsafe fn label(text: &str, t: TypeStyle, c: Col, align: usize) -> *mut AnyObject {
    let v: *mut AnyObject = msg_send![class!(NSTextField), labelWithString: ns(text)];
    let _: () = msg_send![v, setFont: font(t)];
    let _: () = msg_send![v, setTextColor: color(c)];
    let _: () = msg_send![v, setAlignment: align as isize];
    let cell: *mut AnyObject = msg_send![v, cell];
    let _: () = msg_send![cell, setLineBreakMode: 4usize]; // truncating tail
    let _: () = msg_send![v, setMaximumNumberOfLines: 1isize];
    v
}
/// Multi-line label that wraps (autoreleased).
pub unsafe fn wrapping_label(text: &str, t: TypeStyle, c: Col, align: usize) -> *mut AnyObject {
    let v: *mut AnyObject = msg_send![class!(NSTextField), wrappingLabelWithString: ns(text)];
    let _: () = msg_send![v, setFont: font(t)];
    let _: () = msg_send![v, setTextColor: color(c)];
    let _: () = msg_send![v, setAlignment: align as isize];
    v
}
pub unsafe fn set_label_color(v: *mut AnyObject, c: Col) {
    let _: () = msg_send![v, setTextColor: color(c)];
}

// ---- Symbols ----------------------------------------------------------------------------------
/// SF Symbol at a size and weight (autoreleased; nil when the symbol is missing).
pub unsafe fn symbol(name: &str, point_size: f64, weight: Weight) -> *mut AnyObject {
    let img: *mut AnyObject =
        msg_send![class!(NSImage), imageWithSystemSymbolName: ns(name), accessibilityDescription: nil()];
    if img.is_null() {
        return img;
    }
    let cfg: *mut AnyObject = msg_send![class!(NSImageSymbolConfiguration),
        configurationWithPointSize: point_size, weight: weight.value()];
    let scaled: *mut AnyObject = msg_send![img, imageWithSymbolConfiguration: cfg];
    if scaled.is_null() {
        img
    } else {
        scaled
    }
}

// ---- NSBox "custom" surface (redraws dynamically with the appearance) ---------------------------
pub unsafe fn surface(frame: NSRect, fill: Col, radius: f64) -> *mut AnyObject {
    let b: *mut AnyObject = msg_send![class!(NSBox), alloc];
    let b: *mut AnyObject = msg_send![b, initWithFrame: frame];
    let _: () = msg_send![b, setBoxType: 4usize]; // NSBoxCustom
    let _: () = msg_send![b, setBorderWidth: 0.0f64];
    let _: () = msg_send![b, setTitlePosition: 0usize]; // NSNoTitle
    let _: () = msg_send![b, setContentViewMargins: size(0., 0.)];
    let _: () = msg_send![b, setCornerRadius: radius];
    let _: () = msg_send![b, setFillColor: color(fill)];
    autorel(b)
}
/// Hairline separator (NSBox separator type, 1 pt).
pub unsafe fn hairline(frame: NSRect) -> *mut AnyObject {
    let b: *mut AnyObject = msg_send![class!(NSBox), alloc];
    let b: *mut AnyObject = msg_send![b, initWithFrame: frame];
    let _: () = msg_send![b, setBoxType: 2usize]; // NSBoxSeparator
    autorel(b)
}

// ---- CoreAnimation / CoreGraphics bits not covered by msg_send ------------------------------------
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CATransform3D {
    pub m: [f64; 16],
}
unsafe impl Encode for CATransform3D {
    const ENCODING: Encoding = Encoding::Struct(
        "CATransform3D",
        &[
            Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double,
            Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double,
            Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double,
            Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double,
        ],
    );
}
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {
    pub fn CATransform3DMakeScale(sx: f64, sy: f64, sz: f64) -> CATransform3D;
    pub static CATransform3DIdentity: CATransform3D;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPathCreateWithRoundedRect(rect: NSRect, cw: f64, ch: f64, t: *const std::os::raw::c_void) -> *mut CGPath;
    fn CGPathRelease(p: *mut CGPath);
}
/// Set a rounded-rect `shadowPath` for the layer (cheap shadows, no offscreen pass).
pub unsafe fn set_shadow_path(layer: *mut AnyObject, w: f64, h: f64, r: f64) {
    let path = CGPathCreateWithRoundedRect(rect(0., 0., w, h), r, r, std::ptr::null());
    let _: () = msg_send![layer, setShadowPath: path];
    CGPathRelease(path);
}
/// CGImage backing of an NSImage (not retained; valid while the image lives).
pub unsafe fn cg_image(img: *mut AnyObject) -> *mut CGImage {
    msg_send![img, CGImageForProposedRect: std::ptr::null_mut::<NSRect>(), context: nil(), hints: nil()]
}
pub unsafe fn black_cg() -> *mut CGColor {
    cg_black(1.0)
}

pub const _UNUSED_NO: Bool = Bool::NO;
