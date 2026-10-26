//! Requests tab: search movies/TV, request them, and list "My requests".
//!
//! A full-size view in the main window's content area, the same slot Home and the
//! library grid use. The sidebar "Requests" row selects it (shown once Seerr is
//! configured); `enter` / `leave` swap it in and out like Home does. Results are a
//! poster grid of cards laid out for the current width. The view is built once at
//! launch and only shown or hidden. All state lives in `Ui.seerr`.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use crate::ui::home::POSTER_W;
use crate::ui::shell::{header_layout, hide_state, set_count, sidebar_rebuild, sidebar_refresh_selection};
use crate::ui::theme::*;
use std::collections::HashMap;

/// `cfg.last_view` value meaning "Requests was the last selected place".
pub const REQUESTS_ID: &str = "requests";

pub struct SeerrUi {
    /// Full-size host in the content area; shown only while Requests is the screen.
    pub host: Ptr,
    pub tabs: Ptr,
    pub search: Ptr,
    pub status: Ptr,
    pub scroll: Ptr,
    pub doc: Ptr,
    /// Full-area "not set up" state (instead of the search UI).
    pub hint: Ptr,
    pub hint_btn: Ptr,
    /// Centred empty-state over the grid (hidden while cards exist).
    pub empty: Ptr,
    pub empty_icon: Ptr,
    pub empty_title: Ptr,
    pub empty_body: Ptr,
    /// Posters in the current cards: (poster_path, art view, placeholder glyph).
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
    /// Card views currently inside `doc` (the superview owns them).
    pub cards: Vec<*mut AnyObject>,
    /// Width the cards were last laid out for (0 = never).
    pub laid_w: f64,
    /// Requests is the visible screen.
    pub active: bool,
    /// The sidebar "Requests" row (null while Seerr is not configured).
    pub row: Ptr,
    /// Whether the sidebar currently has the Requests row.
    pub in_sidebar: bool,
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
            host: n,
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
            cards: vec![],
            laid_w: 0.,
            active: false,
            row: n,
            in_sidebar: false,
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
    /// SF Symbol for the poster placeholder.
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
    // One short pill: the request status, or the availability when the status is unknown.
    let badge = match r.status {
        RequestStatus::Unknown => r.availability.label(),
        s => s.label(),
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
        badge: badge.to_string(),
        badge_color: color,
        can_request: false,
        poster: r.poster_path.clone(),
        glyph: media_glyph(&r.media_type),
    }
}

pub fn is_configured(s: &Settings) -> bool {
    s.seerr_enabled && !s.seerr_url.trim().is_empty()
}

/// Columns and tile width for `width` points of grid. Same rules as the library
/// grid (tiles at least `POSTER_W` wide, stretched to fill), so the posters match Home.
pub fn card_columns(width: f64) -> (usize, f64) {
    let avail = (width - 2. * EDGE).max(POSTER_W);
    let cols = (((avail + CARD_GAP_X) / (POSTER_W + CARD_GAP_X)).floor() as usize).max(1);
    let tile_w = ((avail - CARD_GAP_X * (cols as f64 - 1.)) / cols as f64).floor();
    (cols, tile_w)
}

/// What the grid area shows when there are no cards: (symbol, title, body).
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

/// The content-area host: re-lays out its toolbar and grid when it is resized.
extern "C" fn host_resize(this: *mut AnyObject, _c: Sel, old: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), resizeSubviewsWithOldSize: old];
        layout_host();
    }
}

/// The grid's document view: when its width changes the cards reflow (column count).
extern "C" fn doc_set_frame(this: *mut AnyObject, _c: Sel, s: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        if let Some(ui) = UI.get() {
            if ui.seerr.active && ui.seerr.laid_w > 0. && (s.w - ui.seerr.laid_w).abs() > 0.5 {
                rebuild();
            }
        }
    }
}

pub(crate) unsafe fn register_seerr_classes() {
    let mut d = ClassBuilder::new(c"JRFlipView", class!(NSView)).unwrap();
    d.add_method(sel!(isFlipped), yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    d.register();
    let mut h = ClassBuilder::new(c"JRRequestsHost", class!(NSView)).unwrap();
    h.add_method(
        sel!(resizeSubviewsWithOldSize:),
        host_resize as extern "C" fn(*mut AnyObject, Sel, NSSize),
    );
    h.register();
    let mut g = ClassBuilder::new(c"JRRequestsDoc", class!(NSView)).unwrap();
    g.add_method(sel!(isFlipped), yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    g.add_method(sel!(setFrameSize:), doc_set_frame as extern "C" fn(*mut AnyObject, Sel, NSSize));
    g.register();
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

// ---- Layout constants --------------------------------------------------------

const CARD_GAP_X: f64 = GRID_COL_GAP;
const CARD_GAP_Y: f64 = GRID_ROW_GAP;
/// Gap between a card's poster, title, subtitle and control line.
const TEXT_GAP: f64 = 2.;
const CONTROL_H: f64 = 22.;
const PILL_H: f64 = 20.;
const BTN_W: f64 = 110.;
const SPIN: f64 = 16.;
const SEG_W: f64 = 240.;
const SEG_H: f64 = 32.;
const FIELD_H: f64 = 28.;
const FIELD_MAX_W: f64 = 520.;
const STATUS_H: f64 = 16.;

// ---- Host view ----------------------------------------------------------------

/// Build the Requests view inside the content area (hidden until selected).
pub(crate) unsafe fn build_host(content_host: *mut AnyObject, frame: NSRect) {
    let ui = UI.get_mut().unwrap();
    let host = class_view(b"JRRequestsHost\0", frame);
    set_autoresize(host, RESIZE_W | RESIZE_H);
    set_hidden(host, true);
    add_sub(content_host, host);
    autorel(host);
    let b = bounds_of(host);

    // Opaque canvas behind everything.
    let canvas = surface(b, C_CANVAS, 0.);
    set_autoresize(canvas, RESIZE_W | RESIZE_H);
    add_sub(host, canvas);

    let labels: *mut AnyObject = msg_send![class!(NSMutableArray), array];
    for t in ["Search", "My Requests"] {
        let _: () = msg_send![labels, addObject: ns(t)];
    }
    let tabs: *mut AnyObject = msg_send![class!(NSSegmentedControl),
        segmentedControlWithLabels: labels, trackingMode: 0i64, target: ui.ctl.get(), action: sel!(seerrTab:)];
    let _: () = msg_send![tabs, setControlSize: 3isize];
    let _: () = msg_send![tabs, setSelectedSegment: 0isize];
    add_sub(host, tabs);

    let sf: *mut AnyObject = msg_send![class!(NSSearchField), alloc];
    let sf: *mut AnyObject = msg_send![sf, initWithFrame: rect(0., 0., 240., FIELD_H)];
    let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search movies and shows")];
    let _: () = msg_send![sf, setControlSize: 3isize];
    let _: () = msg_send![sf, setDelegate: ui.ctl.get()]; // controlTextDidChange: -> SeerrSearch
    let _: () = msg_send![sf, setSendsWholeSearchString: Bool::NO];
    add_sub(host, sf);
    autorel(sf);

    let st = label("", T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    add_sub(host, st);

    let sc: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let sc: *mut AnyObject = msg_send![sc, initWithFrame: rect(0., 0., b.size.w, b.size.h)];
    let _: () = msg_send![sc, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![sc, setDrawsBackground: Bool::NO];
    let _: () = msg_send![sc, setBorderType: 0usize];
    let doc = class_view(b"JRRequestsDoc\0", rect(0., 0., b.size.w, 10.));
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    add_sub(host, sc);
    autorel(sc);
    autorel(doc);

    // Empty state over the grid area (hidden while cards exist).
    let empty = make_state(rect(0., 0., b.size.w, b.size.h), false);
    add_sub(host, empty.host);
    autorel(empty.host);
    set_hidden(empty.host, true);

    // "Not set up" state over the whole area (instead of the search UI).
    let hint = make_state(rect(0., 0., b.size.w, b.size.h), true);
    fill_state(
        &hint,
        "gearshape",
        "Requests aren't set up",
        "Turn on Seerr in Settings and add its URL and API key.",
    );
    add_sub(host, hint.host);
    autorel(hint.host);

    ui.seerr.host = Ptr::retained(host);
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
    layout_host();
}

/// Place the toolbar, status line, grid and states for the host's current size.
/// Coordinates are bottom-up (the host is not flipped).
unsafe fn layout_host() {
    let ui = UI.get_mut().unwrap();
    let s = &ui.seerr;
    if s.host.get().is_null() {
        return;
    }
    let b = bounds_of(s.host.get());
    let (w, h) = (b.size.w, b.size.h);
    set_frame(s.hint.get(), rect(0., 0., w, h));
    // Toolbar: search on the left, segmented control on the right, status line below.
    let row_y = h - SP2 - SEG_H;
    set_frame(s.tabs.get(), rect(w - EDGE - SEG_W, row_y, SEG_W, SEG_H));
    let fw = (w - 2. * EDGE - SEG_W - SP3).clamp(160., FIELD_MAX_W);
    set_frame(s.search.get(), rect(EDGE, row_y + (SEG_H - FIELD_H) / 2., fw, FIELD_H));
    let st_y = row_y - SP2 - STATUS_H;
    set_frame(s.status.get(), rect(EDGE, st_y, w - 2. * EDGE, STATUS_H));
    let grid_h = (st_y - SP2).max(0.);
    set_frame(s.scroll.get(), rect(0., 0., w, grid_h));
    set_frame(s.empty.get(), rect(0., 0., w, grid_h));
}

/// Show or hide the setup state and the search UI to match the Seerr settings,
/// and keep the sidebar's Requests row in step with them.
pub(crate) unsafe fn apply_config_state() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.host.get().is_null() {
        return;
    }
    let ok = is_configured(&ui.cfg.settings);
    if ok != ui.seerr.in_sidebar {
        let user = ui.user_name.clone();
        let views = ui.views.clone();
        sidebar_rebuild(user, &views, false);
        sidebar_refresh_selection();
    }
    let ui = UI.get_mut().unwrap();
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

/// Show Requests in the content area and remember it as the last selected place.
pub(crate) unsafe fn enter() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.host.get().is_null() {
        return;
    }
    if ui.seerr.active {
        // Already showing: still pick up the current Seerr settings (sidebar row, setup state).
        apply_config_state();
        return;
    }
    ui.home.active = false;
    ui.seerr.active = true;
    ui.selected_view = None;
    ui.browse_active = false;
    set_hidden(ui.home.scroll.get(), true);
    set_hidden(ui.lib_scroll.get(), true);
    set_hidden(ui.sort_popup.get(), true);
    set_hidden(ui.lib_back.get(), true);
    hide_state();
    crate::platform::appkit::set_busy(false);
    let ui = UI.get_mut().unwrap();
    ui.cfg.last_view = Some(REQUESTS_ID.to_string());
    crate::config::save_config(&ui.cfg);
    ui.cur_title = "Requests".to_string();
    set_text(ui.header_title.get(), "Requests");
    set_count("", false);
    header_layout();
    apply_config_state();
    set_hidden(UI.get().unwrap().seerr.host.get(), false);
    sidebar_refresh_selection();
}

/// Leave Requests for a library screen or Home. A no-op unless Requests is showing.
pub(crate) unsafe fn leave() {
    let ui = UI.get_mut().unwrap();
    if !ui.seerr.active {
        return;
    }
    ui.seerr.active = false;
    set_hidden(ui.seerr.host.get(), true);
    set_hidden(ui.lib_scroll.get(), false);
    set_hidden(ui.sort_popup.get(), false);
    let name = ui.mode_stack.last().map(|m| m.1.clone()).unwrap_or_default();
    set_text(ui.header_title.get(), &name);
    ui.cur_title = name;
    header_layout();
    sidebar_refresh_selection();
}

pub(crate) unsafe fn set_status(msg: &str, err: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.status.get().is_null() {
        return;
    }
    set_text(ui.seerr.status.get(), msg);
    set_label_color(ui.seerr.status.get(), if err { C_DANGER } else { C_TEXT2 });
}

// ---- Cards ------------------------------------------------------------------

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

/// Rounded status pill with its caption; returns (box, width), never wider than `max_w`.
unsafe fn make_pill(text: &str, role: &str, max_w: f64) -> (*mut AnyObject, f64) {
    let (tc, fill) = pill_colors(role);
    let l = label(text, T_CAPTION_EM, tc, ALIGN_CENTER);
    let cell: *mut AnyObject = msg_send![l, cell];
    let sz: NSSize = msg_send![cell, cellSize];
    let w = (sz.w.ceil() + 2. * SP2 + 2.).min(max_w);
    let b = surface(rect(0., 0., w, PILL_H), C_FILL_SEL, PILL_H / 2.);
    let _: () = msg_send![b, setFillColor: fill];
    let cv: *mut AnyObject = msg_send![b, contentView];
    set_frame(l, rect(SP2, 2., w - 2. * SP2, 16.));
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

/// Height a wrapping label needs for `text` at `width`, so nothing is clipped.
unsafe fn text_h(text: &str, t: TypeStyle, width: f64) -> f64 {
    let v = wrapping_label(text, t, C_TEXT, ALIGN_LEFT);
    let cell: *mut AnyObject = msg_send![v, cell];
    let s: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., width, 10_000.)];
    s.h.ceil().max(1.)
}

/// Placeholder (fill + glyph) with the poster layer on top; registers the poster for later images.
unsafe fn make_poster(card: *mut AnyObject, spec: &RowSpec, f: NSRect) {
    let ui = UI.get_mut().unwrap();
    let ph = surface(f, C_FILL_SEL, R_POSTER);
    add_sub(card, ph);
    let g = 20.;
    let gf = rect(f.origin.x + (f.size.w - g) / 2., f.origin.y + (f.size.h - g) / 2., g, g);
    let glyph: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let glyph: *mut AnyObject = autorel(msg_send![glyph, initWithFrame: gf]);
    let img = symbol(spec.glyph, 16., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![glyph, setImage: img];
    }
    let _: () = msg_send![glyph, setContentTintColor: color(C_TEXT3)];
    add_sub(card, glyph);
    let art: *mut AnyObject = msg_send![class!(NSView), alloc];
    let art: *mut AnyObject = autorel(msg_send![art, initWithFrame: f]);
    let layer = layer_of(art);
    set_corner(layer, R_POSTER);
    let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    add_sub(card, art);
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

/// One grid card: poster, title, subtitle, then either the Request button or a status pill.
/// `frame` is in the document's flipped coordinates; `text` is (title h, subtitle h) and
/// `block` the tallest title+subtitle stack in the grid, so the controls share one line.
unsafe fn make_card(frame: NSRect, spec: &RowSpec, poster_h: f64, text: (f64, f64), block: f64, requesting: bool, index: usize) -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let card = flip_view(frame);
    let tile_w = frame.size.w;
    make_poster(card, spec, rect(0., 0., tile_w, poster_h));

    let ty = poster_h + SP2;
    let title = wrapping_label(&spec.title, T_HEADLINE, C_TEXT, ALIGN_LEFT);
    set_frame(title, rect(0., ty, tile_w, text.0));
    add_sub(card, title);
    let sub = wrapping_label(&spec.sub, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(sub, rect(0., ty + text.0 + TEXT_GAP, tile_w, text.1));
    add_sub(card, sub);

    let cy = ty + block + SP2;
    if spec.can_request {
        let btn: *mut AnyObject = msg_send![class!(NSButton),
            buttonWithTitle: ns(if requesting { "Requesting…" } else { "Request" }), target: ui.ctl.get(), action: sel!(seerrRequest:)];
        let _: () = msg_send![btn, setBezelStyle: 1isize];
        let _: () = msg_send![btn, setControlSize: 1isize];
        let plus = symbol("plus", 11., Weight::Medium);
        if !plus.is_null() {
            let _: () = msg_send![btn, setImage: plus];
            let _: () = msg_send![btn, setImagePosition: 7isize]; // leading
        }
        let _: () = msg_send![btn, setEnabled: Bool::new(!requesting)];
        set_frame(btn, rect(0., cy, BTN_W, CONTROL_H));
        add_sub(card, btn);
        if requesting {
            let sp = make_spinner(rect(BTN_W + SP2, cy + (CONTROL_H - SPIN) / 2., SPIN, SPIN));
            add_sub(card, sp);
        }
        // The tag is the card's index in `results`, which `seerr_request` looks up.
        let _: () = msg_send![btn, setTag: index as NSInteger];
    } else {
        let (pill, pw) = make_pill(&spec.badge, spec.badge_color, tile_w);
        set_frame(pill, rect(0., cy + (CONTROL_H - PILL_H) / 2., pw, PILL_H));
        add_sub(card, pill);
    }
    card
}

/// Rebuild the grid from the current mode's data, laid out for the grid's width.
pub(crate) unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.host.get().is_null() {
        return;
    }
    ui.seerr.thumbs.clear();
    for c in ui.seerr.cards.drain(..) {
        let _: () = msg_send![c, removeFromSuperview];
    }
    let doc = ui.seerr.doc.get();
    let clip: *mut AnyObject = msg_send![ui.seerr.scroll.get(), contentView];
    let clip_h = bounds_of(clip).size.h;
    let width = bounds_of(clip).size.w.max(200.0);
    ui.seerr.laid_w = width;

    let mut specs: Vec<(RowSpec, bool)> = vec![];
    if ui.seerr.requests_mode {
        specs = ui.seerr.requests.iter().map(|r| (request_spec(r), false)).collect();
    } else {
        for r in &ui.seerr.results {
            let busy = ui.seerr.inflight.iter().any(|(m, id, _)| *m == r.media_type && *id == r.tmdb_id);
            specs.push((result_spec(r), busy));
        }
    }
    let empty = if specs.is_empty() {
        let q = read_field(ui.seerr.search.get());
        let loaded = if ui.seerr.requests_mode { ui.seerr.requests_loaded } else { ui.seerr.results_loaded };
        empty_state(ui.seerr.requests_mode, loaded, &q)
    } else {
        None
    };
    let (empty_host, empty_icon, empty_title, empty_body) =
        (ui.seerr.empty.get(), ui.seerr.empty_icon.get(), ui.seerr.empty_title.get(), ui.seerr.empty_body.get());
    match &empty {
        Some((sym, title, body)) => {
            let p = StateParts { host: empty_host, icon: empty_icon, title: empty_title, body: empty_body, btn: std::ptr::null_mut() };
            fill_state(&p, sym, title, body);
            set_hidden(empty_host, false);
        }
        None => set_hidden(empty_host, true),
    }

    let (cols, tile_w) = card_columns(width);
    let poster_h = (tile_w * POSTER_ASPECT).round();
    let texts: Vec<(f64, f64)> = specs
        .iter()
        .map(|(sp, _)| (text_h(&sp.title, T_HEADLINE, tile_w), text_h(&sp.sub, T_CALLOUT, tile_w)))
        .collect();
    // Each row is as tall as its tallest title/subtitle stack, so rows with short names stay tight.
    let rows = (specs.len() + cols - 1) / cols;
    let row_block: Vec<f64> = (0..rows)
        .map(|r| {
            texts[r * cols..((r + 1) * cols).min(texts.len())]
                .iter()
                .map(|(t, s)| t + TEXT_GAP + s)
                .fold(0., f64::max)
        })
        .collect();
    let row_h: Vec<f64> = row_block.iter().map(|b| poster_h + SP2 + b + SP2 + CONTROL_H).collect();

    tx_begin_instant();
    let mut cards = vec![];
    let mut y = SP2;
    for (r, rh) in row_h.iter().enumerate() {
        for c in 0..cols {
            let i = r * cols + c;
            if i >= specs.len() {
                break;
            }
            let (spec, busy) = &specs[i];
            let x = EDGE + c as f64 * (tile_w + CARD_GAP_X);
            let card = make_card(rect(x, y, tile_w, *rh), spec, poster_h, texts[i], row_block[r], *busy, i);
            add_sub(doc, card);
            autorel(card);
            cards.push(card);
        }
        y += rh + CARD_GAP_Y;
    }
    let content_h = 2. * SP2 + row_h.iter().sum::<f64>() + rows.saturating_sub(1) as f64 * CARD_GAP_Y;
    let ui = UI.get_mut().unwrap();
    ui.seerr.cards = cards;
    tx_end();
    let h = content_h.max(clip_h);
    let _: () = msg_send![doc, setFrameSize: size(width, h)];
}

/// Poster bytes arrived (via the Seerr server): cache the image and fill matching posters.
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

/// Whether the poster for `path` currently shows an image (None: no such card).
pub(crate) unsafe fn thumb_has_image(path: &str) -> Option<bool> {
    let ui = UI.get().unwrap();
    let (_, art, _) = ui.seerr.thumbs.iter().find(|(p, _, _)| p == path)?;
    let layer: *mut AnyObject = msg_send![*art, layer];
    let c: *mut AnyObject = msg_send![layer, contents];
    Some(!c.is_null())
}

// ---- Actions ----------------------------------------------------------------

/// Sidebar "Requests" row.
pub(crate) extern "C" fn requests_selected(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { enter() }
}

/// Menu item and shortcut (⌥⌘K): show Requests on the search tab, focus the field.
pub(crate) extern "C" fn open_seerr(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        enter();
        select_mode(false);
        let ui = UI.get_mut().unwrap();
        if is_configured(&ui.cfg.settings) {
            let _: Bool = msg_send![ui.win.get(), makeFirstResponder: ui.seerr.search.get()];
        }
    }
}

pub(crate) extern "C" fn seerr_tab(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let seg: NSInteger = msg_send![sender, selectedSegment];
        select_mode(seg == 1);
    }
}

/// Switch between the search results and My Requests. Opens Requests first if needed.
pub(crate) unsafe fn select_mode(requests: bool) {
    enter();
    let ui = UI.get_mut().unwrap();
    ui.seerr.requests_mode = requests;
    if !ui.seerr.tabs.get().is_null() {
        let _: () = msg_send![ui.seerr.tabs.get(), setSelectedSegment: requests as NSInteger];
    }
    if requests {
        ui.seerr.requests_loaded = false;
        if is_configured(&ui.cfg.settings) {
            set_status("Loading your requests…", false);
            let _ = ui.ui_tx.send(UiCmd::SeerrRequests);
        }
    } else {
        set_status("", false);
    }
    rebuild();
}

/// Refresh (⌘R) while Requests is showing: reload My Requests, or search again.
pub(crate) unsafe fn refresh() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.requests_mode {
        select_mode(true);
        return;
    }
    let q = read_field(ui.seerr.search.get());
    if !q.trim().is_empty() {
        ui.seerr.results_loaded = false;
        set_status("Searching…", false);
        let _ = ui.ui_tx.send(UiCmd::SeerrSearch { q });
    }
    rebuild();
}

/// controlTextDidChange: from the Requests search field.
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
        let card: *mut AnyObject = msg_send![sender, superview];
        let bf = frame_of(sender);
        if !card.is_null() {
            let sp = make_spinner(rect(bf.origin.x + bf.size.w + SP2, bf.origin.y + (bf.size.h - SPIN) / 2., SPIN, SPIN));
            add_sub(card, sp);
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
    fn request_pill_is_the_status_alone() {
        let req = |st, av| {
            request_spec(&SeerrRequestItem {
                request_id: 1,
                media_type: "tv".into(),
                tmdb_id: 1,
                title: "T".into(),
                year: None,
                poster_path: None,
                status: st,
                availability: av,
                requested_by: String::new(),
                created_at: String::new(),
            })
            .badge
        };
        assert_eq!(req(RequestStatus::Approved, Availability::Processing), "Approved");
        assert_eq!(req(RequestStatus::Completed, Availability::Available), "Completed");
        assert_eq!(req(RequestStatus::Unknown, Availability::PartiallyAvailable), "Partially available");
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
    fn grid_columns_keep_tiles_at_poster_width_or_more() {
        for w in [720., 900., 1040., 1280., 1700., 2400.] {
            let (cols, tile) = card_columns(w);
            assert!(cols >= 1);
            assert!(tile >= POSTER_W, "tile {tile} at width {w}");
            let used = 2. * EDGE + cols as f64 * tile + (cols as f64 - 1.) * CARD_GAP_X;
            assert!(used <= w + 0.5, "grid {used} overflows {w}");
        }
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
