//! Seerr window: search movies/TV, request them, and list "My requests".
//!
//! Non-modal NSWindow created once and reused (closing only hides it). Rows are
//! plain views inside a flipped document view of an NSScrollView, rebuilt on
//! every state change (result sets are small). All state lives in `Ui.seerr`.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use crate::ui::theme::*;
use std::collections::HashMap;
use std::ffi::c_void;

pub struct SeerrUi {
    pub win: Ptr,
    pub tabs: Ptr,
    pub search: Ptr,
    pub status: Ptr,
    pub scroll: Ptr,
    pub doc: Ptr,
    pub hint: Ptr,
    pub hint_btn: Ptr,
    /// Centred empty-state over the list (hidden while rows exist).
    pub empty: Ptr,
    pub empty_icon: Ptr,
    pub empty_title: Ptr,
    pub empty_body: Ptr,
    /// Thumbnails in the current rows: (poster_path, art view, placeholder glyph).
    pub thumbs: Vec<(String, *mut AnyObject, *mut AnyObject)>,
    /// Decoded poster images by poster_path (owned, +1).
    pub images: HashMap<String, *mut AnyObject>,
    pub requests_mode: bool,
    pub results: Vec<SeerrResult>,
    pub results_loaded: bool,
    pub requests: Vec<SeerrRequestItem>,
    pub requests_loaded: bool,
    /// (media_type, tmdb_id, title) of requests sent and not yet answered.
    pub inflight: Vec<(String, u64, String)>,
    /// Row views currently inside `doc` (the superview owns them).
    pub rows: Vec<*mut AnyObject>,
    // Settings sheet widgets
    pub s_on: Ptr,
    pub s_url: Ptr,
    pub s_key: Ptr,
    pub s_user: Ptr,
    pub s_pass: Ptr,
    /// Popup: 0 = "Jellyfin account", 1 = "API key".
    pub s_mode: Ptr,
    /// Views shown only in Jellyfin-account mode / only in API-key mode.
    pub s_login_group: Vec<Ptr>,
    pub s_key_group: Vec<Ptr>,
    pub s_status: Ptr,
    /// (enabled, url, username) of the SaveSeerr awaiting a SeerrStatus reply.
    pub pending: Option<(bool, String, String)>,
}
impl SeerrUi {
    pub fn new() -> SeerrUi {
        let n = Ptr(std::ptr::null_mut());
        SeerrUi {
            win: n,
            tabs: n,
            search: n,
            status: n,
            scroll: n,
            doc: n,
            hint: n,
            hint_btn: n,
            empty: n,
            empty_icon: n,
            empty_title: n,
            empty_body: n,
            thumbs: vec![],
            images: HashMap::new(),
            requests_mode: false,
            results: vec![],
            results_loaded: false,
            requests: vec![],
            requests_loaded: false,
            inflight: vec![],
            rows: vec![],
            s_on: n,
            s_url: n,
            s_key: n,
            s_user: n,
            s_pass: n,
            s_mode: n,
            s_login_group: vec![],
            s_key_group: vec![],
            s_status: n,
            pending: None,
        }
    }
}

// ---- Pure view-model (unit tested) ------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct RowSpec {
    pub title: String,
    pub sub: String,
    pub badge: String,
    /// Role name: "ok" | "warn" | "danger" | "neutral".
    pub badge_color: &'static str,
    pub can_request: bool,
    /// TMDB-relative poster path, fetched through the Seerr server.
    pub poster: Option<String>,
    /// SF Symbol for the thumbnail placeholder.
    pub glyph: &'static str,
}

pub fn media_label(media_type: &str) -> String {
    match media_type {
        "movie" => "Movie".into(),
        "tv" => "TV".into(),
        o => o.to_string(),
    }
}
fn media_glyph(media_type: &str) -> &'static str {
    if media_type == "tv" {
        "tv"
    } else {
        "film"
    }
}
fn avail_color(a: Availability) -> &'static str {
    match a {
        Availability::Available => "ok",
        Availability::Pending | Availability::Processing | Availability::PartiallyAvailable => "warn",
        Availability::Blocklisted => "danger",
        Availability::NotRequested => "neutral",
    }
}
pub fn result_spec(r: &SeerrResult) -> RowSpec {
    let mut sub = media_label(&r.media_type);
    if let Some(y) = r.year {
        sub.push_str(&format!(" · {}", y));
    }
    RowSpec {
        title: r.title.clone(),
        sub,
        badge: r.availability.label().to_string(),
        badge_color: avail_color(r.availability),
        can_request: r.availability == Availability::NotRequested,
        poster: r.poster_path.clone(),
        glyph: media_glyph(&r.media_type),
    }
}
pub fn request_spec(r: &SeerrRequestItem) -> RowSpec {
    let mut sub = media_label(&r.media_type);
    if let Some(y) = r.year {
        sub.push_str(&format!(" · {}", y));
    }
    if !r.requested_by.is_empty() {
        sub.push_str(&format!(" · requested by {}", r.requested_by));
    }
    let badge = if matches!(r.availability, Availability::NotRequested | Availability::Pending) {
        r.status.label().to_string()
    } else {
        format!("{} · {}", r.status.label(), r.availability.label())
    };
    let color = match r.status {
        RequestStatus::Declined | RequestStatus::Failed => "danger",
        RequestStatus::Completed => "ok",
        RequestStatus::Pending | RequestStatus::Approved => "warn",
        RequestStatus::Unknown => avail_color(r.availability),
    };
    RowSpec {
        title: r.title.clone(),
        sub,
        badge,
        badge_color: color,
        can_request: false,
        poster: r.poster_path.clone(),
        glyph: media_glyph(&r.media_type),
    }
}

pub fn is_configured(s: &Settings) -> bool {
    s.seerr_enabled && !s.seerr_url.trim().is_empty()
}

/// What the list area shows when there are no rows: (symbol, title, body).
/// `None` while a search or load is still in flight (the status line says so).
pub fn empty_state(requests_mode: bool, loaded: bool, query: &str) -> Option<(&'static str, &'static str, String)> {
    if requests_mode {
        return loaded.then(|| ("tray", "No requests yet", "You haven't requested anything yet.".to_string()));
    }
    let q = query.trim();
    if q.is_empty() {
        Some(("magnifyingglass", "Find a title", "Search for a movie or show to request.".to_string()))
    } else if loaded {
        Some(("magnifyingglass", "No results", format!("No matches for “{}”.", clip(q, 40))))
    } else {
        None
    }
}
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

// ---- Classes ----------------------------------------------------------------

extern "C" fn yes(_t: *mut AnyObject, _c: Sel) -> Bool {
    Bool::YES
}

/// Hover: the row's background box is transparent unless the pointer is inside.
fn row_hover(this: *mut AnyObject, on: bool) {
    unsafe {
        let bg = *ivar_ptr::<*mut c_void>(&*this, c"jrBg") as *mut AnyObject;
        if !bg.is_null() {
            let _: () = msg_send![bg, setTransparent: Bool::new(!on)];
        }
    }
}
extern "C" fn row_enter(this: *mut AnyObject, _c: Sel, _e: *mut AnyObject) {
    row_hover(this, true);
}
extern "C" fn row_exit(this: *mut AnyObject, _c: Sel, _e: *mut AnyObject) {
    row_hover(this, false);
}

pub(crate) unsafe fn register_seerr_classes() {
    let mut d = ClassBuilder::new(c"JRFlipView", class!(NSView)).unwrap();
    d.add_method(sel!(isFlipped), yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    d.register();
    let mut r = ClassBuilder::new(c"JRSeerrRow", class!(NSView)).unwrap();
    r.add_ivar::<*mut c_void>(c"jrBg");
    r.add_method(sel!(isFlipped), yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    r.add_method(sel!(mouseEntered:), row_enter as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    r.add_method(sel!(mouseExited:), row_exit as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    r.register();
}

unsafe fn class_view(name: &[u8], frame: NSRect) -> *mut AnyObject {
    let cls = AnyClass::get(std::ffi::CStr::from_bytes_with_nul(name).expect("NUL-terminated class name")).expect("class registered");
    let v: *mut AnyObject = msg_send![cls, alloc];
    msg_send![v, initWithFrame: frame] // +1; caller autorels after add
}
unsafe fn flip_view(frame: NSRect) -> *mut AnyObject {
    class_view(b"JRFlipView\0", frame)
}

// ---- Centered empty/hint state ----------------------------------------------

const STATE_W: f64 = 360.;

pub struct StateParts {
    pub host: *mut AnyObject,
    pub icon: *mut AnyObject,
    pub title: *mut AnyObject,
    pub body: *mut AnyObject,
    pub btn: *mut AnyObject,
}

/// Symbol, title, body (and optionally a button) stacked and centred in `frame`.
unsafe fn make_state(frame: NSRect, with_btn: bool) -> StateParts {
    let ui = UI.get_mut().unwrap();
    let host: *mut AnyObject = msg_send![class!(NSView), alloc];
    let host: *mut AnyObject = msg_send![host, initWithFrame: frame];
    set_autoresize(host, RESIZE_W | RESIZE_H);
    let ih = 40. + SP3 + 28. + SP2 + 36. + if with_btn { SP4 + 28. } else { 0. };
    let inner: *mut AnyObject = autorel(msg_send![class!(NSView), alloc]);
    let inner: *mut AnyObject = msg_send![inner, initWithFrame: rect(
        ((frame.size.w - STATE_W) / 2.).floor(), ((frame.size.h - ih) / 2.).floor(), STATE_W, ih)];
    set_autoresize(inner, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(host, inner);
    let mut y = ih - 40.;
    let icon: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let icon: *mut AnyObject = autorel(msg_send![icon, initWithFrame: rect((STATE_W - 40.) / 2., y, 40., 40.)]);
    let _: () = msg_send![icon, setContentTintColor: color(C_TEXT3)];
    add_sub(inner, icon);
    y -= SP3 + 28.;
    let title = label("", T_TITLE, C_TEXT, ALIGN_CENTER);
    set_frame(title, rect(0., y, STATE_W, 28.));
    add_sub(inner, title);
    y -= SP2 + 36.;
    let body = wrapping_label("", T_CALLOUT, C_TEXT2, ALIGN_CENTER);
    set_frame(body, rect(0., y, STATE_W, 36.));
    add_sub(inner, body);
    let mut btn = std::ptr::null_mut();
    if with_btn {
        y -= SP4 + 28.;
        btn = msg_send![class!(NSButton), buttonWithTitle: ns("Open Settings…"), target: ui.ctl.get(), action: sel!(showSettings:)];
        set_frame(btn, rect((STATE_W - 140.) / 2., y, 140., 28.));
        add_sub(inner, btn);
    }
    StateParts { host, icon, title, body, btn }
}
unsafe fn fill_state(p: &StateParts, sym: &str, title: &str, body: &str) {
    let img = symbol(sym, 40., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![p.icon, setImage: img];
    }
    set_text(p.title, title);
    set_text(p.body, body);
}

// ---- Window -----------------------------------------------------------------

const ROW_H: f64 = 76.0;
const THUMB_W: f64 = 40.0;
const THUMB_H: f64 = 60.0;
const PILL_H: f64 = 20.0;
const PILL_MAX_W: f64 = 220.0;
const BTN_W: f64 = 100.0;
const SPIN: f64 = 16.0;
const SEG_H: f64 = 32.0;
const FIELD_H: f64 = 28.0;
const STATUS_H: f64 = 16.0;

unsafe fn build_window() {
    let ui = UI.get_mut().unwrap();
    let w: *mut AnyObject = msg_send![jr_window_class(), alloc];
    let w: *mut AnyObject = msg_send![w,
        initWithContentRect: rect(0., 0., 680., 680.), styleMask: (1 | 2 | 8 | (1 << 15)) as NSUInteger, backing: 2usize, defer: Bool::NO];
    let _: () = msg_send![w, setTitle: ns("Request Movies & Shows")];
    let _: () = msg_send![w, setTitlebarAppearsTransparent: Bool::YES];
    let _: () = msg_send![w, setTitleVisibility: 1isize];
    let _: () = msg_send![w, setReleasedWhenClosed: Bool::NO];
    let _: () = msg_send![w, setMinSize: size(520., 420.)];
    place_window(w);
    let c: *mut AnyObject = msg_send![w, contentView];
    let b = bounds_of(c);
    let hdr_w = b.size.w - 2. * SP6;

    // Opaque canvas behind everything.
    let canvas = surface(b, C_CANVAS, 0.);
    set_autoresize(canvas, RESIZE_W | RESIZE_H);
    add_sub(c, canvas);

    // Header, measured from the top below the reserved title band.
    let mut top = TITLEBAR_H;
    let labels: *mut AnyObject = msg_send![class!(NSMutableArray), array];
    for t in ["Search", "My Requests"] {
        let _: () = msg_send![labels, addObject: ns(t)];
    }
    let tabs: *mut AnyObject = msg_send![class!(NSSegmentedControl),
        segmentedControlWithLabels: labels, trackingMode: 0i64, target: ui.ctl.get(), action: sel!(seerrTab:)];
    let _: () = msg_send![tabs, setControlSize: 3isize];
    let _: () = msg_send![tabs, setSelectedSegment: 0isize];
    let tw = 240.;
    set_frame(tabs, rect(((b.size.w - tw) / 2.).floor(), b.size.h - top - SEG_H, tw, SEG_H));
    set_autoresize(tabs, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY);
    add_sub(c, tabs);
    top += SEG_H + SP3;

    let sf: *mut AnyObject = msg_send![class!(NSSearchField), alloc];
    let sf: *mut AnyObject = msg_send![sf, initWithFrame: rect(SP6, b.size.h - top - FIELD_H, hdr_w, FIELD_H)];
    let _: () = msg_send![sf, setControlSize: 3isize];
    let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search movies and shows")];
    let _: () = msg_send![sf, setDelegate: ui.ctl.get()]; // controlTextDidChange: -> SeerrSearch
    let _: () = msg_send![sf, setSendsWholeSearchString: Bool::NO];
    set_autoresize(sf, RESIZE_W | MARGIN_MINY);
    add_sub(c, sf);
    autorel(sf);
    top += FIELD_H + SP2;

    let st = label("", T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(st, rect(SP6, b.size.h - top - STATUS_H, hdr_w, STATUS_H));
    set_autoresize(st, RESIZE_W | MARGIN_MINY);
    add_sub(c, st);
    top += STATUS_H + SP2;

    let sc: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let sc: *mut AnyObject = msg_send![sc, initWithFrame: rect(0., 0., b.size.w, b.size.h - top)];
    let _: () = msg_send![sc, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![sc, setDrawsBackground: Bool::NO];
    let _: () = msg_send![sc, setBorderType: 0usize];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    let doc = flip_view(rect(0., 0., b.size.w, 10.));
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    add_sub(c, sc);
    autorel(sc);
    autorel(doc);

    // Empty state over the list area (hidden while rows exist).
    let empty = make_state(rect(0., 0., b.size.w, b.size.h - top), false);
    add_sub(c, empty.host);
    autorel(empty.host);
    set_hidden(empty.host, true);

    // "Not set up" state over the whole content area (instead of the search UI).
    let hint = make_state(rect(0., 0., b.size.w, b.size.h - TITLEBAR_H), true);
    fill_state(
        &hint,
        "gearshape",
        "Requests aren't set up",
        "Turn on Seerr in Settings and add its URL and API key.",
    );
    add_sub(c, hint.host);
    autorel(hint.host);

    ui.seerr.win = Ptr::retained(w);
    ui.seerr.tabs = Ptr(tabs);
    ui.seerr.search = Ptr(sf);
    ui.seerr.status = Ptr(st);
    ui.seerr.scroll = Ptr(sc);
    ui.seerr.doc = Ptr(doc);
    ui.seerr.hint = Ptr(hint.host);
    ui.seerr.hint_btn = Ptr(hint.btn);
    ui.seerr.empty = Ptr(empty.host);
    ui.seerr.empty_icon = Ptr(empty.icon);
    ui.seerr.empty_title = Ptr(empty.title);
    ui.seerr.empty_body = Ptr(empty.body);
    release_obj(w); // Ptr::retained took its own +1
}

/// Show/hide the search UI vs the "set up Seerr" hint according to settings.
pub(crate) unsafe fn apply_config_state() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.win.get().is_null() {
        return;
    }
    let ok = is_configured(&ui.cfg.settings);
    for v in [ui.seerr.tabs, ui.seerr.search, ui.seerr.status, ui.seerr.scroll] {
        set_hidden(v.get(), !ok);
    }
    set_hidden(ui.seerr.hint.get(), ok);
    if ok {
        rebuild();
    } else {
        set_hidden(ui.seerr.empty.get(), true);
    }
}

/// Entry point for the sidebar button, Go menu item and `openSeerr:`.
pub fn open_window() {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        if ui.seerr.win.get().is_null() {
            build_window();
        }
        apply_config_state();
        let w = ui.seerr.win.get();
        let _: () = msg_send![w, makeKeyAndOrderFront: nil()];
        if is_configured(&ui.cfg.settings) {
            let _: Bool = msg_send![w, makeFirstResponder: ui.seerr.search.get()];
        }
    }
}

/// Sidebar "Requests" (unified layout): the Seerr window on its "My requests" tab.
pub(crate) extern "C" fn open_requests(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    open_window();
    unsafe { select_mode(true) }
}

pub(crate) unsafe fn set_status(msg: &str, err: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.status.get().is_null() {
        return;
    }
    set_text(ui.seerr.status.get(), msg);
    set_label_color(ui.seerr.status.get(), if err { C_DANGER } else { C_TEXT2 });
}

// ---- Rows -------------------------------------------------------------------

/// (text colour, fill) for a pill role; fill is the role colour at 16% alpha.
unsafe fn pill_colors(role: &str) -> (Col, *mut AnyObject) {
    let base = match role {
        "ok" => C_OK,
        "warn" => C_WARN,
        "danger" => C_DANGER,
        _ => return (C_TEXT2, color(C_FILL_SEL)),
    };
    (base, color_a(base, 0.16))
}

/// Rounded status pill (NSBox, radius 10) with its caption; returns (box, width).
unsafe fn make_pill(text: &str, role: &str) -> (*mut AnyObject, f64) {
    let (tc, fill) = pill_colors(role);
    let l = label(text, T_CAPTION_EM, tc, ALIGN_CENTER);
    let cell: *mut AnyObject = msg_send![l, cell];
    let sz: NSSize = msg_send![cell, cellSize];
    let w = (sz.w.ceil() + 2. * SP2 + 2.).min(PILL_MAX_W);
    let b = surface(rect(0., 0., w, PILL_H), C_FILL_SEL, PILL_H / 2.);
    let _: () = msg_send![b, setFillColor: fill];
    let cv: *mut AnyObject = msg_send![b, contentView];
    set_frame(l, rect(SP2, 3., w - 2. * SP2, 14.));
    add_sub(cv, l);
    (b, w)
}

unsafe fn make_spinner(frame: NSRect) -> *mut AnyObject {
    let sp: *mut AnyObject = msg_send![class!(NSProgressIndicator), alloc];
    let sp: *mut AnyObject = autorel(msg_send![sp, initWithFrame: frame]);
    let _: () = msg_send![sp, setStyle: 1usize]; // spinning
    let _: () = msg_send![sp, setControlSize: 1usize];
    let _: () = msg_send![sp, setIndeterminate: Bool::YES];
    let _: () = msg_send![sp, startAnimation: nil()];
    sp
}

/// Placeholder (fill + glyph) with the poster layer on top; registers the thumb for later images.
unsafe fn make_thumb(row: *mut AnyObject, spec: &RowSpec) {
    let ui = UI.get_mut().unwrap();
    let f = rect(SP6, (ROW_H - THUMB_H) / 2., THUMB_W, THUMB_H);
    let ph = surface(f, C_FILL_SEL, R_THUMB);
    add_sub(row, ph);
    let glyph: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let glyph: *mut AnyObject = autorel(msg_send![glyph, initWithFrame: rect(f.origin.x + 10., f.origin.y + 20., 20., 20.)]);
    let img = symbol(spec.glyph, 16., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![glyph, setImage: img];
    }
    let _: () = msg_send![glyph, setContentTintColor: color(C_TEXT3)];
    add_sub(row, glyph);
    let art: *mut AnyObject = msg_send![class!(NSView), alloc];
    let art: *mut AnyObject = autorel(msg_send![art, initWithFrame: f]);
    let layer = layer_of(art);
    set_corner(layer, R_THUMB);
    let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    add_sub(row, art);
    if let Some(p) = &spec.poster {
        ui.seerr.thumbs.push((p.clone(), art, glyph));
        if let Some(img) = ui.seerr.images.get(p) {
            apply_thumb(art, glyph, *img, false);
        }
    }
}
unsafe fn apply_thumb(art: *mut AnyObject, glyph: *mut AnyObject, img: *mut AnyObject, animate: bool) {
    let cg = cg_image(img);
    if cg.is_null() {
        return;
    }
    if animate {
        tx_begin(M_STD);
    } else {
        tx_begin_instant();
    }
    let layer: *mut AnyObject = msg_send![art, layer];
    let _: () = msg_send![layer, setContents: cg as *mut AnyObject];
    tx_end();
    set_hidden(glyph, true);
}

unsafe fn make_row(i: usize, spec: &RowSpec, width: f64, requesting: bool) -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let row = class_view(b"JRSeerrRow\0", rect(0., i as f64 * ROW_H, width, ROW_H));
    set_autoresize(row, RESIZE_W);

    // Hover background (transparent until the pointer is inside).
    let bg = surface(rect(SP3, 0., width - 2. * SP3, ROW_H), C_FILL_HOVER, R_ROW);
    let _: () = msg_send![bg, setTransparent: Bool::YES];
    set_autoresize(bg, RESIZE_W);
    add_sub(row, bg);
    *ivar_ptr::<*mut c_void>(&*row, c"jrBg") = bg as *mut c_void;
    let ta: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut AnyObject = msg_send![ta,
        initWithRect: rect(0., 0., width, ROW_H), options: (0x01usize | 0x80usize | 0x200usize), owner: row, userInfo: nil()];
    let _: () = msg_send![row, addTrackingArea: ta];
    autorel(ta);

    make_thumb(row, spec);

    // Right cluster, laid out from the trailing edge: pill, then (Request) button + spinner.
    // A requestable row shows only the Request button; a "Not requested" pill beside it says nothing new.
    let mut left = width - SP6;
    if !spec.can_request {
        let (pill, pw) = make_pill(&spec.badge, spec.badge_color);
        left -= pw;
        set_frame(pill, rect(left, (ROW_H - PILL_H) / 2., pw, PILL_H));
        set_autoresize(pill, MARGIN_MINX);
        add_sub(row, pill);
    }
    if spec.can_request {
        let bx = left - BTN_W;
        let btn: *mut AnyObject = msg_send![class!(NSButton),
            buttonWithTitle: ns(if requesting { "Requesting…" } else { "Request" }), target: ui.ctl.get(), action: sel!(seerrRequest:)];
        let _: () = msg_send![btn, setBezelStyle: 1isize];
        let _: () = msg_send![btn, setControlSize: 1isize];
        let plus = symbol("plus", 11., Weight::Medium);
        if !plus.is_null() {
            let _: () = msg_send![btn, setImage: plus];
            let _: () = msg_send![btn, setImagePosition: 7isize]; // leading
        }
        let _: () = msg_send![btn, setTag: i as NSInteger];
        let _: () = msg_send![btn, setEnabled: Bool::new(!requesting)];
        set_frame(btn, rect(bx, (ROW_H - 22.) / 2., BTN_W, 22.));
        set_autoresize(btn, MARGIN_MINX);
        add_sub(row, btn);
        left = bx - SP2 - SPIN;
        if requesting {
            let sp = make_spinner(rect(left, (ROW_H - SPIN) / 2., SPIN, SPIN));
            set_autoresize(sp, MARGIN_MINX);
            add_sub(row, sp);
        }
    }

    let tx = SP6 + THUMB_W + SP3;
    let tw = (left - SP3 - tx).max(40.);
    let title = label(&spec.title, T_HEADLINE, C_TEXT, ALIGN_LEFT);
    set_frame(title, rect(tx, 19., tw, 20.));
    set_autoresize(title, RESIZE_W);
    add_sub(row, title);
    let sub = label(&spec.sub, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(sub, rect(tx, 41., tw, 16.));
    set_autoresize(sub, RESIZE_W);
    add_sub(row, sub);
    row
}

/// Rebuild the list from the current mode's data.
pub(crate) unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    let s = &mut ui.seerr;
    if s.win.get().is_null() {
        return;
    }
    s.thumbs.clear();
    for r in s.rows.drain(..) {
        let _: () = msg_send![r, removeFromSuperview];
    }
    let doc = s.doc.get();
    let width = bounds_of(doc).size.w.max(200.0);
    let mut specs: Vec<(RowSpec, bool)> = vec![];
    if s.requests_mode {
        specs = s.requests.iter().map(|r| (request_spec(r), false)).collect();
    } else {
        for r in &s.results {
            let busy = s.inflight.iter().any(|(m, id, _)| *m == r.media_type && *id == r.tmdb_id);
            specs.push((result_spec(r), busy));
        }
    }
    let empty = if specs.is_empty() {
        let q = read_field(s.search.get());
        empty_state(s.requests_mode, s.requests_loaded && s.requests_mode || s.results_loaded && !s.requests_mode, &q)
    } else {
        None
    };
    match &empty {
        Some((sym, title, body)) => {
            let p = StateParts {
                host: s.empty.get(),
                icon: s.empty_icon.get(),
                title: s.empty_title.get(),
                body: s.empty_body.get(),
                btn: std::ptr::null_mut(),
            };
            fill_state(&p, sym, title, body);
            set_hidden(s.empty.get(), false);
        }
        None => set_hidden(s.empty.get(), true),
    }
    for (i, (spec, busy)) in specs.iter().enumerate() {
        let r = make_row(i, spec, width, *busy);
        add_sub(doc, r);
        autorel(r);
        s.rows.push(r);
    }
    let h = (specs.len() as f64 * ROW_H).max(bounds_of(s.scroll.get()).size.h);
    let _: () = msg_send![doc, setFrameSize: size(width, h)];
}

/// Poster bytes arrived (via the Seerr server): cache the image and fill matching thumbnails.
pub(crate) fn on_poster(path: String, bytes: &[u8]) {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        let data: *mut AnyObject = msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr() as *const std::ffi::c_void, length: bytes.len() as NSUInteger];
        if data.is_null() {
            return;
        }
        let img: *mut AnyObject = msg_send![class!(NSImage), alloc];
        let img: *mut AnyObject = msg_send![img, initWithData: data];
        if img.is_null() {
            return; // undecodable: the placeholder stays
        }
        if cg_image(img).is_null() {
            release_obj(img);
            return;
        }
        if let Some(old) = ui.seerr.images.insert(path.clone(), img) {
            release_obj(old);
        }
        for (p, art, glyph) in ui.seerr.thumbs.iter() {
            if *p == path {
                apply_thumb(*art, *glyph, img, true);
            }
        }
    }
}

/// Whether the thumbnail for `path` currently shows an image (None: no such row).
pub(crate) unsafe fn thumb_has_image(path: &str) -> Option<bool> {
    let ui = UI.get().unwrap();
    let (_, art, _) = ui.seerr.thumbs.iter().find(|(p, _, _)| p == path)?;
    let layer: *mut AnyObject = msg_send![*art, layer];
    let c: *mut AnyObject = msg_send![layer, contents];
    Some(!c.is_null())
}

// ---- Actions ----------------------------------------------------------------

pub(crate) extern "C" fn open_seerr(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    open_window();
}

pub(crate) extern "C" fn seerr_tab(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let seg: NSInteger = msg_send![sender, selectedSegment];
        select_mode(seg == 1);
    }
}

pub(crate) unsafe fn select_mode(requests: bool) {
    let ui = UI.get_mut().unwrap();
    ui.seerr.requests_mode = requests;
    if !ui.seerr.tabs.get().is_null() {
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: requests as NSInteger];
    }
    if requests {
        ui.seerr.requests_loaded = false;
        set_status("Loading your requests…", false);
        let _ = ui.ui_tx.send(UiCmd::SeerrRequests);
    } else {
        set_status("", false);
    }
    rebuild();
}

/// controlTextDidChange: from the Seerr search field.
pub(crate) unsafe fn search_changed() {
    let ui = UI.get_mut().unwrap();
    let q = read_field(ui.seerr.search.get());
    if ui.seerr.requests_mode {
        ui.seerr.requests_mode = false;
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: 0isize];
    }
    ui.seerr.results_loaded = false;
    if q.trim().is_empty() {
        ui.seerr.results.clear();
        set_status("", false);
    } else {
        set_status("Searching…", false);
    }
    let _ = ui.ui_tx.send(UiCmd::SeerrSearch { q });
    rebuild();
}

pub(crate) extern "C" fn seerr_request(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        let r = match ui.seerr.results.get(tag.max(0) as usize) {
            Some(r) if r.availability == Availability::NotRequested => r.clone(),
            _ => return,
        };
        if ui.seerr.inflight.iter().any(|(m, id, _)| *m == r.media_type && *id == r.tmdb_id) {
            return;
        }
        ui.seerr.inflight.push((r.media_type.clone(), r.tmdb_id, r.title.clone()));
        // Update the clicked button in place (never remove a control from inside its own action).
        let _: () = msg_send![sender, setEnabled: Bool::NO];
        let _: () = msg_send![sender, setTitle: ns("Requesting…")];
        let row: *mut AnyObject = msg_send![sender, superview];
        let bf = frame_of(sender);
        if !row.is_null() {
            let sp = make_spinner(rect(bf.origin.x - SP2 - SPIN, bf.origin.y + (bf.size.h - SPIN) / 2., SPIN, SPIN));
            set_autoresize(sp, MARGIN_MINX);
            add_sub(row, sp);
        }
        set_status(&format!("Requesting “{}”…", r.title), false);
        let _ = ui.ui_tx.send(UiCmd::SeerrRequest {
            media_type: r.media_type,
            tmdb_id: r.tmdb_id,
            title: r.title,
        });
    }
}

// ---- MainMsg handlers -------------------------------------------------------

pub(crate) fn on_results(v: Vec<SeerrResult>) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.seerr.results = v;
        ui.seerr.results_loaded = true;
        if !ui.seerr.requests_mode {
            set_status("", false);
        }
        rebuild();
    }
}

pub(crate) fn on_requested(title: String) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(pos) = ui.seerr.inflight.iter().position(|(_, _, t)| *t == title) {
            let (m, id, _) = ui.seerr.inflight.remove(pos);
            for r in ui.seerr.results.iter_mut() {
                if r.media_type == m && r.tmdb_id == id {
                    r.availability = Availability::Pending;
                    r.request_status = Some(RequestStatus::Pending);
                }
            }
        }
        set_status(&format!("Requested “{}”.", title), false);
        rebuild();
    }
}

pub(crate) fn on_request_list(v: Vec<SeerrRequestItem>) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.seerr.requests = v;
        ui.seerr.requests_loaded = true;
        if ui.seerr.requests_mode {
            set_status("", false);
        }
        rebuild();
    }
}

pub(crate) fn on_error(msg: String) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.seerr.inflight.clear();
        ui.seerr.results_loaded = true;
        ui.seerr.requests_loaded = true;
        set_status(&msg, true);
        rebuild();
    }
}

/// Show the fields that belong to the selected sign-in mode (settings sheet).
pub(crate) unsafe fn apply_mode() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.s_mode.get().is_null() {
        return;
    }
    let idx: NSInteger = msg_send![ui.seerr.s_mode.get(), indexOfSelectedItem];
    for v in &ui.seerr.s_login_group {
        set_hidden(v.get(), idx != 0);
    }
    for v in &ui.seerr.s_key_group {
        set_hidden(v.get(), idx != 1);
    }
}

pub(crate) extern "C" fn seerr_mode_changed(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { apply_mode() }
}

/// Reply to `SaveSeerr`. On success the confirmed values become the UI's copy
/// of the settings and the sheet closes; on failure the sheet stays open.
pub(crate) unsafe fn on_save_status(ok: bool, message: String) {
    let ui = UI.get_mut().unwrap();
    if ok {
        if let Some((on, url, user)) = ui.seerr.pending.take() {
            ui.cfg.settings.seerr_enabled = on;
            if !url.is_empty() {
                ui.cfg.settings.seerr_url = url;
            }
            ui.cfg.settings.seerr_username = user;
        }
    }
    if !ui.seerr.s_status.get().is_null() {
        set_text(ui.seerr.s_status.get(), &message);
        set_label_color(ui.seerr.s_status.get(), if ok { C_TEXT2 } else { C_DANGER });
    }
    if ok {
        set_status(&message, false);
        apply_config_state();
        crate::platform::appkit::close_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(media: &str, year: Option<u16>, a: Availability) -> SeerrResult {
        SeerrResult {
            media_type: media.into(),
            tmdb_id: 1,
            title: "T".into(),
            year,
            poster_path: None,
            overview: String::new(),
            availability: a,
            request_status: None,
        }
    }

    #[test]
    fn request_button_only_when_not_requested() {
        for a in [
            Availability::Pending,
            Availability::Processing,
            Availability::PartiallyAvailable,
            Availability::Available,
            Availability::Blocklisted,
        ] {
            assert!(!result_spec(&res("movie", None, a)).can_request);
        }
        assert!(result_spec(&res("movie", None, Availability::NotRequested)).can_request);
    }

    #[test]
    fn badge_roles() {
        let role = |a| result_spec(&res("movie", None, a)).badge_color;
        assert_eq!(role(Availability::Available), "ok");
        assert_eq!(role(Availability::Pending), "warn");
        assert_eq!(role(Availability::Processing), "warn");
        assert_eq!(role(Availability::PartiallyAvailable), "warn");
        assert_eq!(role(Availability::Blocklisted), "danger");
        assert_eq!(role(Availability::NotRequested), "neutral");
        let req = |st, av| {
            request_spec(&SeerrRequestItem {
                request_id: 1,
                media_type: "movie".into(),
                tmdb_id: 1,
                title: "T".into(),
                year: None,
                poster_path: Some("/p.jpg".into()),
                status: st,
                availability: av,
                requested_by: String::new(),
                created_at: String::new(),
            })
        };
        assert_eq!(req(RequestStatus::Declined, Availability::NotRequested).badge_color, "danger");
        assert_eq!(req(RequestStatus::Failed, Availability::NotRequested).badge_color, "danger");
        assert_eq!(req(RequestStatus::Completed, Availability::Available).badge_color, "ok");
        assert_eq!(req(RequestStatus::Approved, Availability::Processing).badge_color, "warn");
        assert_eq!(req(RequestStatus::Pending, Availability::Pending).badge_color, "warn");
        assert_eq!(req(RequestStatus::Unknown, Availability::Blocklisted).badge_color, "danger");
        assert_eq!(req(RequestStatus::Unknown, Availability::NotRequested).badge_color, "neutral");
        assert_eq!(req(RequestStatus::Pending, Availability::Pending).poster.as_deref(), Some("/p.jpg"));
    }

    #[test]
    fn glyph_follows_media_type() {
        assert_eq!(result_spec(&res("movie", None, Availability::Available)).glyph, "film");
        assert_eq!(result_spec(&res("tv", None, Availability::Available)).glyph, "tv");
    }

    #[test]
    fn empty_states() {
        assert_eq!(empty_state(false, false, "  ").unwrap().0, "magnifyingglass");
        assert_eq!(empty_state(false, false, "dune"), None); // still searching
        assert_eq!(empty_state(false, true, "dune").unwrap().2, "No matches for “dune”.");
        assert_eq!(empty_state(true, false, ""), None); // still loading
        assert_eq!(empty_state(true, true, "").unwrap().0, "tray");
    }

    #[test]
    fn subtitle_formats_type_and_year() {
        assert_eq!(result_spec(&res("movie", Some(2010), Availability::Available)).sub, "Movie · 2010");
        assert_eq!(result_spec(&res("tv", None, Availability::Available)).sub, "TV");
    }

    #[test]
    fn configured_needs_enabled_and_url() {
        let mut s = Settings::default();
        assert!(!is_configured(&s));
        s.seerr_enabled = true;
        assert!(!is_configured(&s));
        s.seerr_url = "  ".into();
        assert!(!is_configured(&s));
        s.seerr_url = "http://h:5055".into();
        assert!(is_configured(&s));
    }
}
