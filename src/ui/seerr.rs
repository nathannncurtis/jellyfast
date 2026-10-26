//! Discover tab (Seerr): a streaming-style browse surface, search, and My Requests.
//!
//! A full-size view in the main window's content area, the same slot Home and the
//! library grid use. The sidebar "Discover" row selects it (shown once Seerr is
//! configured); `enter` / `leave` swap it in and out like Home does. With an empty
//! search field it shows the billboard and rows (see `ui::discover`); typing shows a
//! poster grid of results; "My Requests" lists requests by status. A title page
//! covers the view when a poster is opened. The view is built once at launch and
//! only shown or hidden. All state lives in `Ui.seerr`.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use crate::seerr::models::{image_key, ImageSize};
use crate::ui::discover::{self, BrowseRow, TitlePage};
use crate::ui::home::POSTER_W;
use crate::ui::shell::{header_layout, hide_state, set_count, sidebar_rebuild, sidebar_refresh_selection};
use crate::ui::theme::*;
use std::collections::HashMap;

/// `cfg.last_view` value meaning "Discover was the last selected place" (kept as "requests"
/// so a saved choice from before the rename still opens this screen).
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
    /// Title page: its own scroll view in the host, shown instead of `scroll` while open.
    pub title_scroll: Ptr,
    pub title_doc: Ptr,
    /// The open title page (None: Discover, search or My Requests is showing).
    pub title: Option<TitlePage>,
    /// Discover's rows (see `discover::default_rows`) and the billboard's title.
    pub rows: Vec<BrowseRow>,
    pub billboard: Option<SeerrResult>,
    /// Discover rows were asked for at least once (Discover asks again on ⌘R).
    pub discover_asked: bool,
    /// Items behind the poster taps: a tap button's tag is its index here.
    pub tagged: Vec<SeerrResult>,
    /// TMDB id of a title whose Jellyfin item is being looked up for Play.
    pub play_lookup: Option<u64>,
    /// Centred empty-state over the grid (hidden while cards exist).
    pub empty: Ptr,
    pub empty_icon: Ptr,
    pub empty_title: Ptr,
    pub empty_body: Ptr,
    /// Images in the current views: (image key, art view, placeholder glyph).
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
            title_scroll: n,
            title_doc: n,
            title: None,
            rows: discover::default_rows(),
            billboard: None,
            discover_asked: false,
            tagged: vec![],
            play_lookup: None,
            requests_mode: false,
            results: vec![],
            results_loaded: false,
            requests: vec![],
            requests_loaded: false,
            inflight: vec![],
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
pub(crate) unsafe fn flip_view(frame: NSRect) -> *mut AnyObject {
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
pub(crate) const PILL_H: f64 = 20.;
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
    for t in ["Discover", "My Requests"] {
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
    // The title page: a flipped document in its own scroll view (hidden until a poster is opened).
    let tsc: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let tsc: *mut AnyObject = autorel(msg_send![tsc, initWithFrame: rect(0., 0., b.size.w, b.size.h)]);
    let _: () = msg_send![tsc, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![tsc, setDrawsBackground: Bool::NO];
    let _: () = msg_send![tsc, setBorderType: 0usize];
    set_autoresize(tsc, RESIZE_W | RESIZE_H);
    set_hidden(tsc, true);
    let tdoc = flip_view(rect(0., 0., b.size.w, 10.));
    set_autoresize(tdoc, RESIZE_W);
    let _: () = msg_send![tsc, setDocumentView: tdoc];
    add_sub(host, tsc);
    autorel(tdoc);
    ui.seerr.title_scroll = Ptr(tsc);
    ui.seerr.title_doc = Ptr(tdoc);
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
    set_frame(s.title_scroll.get(), rect(0., 0., w, grid_h));
    set_frame(s.empty.get(), rect(0., 0., w, grid_h));
    if s.title.is_some() {
        rebuild_title();
    }
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
        let ui = UI.get_mut().unwrap();
        if is_configured(&ui.cfg.settings) && !ui.seerr.discover_asked {
            discover::request_all();
        }
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
    ui.cur_title = "Discover".to_string();
    set_text(ui.header_title.get(), "Discover");
    set_count("", false);
    header_layout();
    apply_config_state();
    set_hidden(UI.get().unwrap().seerr.host.get(), false);
    sidebar_refresh_selection();
    let ui = UI.get_mut().unwrap();
    if ui.seerr.title.is_none() {
        // Entering on a browse view: a title page left open earlier must not show through.
        set_hidden(ui.seerr.title_scroll.get(), true);
        set_hidden(ui.seerr.scroll.get(), false);
    }
    if is_configured(&ui.cfg.settings) && !ui.seerr.discover_asked {
        discover::request_all();
    }
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
pub(crate) unsafe fn make_pill(text: &str, role: &str, max_w: f64) -> (*mut AnyObject, f64) {
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

/// Height a wrapping label needs for `text` at `width`, so nothing is clipped.
pub(crate) unsafe fn text_h(text: &str, t: TypeStyle, width: f64) -> f64 {
    let v = wrapping_label(text, t, C_TEXT, ALIGN_LEFT);
    let cell: *mut AnyObject = msg_send![v, cell];
    let s: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., width, 10_000.)];
    s.h.ceil().max(1.)
}

/// An image view (fills `f`) with a placeholder fill and glyph under it. The image is
/// registered under `key` so `on_poster` can fill it when the bytes arrive.
pub(crate) unsafe fn art_view(parent: *mut AnyObject, f: NSRect, key: Option<String>, glyph_name: &str, radius: f64) -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let ph = surface(f, C_FILL_SEL, radius);
    add_sub(parent, ph);
    let g = 20.;
    let gf = rect(f.origin.x + (f.size.w - g) / 2., f.origin.y + (f.size.h - g) / 2., g, g);
    let glyph: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let glyph: *mut AnyObject = autorel(msg_send![glyph, initWithFrame: gf]);
    let img = symbol(glyph_name, 16., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![glyph, setImage: img];
    }
    let _: () = msg_send![glyph, setContentTintColor: color(C_TEXT3)];
    add_sub(parent, glyph);
    let art: *mut AnyObject = msg_send![class!(NSView), alloc];
    let art: *mut AnyObject = autorel(msg_send![art, initWithFrame: f]);
    let layer = layer_of(art);
    set_corner(layer, radius);
    let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    add_sub(parent, art);
    if let Some(k) = key {
        if let Some(img) = ui.seerr.images.get(&k) {
            apply_thumb(art, glyph, *img, false);
        }
        ui.seerr.thumbs.push((k, art, glyph));
    }
    art
}

unsafe fn result_glyph(media_type: &str) -> &'static str {
    media_glyph(media_type)
}

/// A poster card at `f`: 2:3 art (w342), the availability pill on its lower left, and a
/// tap target that opens the title page. Autoreleased; the caller adds it to a view.
pub(crate) unsafe fn poster_tile(item: &SeerrResult, f: NSRect) -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let card = flip_view(f);
    autorel(card);
    let (w, h) = (f.size.w, f.size.h);
    let key = item.poster_path.as_deref().map(|p| image_key(ImageSize::PosterMedium, p));
    let _ = art_view(card, rect(0., 0., w, h), key, result_glyph(&item.media_type), R_POSTER);
    if let Some((text, role)) = discover::badge_for(item) {
        let (pill, pw) = make_pill(text, role, w - 2. * SP2);
        set_frame(pill, rect(SP2, h - SP2 - PILL_H, pw, PILL_H));
        add_sub(card, pill);
    }
    ui.seerr.tagged.push(item.clone());
    let tap: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: sel!(seerrOpenTitle:)];
    let _: () = msg_send![tap, setBordered: Bool::NO];
    let _: () = msg_send![tap, setTag: (ui.seerr.tagged.len() - 1) as NSInteger];
    set_frame(tap, rect(0., 0., w, h));
    add_sub(card, tap);
    card
}

/// A poster-sized image (My Requests rows): placeholder and art in one view.
pub(crate) unsafe fn thumb_view(path: &str, size: ImageSize, f: NSRect) -> *mut AnyObject {
    let host = flip_view(f);
    autorel(host);
    let _ = art_view(host, rect(0., 0., f.size.w, f.size.h), Some(image_key(size, path)), "film", R_THUMB);
    host
}

/// A round headshot (cast): placeholder and photo in one view, cropped to a circle.
pub(crate) unsafe fn thumb_view_round(path: &str, f: NSRect) -> *mut AnyObject {
    let host = flip_view(f);
    autorel(host);
    let r = f.size.w / 2.;
    let _ = art_view(host, rect(0., 0., f.size.w, f.size.h), Some(image_key(ImageSize::PosterSmall, path)), "person", r);
    host
}

pub(crate) unsafe fn apply_thumb(art: *mut AnyObject, glyph: *mut AnyObject, img: *mut AnyObject, animate: bool) {
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

/// A search result's caption: type and year.
fn result_sub(r: &SeerrResult) -> String {
    result_spec(r).sub
}

/// Grid of search results: poster cards with the title and type under each.
unsafe fn build_results(doc: *mut AnyObject, width: f64, results: &[SeerrResult]) -> f64 {
    let (cols, tile_w) = card_columns(width);
    let poster_h = (tile_w * POSTER_ASPECT).round();
    let texts: Vec<(f64, f64)> = results
        .iter()
        .map(|r| (text_h(&r.title, T_HEADLINE, tile_w), text_h(&result_sub(r), T_CALLOUT, tile_w)))
        .collect();
    let rows = (results.len() + cols - 1) / cols;
    let row_block: Vec<f64> = (0..rows)
        .map(|r| {
            texts[r * cols..((r + 1) * cols).min(texts.len())]
                .iter()
                .map(|(t, s)| t + TEXT_GAP + s)
                .fold(0., f64::max)
        })
        .collect();
    let mut y = SP2;
    for (r, block) in row_block.iter().enumerate() {
        for c in 0..cols {
            let i = r * cols + c;
            if i >= results.len() {
                break;
            }
            let x = EDGE + c as f64 * (tile_w + CARD_GAP_X);
            add_sub(doc, poster_tile(&results[i], rect(x, y, tile_w, poster_h)));
            let (th, sh) = texts[i];
            let title = wrapping_label(&results[i].title, T_HEADLINE, C_TEXT, ALIGN_LEFT);
            set_frame(title, rect(x, y + poster_h + SP2, tile_w, th));
            add_sub(doc, title);
            let sub = wrapping_label(&result_sub(&results[i]), T_CALLOUT, C_TEXT2, ALIGN_LEFT);
            set_frame(sub, rect(x, y + poster_h + SP2 + th + TEXT_GAP, tile_w, sh));
            add_sub(doc, sub);
        }
        y += poster_h + SP2 + block + CARD_GAP_Y;
    }
    y + SP2
}

/// Rebuild the document for the current mode: Discover (billboard and rows), search
/// results, or My Requests. Does nothing to the title page's content; that is `rebuild_title`.
pub(crate) unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.host.get().is_null() {
        return;
    }
    if ui.seerr.title.is_some() {
        rebuild_title();
        return;
    }
    ui.seerr.thumbs.clear();
    ui.seerr.tagged.clear();
    let doc = ui.seerr.doc.get();
    let none: *mut AnyObject = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];
    let clip: *mut AnyObject = msg_send![ui.seerr.scroll.get(), contentView];
    let clip_h = bounds_of(clip).size.h;
    let width = bounds_of(clip).size.w.max(200.0);
    ui.seerr.laid_w = width;

    let query = read_field(ui.seerr.search.get());
    let searching = !ui.seerr.requests_mode && !query.trim().is_empty();
    let (h, empty) = if ui.seerr.requests_mode {
        let h = discover::build_requests(doc, width, &ui.seerr.requests);
        let e = if ui.seerr.requests.is_empty() { empty_state(true, ui.seerr.requests_loaded, "") } else { None };
        (h, e)
    } else if searching {
        let h = build_results(doc, width, &ui.seerr.results);
        let e = if ui.seerr.results.is_empty() { empty_state(false, ui.seerr.results_loaded, &query) } else { None };
        (h, e)
    } else {
        (discover::build_discover(doc, width), None)
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
    let _: () = msg_send![doc, setFrameSize: size(width, h.max(clip_h))];
}

/// Rebuild the open title page (no-op when none is open).
pub(crate) unsafe fn rebuild_title() {
    let ui = UI.get_mut().unwrap();
    let Some(page) = ui.seerr.title.clone() else { return };
    if ui.seerr.title_doc.get().is_null() {
        return;
    }
    ui.seerr.thumbs.clear();
    let doc = ui.seerr.title_doc.get();
    let none: *mut AnyObject = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];
    let clip: *mut AnyObject = msg_send![ui.seerr.title_scroll.get(), contentView];
    let clip_h = bounds_of(clip).size.h;
    let width = bounds_of(clip).size.w.max(200.0);
    let h = discover::build_title(doc, width, &page);
    let _: () = msg_send![doc, setFrameSize: size(width, h.max(clip_h))];
}

/// Show the title page (covering the toolbar and grid) or go back to the browse views.
pub(crate) unsafe fn show_title_page(open: bool) {
    let ui = UI.get_mut().unwrap();
    let toolbar = is_configured(&ui.cfg.settings) && !open;
    for v in [ui.seerr.tabs, ui.seerr.search, ui.seerr.status] {
        set_hidden(v.get(), !toolbar);
    }
    set_hidden(ui.seerr.scroll.get(), open);
    set_hidden(ui.seerr.title_scroll.get(), !open);
    if open {
        set_hidden(ui.seerr.empty.get(), true);
        rebuild_title();
    } else {
        ui.seerr.title = None;
        rebuild();
    }
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
    if ui.seerr.title.is_some() {
        show_title_page(false);
    }
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

/// Refresh (⌘R) while Discover is showing: reload My Requests, the rows, or search again.
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
    } else if is_configured(&ui.cfg.settings) {
        discover::request_all();
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

/// Title page: the Request button. The first press asks for confirmation; the second sends.
pub(crate) extern "C" fn seerr_request(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        match ui.seerr.title.as_mut() {
            Some(p) if !p.confirming && p.detail.is_some() => {
                p.confirming = true;
                rebuild_title();
            }
            Some(_) => discover::send_request(),
            None => {}
        }
    }
}

/// Title page: Cancel while confirming a request.
pub(crate) extern "C" fn seerr_cancel_request(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(p) = ui.seerr.title.as_mut() {
            p.confirming = false;
        }
        rebuild_title();
    }
}

/// A poster tap (or a billboard button): open that title's page.
pub(crate) extern "C" fn seerr_open_title(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        let Some(r) = ui.seerr.tagged.get(tag.max(0) as usize).cloned() else { return };
        discover::open_title(&r.media_type, r.tmdb_id, &r.title);
    }
}

/// Title page: Back to the browse views.
pub(crate) extern "C" fn seerr_title_back(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { show_title_page(false) }
}

/// Title page: Play (the title is in the library; open its Jellyfin item).
pub(crate) extern "C" fn seerr_play(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { discover::play_clicked() }
}

/// Title page: Trailer (YouTube, in the browser; a no-op in the self-test).
pub(crate) extern "C" fn seerr_trailer(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { discover::trailer_clicked() }
}

/// Title page: a season checkbox (its tag is the season number).
pub(crate) extern "C" fn seerr_season(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        discover::season_toggled(tag.max(0) as u32);
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
            discover::on_requested(&m, id);
        }
        set_status(&format!("Requested “{}”.", title), false);
        rebuild();
        if ui.seerr.title.is_some() {
            rebuild_title();
        }
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
            overview: String::new(),
            availability: a,
            ..Default::default()
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
                seasons: vec![],
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
                seasons: vec![],
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
