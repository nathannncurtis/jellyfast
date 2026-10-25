//! Seerr window: search movies/TV, request them, and list "My requests".
//!
//! Non-modal NSWindow created once and reused (closing only hides it). Rows are
//! plain views inside a flipped document view of an NSScrollView, rebuilt on
//! every state change (result sets are small). All state lives in `Ui.seerr`.
#![allow(unsafe_code)]
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::seerr::models::{Availability, RequestStatus, SeerrRequestItem, SeerrResult};
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};

const ROW_H: f64 = 64.0;

pub struct SeerrUi {
    pub win: Ptr,
    pub tabs: Ptr,
    pub search: Ptr,
    pub status: Ptr,
    pub scroll: Ptr,
    pub doc: Ptr,
    pub hint: Ptr,
    pub hint_btn: Ptr,
    pub requests_mode: bool,
    pub results: Vec<SeerrResult>,
    pub results_loaded: bool,
    pub requests: Vec<SeerrRequestItem>,
    pub requests_loaded: bool,
    /// (media_type, tmdb_id, title) of requests sent and not yet answered.
    pub inflight: Vec<(String, u64, String)>,
    /// Row views currently inside `doc` (the superview owns them).
    pub rows: Vec<*mut Object>,
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
    /// "green" | "red" | "secondary"
    pub badge_color: &'static str,
    pub can_request: bool,
}

pub fn media_label(media_type: &str) -> String {
    match media_type {
        "movie" => "Movie".into(),
        "tv" => "TV".into(),
        o => o.to_string(),
    }
}
fn avail_color(a: Availability) -> &'static str {
    match a {
        Availability::Available => "green",
        Availability::Blocklisted => "red",
        _ => "secondary",
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
        RequestStatus::Declined | RequestStatus::Failed => "red",
        RequestStatus::Completed => "green",
        _ => avail_color(r.availability),
    };
    RowSpec { title: r.title.clone(), sub, badge, badge_color: color, can_request: false }
}

pub fn is_configured(s: &Settings) -> bool {
    s.seerr_enabled && !s.seerr_url.trim().is_empty()
}

// ---- Classes ----------------------------------------------------------------

extern "C" fn yes(_t: &mut Object, _c: Sel) -> BOOL {
    YES
}

pub(crate) unsafe fn register_seerr_classes() {
    let mut d = ClassDecl::new("JRFlipView", class!(NSView)).unwrap();
    d.add_method(sel!(isFlipped), yes as extern "C" fn(&mut Object, Sel) -> BOOL);
    d.register();
}

unsafe fn flip_view(frame: NSRect) -> *mut Object {
    let cls = objc::runtime::objc_getClass(b"JRFlipView\0".as_ptr() as *const std::os::raw::c_char);
    let v: *mut Object = msg_send![cls as *mut Object, alloc];
    msg_send![v, initWithFrame: frame] // +1; caller autorels after add
}

// ---- Window -----------------------------------------------------------------

unsafe fn build_window() {
    let ui = UI.get_mut().unwrap();
    let w: *mut Object = msg_send![jr_window_class(), alloc];
    let w: *mut Object = msg_send![w,
        initWithContentRect: rect(0., 0., 620., 640.)
        styleMask: (1 | 2 | 8) as NSUInteger backing: 2usize defer: NO];
    let _: () = msg_send![w, setTitle: ns("Request something new")];
    let _: () = msg_send![w, setReleasedWhenClosed: NO];
    let _: () = msg_send![w, setMinSize: size(460., 360.)];
    place_window(w);
    let c: *mut Object = msg_send![w, contentView];
    let b = bounds_of(c);

    // Segmented control: Search | My requests
    let labels: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in ["Search", "My requests"] {
        let _: () = msg_send![labels, addObject: ns(t)];
    }
    let tabs: *mut Object = msg_send![class!(NSSegmentedControl),
        segmentedControlWithLabels: labels trackingMode: 0i64
        target: ui.ctl.get() action: sel!(seerrTab:)];
    let _: () = msg_send![tabs, setSelectedSegment: 0isize];
    set_frame(tabs, rect(16., b.size.h - 44., 220., 24.));
    set_autoresize(tabs, MARGIN_MINY);
    add_sub(c, tabs);

    let sf: *mut Object = msg_send![class!(NSSearchField), alloc];
    let sf: *mut Object = msg_send![sf, initWithFrame: rect(16., b.size.h - 80., b.size.w - 32., 28.)];
    let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search movies and TV shows")];
    let _: () = msg_send![sf, setDelegate: ui.ctl.get()]; // controlTextDidChange: -> SeerrSearch
    let _: () = msg_send![sf, setSendsWholeSearchString: NO];
    set_autoresize(sf, RESIZE_W | MARGIN_MINY);
    add_sub(c, sf);
    autorel(sf);

    let st = make_label("", 12.0, false, false);
    set_frame(st, rect(16., b.size.h - 104., b.size.w - 32., 16.));
    let _: () = msg_send![st, setTextColor: color_named("secondary")];
    let _: () = msg_send![st.cell(), setLineBreakMode: 4usize];
    set_autoresize(st, RESIZE_W | MARGIN_MINY);
    add_sub(c, st);

    let sc: *mut Object = msg_send![class!(NSScrollView), alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: rect(0., 0., b.size.w, b.size.h - 112.)];
    let _: () = msg_send![sc, setHasVerticalScroller: YES];
    let _: () = msg_send![sc, setDrawsBackground: NO];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    let doc = flip_view(rect(0., 0., b.size.w, 10.));
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    add_sub(c, sc);
    autorel(sc);
    autorel(doc);

    // Inline "not set up" hint (shown instead of the search UI).
    let hint: *mut Object = msg_send![class!(NSTextField),
        wrappingLabelWithString: ns("Seerr isn't set up yet. Open Settings, turn on Seerr, then enter its URL and API key.")];
    set_frame(hint, rect(24., b.size.h - 90., b.size.w - 48., 48.));
    set_autoresize(hint, RESIZE_W | MARGIN_MINY);
    add_sub(c, hint);
    let hb: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Open Settings…")
        target: ui.ctl.get() action: sel!(showSettings:)];
    set_frame(hb, rect(24., b.size.h - 128., 140., 28.));
    set_autoresize(hb, MARGIN_MINY);
    add_sub(c, hb);

    ui.seerr.win = Ptr::retained(w);
    ui.seerr.tabs = Ptr(tabs);
    ui.seerr.search = Ptr(sf);
    ui.seerr.status = Ptr(st);
    ui.seerr.scroll = Ptr(sc);
    ui.seerr.doc = Ptr(doc);
    ui.seerr.hint = Ptr(hint);
    ui.seerr.hint_btn = Ptr(hb);
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
    set_hidden(ui.seerr.hint_btn.get(), ok);
    if ok {
        rebuild();
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
            let _: BOOL = msg_send![w, makeFirstResponder: ui.seerr.search.get()];
        }
    }
}

pub(crate) unsafe fn set_status(msg: &str, err: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.status.get().is_null() {
        return;
    }
    set_text(ui.seerr.status.get(), msg);
    let c = if err { color_named("red") } else { color_named("secondary") };
    let _: () = msg_send![ui.seerr.status.get(), setTextColor: c];
}

// ---- Rows -------------------------------------------------------------------

unsafe fn badge_color(name: &str) -> *mut Object {
    match name {
        "green" => msg_send![class!(NSColor), systemGreenColor],
        "red" => color_named("red"),
        _ => color_named("secondary"),
    }
}

unsafe fn make_row(i: usize, spec: &RowSpec, width: f64, requesting: bool) -> *mut Object {
    let ui = UI.get_mut().unwrap();
    let row = flip_view(rect(0., i as f64 * ROW_H, width, ROW_H));
    set_autoresize(row, RESIZE_W);
    let btn_w = if spec.can_request { 104.0 } else { 0.0 };
    let badge_w = 190.0;
    let right_pad = 16.0;

    let title = make_label(&spec.title, 14.0, true, false);
    let _: () = msg_send![title.cell(), setLineBreakMode: 4usize];
    set_frame(title, rect(16., 10., width - 16. - badge_w - btn_w - right_pad - 8., 20.));
    set_autoresize(title, RESIZE_W);
    add_sub(row, title);

    let sub = make_label(&spec.sub, 12.0, false, false);
    let _: () = msg_send![sub, setTextColor: color_named("secondary")];
    let _: () = msg_send![sub.cell(), setLineBreakMode: 4usize];
    set_frame(sub, rect(16., 34., width - 16. - badge_w - btn_w - right_pad - 8., 16.));
    set_autoresize(sub, RESIZE_W);
    add_sub(row, sub);

    let badge = make_label(&spec.badge, 12.0, false, false);
    let _: () = msg_send![badge, setAlignment: crate::ui::theme::ALIGN_RIGHT as isize]; // right
    let _: () = msg_send![badge, setTextColor: badge_color(spec.badge_color)];
    let _: () = msg_send![badge.cell(), setLineBreakMode: 4usize];
    set_frame(badge, rect(width - right_pad - btn_w - badge_w - if btn_w > 0. { 8. } else { 0. }, 22., badge_w, 16.));
    set_autoresize(badge, MARGIN_MINX);
    add_sub(row, badge);

    if spec.can_request {
        let btn: *mut Object = msg_send![class!(NSButton),
            buttonWithTitle: ns(if requesting { "Requesting…" } else { "Request" })
            target: ui.ctl.get() action: sel!(seerrRequest:)];
        let _: () = msg_send![btn, setTag: i as NSInteger];
        let _: () = msg_send![btn, setEnabled: (!requesting) as BOOL];
        set_frame(btn, rect(width - right_pad - btn_w, 18., btn_w, 28.));
        set_autoresize(btn, MARGIN_MINX);
        add_sub(row, btn);
    }

    let sep: *mut Object = msg_send![class!(NSBox), alloc];
    let sep: *mut Object = msg_send![sep, initWithFrame: rect(16., ROW_H - 1., width - 32., 1.)];
    let _: () = msg_send![sep, setBoxType: 2isize]; // NSBoxSeparator
    set_autoresize(sep, RESIZE_W);
    add_sub(row, sep);
    autorel(sep);
    row
}

unsafe fn empty_row(text: &str, width: f64) -> *mut Object {
    let row = flip_view(rect(0., 0., width, ROW_H));
    set_autoresize(row, RESIZE_W);
    let l = make_label(text, 13.0, false, false);
    let _: () = msg_send![l, setTextColor: color_named("secondary")];
    set_frame(l, rect(16., 20., width - 32., 18.));
    set_autoresize(l, RESIZE_W);
    add_sub(row, l);
    row
}

/// Rebuild the list from the current mode's data.
pub(crate) unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    let s = &mut ui.seerr;
    if s.win.get().is_null() {
        return;
    }
    for r in s.rows.drain(..) {
        let _: () = msg_send![r, removeFromSuperview];
    }
    let doc = s.doc.get();
    let width = bounds_of(doc).size.w.max(200.0);
    let mut specs: Vec<(RowSpec, bool)> = vec![];
    let empty_text;
    if s.requests_mode {
        specs = s.requests.iter().map(|r| (request_spec(r), false)).collect();
        empty_text = if s.requests_loaded { "No requests yet." } else { "Loading your requests…" };
    } else {
        for r in &s.results {
            let busy = s.inflight.iter().any(|(m, id, _)| *m == r.media_type && *id == r.tmdb_id);
            specs.push((result_spec(r), busy));
        }
        let blank = read_field(s.search.get()).trim().is_empty();
        empty_text = if blank {
            "Search for a movie or TV show to request."
        } else if s.results_loaded {
            "No results."
        } else {
            "Searching…"
        };
    }
    if specs.is_empty() {
        let r = empty_row(empty_text, width);
        add_sub(doc, r);
        autorel(r);
        s.rows.push(r);
    }
    for (i, (spec, busy)) in specs.iter().enumerate() {
        let r = make_row(i, spec, width, *busy);
        add_sub(doc, r);
        autorel(r);
        s.rows.push(r);
    }
    let h = (specs.len().max(1) as f64 * ROW_H).max(bounds_of(s.scroll.get()).size.h);
    let _: () = msg_send![doc, setFrameSize: size(width, h)];
}

// ---- Actions ----------------------------------------------------------------

pub(crate) extern "C" fn open_seerr(_t: &mut Object, _c: Sel, _s: *mut Object) {
    open_window();
}

pub(crate) extern "C" fn seerr_tab(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let seg: NSInteger = msg_send![sender, selectedSegment];
        select_mode(seg == 1);
    }
}

unsafe fn select_mode(requests: bool) {
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

pub(crate) extern "C" fn seerr_request(_t: &mut Object, _c: Sel, sender: *mut Object) {
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
        let _: () = msg_send![sender, setEnabled: NO];
        let _: () = msg_send![sender, setTitle: ns("Requesting…")];
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

pub(crate) extern "C" fn seerr_mode_changed(_t: &mut Object, _c: Sel, _s: *mut Object) {
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
        let c = if ok { color_named("secondary") } else { color_named("red") };
        let _: () = msg_send![ui.seerr.s_status.get(), setTextColor: c];
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
