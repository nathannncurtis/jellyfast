//! Discover: the browse surface on the Seerr screen. A billboard with the top trending
//! title, then horizontal rows of posters (trending, popular, upcoming, genres and
//! "Because you watched"). Tapping a poster opens its title page: backdrop, facts,
//! overview, cast, trailer, and the Request or Play button (TV: a season picker).
//! The screen's shell, search and My Requests live in `seerr`; this file holds what
//! Discover shows and the replies that fill it. Pure rules are unit tested at the end.
#![allow(unsafe_code)]
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::seerr::models::{image_key, Availability, DiscoverRow, ImageSize, RequestStatus, SearchPage, SeasonInfo, SeerrRequestItem, SeerrResult, TitleDetail};
use crate::ui::seerr::{self, flip_view, make_pill, text_h, PILL_H};
use crate::ui::theme::*;

/// Genre rows: (media type, TMDB genre id, row title). Ids are TMDB's fixed ones.
pub const GENRE_ROWS: [(&str, u32, &str); 2] = [("movie", 28, "Action Movies"), ("tv", 35, "Comedy Series")];

/// Where a row's titles come from.
#[derive(Debug, Clone, PartialEq)]
pub enum RowSrc {
    Discover(DiscoverRow),
    Genre { media_type: String, genre_id: u32 },
    /// "Because you watched …" rows; replaced as a group when the reply arrives.
    Watched,
}

/// One horizontal row of posters.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowseRow {
    pub src: RowSrc,
    pub title: String,
    pub items: Vec<SeerrResult>,
    /// The first page has arrived (until then the row shows placeholder tiles).
    pub loaded: bool,
}

impl BrowseRow {
    fn new(src: RowSrc, title: &str) -> BrowseRow {
        BrowseRow { src, title: title.to_string(), items: vec![], loaded: false }
    }
}

/// The Discover rows in order: the five home rows, then the two genre rows.
/// "Because you watched" rows are appended when their reply arrives.
pub fn default_rows() -> Vec<BrowseRow> {
    let home = [
        (DiscoverRow::Trending, "Trending Now"),
        (DiscoverRow::Movies, "Popular Movies"),
        (DiscoverRow::Tv, "Popular TV Shows"),
        (DiscoverRow::UpcomingMovies, "Upcoming Movies"),
        (DiscoverRow::UpcomingTv, "Upcoming TV Shows"),
    ];
    let mut rows: Vec<BrowseRow> = home.into_iter().map(|(d, t)| BrowseRow::new(RowSrc::Discover(d), t)).collect();
    for (media, id, title) in GENRE_ROWS {
        rows.push(BrowseRow::new(RowSrc::Genre { media_type: media.into(), genre_id: id }, title));
    }
    rows
}

/// Availability pill for a poster: (label, colour role), or None when there is nothing to say.
pub fn badge_for(r: &SeerrResult) -> Option<(&'static str, &'static str)> {
    match r.availability {
        Availability::Available => Some(("Available", "ok")),
        Availability::PartiallyAvailable => Some(("Partially available", "ok")),
        Availability::Processing => Some(("Processing", "warn")),
        Availability::Pending => Some(("Pending", "warn")),
        Availability::NotRequested => match r.request_status {
            Some(RequestStatus::Pending) | Some(RequestStatus::Approved) => Some(("Requested", "warn")),
            _ => None,
        },
        Availability::Blocklisted => None,
    }
}

/// Whether the title is already on this server's library (Play, not Request).
pub fn in_library(a: Availability) -> bool {
    matches!(a, Availability::Available | Availability::PartiallyAvailable)
}

/// "2h 28m", "45m", "2h".
pub fn runtime_text(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{}m", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h {}m", h, m),
    }
}

/// "2010 · 2h 28m · Action, Sci-Fi · ★ 8.4": only the parts that are known.
pub fn meta_line(d: &TitleDetail) -> String {
    let mut parts: Vec<String> = vec![];
    if let Some(y) = d.year {
        parts.push(y.to_string());
    }
    if let Some(m) = d.runtime.filter(|m| *m > 0) {
        parts.push(runtime_text(m));
    }
    if !d.genres.is_empty() {
        parts.push(d.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
    }
    if let Some(v) = d.vote_average.filter(|v| *v > 0.) {
        parts.push(format!("★ {:.1}", v));
    }
    parts.join(" · ")
}

/// The seasons a request starts with: every regular season nobody has asked for yet.
pub fn default_seasons(d: &TitleDetail) -> Vec<u32> {
    d.seasons.iter().filter(|s| s.number > 0 && s.availability == Availability::NotRequested).map(|s| s.number).collect()
}

/// "Season 2 · 10 episodes".
pub fn season_text(s: &SeasonInfo) -> String {
    let eps = if s.episode_count == 1 { "1 episode".to_string() } else { format!("{} episodes", s.episode_count) };
    format!("Season {} · {}", s.number, eps)
}

/// YouTube trailer link for the title page. Only YouTube keys of plausible shape are used.
pub fn trailer_url(d: &TitleDetail) -> Option<String> {
    d.trailers
        .iter()
        .find(|t| t.site.eq_ignore_ascii_case("youtube") && !t.key.is_empty())
        .filter(|t| t.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .map(|t| format!("https://www.youtube.com/watch?v={}", t.key))
}

/// Requests page groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReqGroup {
    Pending,
    Approved,
    Available,
}

impl ReqGroup {
    pub const ALL: [ReqGroup; 3] = [ReqGroup::Pending, ReqGroup::Approved, ReqGroup::Available];
    pub fn title(self) -> &'static str {
        match self {
            ReqGroup::Pending => "Pending",
            ReqGroup::Approved => "Approved",
            ReqGroup::Available => "Available",
        }
    }
}

/// Which group a request sits in. Declined and failed requests stay with Pending, where
/// their red status pill says what happened.
pub fn req_group(r: &SeerrRequestItem) -> ReqGroup {
    if r.status == RequestStatus::Completed || r.availability == Availability::Available {
        ReqGroup::Available
    } else if r.status == RequestStatus::Approved || matches!(r.availability, Availability::Processing | Availability::PartiallyAvailable) {
        ReqGroup::Approved
    } else {
        ReqGroup::Pending
    }
}

/// "2026-10-25T…" → "Oct 25, 2026"; anything else → "".
pub fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let d = iso.get(..10).unwrap_or("");
    let p: Vec<&str> = d.split('-').collect();
    if p.len() != 3 {
        return String::new();
    }
    match (p[0].parse::<u32>(), p[1].parse::<usize>(), p[2].parse::<u32>()) {
        (Ok(y), Ok(m), Ok(day)) if (1..=12).contains(&m) && day > 0 => format!("{} {}, {}", MONTHS[m - 1], day, y),
        _ => String::new(),
    }
}

/// Seasons as a short list: "Seasons 1, 2" (empty for movies or when Seerr sent none).
pub fn seasons_text(r: &SeerrRequestItem) -> String {
    match r.seasons.len() {
        0 => String::new(),
        1 => format!("Season {}", r.seasons[0]),
        _ => format!("Seasons {}", r.seasons.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ")),
    }
}

/// The title page as it stands: what was tapped and what has arrived for it.
#[derive(Debug, Clone, PartialEq)]
pub struct TitlePage {
    pub media_type: String,
    pub tmdb_id: u64,
    /// The card's title, shown until the detail arrives.
    pub title: String,
    pub detail: Option<TitleDetail>,
    /// TV: the seasons ticked for the request.
    pub selected: Vec<u32>,
    /// The Request button was pressed once; the next press sends it.
    pub confirming: bool,
    pub requesting: bool,
}

// ---- State changes (called from the controller and from seerr's actions) ----------------

/// Ask for every Discover row and the "Because you watched" rows (one request for those).
pub(crate) unsafe fn request_all() {
    let ui = UI.get_mut().unwrap();
    ui.seerr.discover_asked = true;
    ui.seerr.rows.retain(|r| r.src != RowSrc::Watched);
    for r in ui.seerr.rows.iter_mut() {
        r.loaded = false;
        match &r.src {
            RowSrc::Discover(d) => {
                let _ = ui.ui_tx.send(UiCmd::SeerrDiscover { row: *d, page: 1 });
            }
            RowSrc::Genre { media_type, genre_id } => {
                let _ = ui.ui_tx.send(UiCmd::SeerrGenre { media_type: media_type.clone(), genre_id: *genre_id });
            }
            RowSrc::Watched => {}
        }
    }
    let _ = ui.ui_tx.send(UiCmd::SeerrWatched);
}

fn row_index(rows: &[BrowseRow], src: &RowSrc) -> Option<usize> {
    rows.iter().position(|r| &r.src == src)
}

/// A Discover row's first page arrived.
pub fn on_row(row: DiscoverRow, page: SearchPage) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let src = RowSrc::Discover(row);
        if let Some(i) = row_index(&ui.seerr.rows, &src) {
            ui.seerr.rows[i].items = page.results.clone();
            ui.seerr.rows[i].loaded = true;
        }
        if row == DiscoverRow::Trending {
            // The billboard is the first trending title that has a backdrop to show.
            ui.seerr.billboard = page.results.into_iter().find(|r| r.backdrop_path.is_some());
        }
        after_rows_changed();
    }
}

/// A genre row's first page arrived.
pub fn on_genre(media_type: &str, genre_id: u32, page: SearchPage) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let src = RowSrc::Genre { media_type: media_type.into(), genre_id };
        if let Some(i) = row_index(&ui.seerr.rows, &src) {
            ui.seerr.rows[i].items = page.results;
            ui.seerr.rows[i].loaded = true;
        }
        after_rows_changed();
    }
}

/// The "Because you watched" rows arrived: they replace the previous ones.
pub fn on_watched(rows: Vec<(String, Vec<SeerrResult>)>) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.seerr.rows.retain(|r| r.src != RowSrc::Watched);
        for (title, items) in rows {
            ui.seerr.rows.push(BrowseRow { src: RowSrc::Watched, title, items, loaded: true });
        }
        after_rows_changed();
    }
}

/// Redraw Discover when it is the visible mode (rows are kept either way).
unsafe fn after_rows_changed() {
    let ui = UI.get_mut().unwrap();
    if ui.seerr.active && !ui.seerr.requests_mode && ui.seerr.title.is_none() && read_field(ui.seerr.search.get()).trim().is_empty() {
        seerr::rebuild();
    }
    seerr::set_status("", false);
}

/// A title's full detail arrived. Fills the open title page if it is the one asked for.
pub fn on_title(d: TitleDetail) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let Some(page) = ui.seerr.title.as_mut() else { return };
        if page.tmdb_id != d.tmdb_id || page.media_type != d.media_type {
            return;
        }
        page.selected = default_seasons(&d);
        page.detail = Some(d);
        seerr::rebuild_title();
    }
}

/// Library lookup answered: open the Jellyfin item (or say it is not on this server).
pub fn on_library(tmdb_id: u64, item_id: Option<String>) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.seerr.play_lookup != Some(tmdb_id) {
            return;
        }
        ui.seerr.play_lookup = None;
        match item_id {
            Some(id) => {
                seerr::leave();
                crate::platform::appkit::set_busy(true);
                let _ = ui.ui_tx.send(UiCmd::OpenDetail { item_id: id });
            }
            None => crate::ui::player::toast("This title isn't in your Jellyfin library yet.", Some("exclamationmark.triangle"), true),
        }
    }
}

/// A Discover or title request failed: show it on the status line and re-enable buttons.
pub fn on_error(msg: String) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.seerr.play_lookup = None;
        if let Some(p) = ui.seerr.title.as_mut() {
            p.requesting = false;
            p.confirming = false;
        }
        for r in ui.seerr.rows.iter_mut() {
            r.loaded = true;
        }
        seerr::set_status(&msg, true);
        seerr::rebuild();
        if ui.seerr.title.is_some() {
            seerr::rebuild_title();
        }
    }
}

/// A request for a title went through: mark its posters and the title page as requested.
/// The seasons marked are the ones the title page had ticked.
pub(crate) unsafe fn on_requested(media_type: &str, tmdb_id: u64) {
    let ui = UI.get_mut().unwrap();
    let mark = |r: &mut SeerrResult| {
        if r.media_type == media_type && r.tmdb_id == tmdb_id {
            r.availability = Availability::Pending;
            r.request_status = Some(RequestStatus::Pending);
        }
    };
    for row in ui.seerr.rows.iter_mut() {
        row.items.iter_mut().for_each(mark);
    }
    if let Some(b) = ui.seerr.billboard.as_mut() {
        mark(b);
    }
    if let Some(p) = ui.seerr.title.as_mut() {
        if p.media_type == media_type && p.tmdb_id == tmdb_id {
            p.requesting = false;
            p.confirming = false;
            let asked = std::mem::take(&mut p.selected);
            if let Some(d) = p.detail.as_mut() {
                d.availability = Availability::Pending;
                d.request_status = Some(RequestStatus::Pending);
                for s in d.seasons.iter_mut().filter(|s| asked.contains(&s.number)) {
                    s.availability = Availability::Pending;
                }
            }
            p.selected = asked;
        }
    }
}

/// Open a title page for a card (by its media type and TMDB id).
pub(crate) unsafe fn open_title(media_type: &str, tmdb_id: u64, title: &str) {
    let ui = UI.get_mut().unwrap();
    ui.seerr.title = Some(TitlePage {
        media_type: media_type.into(),
        tmdb_id,
        title: title.into(),
        detail: None,
        selected: vec![],
        confirming: false,
        requesting: false,
    });
    let _ = ui.ui_tx.send(UiCmd::SeerrTitle { media_type: media_type.into(), tmdb_id });
    seerr::show_title_page(true);
}

/// Send the request the title page describes (movie, or the ticked seasons of a series).
pub(crate) unsafe fn send_request() {
    let ui = UI.get_mut().unwrap();
    let Some(p) = ui.seerr.title.as_mut() else { return };
    let Some(d) = p.detail.clone() else { return };
    if p.requesting {
        return;
    }
    let seasons = if p.media_type == "tv" { p.selected.clone() } else { vec![] };
    if p.media_type == "tv" && seasons.is_empty() {
        return;
    }
    p.requesting = true;
    ui.seerr.inflight.push((p.media_type.clone(), p.tmdb_id, d.title.clone()));
    let _ = ui.ui_tx.send(UiCmd::SeerrRequest {
        media_type: p.media_type.clone(),
        tmdb_id: p.tmdb_id,
        title: d.title.clone(),
        seasons,
    });
    seerr::rebuild_title();
}

// ---- Actions (targets of the title page and billboard buttons) -------------------------

/// Open an external URL in the browser. Never in the self-test.
unsafe fn open_url(url: &str) {
    if crate::app::selftest::enabled() {
        return;
    }
    let u: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns(url)];
    if u.is_null() {
        return;
    }
    let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
    let _: Bool = msg_send![ws, openURL: u];
}

pub(crate) unsafe fn trailer_clicked() {
    let ui = UI.get().unwrap();
    if let Some(d) = ui.seerr.title.as_ref().and_then(|p| p.detail.as_ref()) {
        if let Some(url) = trailer_url(d) {
            open_url(&url);
        }
    }
}

pub(crate) unsafe fn play_clicked() {
    let ui = UI.get_mut().unwrap();
    let Some(p) = ui.seerr.title.as_ref() else { return };
    ui.seerr.play_lookup = Some(p.tmdb_id);
    let _ = ui.ui_tx.send(UiCmd::SeerrLibrary { media_type: p.media_type.clone(), tmdb_id: p.tmdb_id });
}

pub(crate) unsafe fn season_toggled(number: u32) {
    let ui = UI.get_mut().unwrap();
    if let Some(p) = ui.seerr.title.as_mut() {
        if let Some(i) = p.selected.iter().position(|n| *n == number) {
            p.selected.remove(i);
        } else {
            p.selected.push(number);
            p.selected.sort_unstable();
        }
        p.confirming = false;
    }
    seerr::rebuild_title();
}

// ---- Views -------------------------------------------------------------------------------

/// Billboard height for a document `w` wide: a wide hero, clamped so it reads as a banner.
pub fn billboard_h(w: f64) -> f64 {
    (w * 0.46).clamp(340., 520.)
}

/// Title page hero height.
pub fn hero_h(w: f64) -> f64 {
    (w * 0.42).clamp(300., 460.)
}

const ROW_CARD_W: f64 = 200.;
const ROW_HEAD_H: f64 = 34.;
const ROW_GAP: f64 = SP8;
const TILE_GAP: f64 = SP4;
const BTN_H: f64 = 32.;

/// A plain button with a title and an action; `primary` gives it the accent bezel.
unsafe fn button(title: &str, action: Sel, primary: bool, symbol_name: Option<&str>) -> *mut AnyObject {
    let ui = UI.get_mut().unwrap();
    let b: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(title), target: ui.ctl.get(), action: action];
    let _: () = msg_send![b, setBezelStyle: 1isize];
    if primary {
        let _: () = msg_send![b, setBezelColor: color(C_ACCENT)];
    }
    if let Some(s) = symbol_name {
        let img = symbol(s, 12., Weight::Medium);
        if !img.is_null() {
            let _: () = msg_send![b, setImage: img];
            let _: () = msg_send![b, setImagePosition: 7isize]; // leading
        }
    }
    b
}

/// Button width that fits its title with room either side.
unsafe fn button_w(b: *mut AnyObject) -> f64 {
    let cell: *mut AnyObject = msg_send![b, cell];
    let sz: NSSize = msg_send![cell, cellSize];
    (sz.w.ceil() + 2. * SP5).max(96.)
}

/// Tag and action helpers: a button's tag is the index of its item in `tagged`.
unsafe fn tag_item(item: &SeerrResult) -> NSInteger {
    let ui = UI.get_mut().unwrap();
    ui.seerr.tagged.push(item.clone());
    (ui.seerr.tagged.len() - 1) as NSInteger
}

/// Gradient that fades a backdrop into the canvas along its lower part.
unsafe fn fade_into_canvas(parent: *mut AnyObject, frame: NSRect) {
    let v: *mut AnyObject = msg_send![class!(NSView), alloc];
    let v: *mut AnyObject = autorel(msg_send![v, initWithFrame: frame]);
    set_autoresize(v, RESIZE_W | RESIZE_H);
    let layer = layer_of(v);
    let g: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
    let _: () = msg_send![g, setFrame: rect(0., 0., frame.size.w, frame.size.h)];
    let colors: *mut AnyObject = msg_send![class!(NSMutableArray), array];
    let locs: *mut AnyObject = msg_send![class!(NSMutableArray), array];
    for (a, l) in [(0.0, 0.0), (0.35, 0.45), (0.92, 0.8), (1.0, 1.0)] {
        let _: () = msg_send![colors, addObject: cg_a(C_CANVAS, a) as *mut AnyObject];
        let n: *mut AnyObject = msg_send![class!(NSNumber), numberWithDouble: l as f64];
        let _: () = msg_send![locs, addObject: n];
    }
    let _: () = msg_send![g, setColors: colors];
    let _: () = msg_send![g, setLocations: locs];
    let _: () = msg_send![layer, addSublayer: g];
    add_sub(parent, v);
}

/// A backdrop image view (fills `frame`), registered so its picture arrives later.
unsafe fn backdrop(parent: *mut AnyObject, frame: NSRect, path: Option<&str>) {
    let ui = UI.get_mut().unwrap();
    let art: *mut AnyObject = msg_send![class!(NSView), alloc];
    let art: *mut AnyObject = autorel(msg_send![art, initWithFrame: frame]);
    set_autoresize(art, RESIZE_W | RESIZE_H);
    let layer = layer_of(art);
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
    add_sub(parent, art);
    if let Some(p) = path {
        // The glyph is never shown for a backdrop; a detached view keeps `apply_thumb` simple.
        let glyph: *mut AnyObject = msg_send![class!(NSImageView), alloc];
        let glyph: *mut AnyObject = autorel(msg_send![glyph, initWithFrame: rect(0., 0., 0., 0.)]);
        let key = image_key(ImageSize::BackdropLarge, p);
        ui.seerr.thumbs.push((key.clone(), art, glyph));
        if let Some(img) = ui.seerr.images.get(&key) {
            seerr::apply_thumb(art, glyph, *img, false);
        }
    }
    fade_into_canvas(parent, frame);
}

/// The billboard: backdrop of the top trending title, its name and overview, and two buttons.
unsafe fn billboard(doc: *mut AnyObject, w: f64, item: &SeerrResult) -> f64 {
    let h = billboard_h(w);
    let host = flip_view(rect(0., 0., w, h));
    autorel(host);
    add_sub(doc, host);
    backdrop(host, rect(0., 0., w, h), item.backdrop_path.as_deref());

    let tw = (w * 0.56).clamp(260., 720.) - EDGE;
    let ow = (w * 0.42).clamp(260., 520.);
    let bw_btn = 124.;
    let mut y = h - SP8 - BTN_H;
    let lib = in_library(item.availability);
    let bt = button(if lib { "Play" } else { "Request" }, sel!(seerrOpenTitle:), true, Some(if lib { "play.fill" } else { "plus" }));
    let tag = tag_item(item);
    let _: () = msg_send![bt, setTag: tag];
    let bw = bw_btn;
    set_frame(bt, rect(EDGE, y, bw, BTN_H));
    add_sub(host, bt);
    let info = button("More Info", sel!(seerrOpenTitle:), false, Some("info.circle"));
    let _: () = msg_send![info, setTag: tag];
    set_frame(info, rect(EDGE + bw + SP3, y, 128., BTN_H));
    add_sub(host, info);

    if !item.overview.is_empty() {
        let oh = text_h(&item.overview, T_BODY, ow);
        y -= SP4 + oh;
        let ov = wrapping_label(&item.overview, T_BODY, C_TEXT, ALIGN_LEFT);
        set_frame(ov, rect(EDGE, y, ow, oh));
        add_sub(host, ov);
    }
    let mut meta = vec![];
    if let Some(y) = item.year {
        meta.push(y.to_string());
    }
    meta.push(crate::ui::seerr::media_label(&item.media_type));
    if let Some(v) = item.vote_average.filter(|v| *v > 0.) {
        meta.push(format!("★ {:.1}", v));
    }
    y -= SP3 + 18.;
    let ml = label(&meta.join(" · "), T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(ml, rect(EDGE, y, tw + EDGE, 18.));
    add_sub(host, ml);

    let th = text_h(&item.title, T_DISPLAY, tw);
    y -= SP2 + th;
    let tl = wrapping_label(&item.title, T_DISPLAY, C_TEXT, ALIGN_LEFT);
    set_frame(tl, rect(EDGE, y, tw, th));
    add_sub(host, tl);

    let eyebrow = label("Trending now", T_CAPTION_EM, C_ACCENT, ALIGN_LEFT);
    y -= SP2 + 16.;
    set_frame(eyebrow, rect(EDGE, y, 240., 16.));
    add_sub(host, eyebrow);
    h
}

/// One poster row: heading, then a horizontal strip of cards (placeholders until loaded).
unsafe fn row(doc: *mut AnyObject, y: f64, w: f64, r: &BrowseRow) -> f64 {
    let head = label(&r.title, T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, y, w - 2. * EDGE, ROW_HEAD_H));
    add_sub(doc, head);
    let poster_h = (ROW_CARD_W * POSTER_ASPECT).round();
    let strip_y = y + ROW_HEAD_H + SP2;
    let strip_h = poster_h + 2. * SP1;
    let n = if r.items.is_empty() { 7 } else { r.items.len() };
    let content_w = EDGE + n as f64 * (ROW_CARD_W + TILE_GAP) - TILE_GAP + EDGE;

    let sc: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let sc: *mut AnyObject = autorel(msg_send![sc, initWithFrame: rect(0., strip_y, w, strip_h)]);
    set_autoresize(sc, RESIZE_W);
    let _: () = msg_send![sc, setHasHorizontalScroller: Bool::YES];
    let _: () = msg_send![sc, setHasVerticalScroller: Bool::NO];
    let _: () = msg_send![sc, setDrawsBackground: Bool::NO];
    let _: () = msg_send![sc, setBorderType: 0usize];
    let inner = flip_view(rect(0., 0., content_w.max(w), strip_h));
    autorel(inner);
    let _: () = msg_send![sc, setDocumentView: inner];
    add_sub(doc, sc);
    for i in 0..n {
        let x = EDGE + i as f64 * (ROW_CARD_W + TILE_GAP);
        let f = rect(x, SP1, ROW_CARD_W, poster_h);
        if let Some(item) = r.items.get(i) {
            let card = seerr::poster_tile(item, f);
            add_sub(inner, card);
        } else {
            let ph = surface(f, C_FILL_SEL, R_POSTER);
            add_sub(inner, ph);
        }
    }
    ROW_HEAD_H + SP2 + strip_h + ROW_GAP
}

/// Discover's document: the billboard, then every row that has items or is still loading.
/// Returns the document height.
pub(crate) unsafe fn build_discover(doc: *mut AnyObject, w: f64) -> f64 {
    let ui = UI.get_mut().unwrap();
    let mut y = 0.;
    if let Some(b) = ui.seerr.billboard.clone() {
        y += billboard(doc, w, &b) + SP8;
    }
    let rows = ui.seerr.rows.clone();
    for r in rows.iter().filter(|r| !(r.loaded && r.items.is_empty())) {
        y += row(doc, y, w, r);
    }
    y + SP8
}

/// My Requests, grouped by status. Each row opens its title page.
pub(crate) unsafe fn build_requests(doc: *mut AnyObject, w: f64, items: &[SeerrRequestItem]) -> f64 {
    let mut y = SP6;
    for g in ReqGroup::ALL {
        let members: Vec<&SeerrRequestItem> = items.iter().filter(|r| req_group(r) == g).collect();
        if members.is_empty() {
            continue;
        }
        let head = label(&format!("{} · {}", g.title(), members.len()), T_TITLE, C_TEXT, ALIGN_LEFT);
        set_frame(head, rect(EDGE, y, w - 2. * EDGE, 30.));
        add_sub(doc, head);
        y += 30. + SP3;
        for r in members {
            y += request_row(doc, y, w, r) + SP2;
        }
        y += SP6;
    }
    y
}

/// One request: poster, title, date and seasons, status pill, and a tap target over it all.
unsafe fn request_row(doc: *mut AnyObject, y: f64, w: f64, r: &SeerrRequestItem) -> f64 {
    let rh = 92.;
    let host = flip_view(rect(EDGE, y, w - 2. * EDGE, rh));
    autorel(host);
    add_sub(doc, host);
    let surf = surface(rect(0., 0., w - 2. * EDGE, rh), C_CARD, R_CARD);
    add_sub(host, surf);
    let pw = 56.;
    let ph = (pw * POSTER_ASPECT).round();
    let poster = match &r.poster_path {
        Some(p) => seerr::thumb_view(p, ImageSize::PosterSmall, rect(SP3, (rh - ph) / 2., pw, ph)),
        None => surface(rect(SP3, (rh - ph) / 2., pw, ph), C_FILL_SEL, R_THUMB),
    };
    add_sub(host, poster);

    let tx = SP3 + pw + SP4;
    let pill_w_max = 180.;
    let (pill, pwid) = make_pill(&request_status_text(r), request_role(r), pill_w_max);
    let pill_x = w - 2. * EDGE - SP4 - pwid;
    set_frame(pill, rect(pill_x, rh - SP3 - PILL_H - 14., pwid, PILL_H));
    add_sub(host, pill);

    let text_w = (pill_x - SP3 - tx).max(60.);
    let title = label(&r.title, T_BODY_EM, C_TEXT, ALIGN_LEFT);
    set_frame(title, rect(tx, rh - SP3 - 22., text_w, 22.));
    add_sub(host, title);
    let mut sub = crate::ui::seerr::media_label(&r.media_type);
    let date = short_date(&r.created_at);
    if !date.is_empty() {
        sub.push_str(&format!(" · Requested {}", date));
    }
    let seasons = seasons_text(r);
    if !seasons.is_empty() {
        sub.push_str(&format!(" · {}", seasons));
    }
    let sl = label(&sub, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(sl, rect(tx, SP3 + 6., text_w, 18.));
    add_sub(host, sl);

    let item = SeerrResult {
        media_type: r.media_type.clone(),
        tmdb_id: r.tmdb_id,
        title: r.title.clone(),
        year: r.year,
        poster_path: r.poster_path.clone(),
        availability: r.availability,
        request_status: Some(r.status),
        ..Default::default()
    };
    let tap: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: UI.get().unwrap().ctl.get(), action: sel!(seerrOpenTitle:)];
    let _: () = msg_send![tap, setBordered: Bool::NO];
    let _: () = msg_send![tap, setTag: tag_item(&item)];
    set_frame(tap, rect(0., 0., w - 2. * EDGE, rh));
    add_sub(host, tap);
    rh
}

/// Status text for a request row: the request status, or the availability when unknown.
pub fn request_status_text(r: &SeerrRequestItem) -> String {
    match r.status {
        RequestStatus::Unknown => r.availability.label().to_string(),
        s => s.label().to_string(),
    }
}

/// Colour role for a request row's pill.
pub fn request_role(r: &SeerrRequestItem) -> &'static str {
    match r.status {
        RequestStatus::Declined | RequestStatus::Failed => "danger",
        RequestStatus::Completed => "ok",
        RequestStatus::Pending | RequestStatus::Approved => "warn",
        RequestStatus::Unknown => match r.availability {
            Availability::Available => "ok",
            Availability::Blocklisted => "danger",
            Availability::NotRequested => "neutral",
            _ => "warn",
        },
    }
}

/// The title page: hero backdrop, facts, the action buttons, overview, seasons and cast.
/// Returns the document height.
pub(crate) unsafe fn build_title(doc: *mut AnyObject, w: f64, page: &TitlePage) -> f64 {
    let d = page.detail.as_ref();
    let hh = hero_h(w);
    let hero = flip_view(rect(0., 0., w, hh));
    autorel(hero);
    add_sub(doc, hero);
    backdrop(hero, rect(0., 0., w, hh), d.and_then(|d| d.backdrop_path.as_deref()));
    let back = button("Back", sel!(seerrTitleBack:), false, Some("chevron.left"));
    set_frame(back, rect(EDGE, SP4, button_w(back), BTN_H));
    add_sub(doc, back);

    // The facts start on the faded lower part of the hero and run on into the page.
    let cw = w - 2. * EDGE;
    let title = d.map(|d| d.title.clone()).unwrap_or_else(|| page.title.clone());
    let mut y = hh - 170.;
    let th = text_h(&title, T_DISPLAY, cw);
    add_text(doc, &title, T_DISPLAY, C_TEXT, rect(EDGE, y, cw, th));
    y += th + SP2;
    if let Some(d) = d {
        add_text(doc, &meta_line(d), T_CALLOUT, C_TEXT2, rect(EDGE, y, cw, 18.));
        y += 18. + SP3;
        if !d.tagline.is_empty() {
            let tg = text_h(&d.tagline, T_BODY, cw);
            add_text(doc, &d.tagline, T_BODY, C_TEXT2, rect(EDGE, y, cw, tg));
            y += tg + SP3;
        }
    }
    y += SP2;

    let buttons = title_buttons(page, d);
    let mut bx = EDGE;
    for b in buttons.iter() {
        let bw = button_w(*b);
        set_frame(*b, rect(bx, y, bw, BTN_H));
        add_sub(doc, *b);
        bx += bw + SP3;
    }
    y += BTN_H + SP6;

    match d {
        None => {
            add_text(doc, "Loading…", T_CALLOUT, C_TEXT2, rect(EDGE, y, cw, 18.));
            y += 18. + SP6;
        }
        Some(d) => {
            if !d.overview.is_empty() {
                let ow = cw.min(680.);
                let oh = text_h(&d.overview, T_BODY, ow);
                add_text(doc, &d.overview, T_BODY, C_TEXT, rect(EDGE, y, ow, oh));
                y += oh + SP8;
            }
            if !d.seasons.is_empty() {
                y = seasons_block(doc, y, w, d, page);
            }
            if !d.cast.is_empty() {
                y = cast_block(doc, y, w, d);
            }
        }
    }
    y + SP8
}

/// The buttons under a title's facts: what the title allows right now.
unsafe fn title_buttons(page: &TitlePage, d: Option<&TitleDetail>) -> Vec<*mut AnyObject> {
    let Some(d) = d else { return vec![] };
    let mut out = vec![];
    let requested = matches!(d.availability, Availability::Pending | Availability::Processing)
        || matches!(d.request_status, Some(RequestStatus::Pending) | Some(RequestStatus::Approved));
    if page.requesting {
        let b = button("Requesting…", sel!(seerrRequest:), true, None);
        let _: () = msg_send![b, setEnabled: Bool::NO];
        out.push(b);
    } else if in_library(d.availability) {
        out.push(button("Play", sel!(seerrPlay:), true, Some("play.fill")));
    } else if requested {
        let b = button("Requested", sel!(seerrRequest:), false, Some("clock"));
        let _: () = msg_send![b, setEnabled: Bool::NO];
        out.push(b);
    } else if page.confirming {
        let n = page.selected.len();
        let text = if page.media_type == "tv" {
            format!("Confirm {} season{}", n, if n == 1 { "" } else { "s" })
        } else {
            "Confirm request".to_string()
        };
        let b = button(&text, sel!(seerrRequest:), true, Some("checkmark"));
        let _: () = msg_send![b, setEnabled: Bool::new(page.media_type != "tv" || n > 0)];
        out.push(b);
        out.push(button("Cancel", sel!(seerrCancelRequest:), false, None));
    } else {
        let n = page.selected.len();
        let text = if page.media_type != "tv" {
            "Request".to_string()
        } else {
            match n {
                0 => "Pick seasons".to_string(),
                1 => "Request season".to_string(),
                n => format!("Request {} seasons", n),
            }
        };
        let b = button(&text, sel!(seerrRequest:), true, Some("plus"));
        let _: () = msg_send![b, setEnabled: Bool::new(page.media_type != "tv" || n > 0)];
        out.push(b);
    }
    if trailer_url(d).is_some() {
        out.push(button("Trailer", sel!(seerrTrailer:), false, Some("play.rectangle")));
    }
    out
}

/// A wrapping text label placed at `f` (the caller owns the geometry).
unsafe fn add_text(parent: *mut AnyObject, text: &str, t: TypeStyle, c: Col, f: NSRect) {
    let v = wrapping_label(text, t, c, ALIGN_LEFT);
    set_frame(v, f);
    add_sub(parent, v);
}

/// TV: one checkbox per season (available or requested seasons are shown, not ticked).
unsafe fn seasons_block(doc: *mut AnyObject, y0: f64, w: f64, d: &TitleDetail, page: &TitlePage) -> f64 {
    let ui = UI.get_mut().unwrap();
    let head = label("Seasons", T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, y0, w - 2. * EDGE, 30.));
    add_sub(doc, head);
    let mut y = y0 + 30. + SP3;
    for s in d.seasons.iter().filter(|s| s.number > 0) {
        let open = s.availability == Availability::NotRequested;
        let cb: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(&season_text(s)), target: ui.ctl.get(), action: sel!(seerrSeason:)];
        let _: () = msg_send![cb, setButtonType: 3usize]; // checkbox
        let _: () = msg_send![cb, setTag: s.number as NSInteger];
        let _: () = msg_send![cb, setEnabled: Bool::new(open && !page.requesting)];
        let on = open && page.selected.contains(&s.number);
        let _: () = msg_send![cb, setState: if on { 1isize } else { 0isize }];
        set_frame(cb, rect(EDGE, y, 320., 24.));
        add_sub(doc, cb);
        if !open {
            let (pill, pw) = make_pill(s.availability.label(), avail_role(s.availability), 200.);
            set_frame(pill, rect(EDGE + 340., y + (24. - PILL_H) / 2., pw, PILL_H));
            add_sub(doc, pill);
        }
        y += 24. + SP2;
    }
    y + SP6
}

fn avail_role(a: Availability) -> &'static str {
    match a {
        Availability::Available => "ok",
        Availability::Pending | Availability::Processing | Availability::PartiallyAvailable => "warn",
        Availability::Blocklisted => "danger",
        Availability::NotRequested => "neutral",
    }
}

/// Cast: round headshots in a wrapping grid, name and character under each.
unsafe fn cast_block(doc: *mut AnyObject, y0: f64, w: f64, d: &TitleDetail) -> f64 {
    let head = label("Cast", T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, y0, w - 2. * EDGE, 30.));
    add_sub(doc, head);
    let cell_w = 150.;
    let cols = (((w - 2. * EDGE + SP4) / (cell_w + SP4)).floor() as usize).max(1);
    let photo = 72.;
    let mut y = y0 + 30. + SP4;
    for (i, m) in d.cast.iter().take(12).enumerate() {
        let x = EDGE + (i % cols) as f64 * (cell_w + SP4);
        if i % cols == 0 && i > 0 {
            y += photo + SP3 + 36. + SP5;
        }
        let img = match &m.profile_path {
            Some(p) => seerr::thumb_view_round(p, rect(x, y, photo, photo)),
            None => surface(rect(x, y, photo, photo), C_FILL_SEL, photo / 2.),
        };
        add_sub(doc, img);
        let nm = label(&m.name, T_BODY_EM, C_TEXT, ALIGN_LEFT);
        set_frame(nm, rect(x, y + photo + SP2, cell_w, 18.));
        add_sub(doc, nm);
        let ch = label(&m.character, T_CAPTION, C_TEXT2, ALIGN_LEFT);
        set_frame(ch, rect(x, y + photo + SP2 + 18., cell_w, 16.));
        add_sub(doc, ch);
    }
    let rows = (d.cast.iter().take(12).count() + cols - 1) / cols.max(1);
    y0 + 30. + SP4 + rows as f64 * (photo + SP3 + 36. + SP5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seerr::models::{SeasonInfo, Trailer};

    fn item(a: Availability, st: Option<RequestStatus>) -> SeerrResult {
        SeerrResult { media_type: "movie".into(), tmdb_id: 1, title: "T".into(), availability: a, request_status: st, ..Default::default() }
    }

    fn detail() -> TitleDetail {
        TitleDetail { media_type: "tv".into(), tmdb_id: 9, title: "Show".into(), ..Default::default() }
    }

    #[test]
    fn default_rows_are_the_home_rows_then_genres() {
        let rows = default_rows();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].title, "Trending Now");
        assert_eq!(rows[4].title, "Upcoming TV Shows");
        assert!(matches!(&rows[5].src, RowSrc::Genre { media_type, genre_id: 28 } if media_type == "movie"));
        assert!(rows.iter().all(|r| !r.loaded && r.items.is_empty()));
    }

    #[test]
    fn badges_follow_availability_then_request_status() {
        assert_eq!(badge_for(&item(Availability::Available, None)), Some(("Available", "ok")));
        assert_eq!(badge_for(&item(Availability::PartiallyAvailable, None)).unwrap().0, "Partially available");
        assert_eq!(badge_for(&item(Availability::Processing, None)).unwrap().0, "Processing");
        assert_eq!(badge_for(&item(Availability::Pending, None)).unwrap().0, "Pending");
        assert_eq!(badge_for(&item(Availability::NotRequested, Some(RequestStatus::Approved))).unwrap().0, "Requested");
        assert_eq!(badge_for(&item(Availability::NotRequested, None)), None);
        assert_eq!(badge_for(&item(Availability::Blocklisted, None)), None);
    }

    #[test]
    fn runtime_and_meta_line() {
        assert_eq!(runtime_text(45), "45m");
        assert_eq!(runtime_text(120), "2h");
        assert_eq!(runtime_text(148), "2h 28m");
        let d = TitleDetail {
            year: Some(2010),
            runtime: Some(148),
            genres: vec!["Action".into(), "Sci-Fi".into(), "Thriller".into(), "Drama".into()],
            vote_average: Some(8.36),
            ..detail()
        };
        assert_eq!(meta_line(&d), "2010 · 2h 28m · Action, Sci-Fi, Thriller · ★ 8.4");
        assert_eq!(meta_line(&detail()), "");
    }

    #[test]
    fn default_seasons_skip_specials_and_owned_seasons() {
        let s = |n, a| SeasonInfo { number: n, name: String::new(), episode_count: 10, availability: a };
        let d = TitleDetail {
            seasons: vec![
                s(0, Availability::NotRequested),
                s(1, Availability::Available),
                s(2, Availability::NotRequested),
                s(3, Availability::Pending),
                s(4, Availability::NotRequested),
            ],
            ..detail()
        };
        assert_eq!(default_seasons(&d), vec![2, 4]);
        assert_eq!(season_text(&SeasonInfo { number: 2, name: String::new(), episode_count: 1, availability: Availability::NotRequested }), "Season 2 · 1 episode");
    }

    #[test]
    fn trailers_are_youtube_only_and_key_checked() {
        let t = |site: &str, key: &str| Trailer { name: "x".into(), key: key.into(), site: site.into() };
        let d = TitleDetail { trailers: vec![t("Vimeo", "abc"), t("YouTube", "dQw4w9WgXcQ")], ..detail() };
        assert_eq!(trailer_url(&d).as_deref(), Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        let bad = TitleDetail { trailers: vec![t("YouTube", "x&evil=1")], ..detail() };
        assert_eq!(trailer_url(&bad), None);
        assert_eq!(trailer_url(&detail()), None);
    }

    #[test]
    fn request_groups() {
        let req = |st, av| SeerrRequestItem {
            request_id: 1,
            media_type: "movie".into(),
            tmdb_id: 1,
            title: "T".into(),
            year: None,
            poster_path: None,
            status: st,
            availability: av,
            requested_by: String::new(),
            created_at: String::new(),
            seasons: vec![],
        };
        assert_eq!(req_group(&req(RequestStatus::Pending, Availability::Pending)), ReqGroup::Pending);
        assert_eq!(req_group(&req(RequestStatus::Declined, Availability::NotRequested)), ReqGroup::Pending);
        assert_eq!(req_group(&req(RequestStatus::Approved, Availability::Processing)), ReqGroup::Approved);
        assert_eq!(req_group(&req(RequestStatus::Unknown, Availability::PartiallyAvailable)), ReqGroup::Approved);
        assert_eq!(req_group(&req(RequestStatus::Completed, Availability::Available)), ReqGroup::Available);
        assert_eq!(req_group(&req(RequestStatus::Unknown, Availability::Available)), ReqGroup::Available);
    }

    #[test]
    fn dates_and_season_lists() {
        assert_eq!(short_date("2026-10-25T12:00:00.000Z"), "Oct 25, 2026");
        assert_eq!(short_date("2026-13-01"), "");
        assert_eq!(short_date(""), "");
        let mut r = SeerrRequestItem {
            request_id: 1,
            media_type: "tv".into(),
            tmdb_id: 1,
            title: "T".into(),
            year: None,
            poster_path: None,
            status: RequestStatus::Pending,
            availability: Availability::Pending,
            requested_by: String::new(),
            created_at: String::new(),
            seasons: vec![],
        };
        assert_eq!(seasons_text(&r), "");
        r.seasons = vec![2];
        assert_eq!(seasons_text(&r), "Season 2");
        r.seasons = vec![1, 2, 3];
        assert_eq!(seasons_text(&r), "Seasons 1, 2, 3");
    }

    #[test]
    fn cast_and_billboard_geometry_stay_in_range() {
        for w in [720., 1040., 1700.] {
            assert!(billboard_h(w) >= 340. && billboard_h(w) <= 520.);
            assert!(hero_h(w) >= 300. && hero_h(w) <= 460.);
        }
    }
}
