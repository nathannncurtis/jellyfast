//! Home screen: a vertical scroll of horizontal rows (Continue Watching, Next Up,
//! Recently Added, recommendations). Cards are the library grid's own cells, so
//! hover lift, progress and posters behave the same. Selecting Home swaps the
//! library grid for this scroll view; any library load swaps it back.
//!
//! One big library adds a segmented control in the header (Home | Movies | TV
//! Shows). The Movies and TV Shows tabs reuse this scroll view: their rows, then a
//! full poster grid of every movie or show in the visible libraries.
#![allow(unsafe_code)]
use crate::app::messages::{HomeRow, HomeRows, UiCmd};
use crate::config::{HomeSection, LayoutMode};
use crate::jellyfin::models::ListItem;
use crate::platform::appkit::*;
use crate::ui::library::{build_cell_keyed, free_cell_data, grid_metrics, layout_cell, poster_key, Metrics};
use crate::ui::shell::{header_layout, hide_state, set_count, show_state, sidebar_refresh_selection};
use crate::ui::theme::*;
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};

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
const ROW_TITLE_H: f64 = 24.;
const ROW_GAP: f64 = SP6;

/// Segment labels, in the order `section_for_index` reads them.
const SEGMENTS: [&str; 3] = ["Home", "Movies", "TV Shows"];

/// Artwork key for a wide card. Separate from the poster key, because the same
/// item can appear as a wide card and as a poster (Thumb/Backdrop vs Primary).
pub fn wide_key(item_id: &str) -> u64 {
    poster_key(&format!("wide:{item_id}"))
}

/// The cells of one laid-out row (kept so poster arrivals can update them).
pub struct HomeRowView {
    pub cells: Vec<CellHandle>,
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
        }
    }
}

/// A flipped plain view (`JRFlipView`, registered by the Seerr module): rows and
/// cards are laid out top-down.
unsafe fn flip_view(frame: NSRect) -> *mut Object {
    let cls = Class::get("JRFlipView").expect("JRFlipView registered at startup");
    let v: *mut Object = msg_send![cls, alloc];
    msg_send![v, initWithFrame: frame] // +1; the caller autorels after adding
}

extern "C" fn flipped(_this: &mut Object, _c: Sel) -> BOOL {
    YES
}

/// Home's document view: flipped, and it re-places the grid when its width changes
/// (the grid has as many columns as the width allows).
extern "C" fn home_doc_set_frame_size(this: &mut Object, _c: Sel, s: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        if this as *mut Object != ui.home.doc.get() || ui.home.grid.is_empty() || (s.w - ui.home.laid_w).abs() < 0.5 {
            return;
        }
        let bottom = place_grid(s.w);
        let _: () = msg_send![this as *mut Object, setFrameSize: size(s.w, bottom)];
    }
}

/// Register `JRHomeDoc` once (the document view class with the grid re-layout).
fn register_doc_class() {
    if Class::get("JRHomeDoc").is_some() {
        return;
    }
    let mut d = ClassDecl::new("JRHomeDoc", class!(NSView)).expect("JRHomeDoc is new");
    unsafe {
        d.add_method(sel!(isFlipped), flipped as extern "C" fn(&mut Object, Sel) -> BOOL);
        d.add_method(
            sel!(setFrameSize:),
            home_doc_set_frame_size as extern "C" fn(&mut Object, Sel, NSSize),
        );
    }
    d.register();
}

/// The segmented Home | Movies | TV Shows control, at the header's right edge.
/// Hidden until Home is showing in the one-big-library layout.
unsafe fn build_segment(header: *mut Object, target: *mut Object) -> *mut Object {
    let labels: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in SEGMENTS {
        let _: () = msg_send![labels, addObject: ns(t)];
    }
    // Same control and size as the Seerr tabs.
    let seg: *mut Object = msg_send![class!(NSSegmentedControl),
        segmentedControlWithLabels: labels trackingMode: 0i64
        target: target action: sel!(homeSegment:)];
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
pub(crate) unsafe fn build_scroll(content_host: *mut Object, frame: NSRect) {
    let ui = UI.get_mut().unwrap();
    let sc: *mut Object = msg_send![class!(NSScrollView), alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: frame];
    let _: () = msg_send![sc, setHasVerticalScroller: YES];
    let _: () = msg_send![sc, setDrawsBackground: YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    set_hidden(sc, true);
    add_sub(content_host, sc);
    autorel(sc);
    register_doc_class();
    let doc: *mut Object = msg_send![class!(JRHomeDoc), alloc];
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
    ui.home.rows = rows;
    ui.home.grid = grid;
    ui.home.loading = false;
    rebuild();
    update_state();
}

/// Lay the grid's cells out for document width `w`. Returns the grid's bottom edge.
unsafe fn place_grid(w: f64) -> f64 {
    let ui = UI.get_mut().unwrap();
    ui.home.laid_w = w;
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

unsafe fn rebuild() {
    rebuild_drop_cells();
    let ui = UI.get_mut().unwrap();
    let doc = ui.home.doc.get();
    let none: *mut Object = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];

    let w = bounds_of(ui.home.scroll.get()).size.w.max(200.);
    let scale: f64 = msg_send![ui.win.get(), backingScaleFactor];
    tx_begin_instant();
    let mut y = SP2;
    for r in ui.home.rows.iter() {
        let wide = r.kind.is_wide();
        let (tile_w, ratio) = if wide { (WIDE_W, WIDE_RATIO) } else { (POSTER_W, POSTER_ASPECT) };
        let poster_h = (tile_w * ratio).round();
        let m = Metrics { cols: 1, tile_w, poster_h, cell_h: poster_h + CELL_TEXT_H };

        let title = label(&r.title, T_TITLE, C_TEXT, ALIGN_LEFT);
        set_frame(title, rect(EDGE, y, w - 2. * EDGE, ROW_TITLE_H));
        set_autoresize(title, RESIZE_W);
        add_sub(doc, title);

        let strip_y = y + ROW_TITLE_H + SP2;
        let strip_h = m.cell_h + 2. * STRIP_PAD;
        let n = r.items.len() as f64;
        let cards_w = 2. * EDGE + n * tile_w + (n - 1.).max(0.) * CARD_GAP;
        let strip: *mut Object = msg_send![class!(NSScrollView), alloc];
        let strip: *mut Object = msg_send![strip, initWithFrame: rect(0., strip_y, w, strip_h)];
        let _: () = msg_send![strip, setHasHorizontalScroller: YES];
        let _: () = msg_send![strip, setHasVerticalScroller: NO];
        let _: () = msg_send![strip, setDrawsBackground: NO];
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
        set_autoresize(heading, RESIZE_W);
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
        match ui.home.section {
            HomeSection::Home => {
                show_state("house", "Nothing here yet", "Movies and shows you add to Jellyfin will appear here.", false)
            }
            HomeSection::Movies => show_state("film", "No movies yet", "Movies you add to Jellyfin will appear here.", false),
            HomeSection::TvShows => show_state("tv", "No TV shows yet", "Shows you add to Jellyfin will appear here.", false),
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

/// Switch the tab on screen: drop the old tab's content, then ask for the new one.
unsafe fn set_section(section: HomeSection) {
    let ui = UI.get_mut().unwrap();
    ui.home.section = section;
    ui.home.rows.clear();
    ui.home.grid.clear();
    rebuild();
    let title = section_title(section);
    ui.cur_title = title.to_string();
    set_text(ui.header_title.get(), title);
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
    set_text(ui.header_title.get(), section_title(section));
    set_count("", false);
    header_layout();
    update_segment();
    sidebar_refresh_selection();
    reload();
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
    let ui = UI.get_mut().unwrap();
    let sc = ui.home.scroll.get();
    let clip: *mut Object = msg_send![sc, contentView];
    let _: () = msg_send![clip, scrollToPoint: NSPoint { x: 0., y: 0. }];
    let _: () = msg_send![sc, reflectScrolledClipView: clip];
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

pub(crate) extern "C" fn home_selected(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { enter() }
}

pub(crate) extern "C" fn browse_selected(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { enter_browse() }
}

/// The header control was clicked.
pub(crate) extern "C" fn segment_changed(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let i: NSInteger = msg_send![sender, selectedSegment];
        select_section(section_for_index(i as usize));
    }
}

/// Go > Home / Movies / TV Shows (the tag is the tab's index).
pub(crate) extern "C" fn menu_section(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        select_section(section_for_index(tag as usize));
    }
}
