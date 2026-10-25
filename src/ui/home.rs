//! Home screen: a vertical scroll of horizontal rows (Continue Watching, Next Up,
//! Recently Added per library, recommendations). Cards are the library grid's own
//! cells, so hover lift, progress and posters behave the same. Selecting Home swaps
//! the library grid for this scroll view; any library load swaps it back.
#![allow(unsafe_code)]
use crate::app::messages::{HomeRow, UiCmd};
use crate::jellyfin::models::ListItem;
use crate::platform::appkit::*;
use crate::ui::library::{build_cell_keyed, free_cell_data, layout_cell, poster_key, Metrics};
use crate::ui::shell::{header_layout, hide_state, set_count, show_state, sidebar_refresh_selection};
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Class, Object, Sel, NO, YES};
use objc::{sel, sel_impl};

/// `cfg.last_view` value meaning "Home was the last selected place".
pub const HOME_ID: &str = "home";

/// Poster cards (2:3) and wide cards (16:9, Continue Watching and Next Up).
pub const POSTER_W: f64 = 176.;
pub const WIDE_W: f64 = 304.;
const WIDE_RATIO: f64 = 9. / 16.;
const CARD_GAP: f64 = SP4;
/// Room around a strip so a card's hover lift is not clipped.
const STRIP_PAD: f64 = SP3;
const ROW_TITLE_H: f64 = 24.;
const ROW_GAP: f64 = SP6;

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
    /// The Home rows (single source of truth).
    pub rows: Vec<HomeRow>,
    pub views: Vec<HomeRowView>,
    /// Every card's item; a card's button tag is its index here.
    pub flat: Vec<ListItem>,
    /// Home is the visible screen.
    pub active: bool,
    /// A `LoadHome` is in flight.
    pub loading: bool,
}

impl HomeUi {
    pub fn new() -> HomeUi {
        HomeUi {
            scroll: Ptr(std::ptr::null_mut()),
            doc: Ptr(std::ptr::null_mut()),
            row: Ptr(std::ptr::null_mut()),
            rows: Vec::new(),
            views: Vec::new(),
            flat: Vec::new(),
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
    let doc = autorel(flip_view(rect(0., 0., frame.size.w, 10.)));
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    ui.home.scroll = Ptr(sc);
    ui.home.doc = Ptr(doc);
}

/// Replace the Home rows with `rows` (display order; empty rows never arrive).
pub(crate) unsafe fn set_rows(rows: Vec<HomeRow>) {
    let ui = UI.get_mut().unwrap();
    ui.home.rows = rows;
    ui.home.loading = false;
    rebuild();
    update_state();
}

unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    for row in ui.home.views.drain(..) {
        for c in &row.cells {
            free_cell_data(c.container);
        }
    }
    ui.home.flat.clear();
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
    set_frame(doc, rect(0., 0., w, y));
    tx_end();
}

/// Empty, loading and "nothing yet" messages for the Home area.
unsafe fn update_state() {
    let ui = UI.get_mut().unwrap();
    if !ui.home.active {
        return;
    }
    if !ui.home.rows.is_empty() {
        hide_state();
    } else if ui.home.loading {
        show_state("house", "Loading Home…", "Fetching your libraries.", false);
    } else {
        show_state("house", "Nothing here yet", "Movies and shows you add to Jellyfin will appear here.", false);
    }
}

/// Ask the worker for fresh rows.
pub(crate) unsafe fn reload() {
    let ui = UI.get_mut().unwrap();
    ui.home.loading = true;
    let _ = ui.ui_tx.send(UiCmd::LoadHome);
    update_state();
}

/// Show Home and remember it as the last selected place.
pub(crate) unsafe fn enter() {
    let ui = UI.get_mut().unwrap();
    ui.home.active = true;
    ui.selected_view = None;
    ui.cfg.last_view = Some(HOME_ID.to_string());
    crate::config::save_config(&ui.cfg);
    set_hidden(ui.lib_scroll.get(), true);
    set_hidden(ui.sort_popup.get(), true);
    set_hidden(ui.lib_back.get(), true);
    set_hidden(ui.home.scroll.get(), false);
    crate::platform::appkit::set_busy(false);
    ui.cur_title = "Home".to_string();
    set_text(ui.header_title.get(), "Home");
    set_count("", false);
    header_layout();
    sidebar_refresh_selection();
    reload();
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
    let name = ui.mode_stack.last().map(|m| m.1.clone()).unwrap_or_default();
    set_text(ui.header_title.get(), &name);
    ui.cur_title = name;
    header_layout();
    sidebar_refresh_selection();
}

pub(crate) extern "C" fn home_selected(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { enter() }
}
