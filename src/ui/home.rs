//! Home screen: a billboard of featured titles on top, then a vertical scroll of
//! horizontal rows (Continue Watching, Next Up, Recently Added, Because You
//! Watched, Top Rated, genres). Cards are the library grid's own cells, so hover
//! lift, progress and posters behave the same. Selecting Home swaps the library
//! grid for this scroll view; any library load swaps it back.
//!
//! One big library adds a segmented control in the header (Home | Movies | TV
//! Shows). The Movies and TV Shows tabs reuse this scroll view: their rows, then a
//! full poster grid of every movie or show in the visible libraries.
//!
//! "See all" on a row heading replaces the rows with that row's grid, with a
//! "Home" button to come back.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::{HomeRow, HomeRows, UiCmd};
use crate::config::{HomeSection, LayoutMode};
use crate::jellyfin::models::ListItem;
use crate::jellyfin::urls::fmt_runtime;
use crate::platform::appkit::*;
use crate::ui::library::{build_cell_keyed, free_cell_data, grid_metrics, layout_cell, poster_key, Metrics};
use crate::ui::shell::{header_layout, hide_state, set_count, show_state, sidebar_refresh_selection};
use crate::ui::theme::*;

/// `cfg.last_view` value meaning "Home was the last selected place".
pub const HOME_ID: &str = "home";
/// `cfg.last_view` value meaning "Browse (unified layout) was the last place".
pub const BROWSE_ID: &str = "browse";

/// Poster cards (2:3) and wide cards (16:9, Continue Watching and Next Up).
pub const POSTER_W: f64 = 176.;
pub const WIDE_W: f64 = 304.;
const WIDE_RATIO: f64 = 9. / 16.;
const CARD_GAP: f64 = SP4;
/// Room around a strip so a card's hover lift is not clipped.
const STRIP_PAD: f64 = SP3;
const ROW_TITLE_H: f64 = 30.;
const ROW_GAP: f64 = SP6;

/// Billboard: the slide advances after this many seconds, unless the pointer is on it.
const BILLBOARD_SECS: f64 = 8.;
/// Billboard height: this share of the visible Home area, never shorter than the minimum.
const BILLBOARD_SHARE: f64 = 0.55;
const BILLBOARD_MIN_H: f64 = 360.;
/// Largest title logo, in points.
const LOGO_MAX_W: f64 = 420.;
const LOGO_MAX_H: f64 = 120.;
/// Text column on the billboard: this wide at most, and at most half the window.
const TEXT_MAX_W: f64 = 560.;
const OVERVIEW_LINE: f64 = 20.;
const OVERVIEW_LINES: f64 = 3.;
const META_H: f64 = 18.;
const TITLE_H: f64 = 56.;
/// Button row height and its distance from the billboard's bottom edge.
const PILL_H: f64 = 40.;
const TEXT_BOTTOM: f64 = 56.;
/// Page dots: size and gap.
const DOT: f64 = 7.;
const DOT_GAP: f64 = 6.;

/// Segment labels, in the order `section_for_index` reads them.
const SEGMENTS: [&str; 3] = ["Home", "Movies", "TV Shows"];

/// Artwork key for a wide card. Separate from the poster key, because the same
/// item can appear as a wide card and as a poster (Thumb/Backdrop vs Primary).
pub fn wide_key(item_id: &str) -> u64 {
    poster_key(&format!("wide:{item_id}"))
}

/// Artwork key for a billboard slide's backdrop.
pub fn billboard_key(item_id: &str) -> u64 {
    poster_key(&format!("billboard:{item_id}"))
}

/// Artwork key for a billboard slide's transparent title logo.
pub fn logo_key(item_id: &str) -> u64 {
    poster_key(&format!("logo:{item_id}"))
}

/// The cells of one laid-out row (kept so poster arrivals can update them).
pub struct HomeRowView {
    pub cells: Vec<CellHandle>,
}

/// A rounded button drawn as layers: a fill, a centred label, and a transparent
/// NSButton on top that takes the click.
#[derive(Clone, Copy)]
pub struct Pill {
    pub view: *mut AnyObject,
    pub label: *mut AnyObject,
    pub hit: *mut AnyObject,
}

impl Pill {
    fn null() -> Pill {
        Pill { view: std::ptr::null_mut(), label: std::ptr::null_mut(), hit: std::ptr::null_mut() }
    }
}

/// The billboard's views and the slide on screen. The views are rebuilt with the
/// rows; the slides and the current index are kept here.
pub struct Billboard {
    /// Slides, in order (see `app::home::billboard_items`). Empty off the Home tab.
    pub items: Vec<ListItem>,
    /// Slide on screen.
    pub index: usize,
    /// Art layer that shows the current slide; the other fades in on the next one.
    pub front: usize,
    /// Pointer is over the billboard: the timer waits.
    pub hovered: bool,
    /// Size of the views, as last laid out.
    pub w: f64,
    pub h: f64,
    /// Logo aspect (width / height) of the current slide, 0 when it has no logo.
    pub logo_aspect: f64,
    pub host: *mut AnyObject,
    pub art: [*mut AnyObject; 2],
    pub scrim_side: *mut AnyObject,
    pub scrim_bottom: *mut AnyObject,
    pub logo: *mut AnyObject,
    pub title: *mut AnyObject,
    pub meta: *mut AnyObject,
    pub overview: *mut AnyObject,
    pub play: Pill,
    pub info: Pill,
    pub dots: Vec<*mut AnyObject>,
}

impl Billboard {
    fn new() -> Billboard {
        let null = std::ptr::null_mut();
        Billboard {
            items: Vec::new(),
            index: 0,
            front: 0,
            hovered: false,
            w: 0.,
            h: 0.,
            logo_aspect: 0.,
            host: null,
            art: [null, null],
            scrim_side: null,
            scrim_bottom: null,
            logo: null,
            title: null,
            meta: null,
            overview: null,
            play: Pill::null(),
            info: Pill::null(),
            dots: Vec::new(),
        }
    }

    /// Forget the views (they are removed from the document with the rest of Home).
    fn clear_views(&mut self) {
        let null = std::ptr::null_mut();
        self.host = null;
        self.art = [null, null];
        self.scrim_side = null;
        self.scrim_bottom = null;
        self.logo = null;
        self.title = null;
        self.meta = null;
        self.overview = null;
        self.play = Pill::null();
        self.info = Pill::null();
        self.dots.clear();
        self.w = 0.;
        self.h = 0.;
        self.logo_aspect = 0.;
    }
}

pub struct HomeUi {
    /// Vertical scroll view holding every row (shares the library grid's frame).
    pub scroll: crate::platform::appkit::Ptr,
    /// Flipped document view inside `scroll`.
    pub doc: crate::platform::appkit::Ptr,
    /// The "Home" sidebar row.
    pub row: crate::platform::appkit::Ptr,
    /// Home | Movies | TV Shows control in the header (one big library only).
    pub seg: crate::platform::appkit::Ptr,
    /// Tab on screen. Always `Home` in the separate layout.
    pub section: HomeSection,
    /// The rows (single source of truth).
    pub rows: Vec<HomeRow>,
    /// Movies or TV Shows tab: every item of the type, shown as a poster grid under the rows.
    pub grid: Vec<ListItem>,
    /// Cells: one view per row, then the grid (last) when there is one.
    pub views: Vec<HomeRowView>,
    /// Every card's item; a card's button tag is its index here.
    pub flat: Vec<ListItem>,
    /// Y of the grid's first row inside `doc`.
    pub grid_top: f64,
    /// Document width the cells were last placed for.
    pub laid_w: f64,
    /// Home is the visible screen.
    pub active: bool,
    /// A `LoadHome` or `LoadHomeSection` is in flight.
    pub loading: bool,
    /// The billboard on the Home tab.
    pub billboard: Billboard,
    /// Height the billboard was last laid out at (0 when there is none).
    pub billboard_h: f64,
    /// Title of the row open in "See all", if any.
    pub see_all: Option<String>,
    /// Row headings: title label and "See all" button, for width changes.
    pub heads: Vec<(*mut AnyObject, *mut AnyObject)>,
}

impl HomeUi {
    pub fn new() -> HomeUi {
        HomeUi {
            scroll: Ptr(std::ptr::null_mut()),
            doc: Ptr(std::ptr::null_mut()),
            row: Ptr(std::ptr::null_mut()),
            seg: Ptr(std::ptr::null_mut()),
            section: HomeSection::Home,
            rows: Vec::new(),
            grid: Vec::new(),
            views: Vec::new(),
            flat: Vec::new(),
            grid_top: 0.,
            laid_w: 0.,
            active: false,
            loading: false,
            billboard: Billboard::new(),
            billboard_h: 0.,
            see_all: None,
            heads: Vec::new(),
        }
    }
}

/// Billboard height for a visible Home area of `visible_h` points.
pub fn billboard_height_for(visible_h: f64) -> f64 {
    (visible_h * BILLBOARD_SHARE).round().max(BILLBOARD_MIN_H)
}

/// Width of the billboard's text column for a Home area `w` wide.
fn text_width(w: f64) -> f64 {
    TEXT_MAX_W.min(w * 0.5).max(200.)
}

/// Time left in a title as "42m left" or "1h 05m left". None without a runtime,
/// before the first minute of watching, or once the title is finished.
pub fn time_left(run_ticks: Option<i64>, pos_ticks: i64) -> Option<String> {
    let run = run_ticks.filter(|r| *r > 0)?;
    if pos_ticks <= 0 || pos_ticks >= run {
        return None;
    }
    // Ticks are 100 ns: a minute is 600 million. Round up so a near-finished title says 1m.
    let mins = (((run - pos_ticks) as f64 / 600_000_000.0).ceil() as i64).max(1);
    Some(if mins >= 60 { format!("{}h {:02}m left", mins / 60, mins % 60) } else { format!("{}m left", mins) })
}

/// "S1:E3" for an episode with both numbers.
fn episode_code(it: &ListItem) -> Option<String> {
    match (it.season, it.episode) {
        (Some(s), Some(e)) => Some(format!("S{}:E{}", s, e)),
        _ => None,
    }
}

/// Billboard title: the series for an episode, else the item's own name.
pub fn billboard_title(it: &ListItem) -> String {
    match it.item_type.as_str() {
        "Episode" => it.series_name.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| it.name.clone()),
        _ => it.name.clone(),
    }
}

/// Billboard metadata line: episode code, year, rating, stars, runtime, two genres.
pub fn billboard_meta(it: &ListItem) -> String {
    let mut parts: Vec<String> = Vec::new();
    if it.item_type == "Episode" {
        parts.extend(episode_code(it));
    }
    parts.extend(it.year.map(|y| y.to_string()));
    parts.extend(it.official_rating.clone().filter(|r| !r.is_empty()));
    parts.extend(it.community_rating.filter(|c| *c > 0.).map(|c| format!("★ {:.1}", c)));
    let runtime = fmt_runtime(it.run_time_ticks);
    if !runtime.is_empty() {
        parts.push(runtime);
    }
    let genres: Vec<&str> = it.genres.iter().map(String::as_str).filter(|g| !g.is_empty()).take(2).collect();
    if !genres.is_empty() {
        parts.push(genres.join(", "));
    }
    parts.join(" · ")
}

/// Primary billboard button: "Resume · 42m left" when partly watched, "Play S2:E4"
/// for an episode, "Open" for a series (it opens its page), else "Play".
pub fn play_label(it: &ListItem) -> String {
    if it.item_type == "Series" {
        return "Open".into();
    }
    let pos = it.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0);
    if let Some(left) = time_left(it.run_time_ticks, pos) {
        return format!("Resume · {}", left);
    }
    match (it.item_type.as_str(), episode_code(it)) {
        ("Episode", Some(code)) => format!("Play {}", code),
        _ => "Play".into(),
    }
}

/// Caption for a wide card: (title, subtitle). An episode shows its series, then
/// "S1:E3 · name"; a movie shows its name. The time left follows when there is one.
pub fn wide_caption(it: &ListItem) -> (String, String) {
    let pos = it.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0);
    let left = time_left(it.run_time_ticks, pos);
    if it.item_type == "Episode" {
        let sub: Vec<String> = [episode_code(it), Some(it.name.clone()), left].into_iter().flatten().filter(|s| !s.is_empty()).collect();
        (billboard_title(it), sub.join(" · "))
    } else {
        (it.name.clone(), left.unwrap_or_default())
    }
}

/// A flipped plain view (`JRFlipView`, registered by the Seerr module): rows and
/// cards are laid out top-down.
unsafe fn flip_view(frame: NSRect) -> *mut AnyObject {
    let cls = AnyClass::get(c"JRFlipView").expect("JRFlipView registered at startup");
    let v: *mut AnyObject = msg_send![cls, alloc];
    msg_send![v, initWithFrame: frame] // +1; the caller autorels after adding
}

/// A plain view with its own layer, not flipped.
unsafe fn new_view(frame: NSRect) -> *mut AnyObject {
    let v: *mut AnyObject = msg_send![class!(NSView), alloc];
    let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
    autorel(v)
}

/// A bare layer (no view), autoreleased by the layer class.
unsafe fn plain_layer() -> *mut AnyObject {
    msg_send![class!(CALayer), layer]
}

/// A gradient layer between `colors`, from `start` to `end` in unit coordinates.
unsafe fn gradient(colors: &[*mut AnyObject], start: NSPoint, end: NSPoint) -> *mut AnyObject {
    let g: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
    let arr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: colors.as_ptr(), count: colors.len()];
    let _: () = msg_send![g, setColors: arr];
    let _: () = msg_send![g, setStartPoint: start];
    let _: () = msg_send![g, setEndPoint: end];
    g
}

extern "C" fn flipped(_this: *mut AnyObject, _c: Sel) -> Bool {
    Bool::YES
}

/// Home's document view: flipped, and it re-lays Home out when its width changes
/// (the billboard, the rows' headings and the grid all depend on it).
extern "C" fn home_doc_set_frame_size(this: *mut AnyObject, _c: Sel, s: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        if this as *mut AnyObject != ui.home.doc.get() || (s.w - ui.home.laid_w).abs() < 0.5 {
            return;
        }
        relayout(s.w);
    }
}

/// Register `JRHomeDoc` once (the document view class with the grid re-layout).
fn register_doc_class() {
    if AnyClass::get(c"JRHomeDoc").is_some() {
        return;
    }
    let mut d = ClassBuilder::new(c"JRHomeDoc", class!(NSView)).expect("JRHomeDoc is new");
    unsafe {
        d.add_method(sel!(isFlipped), flipped as extern "C" fn(*mut AnyObject, Sel) -> Bool);
        d.add_method(
            sel!(setFrameSize:),
            home_doc_set_frame_size as extern "C" fn(*mut AnyObject, Sel, NSSize),
        );
    }
    d.register();
}

extern "C" fn billboard_entered(_this: *mut AnyObject, _c: Sel, _e: *mut AnyObject) {
    unsafe { billboard_hover(true) }
}

extern "C" fn billboard_exited(_this: *mut AnyObject, _c: Sel, _e: *mut AnyObject) {
    unsafe { billboard_hover(false) }
}

/// Register `JRBillboard` once: the billboard view, which pauses its timer while the
/// pointer is over it.
fn register_billboard_class() {
    if AnyClass::get(c"JRBillboard").is_some() {
        return;
    }
    let mut d = ClassBuilder::new(c"JRBillboard", class!(NSView)).expect("JRBillboard is new");
    unsafe {
        d.add_method(sel!(mouseEntered:), billboard_entered as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
        d.add_method(sel!(mouseExited:), billboard_exited as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    }
    d.register();
}

extern "C" fn home_scroll_set_frame_size(this: *mut AnyObject, _c: Sel, s: NSSize) {
    unsafe {
        let old_h = frame_of(this).size.h;
        let _: () = msg_send![super(this, class!(NSScrollView)), setFrameSize: s];
        // The billboard is a share of the window's height, so a height change re-lays Home.
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        if this == ui.home.scroll.get() && ui.home.active && (old_h - s.h).abs() > 0.5 {
            relayout(bounds_of(this).size.w.max(200.));
        }
    }
}

/// Register `JRHomeScroll` once: the Home scroll view, which reports height changes.
fn register_scroll_class() {
    if AnyClass::get(c"JRHomeScroll").is_some() {
        return;
    }
    let mut d = ClassBuilder::new(c"JRHomeScroll", class!(NSScrollView)).expect("JRHomeScroll is new");
    unsafe {
        d.add_method(
            sel!(setFrameSize:),
            home_scroll_set_frame_size as extern "C" fn(*mut AnyObject, Sel, NSSize),
        );
    }
    d.register();
}

/// The segmented Home | Movies | TV Shows control, at the header's right edge.
/// Hidden until Home is showing in the one-big-library layout.
unsafe fn build_segment(header: *mut AnyObject, target: *mut AnyObject) -> *mut AnyObject {
    let labels: *mut AnyObject = msg_send![class!(NSMutableArray), array];
    for t in SEGMENTS {
        let _: () = msg_send![labels, addObject: ns(t)];
    }
    // Same control and size as the Seerr tabs.
    let seg: *mut AnyObject = msg_send![class!(NSSegmentedControl),
        segmentedControlWithLabels: labels, trackingMode: 0i64, target: target, action: sel!(homeSegment:)];
    let _: () = msg_send![seg, setControlSize: 3isize];
    let _: () = msg_send![seg, setSelectedSegment: 0isize];
    let _: () = msg_send![seg, sizeToFit];
    let f = frame_of(seg);
    let hb = bounds_of(header);
    set_frame(seg, rect(hb.size.w - EDGE - f.size.w, (HEADER_H - f.size.h) / 2., f.size.w, f.size.h));
    set_autoresize(seg, MARGIN_MINX); // stays anchored to the right edge
    set_hidden(seg, true);
    add_sub(header, seg);
    seg
}

/// The Home scroll view, hidden until Home is selected.
pub(crate) unsafe fn build_scroll(content_host: *mut AnyObject, frame: NSRect) {
    let ui = UI.get_mut().unwrap();
    register_scroll_class();
    let sc: *mut AnyObject = msg_send![AnyClass::get(c"JRHomeScroll").expect("registered"), alloc];
    let sc: *mut AnyObject = msg_send![sc, initWithFrame: frame];
    let _: () = msg_send![sc, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![sc, setDrawsBackground: Bool::YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    set_hidden(sc, true);
    add_sub(content_host, sc);
    autorel(sc);
    register_doc_class();
    let doc: *mut AnyObject = msg_send![class!(JRHomeDoc), alloc];
    let doc = autorel(msg_send![doc, initWithFrame: rect(0., 0., frame.size.w, 10.)]);
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    ui.home.scroll = Ptr(sc);
    ui.home.doc = Ptr(doc);
    if !ui.header.get().is_null() {
        let seg = build_segment(ui.header.get(), ui.ctl.get());
        ui.home.seg = Ptr(seg);
    }
}

/// Whether an item belongs on a tab. Home takes everything; the tabs take one type.
fn belongs(section: HomeSection, it: &ListItem) -> bool {
    match section {
        HomeSection::Home => true,
        HomeSection::Movies => it.item_type == "Movie",
        HomeSection::TvShows => matches!(it.item_type.as_str(), "Series" | "Episode"),
    }
}

fn section_title(section: HomeSection) -> &'static str {
    match section {
        HomeSection::Home => "Home",
        HomeSection::Movies => "Movies",
        HomeSection::TvShows => "TV Shows",
    }
}

/// The header's title for a tab. Home has none: the billboard is its heading.
fn header_text(section: HomeSection) -> &'static str {
    match section {
        HomeSection::Home => "",
        other => section_title(other),
    }
}

fn section_for_index(i: usize) -> HomeSection {
    match i {
        1 => HomeSection::Movies,
        2 => HomeSection::TvShows,
        _ => HomeSection::Home,
    }
}

fn index_for_section(section: HomeSection) -> isize {
    match section {
        HomeSection::Home => 0,
        HomeSection::Movies => 1,
        HomeSection::TvShows => 2,
    }
}

/// The tab that should show now: the remembered one in the one-big-library layout,
/// Home otherwise.
fn current_section() -> HomeSection {
    let ui = UI.get().unwrap();
    if ui.cfg.layout_mode == Some(LayoutMode::Unified) {
        ui.cfg.home_section.unwrap_or_default()
    } else {
        HomeSection::Home
    }
}

/// Replace the Home content with `home` (rows and grid for one tab). A reply for a
/// tab the user has already left is dropped.
pub(crate) unsafe fn set_rows(home: HomeRows) {
    let ui = UI.get_mut().unwrap();
    if home.section != ui.home.section {
        return;
    }
    let section = home.section;
    let mut rows = home.rows;
    let mut grid = home.grid;
    if section != HomeSection::Home {
        // Defence in depth: the loader already filters by type.
        for r in rows.iter_mut() {
            r.items.retain(|it| belongs(section, it));
        }
        rows.retain(|r| !r.items.is_empty());
        grid.retain(|it| belongs(section, it));
    }
    if let Some(t) = ui.home.see_all.clone() {
        if !rows.iter().any(|r| r.title == t) {
            ui.home.see_all = None;
        }
    }
    let slides = if section == HomeSection::Home { crate::app::home::billboard_items(&rows) } else { Vec::new() };
    let b = &mut ui.home.billboard;
    if b.index >= slides.len() {
        b.index = 0;
    }
    b.items = slides;
    ui.home.rows = rows;
    ui.home.grid = grid;
    ui.home.loading = false;
    rebuild();
    update_state();
    billboard_timer_sync();
}

/// Lay the grid's cells out for document width `w`. Returns the grid's bottom edge.
unsafe fn place_grid(w: f64) -> f64 {
    let ui = UI.get_mut().unwrap();
    let top = ui.home.grid_top;
    let m = grid_metrics(w, POSTER_ASPECT);
    let view = match ui.home.views.last() {
        Some(v) if !ui.home.grid.is_empty() => v,
        _ => return top,
    };
    tx_begin_instant();
    for (i, c) in view.cells.iter().enumerate() {
        let col = i % m.cols;
        let row = i / m.cols;
        let x = EDGE + col as f64 * (m.tile_w + GRID_COL_GAP);
        let y = top + row as f64 * (m.cell_h + GRID_ROW_GAP);
        set_frame(c.container, rect(x, y, m.tile_w, m.cell_h));
        layout_cell(c, &m);
    }
    tx_end();
    let rows = (view.cells.len() + m.cols - 1) / m.cols;
    top + rows as f64 * (m.cell_h + GRID_ROW_GAP) + SP2
}

fn rebuild_drop_cells() {
    let ui = UI.get_mut().unwrap();
    for row in ui.home.views.drain(..) {
        for c in &row.cells {
            unsafe { free_cell_data(c.container) };
        }
    }
    ui.home.flat.clear();
}

/// Whether the billboard belongs on screen: Home is showing, its tab has slides,
/// and no row is open in "See all".
fn billboard_wanted(ui: &crate::platform::appkit::Ui) -> bool {
    ui.home.active && ui.home.section == HomeSection::Home && ui.home.see_all.is_none() && !ui.home.billboard.items.is_empty()
}

/// Build the billboard's views at the top of `doc`, `bh` high, showing the current slide.
unsafe fn build_billboard(doc: *mut AnyObject, w: f64, bh: f64) {
    let ui = UI.get_mut().unwrap();
    ui.home.billboard.clear_views();
    if bh <= 0. {
        return;
    }
    register_billboard_class();
    let cls = AnyClass::get(c"JRBillboard").expect("JRBillboard registered");
    let host: *mut AnyObject = msg_send![cls, alloc];
    let host = autorel(msg_send![host, initWithFrame: rect(0., 0., w, bh)]);
    add_sub(doc, host);
    let ta: *mut AnyObject = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut AnyObject = msg_send![ta,
        initWithRect: rect(0., 0., w, bh), options: (0x01usize | 0x80usize | 0x200usize), owner: host, userInfo: nil()];
    let _: () = msg_send![host, addTrackingArea: ta];
    autorel(ta);
    let root = layer_of(host);
    let scale: f64 = msg_send![ui.win.get(), backingScaleFactor];
    let full = rect(0., 0., w, bh);

    // Art: two layers, so one can fade in over the other. Then the scrims that
    // blend the art into the canvas on the left and at the bottom.
    let mut art = [std::ptr::null_mut(); 2];
    for (k, slot) in art.iter_mut().enumerate() {
        let l = plain_layer();
        set_frame(l, full);
        let _: () = msg_send![l, setContentsGravity: ns("resizeAspectFill")];
        let _: () = msg_send![l, setContentsScale: scale];
        let _: () = msg_send![l, setMasksToBounds: Bool::YES];
        let _: () = msg_send![l, setBackgroundColor: cg(C_FILL_SEL)];
        let _: () = msg_send![l, setOpacity: if k == 0 { 1.0f32 } else { 0.0f32 }];
        let _: () = msg_send![root, addSublayer: l];
        *slot = l;
    }
    let canvas = |a: f64| cg_a(C_CANVAS, a) as *mut AnyObject;
    let side = gradient(
        &[canvas(0.92), canvas(0.55), canvas(0.)],
        NSPoint { x: 0., y: 0.5 },
        NSPoint { x: 1., y: 0.5 },
    );
    let _: () = msg_send![root, addSublayer: side];
    let bottom = gradient(&[canvas(0.), canvas(1.)], NSPoint { x: 0., y: 1. }, NSPoint { x: 0., y: 0. });
    let _: () = msg_send![root, addSublayer: bottom];

    // Title logo (aspect-fitted), then the text views.
    let logo = plain_layer();
    let _: () = msg_send![logo, setContentsGravity: ns("resizeAspect")];
    let _: () = msg_send![logo, setContentsScale: scale];
    let _: () = msg_send![root, addSublayer: logo];
    let title = label("", T_HERO, C_ON_MEDIA, ALIGN_LEFT);
    let meta = label("", T_CALLOUT, C_ON_MEDIA2, ALIGN_LEFT);
    let overview = wrapping_label("", T_BODY, C_ON_MEDIA2, ALIGN_LEFT);
    let _: () = msg_send![overview, setMaximumNumberOfLines: OVERVIEW_LINES as isize];
    add_sub(host, title);
    add_sub(host, meta);
    add_sub(host, overview);
    let play = make_pill(true, sel!(homeBillboardPlay:), ui.ctl.get());
    let info = make_pill(false, sel!(homeBillboardInfo:), ui.ctl.get());
    add_sub(host, play.view);
    add_sub(host, info.view);

    // Page dots, one per slide; hidden when there is only one.
    let mut dots = Vec::new();
    for _ in 0..ui.home.billboard.items.len() {
        let d = plain_layer();
        let _: () = msg_send![d, setCornerRadius: DOT / 2.];
        let _: () = msg_send![root, addSublayer: d];
        dots.push(d);
    }

    let b = &mut ui.home.billboard;
    b.host = host;
    b.art = art;
    b.scrim_side = side;
    b.scrim_bottom = bottom;
    b.logo = logo;
    b.title = title;
    b.meta = meta;
    b.overview = overview;
    b.play = play;
    b.info = info;
    b.dots = dots;
    b.w = w;
    b.h = bh;
    b.front = 0;
    apply_slide(b.index, false);
}

/// A pill-shaped button: filled white for the primary action, translucent otherwise.
unsafe fn make_pill(primary: bool, action: Sel, target: *mut AnyObject) -> Pill {
    let view = new_view(rect(0., 0., 100., PILL_H));
    let layer = layer_of(view);
    set_corner(layer, PILL_H / 2.);
    let fill = if primary { cg(C_ON_MEDIA) } else { cg_a(C_ON_MEDIA, 0.22) };
    let _: () = msg_send![layer, setBackgroundColor: fill];
    let text = if primary { C_CANVAS } else { C_ON_MEDIA };
    let lb = label("", T_BODY_EM, text, ALIGN_CENTER);
    add_sub(view, lb);
    let hit: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: target, action: action];
    let _: () = msg_send![hit, setBordered: Bool::NO];
    add_sub(view, hit);
    Pill { view, label: lb, hit }
}

/// Set a pill's text and size it to fit; returns its width.
unsafe fn size_pill(p: Pill, text: &str) -> f64 {
    set_text(p.label, text);
    let _: () = msg_send![p.label, sizeToFit];
    let lw = frame_of(p.label).size.w.ceil();
    let lh = frame_of(p.label).size.h;
    let w = lw + 2. * SP6;
    set_frame(p.view, rect(0., 0., w, PILL_H));
    set_frame(p.label, rect(0., (PILL_H - lh) / 2., w, lh));
    set_frame(p.hit, rect(0., 0., w, PILL_H));
    w
}

/// Place a pill's left edge at `x`, bottom at `y`.
unsafe fn place_pill(p: Pill, x: f64, y: f64) {
    let w = frame_of(p.view).size.w;
    set_frame(p.view, rect(x, y, w, PILL_H));
}

/// Position every billboard view for a `w` by `bh` area, from its bottom edge up:
/// buttons, overview, metadata, then the title or logo.
unsafe fn layout_billboard(b: &Billboard) {
    if b.host.is_null() {
        return;
    }
    let (w, bh) = (b.w, b.h);
    let tw = text_width(w);
    let full = rect(0., 0., w, bh);
    for l in b.art {
        set_frame(l, full);
    }
    set_frame(b.scrim_side, full);
    set_frame(b.scrim_bottom, rect(0., 0., w, bh * 0.55));
    let ov_h = OVERVIEW_LINE * OVERVIEW_LINES;
    let ov_y = TEXT_BOTTOM + PILL_H + SP5;
    let meta_y = ov_y + ov_h + SP3;
    let title_y = meta_y + META_H + SP3;
    if b.logo_aspect > 0. {
        let th = (bh * 0.22).min(LOGO_MAX_H);
        let lw = (th * b.logo_aspect).min(LOGO_MAX_W).min(tw);
        let lh = (lw / b.logo_aspect).min(th);
        set_frame(b.logo, rect(EDGE, title_y, lw, lh));
    } else {
        set_frame(b.logo, rect(0., 0., 0., 0.));
    }
    set_frame(b.title, rect(EDGE, title_y, tw, TITLE_H));
    set_frame(b.meta, rect(EDGE, meta_y, tw, META_H));
    set_frame(b.overview, rect(EDGE, ov_y, tw, ov_h));
    place_pill(b.play, EDGE, TEXT_BOTTOM);
    let pw = frame_of(b.play.view).size.w;
    place_pill(b.info, EDGE + pw + SP2, TEXT_BOTTOM);
    let n = b.dots.len();
    let total = if n == 0 { 0. } else { n as f64 * DOT + (n - 1) as f64 * DOT_GAP };
    for (k, d) in b.dots.iter().enumerate() {
        let x = w - EDGE - total + k as f64 * (DOT + DOT_GAP);
        set_frame(*d, rect(x, TEXT_BOTTOM + (PILL_H - DOT) / 2., DOT, DOT));
    }
}

/// Fade a view in from transparent (instant at 0, then animated to 1).
unsafe fn fade_in(v: *mut AnyObject) {
    if v.is_null() {
        return;
    }
    let _: () = msg_send![v, setAlphaValue: 0.0f64];
    anim_begin(M_STD, timing_ease_out());
    let anim: *mut AnyObject = msg_send![v, animator];
    let _: () = msg_send![anim, setAlphaValue: 1.0f64];
    anim_end();
}

/// Put `img` (or nothing, leaving the placeholder fill) on an art layer.
unsafe fn set_art_layer(layer: *mut AnyObject, img: Option<*mut AnyObject>) {
    let cg_img = img.map(|i| cg_image(i) as *mut AnyObject).filter(|p| !p.is_null()).unwrap_or(std::ptr::null_mut());
    let _: () = msg_send![layer, setContents: cg_img];
}

/// Show slide `i`. With `animate`, the art crossfades and the text fades in.
unsafe fn apply_slide(i: usize, animate: bool) {
    let ui = UI.get_mut().unwrap();
    let Some(it) = ui.home.billboard.items.get(i).cloned() else { return };
    let animate = animate && !reduce_motion();
    let art_img = ui.poster_cache.get(&billboard_key(&it.id)).copied();
    let logo_img = ui.poster_cache.get(&logo_key(&it.id)).copied();
    let b = &mut ui.home.billboard;
    b.index = i;
    if b.host.is_null() {
        return;
    }
    // Art: the back layer takes the new slide and fades in over the front one.
    let back = if animate { 1 - b.front } else { 0 };
    tx_begin_instant();
    set_art_layer(b.art[back], art_img);
    let _: () = msg_send![b.art[back], setOpacity: if animate { 0.0f32 } else { 1.0f32 }];
    let _: () = msg_send![b.art[back], setZPosition: 1.0f64];
    let _: () = msg_send![b.art[1 - back], setZPosition: 0.0f64];
    if !animate {
        let _: () = msg_send![b.art[1 - back], setOpacity: 0.0f32];
    }
    tx_end();
    if animate {
        tx_begin(M_EMPH);
        let _: () = msg_send![b.art[back], setOpacity: 1.0f32];
        tx_end();
    }
    b.front = back;

    // Text and logo.
    b.logo_aspect = logo_img
        .map(|img| {
            let s: NSSize = msg_send![img, size];
            if s.h > 0. { s.w / s.h } else { 0. }
        })
        .unwrap_or(0.);
    tx_begin_instant();
    set_art_layer(b.logo, logo_img);
    tx_end();
    set_text(b.title, &billboard_title(&it));
    set_hidden(b.title, b.logo_aspect > 0.);
    set_text(b.meta, &billboard_meta(&it));
    set_text(b.overview, it.overview.as_deref().unwrap_or(""));
    size_pill(b.play, &play_label(&it));
    size_pill(b.info, "More Info");
    for (k, d) in b.dots.iter().enumerate() {
        tx_begin_instant();
        let _: () = msg_send![*d, setBackgroundColor: cg_a(C_ON_MEDIA, if k == i { 1.0 } else { 0.35 })];
        tx_end();
    }
    layout_billboard(b);
    if animate {
        for v in [b.title, b.meta, b.overview, b.play.view, b.info.view] {
            fade_in(v);
        }
    }
}

/// Art or logo for `key` arrived: show it if it belongs to the slide on screen.
pub fn billboard_ready(key: u64) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let b = &ui.home.billboard;
        let Some(it) = b.items.get(b.index).cloned() else { return };
        if b.host.is_null() {
            return;
        }
        if key == billboard_key(&it.id) {
            let img = ui.poster_cache.get(&key).copied();
            tx_begin_instant();
            set_art_layer(b.art[b.front], img);
            tx_end();
        } else if key == logo_key(&it.id) {
            let index = b.index;
            apply_slide(index, false);
        }
    }
}

/// Whether the billboard's slides are on screen (keys work even while hovered).
fn billboard_showing(ui: &crate::platform::appkit::Ui) -> bool {
    billboard_wanted(ui) && !ui.home.billboard.host.is_null()
}

/// Whether the timer should be running: slides on screen, more than one, and the
/// pointer is not over the billboard.
fn billboard_timing(ui: &crate::platform::appkit::Ui) -> bool {
    billboard_showing(ui) && ui.home.billboard.items.len() > 1 && !ui.home.billboard.hovered
}

/// Restart the slide timer from zero (or stop it) to match the screen.
pub(crate) unsafe fn billboard_timer_sync() {
    let ui = UI.get_mut().unwrap();
    let ctl = ui.ctl.get();
    let _: () = msg_send![class!(NSObject), cancelPreviousPerformRequestsWithTarget: ctl,
        selector: sel!(homeBillboardTick:), object: nil()];
    if billboard_timing(ui) {
        let _: () = msg_send![ctl, performSelector: sel!(homeBillboardTick:), withObject: nil(), afterDelay: BILLBOARD_SECS];
    }
}

/// The timer fired: move to the next slide and wait again.
pub(crate) extern "C" fn billboard_tick(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if billboard_timing(ui) {
            let n = ui.home.billboard.items.len();
            let next = (ui.home.billboard.index + 1) % n;
            apply_slide(next, true);
        }
        billboard_timer_sync();
    }
}

/// The pointer entered or left the billboard: pause or resume the timer.
pub(crate) unsafe fn billboard_hover(on: bool) {
    let ui = UI.get_mut().unwrap();
    ui.home.billboard.hovered = on;
    billboard_timer_sync();
}

/// Step to the previous or next slide (← and → with ⌥⌘, or the page dots' order).
pub(crate) unsafe fn billboard_step(delta: isize) {
    let ui = UI.get_mut().unwrap();
    if !billboard_showing(ui) {
        return;
    }
    let n = ui.home.billboard.items.len() as isize;
    if n < 2 {
        return;
    }
    let next = (ui.home.billboard.index as isize + delta).rem_euclid(n) as usize;
    apply_slide(next, true);
    billboard_timer_sync();
}

/// Menu and key entry for `billboard_step`: tag 0 is previous, 1 is next.
pub(crate) extern "C" fn billboard_step_menu(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        billboard_step(if tag == 0 { -1 } else { 1 });
    }
}

/// Play the billboard's current slide (a series opens its page).
pub(crate) extern "C" fn billboard_play(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(it) = ui.home.billboard.items.get(ui.home.billboard.index).cloned() {
            play_item(&it);
        }
    }
}

/// More Info on the billboard's current slide: its detail page.
pub(crate) extern "C" fn billboard_info(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    if let Some(it) = ui.home.billboard.items.get(ui.home.billboard.index).cloned() {
        crate::platform::appkit::set_busy(true);
        let _ = ui.ui_tx.send(UiCmd::OpenDetail { item_id: it.id.clone() });
    }
}

/// The Play button on a card: play it, or open a series.
pub(crate) extern "C" fn card_play(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        if let Some(it) = ui.home.flat.get(tag as usize).cloned() {
            play_item(&it);
        }
    }
}

/// Play an item from Home: movies and episodes play; a series opens its page.
unsafe fn play_item(it: &ListItem) {
    let ui = UI.get_mut().unwrap();
    crate::platform::appkit::set_busy(true);
    let cmd = if it.item_type == "Series" {
        UiCmd::OpenDetail { item_id: it.id.clone() }
    } else {
        UiCmd::Play { item_id: it.id.clone() }
    };
    let _ = ui.ui_tx.send(cmd);
}

/// Put a wide card's caption in place: series or movie title, then the subtitle.
unsafe fn set_wide_caption(c: &CellHandle, it: &ListItem) {
    let (title, sub) = wide_caption(it);
    set_text(c.title, &title);
    set_text(c.sub, &sub);
}

/// Row heading: the title on the left, a "See all ›" button on the right (its tag is
/// the row's index). Returns the title and the button, for width changes.
unsafe fn row_head(doc: *mut AnyObject, index: usize, title: &str, w: f64, y: f64) -> (*mut AnyObject, *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    let btn: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns("See all ›"), target: ui.ctl.get(), action: sel!(homeSeeAll:)];
    let _: () = msg_send![btn, setBordered: Bool::NO];
    let _: () = msg_send![btn, setFont: font(T_BODY_EM)];
    let _: () = msg_send![btn, setTag: index as NSInteger];
    let _: () = msg_send![btn, sizeToFit];
    let bf = frame_of(btn);
    let (bw, bh) = (bf.size.w.ceil(), bf.size.h);
    set_frame(btn, rect(w - EDGE - bw, y + (ROW_TITLE_H - bh) / 2., bw, bh));
    add_sub(doc, btn);
    let t = label(title, T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(t, rect(EDGE, y, w - 2. * EDGE - bw - SP4, ROW_TITLE_H));
    add_sub(doc, t);
    (t, btn)
}

/// Lay out one "See all" page: a back button, the row's title, then its grid.
/// Returns the grid's bottom edge.
unsafe fn build_see_all(doc: *mut AnyObject, title: &str, w: f64, scale: f64) -> f64 {
    let ui = UI.get_mut().unwrap();
    let back: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns("‹ Home"), target: ui.ctl.get(), action: sel!(homeSeeAllBack:)];
    let _: () = msg_send![back, setBordered: Bool::NO];
    let _: () = msg_send![back, setFont: font(T_BODY_EM)];
    let _: () = msg_send![back, sizeToFit];
    let bh = frame_of(back).size.h;
    set_frame(back, rect(EDGE - SP2, SP4, frame_of(back).size.w.ceil(), bh));
    add_sub(doc, back);
    let head_y = SP4 + bh + SP2;
    let head = label(title, T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, head_y, w - 2. * EDGE, ROW_TITLE_H));
    add_sub(doc, head);
    let top = head_y + ROW_TITLE_H + SP4;
    let Some(row) = ui.home.rows.iter().find(|r| r.title == title).cloned() else { return top };
    if row.items.is_empty() {
        return top;
    }
    let wide = row.kind.is_wide();
    let ratio = if wide { WIDE_RATIO } else { POSTER_ASPECT };
    let m = grid_metrics(w, ratio);
    let mut cells = Vec::with_capacity(row.items.len());
    tx_begin_instant();
    for (i, it) in row.items.iter().enumerate() {
        let tag = ui.home.flat.len();
        ui.home.flat.push(it.clone());
        let key = if wide { wide_key(&it.id) } else { poster_key(&it.id) };
        let c = build_cell_keyed(doc, it, tag as NSInteger, sel!(homeItemClicked:), scale, &m, key);
        let col = i % m.cols;
        let r = i / m.cols;
        set_frame(c.container, rect(EDGE + col as f64 * (m.tile_w + GRID_COL_GAP), top + r as f64 * (m.cell_h + GRID_ROW_GAP), m.tile_w, m.cell_h));
        layout_cell(&c, &m);
        if wide {
            set_wide_caption(&c, it);
        }
        cells.push(c);
    }
    tx_end();
    ui.home.views.push(HomeRowView { cells });
    let rows = (row.items.len() + m.cols - 1) / m.cols;
    top + rows as f64 * (m.cell_h + GRID_ROW_GAP) + SP2
}

/// Replace the Home area with the rows (or the open "See all" page) for the tab on screen.
unsafe fn rebuild() {
    rebuild_drop_cells();
    let ui = UI.get_mut().unwrap();
    let doc = ui.home.doc.get();
    let none: *mut AnyObject = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];
    ui.home.heads.clear();
    ui.home.grid_top = 0.;

    let w = bounds_of(ui.home.scroll.get()).size.w.max(200.);
    ui.home.laid_w = w;
    let scale: f64 = msg_send![ui.win.get(), backingScaleFactor];
    let bh = if billboard_wanted(ui) { billboard_height_for(bounds_of(ui.home.scroll.get()).size.h) } else { 0. };
    ui.home.billboard_h = bh;
    build_billboard(doc, w, bh);

    let mut y = if bh > 0. { bh + SP6 } else { SP2 };
    if let Some(title) = ui.home.see_all.clone() {
        y = build_see_all(doc, &title, w, scale);
        let _: () = msg_send![doc, setFrameSize: size(w, y)];
        return;
    }
    tx_begin_instant();
    for (ri, r) in ui.home.rows.iter().enumerate() {
        let wide = r.kind.is_wide();
        let (tile_w, ratio) = if wide { (WIDE_W, WIDE_RATIO) } else { (POSTER_W, POSTER_ASPECT) };
        let poster_h = (tile_w * ratio).round();
        let m = Metrics { cols: 1, tile_w, poster_h, cell_h: poster_h + CELL_TEXT_H };

        let head = row_head(doc, ri, &r.title, w, y);
        ui.home.heads.push(head);

        let strip_y = y + ROW_TITLE_H + SP2;
        let strip_h = m.cell_h + 2. * STRIP_PAD;
        let n = r.items.len() as f64;
        let cards_w = 2. * EDGE + n * tile_w + (n - 1.).max(0.) * CARD_GAP;
        let strip: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
        let strip: *mut AnyObject = msg_send![strip, initWithFrame: rect(0., strip_y, w, strip_h)];
        let _: () = msg_send![strip, setHasHorizontalScroller: Bool::YES];
        let _: () = msg_send![strip, setHasVerticalScroller: Bool::NO];
        let _: () = msg_send![strip, setDrawsBackground: Bool::NO];
        let _: () = msg_send![strip, setBorderType: 0usize];
        set_autoresize(strip, RESIZE_W);
        add_sub(doc, strip);
        autorel(strip);
        let cards = autorel(flip_view(rect(0., 0., cards_w, strip_h)));
        let _: () = msg_send![strip, setDocumentView: cards];

        let mut cells = Vec::with_capacity(r.items.len());
        for (i, it) in r.items.iter().enumerate() {
            let tag = ui.home.flat.len();
            ui.home.flat.push(it.clone());
            let key = if wide { wide_key(&it.id) } else { poster_key(&it.id) };
            let c = build_cell_keyed(cards, it, tag as NSInteger, sel!(homeItemClicked:), scale, &m, key);
            set_frame(c.container, rect(EDGE + i as f64 * (tile_w + CARD_GAP), STRIP_PAD, tile_w, m.cell_h));
            layout_cell(&c, &m);
            if wide {
                set_wide_caption(&c, it);
            }
            cells.push(c);
        }
        ui.home.views.push(HomeRowView { cells });
        y = strip_y + strip_h + ROW_GAP;
    }
    tx_end();

    if !ui.home.grid.is_empty() {
        let heading = label(
            if ui.home.section == HomeSection::Movies { "All Movies" } else { "All TV Shows" },
            T_TITLE,
            C_TEXT,
            ALIGN_LEFT,
        );
        set_frame(heading, rect(EDGE, y, w - 2. * EDGE, ROW_TITLE_H));
        add_sub(doc, heading);
        y += ROW_TITLE_H + SP2;
        ui.home.grid_top = y;
        let m = grid_metrics(w, POSTER_ASPECT);
        let grid = ui.home.grid.clone();
        let mut cells = Vec::with_capacity(grid.len());
        tx_begin_instant();
        for it in grid.iter() {
            let tag = ui.home.flat.len();
            ui.home.flat.push(it.clone());
            cells.push(build_cell_keyed(doc, it, tag as NSInteger, sel!(homeItemClicked:), scale, &m, poster_key(&it.id)));
        }
        tx_end();
        ui.home.views.push(HomeRowView { cells });
        y = place_grid(w);
    }
    let _: () = msg_send![doc, setFrameSize: size(w, y)];
}

/// Re-lay Home for document width `w` and the window's height, without rebuilding
/// the cells: the billboard takes its new height and the rows below move by the
/// difference. A "See all" page, or a billboard that appears or goes, is rebuilt.
pub(crate) unsafe fn relayout(w: f64) {
    let ui = UI.get_mut().unwrap();
    if !ui.home.active || ui.home.doc.get().is_null() {
        return;
    }
    let want = billboard_wanted(ui);
    let built = !ui.home.billboard.host.is_null();
    if ui.home.see_all.is_some() || want != built {
        rebuild();
        return;
    }
    ui.home.laid_w = w;
    let doc = ui.home.doc.get();
    let host = ui.home.billboard.host;
    let new_bh = if want { billboard_height_for(bounds_of(ui.home.scroll.get()).size.h) } else { 0. };
    let delta = new_bh - ui.home.billboard_h;
    if delta.abs() > 0.5 {
        let subs: *mut AnyObject = msg_send![doc, subviews];
        let n: usize = msg_send![subs, count];
        for i in 0..n {
            let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
            if v == host {
                continue;
            }
            let f = frame_of(v);
            set_frame(v, rect(f.origin.x, f.origin.y + delta, f.size.w, f.size.h));
        }
        ui.home.grid_top += delta;
        ui.home.billboard_h = new_bh;
    }
    if want {
        set_frame(host, rect(0., 0., w, new_bh));
        ui.home.billboard.w = w;
        ui.home.billboard.h = new_bh;
        layout_billboard(&ui.home.billboard);
    }
    for (t, b) in ui.home.heads.clone() {
        let y = frame_of(t).origin.y;
        let bf = frame_of(b);
        set_frame(b, rect(w - EDGE - bf.size.w, bf.origin.y, bf.size.w, bf.size.h));
        set_frame(t, rect(EDGE, y, w - 2. * EDGE - bf.size.w - SP4, ROW_TITLE_H));
    }
    if !ui.home.grid.is_empty() {
        place_grid(w);
    }
    let subs: *mut AnyObject = msg_send![doc, subviews];
    let n: usize = msg_send![subs, count];
    let mut bottom: f64 = 0.;
    for i in 0..n {
        let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let f = frame_of(v);
        bottom = bottom.max(f.origin.y + f.size.h);
    }
    let _: () = msg_send![doc, setFrameSize: size(w, bottom + SP2)];
}

/// Empty, loading and "nothing yet" messages for the Home area.
unsafe fn update_state() {
    let ui = UI.get_mut().unwrap();
    if !ui.home.active {
        return;
    }
    if !ui.home.rows.is_empty() || !ui.home.grid.is_empty() {
        hide_state();
    } else if ui.home.loading {
        match ui.home.section {
            HomeSection::Home => show_state("house", "Loading Home…", "Fetching your libraries.", false),
            HomeSection::Movies => show_state("film", "Loading Movies…", "Fetching your libraries.", false),
            HomeSection::TvShows => show_state("tv", "Loading TV Shows…", "Fetching your libraries.", false),
        }
    } else {
        // Every empty state offers Retry (the state's "Retry" button).
        match ui.home.section {
            HomeSection::Home => show_state("house", "Nothing here yet", "Movies and shows you add to Jellyfin will appear here.", true),
            HomeSection::Movies => show_state("film", "No movies yet", "Movies you add to Jellyfin will appear here.", true),
            HomeSection::TvShows => show_state("tv", "No TV shows yet", "Shows you add to Jellyfin will appear here.", true),
        }
    }
}

/// Ask the worker for fresh content for the tab on screen.
pub(crate) unsafe fn reload() {
    let ui = UI.get_mut().unwrap();
    ui.home.loading = true;
    let cmd = match ui.home.section {
        HomeSection::Home => UiCmd::LoadHome,
        section => UiCmd::LoadHomeSection { section },
    };
    let _ = ui.ui_tx.send(cmd);
    update_state();
}

/// Scroll the Home area back to its top.
unsafe fn scroll_top() {
    let ui = UI.get_mut().unwrap();
    let sc = ui.home.scroll.get();
    let clip: *mut AnyObject = msg_send![sc, contentView];
    let _: () = msg_send![clip, scrollToPoint: NSPoint { x: 0., y: 0. }];
    let _: () = msg_send![sc, reflectScrolledClipView: clip];
}

/// Switch the tab on screen: drop the old tab's content, then ask for the new one.
unsafe fn set_section(section: HomeSection) {
    let ui = UI.get_mut().unwrap();
    ui.home.section = section;
    ui.home.rows.clear();
    ui.home.grid.clear();
    ui.home.billboard.items.clear();
    ui.home.see_all = None;
    rebuild();
    ui.cur_title = section_title(section).to_string();
    set_text(ui.header_title.get(), header_text(section));
    header_layout();
}

/// Show or hide the segmented control, and make it show the current tab.
pub(crate) unsafe fn update_segment() {
    let ui = UI.get_mut().unwrap();
    let seg = ui.home.seg.get();
    if seg.is_null() {
        return;
    }
    let show = ui.home.active && ui.cfg.layout_mode == Some(LayoutMode::Unified);
    set_hidden(seg, !show);
    let _: () = msg_send![seg, setSelectedSegment: index_for_section(ui.home.section)];
}

/// Show Home and remember it as the last selected place.
pub(crate) unsafe fn enter() {
    crate::ui::seerr::leave();
    let section = current_section();
    let ui = UI.get_mut().unwrap();
    ui.home.active = true;
    ui.selected_view = None;
    ui.browse_active = false;
    ui.cfg.last_view = Some(HOME_ID.to_string());
    crate::config::save_config(&ui.cfg);
    set_hidden(ui.lib_scroll.get(), true);
    set_hidden(ui.sort_popup.get(), true);
    set_hidden(ui.lib_back.get(), true);
    set_hidden(ui.home.scroll.get(), false);
    crate::platform::appkit::set_busy(false);
    if ui.home.section != section {
        set_section(section);
    }
    let ui = UI.get_mut().unwrap();
    ui.cur_title = section_title(section).to_string();
    set_text(ui.header_title.get(), header_text(section));
    set_count("", false);
    header_layout();
    update_segment();
    sidebar_refresh_selection();
    reload();
    billboard_timer_sync();
}

/// Pick a tab of the one-big-library Home (⌥⌘0/M/T, or the header control).
/// Remembered in config. From another screen this also opens Home.
pub(crate) unsafe fn select_section(section: HomeSection) {
    let ui = UI.get_mut().unwrap();
    if ui.cfg.layout_mode != Some(LayoutMode::Unified) {
        return;
    }
    ui.cfg.home_section = Some(section);
    crate::config::save_config(&ui.cfg);
    if !ui.home.active {
        enter();
        return;
    }
    if ui.home.section == section {
        update_segment();
        return;
    }
    set_section(section);
    update_segment();
    scroll_top();
    reload();
}

/// The layout changed while Home may be showing (Settings or onboarding): show the
/// tab the new layout wants, and hide the control if the layout no longer has tabs.
pub(crate) unsafe fn layout_changed() {
    let ui = UI.get_mut().unwrap();
    if !ui.home.active {
        return;
    }
    let section = current_section();
    if ui.home.section != section {
        set_section(section);
        reload();
    }
    update_segment();
}

/// Leave Home for a library screen. A no-op unless Home is showing.
pub(crate) unsafe fn leave() {
    let ui = UI.get_mut().unwrap();
    if !ui.home.active {
        return;
    }
    ui.home.active = false;
    ui.home.see_all = None;
    billboard_timer_sync();
    set_hidden(ui.home.scroll.get(), true);
    set_hidden(ui.lib_scroll.get(), false);
    set_hidden(ui.sort_popup.get(), false);
    hide_state();
    update_segment();
    let name = ui.mode_stack.last().map(|m| m.1.clone()).unwrap_or_default();
    set_text(ui.header_title.get(), &name);
    ui.cur_title = name;
    header_layout();
    sidebar_refresh_selection();
}

/// Unified layout: the "Browse" grid, every movie and show in one list. Shows
/// the skeleton until `ItemsLoaded` arrives, like a library does.
pub(crate) unsafe fn enter_browse() {
    let ui = UI.get_mut().unwrap();
    ui.browse_active = true;
    ui.selected_view = None;
    ui.mode_stack.clear();
    ui.cfg.last_view = Some(BROWSE_ID.to_string());
    crate::config::save_config(&ui.cfg);
    crate::ui::shell::begin_load(true);
    ui.cur_title = "Browse".to_string();
    set_text(ui.header_title.get(), "Browse");
    header_layout();
    sidebar_refresh_selection();
    let _ = ui.ui_tx.send(UiCmd::LoadBrowse);
}

pub(crate) extern "C" fn home_selected(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { enter() }
}

pub(crate) extern "C" fn browse_selected(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe { enter_browse() }
}

/// The header control was clicked.
pub(crate) extern "C" fn segment_changed(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let i: NSInteger = msg_send![sender, selectedSegment];
        select_section(section_for_index(i as usize));
    }
}

/// Go > Home / Movies / TV Shows (the tag is the tab's index).
pub(crate) extern "C" fn menu_section(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        select_section(section_for_index(tag as usize));
    }
}

/// "See all ›" on a row: open that row's grid (the tag is the row's index).
pub(crate) extern "C" fn see_all(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        let ui = UI.get_mut().unwrap();
        let Some(title) = ui.home.rows.get(tag as usize).map(|r| r.title.clone()) else { return };
        ui.home.see_all = Some(title);
        scroll_top();
        rebuild();
        update_state();
        billboard_timer_sync();
    }
}

/// "‹ Home" on a "See all" page: back to the rows.
pub(crate) extern "C" fn see_all_back(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        ui.home.see_all = None;
        scroll_top();
        rebuild();
        update_state();
        billboard_timer_sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jellyfin::models::UserData;

    /// Ticks for `m` minutes.
    fn mins(m: i64) -> i64 {
        m * 600_000_000
    }

    fn item(ty: &str, name: &str) -> ListItem {
        serde_json::from_value(serde_json::json!({"Id": name, "Name": name, "Type": ty})).unwrap()
    }

    fn watched(mut it: ListItem, pos_min: i64, run_min: i64) -> ListItem {
        it.run_time_ticks = Some(mins(run_min));
        it.user_data = Some(UserData { position_ticks: mins(pos_min), ..Default::default() });
        it
    }

    #[test]
    fn time_left_rounds_up_and_needs_progress() {
        assert_eq!(time_left(Some(mins(100)), mins(58)), Some("42m left".into()));
        assert_eq!(time_left(Some(mins(200)), mins(60)), Some("2h 20m left".into()));
        assert_eq!(time_left(Some(mins(10)), mins(9) + 1), Some("1m left".into()));
        assert_eq!(time_left(Some(mins(10)), 0), None, "nothing watched");
        assert_eq!(time_left(Some(mins(10)), mins(10)), None, "finished");
        assert_eq!(time_left(None, mins(5)), None, "no runtime");
    }

    #[test]
    fn play_label_names_resume_episode_or_play() {
        let mut ep = watched(item("Episode", "Pilot"), 20, 40);
        ep.season = Some(2);
        ep.episode = Some(4);
        assert_eq!(play_label(&ep), "Resume · 20m left");
        let mut fresh = item("Episode", "Pilot");
        fresh.season = Some(2);
        fresh.episode = Some(4);
        assert_eq!(play_label(&fresh), "Play S2:E4");
        assert_eq!(play_label(&item("Movie", "Heat")), "Play");
        assert_eq!(play_label(&item("Series", "Show")), "Open");
    }

    #[test]
    fn billboard_meta_reads_year_rating_runtime_genres() {
        let mut m = item("Movie", "Heat");
        m.year = Some(1995);
        m.official_rating = Some("R".into());
        m.community_rating = Some(8.3);
        m.run_time_ticks = Some(mins(170));
        m.genres = vec!["Crime".into(), "Drama".into(), "Thriller".into()];
        assert_eq!(billboard_meta(&m), "1995 · R · ★ 8.3 · 2h 50m · Crime, Drama");
        let mut ep = item("Episode", "Pilot");
        ep.season = Some(1);
        ep.episode = Some(3);
        ep.series_name = Some("Show".into());
        assert_eq!(billboard_meta(&ep), "S1:E3");
        assert_eq!(billboard_title(&ep), "Show");
    }

    #[test]
    fn wide_caption_puts_episode_code_and_time_left() {
        let mut ep = watched(item("Episode", "Pilot"), 10, 30);
        ep.season = Some(1);
        ep.episode = Some(3);
        ep.series_name = Some("Show".into());
        assert_eq!(wide_caption(&ep), ("Show".into(), "S1:E3 · Pilot · 20m left".into()));
        let mut next = item("Episode", "Pilot");
        next.season = Some(1);
        next.episode = Some(3);
        assert_eq!(wide_caption(&next).1, "S1:E3 · Pilot");
        assert_eq!(wide_caption(&watched(item("Movie", "Heat"), 60, 170)), ("Heat".into(), "1h 50m left".into()));
    }

    #[test]
    fn billboard_height_is_a_share_with_a_floor() {
        assert_eq!(billboard_height_for(600.), 360.);
        assert_eq!(billboard_height_for(1000.), 550.);
    }
}
