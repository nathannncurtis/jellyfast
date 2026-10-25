//! Item detail page for movies and series, in the library's content area.
//!
//! It replaces the grid while open and Back returns to it. The page is a
//! flipped document inside a vertical scroll view: backdrop with a fade, the
//! poster overlapping it, title and facts, action buttons, the overview, and
//! a horizontally scrolling Cast & Crew row. The document is rebuilt whenever
//! the item, the width or the appearance changes. Artwork views are registered
//! as targets and filled from `Ui.poster_cache` when their key arrives. All
//! state lives in `Ui.detail`; the views are only touched on the main thread.
//!
//! A series adds a season section below the overview: a season popup and the
//! selected season's episodes as rows (thumbnail, number and title, runtime,
//! overview snippet, watched or progress state). A row plays its episode.
#![allow(unsafe_code)]
use crate::app::messages::*;
use crate::jellyfin::urls::fmt_runtime;
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};

const HERO_MIN: f64 = 240.;
const HERO_MAX: f64 = 380.;
const POSTER_W: f64 = 168.;
const POSTER_H: f64 = 252.;
const RIGHT_X: f64 = EDGE + POSTER_W + SP6;
const CAST_W: f64 = 112.;
const CAST_GAP: f64 = SP4;
const AVATAR: f64 = 96.;
/// Avatar, gap, name line, gap, role line.
const CAST_H: f64 = AVATAR + SP2 + 16. + 2. + 14.;
const BUTTON_H: f64 = 28.;
const THUMB_W: f64 = 160.;
const THUMB_H: f64 = 90.;
const ROW_PAD: f64 = SP3;
const TITLE_H: f64 = 18.;
const META_H: f64 = 16.;
const PROGRESS_H: f64 = 3.;
/// Overview snippet length for episode rows, in characters.
const SNIPPET_CHARS: usize = 180;

/// An artwork view waiting for its image. Raw pointers: valid only until the
/// next `render`, which clears the list before it removes those views.
#[derive(Clone, Copy)]
pub struct Target {
    key: u64,
    art: *mut Object,
    glyph: *mut Object,
}

pub struct DetailUi {
    pub host: Ptr,
    pub scroll: Ptr,
    pub doc: Ptr,
    pub item: Option<DetailView>,
    pub open: bool,
    /// Season picker and episode list of a series page.
    pub seasons: SeasonsUi,
    /// The season popup of the current layout (null when the page shows none).
    pub season_popup: Ptr,
    laid_w: f64,
    targets: Vec<Target>,
}

/// Season state of a series page. Replaced as a whole when a season arrives.
#[derive(Clone, Default)]
pub struct SeasonsUi {
    pub seasons: Vec<SeasonEntry>,
    /// Index into `seasons` of the season being shown.
    pub selected: usize,
    /// The seasons request has been answered (successfully or not).
    pub loaded: bool,
    /// Episodes of the selected season; None while they load.
    pub episodes: Option<Vec<EpisodeRow>>,
    /// A failure to show in place of the episode list.
    pub error: Option<String>,
}

impl DetailUi {
    pub fn new() -> Self {
        DetailUi {
            host: Ptr(std::ptr::null_mut()),
            scroll: Ptr(std::ptr::null_mut()),
            doc: Ptr(std::ptr::null_mut()),
            item: None,
            open: false,
            seasons: SeasonsUi::default(),
            season_popup: Ptr(std::ptr::null_mut()),
            laid_w: 0.,
            targets: Vec::new(),
        }
    }
}

extern "C" fn always_yes(_t: &mut Object, _c: Sel) -> BOOL {
    YES
}

/// Forward to NSView, then re-render the page when its width changed.
extern "C" fn doc_set_frame(this: &mut Object, _cmd: Sel, s: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        let Some(ui) = UI.get_mut() else { return };
        if ui.detail.open && this as *mut Object == ui.detail.doc.get() && (s.w - ui.detail.laid_w).abs() > 0.5 {
            render(ui);
        }
    }
}

/// Colours on layers are resolved once, so re-render when the appearance flips.
extern "C" fn doc_appearance_changed(this: &mut Object, _cmd: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), viewDidChangeEffectiveAppearance];
        let Some(ui) = UI.get_mut() else { return };
        if ui.detail.open && this as *mut Object == ui.detail.doc.get() {
            render(ui);
        }
    }
}

extern "C" fn on_back(_t: &mut Object, _c: Sel, _s: *mut Object) {
    hide();
}

extern "C" fn on_play(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if let Some(item) = ui.detail.item.as_ref() {
            let id = item.id.clone();
            crate::platform::appkit::set_busy(true);
            let _ = ui.ui_tx.send(UiCmd::Play { item_id: id });
        }
    }
}

/// The series page's "Skip intros automatically": saved for this show on this Mac only.
extern "C" fn on_skip_intros(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(item) = ui.detail.item.as_ref() else { return };
        let on: NSInteger = msg_send![sender, state];
        if crate::config::set_skip_intros(&item.id, on != 0).is_err() {
            let _: () = msg_send![sender, setState: (on == 0) as NSInteger];
            crate::ui::player::toast("Couldn't save that setting", Some("exclamationmark.triangle"), true);
        }
    }
}

extern "C" fn on_watched(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(item) = ui.detail.item.as_mut() else { return };
        item.played = !item.played;
        if item.played {
            item.resume = false;
        }
        let (id, played) = (item.id.clone(), item.played);
        let _ = ui.ui_tx.send(UiCmd::MarkPlayed { item_id: id, played });
        render(ui);
    }
}

/// The season popup changed: load the chosen season's episodes.
extern "C" fn on_season(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let idx: isize = msg_send![sender, indexOfSelectedItem];
        let Ok(idx) = usize::try_from(idx) else { return };
        let Some(series_id) = ui.detail.item.as_ref().map(|i| i.id.clone()) else { return };
        let s = &mut ui.detail.seasons;
        let Some(season_id) = s.seasons.get(idx).map(|x| x.id.clone()) else { return };
        if idx == s.selected && s.episodes.is_some() {
            return;
        }
        s.selected = idx;
        s.episodes = None;
        s.error = None;
        let _ = ui.ui_tx.send(UiCmd::SeasonEpisodes { series_id, season_id });
        render(ui);
    }
}

/// An episode row was clicked (its tag is the row index): play that episode.
extern "C" fn on_episode(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let tag: isize = msg_send![sender, tag];
        let Some(item_id) = ui
            .detail
            .seasons
            .episodes
            .as_ref()
            .and_then(|eps| usize::try_from(tag).ok().and_then(|i| eps.get(i)))
            .map(|e| e.id.clone())
        else {
            return;
        };
        crate::platform::appkit::set_busy(true);
        let _ = ui.ui_tx.send(UiCmd::Play { item_id });
    }
}

/// Classes used by the page. Called once at startup, with the other classes.
pub(crate) unsafe fn register_detail_classes() {
    let mut h = ClassDecl::new("JRDetailHost", class!(NSView)).unwrap();
    h.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
    h.add_method(sel!(detailBack:), on_back as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailPlay:), on_play as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailWatched:), on_watched as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailSeason:), on_season as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailEpisode:), on_episode as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailSkipIntros:), on_skip_intros as extern "C" fn(&mut Object, Sel, *mut Object));
    h.register();
    let mut d = ClassDecl::new("JRDetailDoc", class!(NSView)).unwrap();
    d.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
    d.add_method(sel!(setFrameSize:), doc_set_frame as extern "C" fn(&mut Object, Sel, NSSize));
    d.add_method(
        sel!(viewDidChangeEffectiveAppearance),
        doc_appearance_changed as extern "C" fn(&mut Object, Sel),
    );
    d.register();
    let mut f = ClassDecl::new("JRDetailFlip", class!(NSView)).unwrap();
    f.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
    f.register();
}

/// Create the (hidden) page inside the content host. `frame` is the library area.
pub(crate) unsafe fn build(content_host: *mut Object, frame: NSRect) {
    let ui = UI.get_mut().unwrap();
    let host_cls = Class::get("JRDetailHost").unwrap();
    let host: *mut Object = msg_send![host_cls, alloc];
    let host: *mut Object = msg_send![host, initWithFrame: frame];
    set_autoresize(host, RESIZE_W | RESIZE_H);
    set_hidden(host, true);
    add_sub(content_host, host);
    autorel(host);
    ui.detail.host = Ptr(host);

    let bar_h = HEADER_H;
    let sc: *mut Object = msg_send![class!(NSScrollView), alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: rect(0., bar_h, frame.size.w, frame.size.h - bar_h)];
    let _: () = msg_send![sc, setHasVerticalScroller: YES];
    let _: () = msg_send![sc, setDrawsBackground: YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    add_sub(host, sc);
    autorel(sc);
    ui.detail.scroll = Ptr(sc);

    let doc_cls = Class::get("JRDetailDoc").unwrap();
    let doc: *mut Object = msg_send![doc_cls, alloc];
    let doc: *mut Object = msg_send![doc, initWithFrame: rect(0., 0., frame.size.w, 10.)];
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    autorel(doc);
    ui.detail.doc = Ptr(doc);

    let back: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Back")
        target: host action: sel!(detailBack:)];
    let _: () = msg_send![back, setBordered: NO];
    let chev = symbol("chevron.left", 13., Weight::Medium);
    if !chev.is_null() {
        let _: () = msg_send![back, setImage: chev];
    }
    let _: () = msg_send![back, setImagePosition: 7isize]; // leading
    let _: () = msg_send![back, setContentTintColor: color(C_TEXT2)];
    let _: () = msg_send![back, setFont: font(T_CALLOUT)];
    let _: () = msg_send![back, sizeToFit];
    let f: NSRect = msg_send![back, frame];
    set_frame(back, rect(EDGE - SP2, (bar_h - f.size.h) / 2., f.size.w, f.size.h));
    add_sub(host, back);
}

/// Show the page for `view`, replacing the grid until Back (or another navigation).
pub(crate) fn show(view: DetailView) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let series_id = (view.item_type == "Series").then(|| view.id.clone());
        ui.detail.item = Some(view);
        ui.detail.open = true;
        ui.detail.laid_w = 0.;
        ui.detail.seasons = SeasonsUi::default();
        set_hidden(ui.header.get(), true);
        set_hidden(ui.lib_scroll.get(), true);
        set_hidden(ui.state_host.get(), true);
        set_hidden(ui.detail.host.get(), false);
        render(ui);
        if let Some(series_id) = series_id {
            let _ = ui.ui_tx.send(UiCmd::Seasons { series_id });
        }
        let doc = ui.detail.doc.get();
        let _: () = msg_send![doc, scrollPoint: NSPoint { x: 0., y: 0. }];
    }
}

/// Leave the page and show the grid again.
pub(crate) fn hide() {
    let Some(ui) = (unsafe { UI.get_mut() }) else { return };
    unsafe { close(ui) };
}

unsafe fn close(ui: &mut Ui) {
    if !ui.detail.open {
        return;
    }
    ui.detail.open = false;
    ui.detail.item = None;
    ui.detail.seasons = SeasonsUi::default();
    ui.detail.season_popup = Ptr(std::ptr::null_mut());
    ui.detail.targets.clear();
    set_hidden(ui.detail.host.get(), true);
    set_hidden(ui.header.get(), false);
    set_hidden(ui.lib_scroll.get(), false);
}

pub fn is_open() -> bool {
    UI.get().map(|u| u.detail.open).unwrap_or(false)
}

/// Whether the page is open on the series `series_id`.
fn showing(ui: &Ui, series_id: &str) -> bool {
    ui.detail.open && ui.detail.item.as_ref().map(|i| i.id == series_id).unwrap_or(false)
}

/// The seasons of the open series page arrived; `selected` is the one to show first.
pub fn seasons_loaded(series_id: String, seasons: Vec<SeasonEntry>, selected: usize) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if !showing(ui, &series_id) {
            return;
        }
        let selected = selected.min(seasons.len().saturating_sub(1));
        let first = seasons.get(selected).map(|s| s.id.clone());
        ui.detail.seasons = SeasonsUi { seasons, selected, loaded: true, episodes: None, error: None };
        match first {
            Some(season_id) => {
                let _ = ui.ui_tx.send(UiCmd::SeasonEpisodes { series_id, season_id });
            }
            None => ui.detail.seasons.episodes = Some(Vec::new()),
        }
        render(ui);
    }
}

/// Episodes of a season arrived. Dropped unless that season is still the selected one.
pub fn episodes_loaded(series_id: String, season_id: String, episodes: Vec<EpisodeRow>) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if !showing(ui, &series_id) {
            return;
        }
        let s = &mut ui.detail.seasons;
        if s.seasons.get(s.selected).map(|x| x.id.as_str()) != Some(season_id.as_str()) {
            return;
        }
        s.episodes = Some(episodes);
        render(ui);
    }
}

/// Seasons or episodes of the open series page could not be loaded.
pub fn seasons_failed(series_id: String, message: String) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        if !showing(ui, &series_id) {
            return;
        }
        let s = &mut ui.detail.seasons;
        s.loaded = true;
        s.episodes = None;
        s.error = Some(message);
        render(ui);
    }
}

/// Artwork for `key` arrived: fill every page view waiting for it.
pub fn image_ready(key: u64) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(img) = ui.poster_cache.get(&key).copied() else { return };
        let waiting: Vec<Target> = ui.detail.targets.iter().filter(|t| t.key == key).copied().collect();
        for t in waiting {
            set_art(t.art, t.glyph, img);
        }
    }
}

unsafe fn set_art(art: *mut Object, glyph: *mut Object, img: *mut Object) {
    let cg = cg_image(img);
    if cg.is_null() {
        return;
    }
    let layer = layer_of(art);
    tx_begin_instant();
    let _: () = msg_send![layer, setContents: cg];
    tx_end();
    if !glyph.is_null() {
        set_hidden(glyph, true);
    }
}

/// Register an artwork view for `key` and fill it now if the image is cached.
unsafe fn bind(ui: &mut Ui, key: Option<u64>, art: *mut Object, glyph: *mut Object) {
    let Some(key) = key else { return };
    ui.detail.targets.push(Target { key, art, glyph });
    if let Some(img) = ui.poster_cache.get(&key).copied() {
        set_art(art, glyph, img);
    }
}

unsafe fn new_view(frame: NSRect) -> *mut Object {
    let v: *mut Object = msg_send![class!(NSView), alloc];
    let v: *mut Object = msg_send![v, initWithFrame: frame];
    autorel(v)
}

unsafe fn flip_view(frame: NSRect) -> *mut Object {
    let cls = Class::get("JRDetailFlip").unwrap();
    let v: *mut Object = msg_send![cls, alloc];
    let v: *mut Object = msg_send![v, initWithFrame: frame];
    autorel(v)
}

/// Layer-backed rounded rectangle that shows artwork (aspect-fill) over a fill colour.
unsafe fn art_view(frame: NSRect, radius: f64) -> *mut Object {
    let v = new_view(frame);
    let layer = layer_of(v);
    set_corner(layer, radius);
    let _: () = msg_send![layer, setMasksToBounds: YES];
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![layer, setBackgroundColor: cg(C_FILL_SEL)];
    v
}

/// Centred placeholder glyph inside `frame`.
unsafe fn glyph_in(frame: NSRect, name: &str, pt: f64) -> *mut Object {
    let g: *mut Object = msg_send![class!(NSImageView), alloc];
    let g: *mut Object = msg_send![g, initWithFrame: rect(
        frame.origin.x + (frame.size.w - pt) / 2., frame.origin.y + (frame.size.h - pt) / 2., pt, pt)];
    let img = symbol(name, pt, Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![g, setImage: img];
    }
    let _: () = msg_send![g, setContentTintColor: color(C_TEXT3)];
    autorel(g)
}

/// Height a wrapping label needs at `width`.
unsafe fn text_height(v: *mut Object, width: f64) -> f64 {
    let cell: *mut Object = msg_send![v, cell];
    let sz: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., width, 1.0e6)];
    sz.h.ceil()
}

unsafe fn place_text(doc: *mut Object, text: &str, t: TypeStyle, c: Col, frame: NSRect) {
    let l = label(text, t, c, ALIGN_LEFT);
    set_frame(l, frame);
    add_sub(doc, l);
}

fn meta_line(item: &DetailView) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(y) = item.year {
        parts.push(y.to_string());
    }
    if let Some(r) = &item.official_rating {
        parts.push(r.clone());
    }
    let rt = fmt_runtime(item.run_time_ticks);
    if !rt.is_empty() {
        parts.push(rt);
    }
    if let Some(c) = item.community_rating {
        parts.push(format!("★ {:.1}", c));
    }
    parts.join("  ·  ")
}

/// Action button sized to its title and symbol; returns the view and its width.
unsafe fn action_button(title: &str, symbol_name: &str, action: Sel, target: *mut Object, primary: bool) -> (*mut Object, f64) {
    let b: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns(title) target: target action: action];
    let _: () = msg_send![b, setBezelStyle: 1isize];
    let img = symbol(symbol_name, 12., Weight::Medium);
    if !img.is_null() {
        let _: () = msg_send![b, setImage: img];
    }
    let _: () = msg_send![b, setImagePosition: 7isize];
    if primary {
        let _: () = msg_send![b, setKeyEquivalent: ns("\r")];
    }
    let _: () = msg_send![b, sizeToFit];
    let f: NSRect = msg_send![b, frame];
    let w = f.size.w.ceil();
    set_frame(b, rect(0., 0., w, BUTTON_H));
    // `buttonWithTitle:` is a convenience constructor: already autoreleased.
    (b, w)
}

/// Lay the whole page out into the document for `ui.detail.item` at width `w`.
unsafe fn render(ui: &mut Ui) {
    let Some(item) = ui.detail.item.clone() else { return };
    let doc = ui.detail.doc.get();
    let clip: *mut Object = msg_send![ui.detail.scroll.get(), contentView];
    let w = bounds_of(clip).size.w.max(320.);
    ui.detail.laid_w = w;
    ui.detail.targets.clear();
    let none: *mut Object = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];
    let host = ui.detail.host.get();
    let h = build_page(ui, doc, host, &item, w);
    set_frame(doc, rect(0., 0., w, h));
}

unsafe fn build_page(ui: &mut Ui, doc: *mut Object, host: *mut Object, item: &DetailView, w: f64) -> f64 {
    // Backdrop with a fade into the canvas.
    let hero_h = (w * 0.36).round().clamp(HERO_MIN, HERO_MAX);
    let hero = new_view(rect(0., 0., w, hero_h));
    add_sub(doc, hero);
    let hero_layer = layer_of(hero);
    let _: () = msg_send![hero_layer, setMasksToBounds: YES];
    let _: () = msg_send![hero_layer, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![hero_layer, setBackgroundColor: cg(C_FILL_SEL)];
    let fade: *mut Object = msg_send![class!(CAGradientLayer), layer];
    let _: () = msg_send![fade, setFrame: rect(0., 0., w, hero_h)];
    let clear: *mut Object = msg_send![color_a(C_CANVAS, 0.0), CGColor];
    let solid: *mut Object = msg_send![color(C_CANVAS), CGColor];
    let cols = [clear, solid];
    let arr: *mut Object = msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr() count: 2usize];
    let _: () = msg_send![fade, setColors: arr];
    let _: () = msg_send![fade, setStartPoint: NSPoint { x: 0., y: 1. }];
    let _: () = msg_send![fade, setEndPoint: NSPoint { x: 0., y: 0. }];
    let _: () = msg_send![hero_layer, addSublayer: fade];
    bind(ui, item.backdrop_key, hero, std::ptr::null_mut());

    // Poster, overlapping the lower part of the backdrop.
    let poster_y = hero_h - POSTER_H * 0.5;
    let poster = art_view(rect(EDGE, poster_y, POSTER_W, POSTER_H), R_POSTER);
    add_sub(doc, poster);
    let glyph_name = if item.item_type == "Series" { "tv" } else { "film" };
    let pg = glyph_in(rect(EDGE, poster_y, POSTER_W, POSTER_H), glyph_name, 44.);
    add_sub(doc, pg);
    bind(ui, item.poster_key, poster, pg);

    // Title, facts and actions beside the poster.
    let rw = (w - RIGHT_X - EDGE).max(160.);
    let mut y = hero_h + SP4;
    let title = label(&item.name, T_DISPLAY, C_TEXT, ALIGN_LEFT);
    let _: () = msg_send![title, setToolTip: ns(&item.name)];
    set_frame(title, rect(RIGHT_X, y, rw, 32.));
    add_sub(doc, title);
    y += 40.;
    let meta = meta_line(item);
    if !meta.is_empty() {
        place_text(doc, &meta, T_CALLOUT, C_TEXT2, rect(RIGHT_X, y, rw, 18.));
        y += 24.;
    }
    if !item.genres.is_empty() {
        place_text(doc, &item.genres.join("  ·  "), T_CALLOUT, C_TEXT2, rect(RIGHT_X, y, rw, 18.));
        y += 24.;
    }
    y += SP2;
    let mut bx = RIGHT_X;
    let is_series = item.item_type == "Series";
    if !is_series {
        let title = if item.resume { "Resume" } else { "Play" };
        let (b, bw) = action_button(title, "play.fill", sel!(detailPlay:), host, true);
        set_frame(b, rect(bx, y, bw, BUTTON_H));
        add_sub(doc, b);
        bx += bw + SP2;
    }
    let watched = if item.played { "Mark as unwatched" } else { "Mark as watched" };
    let (b, bw) = action_button(watched, "checkmark.circle", sel!(detailWatched:), host, false);
    set_frame(b, rect(bx, y, bw, BUTTON_H));
    add_sub(doc, b);
    if is_series {
        bx += bw + SP2;
        let on = crate::config::skip_intros_for(&item.id, ui.cfg.settings.skip_intros_default);
        let cb: *mut Object = msg_send![class!(NSButton), checkboxWithTitle: ns("Skip intros automatically")
            target: host action: sel!(detailSkipIntros:)];
        let _: () = msg_send![cb, setState: on as NSInteger];
        let _: () = msg_send![cb, setToolTip: ns("Skip intros as soon as they start, for this show on this Mac.")];
        let _: () = msg_send![cb, sizeToFit];
        let f: NSRect = msg_send![cb, frame];
        set_frame(cb, rect(bx, y + (BUTTON_H - f.size.h) / 2., f.size.w, f.size.h));
        add_sub(doc, cb);
    }
    y += BUTTON_H + SP5;

    if let Some(ov) = item.overview.as_deref() {
        let v = wrapping_label(ov, T_BODY, C_TEXT2, ALIGN_LEFT);
        set_frame(v, rect(RIGHT_X, y, rw, 10.));
        let th = text_height(v, rw);
        set_frame(v, rect(RIGHT_X, y, rw, th));
        add_sub(doc, v);
        y += th;
    }
    let mut sy = (poster_y + POSTER_H).max(y) + SP8;

    if is_series {
        sy = build_seasons(ui, doc, host, w, sy) + SP4;
    }

    // Cast & Crew: one horizontally scrolling row of circular portraits.
    if !item.people.is_empty() {
        let head = label("Cast & Crew", T_TITLE, C_TEXT, ALIGN_LEFT);
        set_frame(head, rect(EDGE, sy, w - 2. * EDGE, 24.));
        add_sub(doc, head);
        sy += 24. + SP4;
        let n = item.people.len();
        let inner_w = EDGE * 2. + n as f64 * CAST_W + (n as f64 - 1.) * CAST_GAP;
        let row_h = CAST_H + SP4; // room for a legacy horizontal scroller
        let row: *mut Object = msg_send![class!(NSScrollView), alloc];
        let row: *mut Object = autorel(msg_send![row, initWithFrame: rect(0., sy, w, row_h)]);
        let _: () = msg_send![row, setHasHorizontalScroller: YES];
        let _: () = msg_send![row, setHasVerticalScroller: NO];
        let _: () = msg_send![row, setDrawsBackground: NO];
        set_autoresize(row, RESIZE_W);
        let inner = flip_view(rect(0., 0., inner_w, CAST_H));
        for (i, p) in item.people.iter().enumerate() {
            let x = EDGE + i as f64 * (CAST_W + CAST_GAP);
            let cell = flip_view(rect(x, 0., CAST_W, CAST_H));
            add_sub(inner, cell);
            let av = art_view(rect((CAST_W - AVATAR) / 2., 0., AVATAR, AVATAR), AVATAR / 2.);
            add_sub(cell, av);
            let g = glyph_in(rect((CAST_W - AVATAR) / 2., 0., AVATAR, AVATAR), "person.fill", 36.);
            add_sub(cell, g);
            bind(ui, p.image_key, av, g);
            let name = label(&p.name, T_CALLOUT, C_TEXT, ALIGN_CENTER);
            let _: () = msg_send![name, setToolTip: ns(&p.name)];
            set_frame(name, rect(0., AVATAR + SP2, CAST_W, 16.));
            add_sub(cell, name);
            let role = label(p.role.as_deref().unwrap_or(""), T_CAPTION, C_TEXT2, ALIGN_CENTER);
            set_frame(role, rect(0., AVATAR + SP2 + 18., CAST_W, 14.));
            add_sub(cell, role);
        }
        let _: () = msg_send![row, setDocumentView: inner];
        add_sub(doc, row);
        sy += row_h + SP6;
    }
    sy + SP8
}

/// Season picker and the selected season's episodes. Returns the y below them.
unsafe fn build_seasons(ui: &mut Ui, doc: *mut Object, host: *mut Object, w: f64, mut y: f64) -> f64 {
    let st = ui.detail.seasons.clone();
    let content_w = w - 2. * EDGE;
    let head_h = 24.;
    let line = |y: f64| y + 18. + SP8;
    let mut head_w = content_w;
    ui.detail.season_popup = Ptr(std::ptr::null_mut());
    if !st.seasons.is_empty() {
        let pop = season_popup(&st, host);
        let pf = frame_of(pop);
        set_frame(pop, rect(w - EDGE - pf.size.w, y + (head_h - pf.size.h) / 2., pf.size.w, pf.size.h));
        add_sub(doc, pop);
        ui.detail.season_popup = Ptr(pop);
        head_w -= pf.size.w + SP4;
    }
    let head = label("Episodes", T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, y, head_w.max(40.), head_h));
    add_sub(doc, head);
    y += head_h + SP4;

    if !st.loaded {
        place_text(doc, "Loading seasons…", T_CALLOUT, C_TEXT2, rect(EDGE, y, content_w, 18.));
        return line(y);
    }
    if let Some(msg) = st.error.as_deref() {
        let v = wrapping_label(&format!("Couldn't load this season. {}", msg), T_CALLOUT, C_TEXT2, ALIGN_LEFT);
        let th = text_height(v, content_w);
        set_frame(v, rect(EDGE, y, content_w, th));
        add_sub(doc, v);
        return y + th + SP8;
    }
    if st.seasons.is_empty() {
        place_text(doc, "No seasons yet.", T_CALLOUT, C_TEXT2, rect(EDGE, y, content_w, 18.));
        return line(y);
    }
    match st.episodes.as_deref() {
        None => {
            place_text(doc, "Loading episodes…", T_CALLOUT, C_TEXT2, rect(EDGE, y, content_w, 18.));
            line(y)
        }
        Some([]) => {
            place_text(doc, "No episodes in this season.", T_CALLOUT, C_TEXT2, rect(EDGE, y, content_w, 18.));
            line(y)
        }
        Some(eps) => {
            for (i, ep) in eps.iter().enumerate() {
                y = episode_row(ui, doc, host, ep, i, w, y);
            }
            y
        }
    }
}

/// Season popup: lists the seasons; choosing one loads its episodes.
unsafe fn season_popup(st: &SeasonsUi, host: *mut Object) -> *mut Object {
    let pop: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let pop: *mut Object = msg_send![pop, initWithFrame: rect(0., 0., 160., 24.) pullsDown: NO];
    let names: *mut Object = msg_send![class!(NSMutableArray), array];
    for s in &st.seasons {
        let _: () = msg_send![names, addObject: ns(&s.name)];
    }
    let _: () = msg_send![pop, addItemsWithTitles: names];
    let _: () = msg_send![pop, selectItemAtIndex: st.selected as isize];
    let _: () = msg_send![pop, setTarget: host];
    let _: () = msg_send![pop, setAction: sel!(detailSeason:)];
    let _: () = msg_send![pop, sizeToFit];
    autorel(pop)
}

/// One episode row: thumbnail (progress bar underneath), number and title, runtime,
/// overview snippet, and a check when watched. A transparent button over the whole
/// row plays it; it is added last so it sits on top. Returns the y below the row.
unsafe fn episode_row(ui: &mut Ui, doc: *mut Object, host: *mut Object, ep: &EpisodeRow, idx: usize, w: f64, y: f64) -> f64 {
    let rw = w - 2. * EDGE;
    let text_x = ROW_PAD + THUMB_W + SP4;
    let check_w = if ep.played { 18. } else { 0. };
    let text_w = (rw - text_x - ROW_PAD - check_w - SP2).max(120.);
    let title = match ep.number {
        Some(n) => format!("E{}  {}", n, ep.name),
        None => ep.name.clone(),
    };
    let snippet = ep.overview.as_deref().map(|o| overview_snippet(o, SNIPPET_CHARS)).filter(|s| !s.is_empty());
    let overview = snippet.as_ref().map(|t| {
        let v = wrapping_label(t, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
        (v, text_height(v, text_w))
    });
    let overview_h = overview.map(|(_, h)| SP1 + h).unwrap_or(0.);
    let text_h = TITLE_H + SP1 + META_H + overview_h;
    let row_h = (text_h.max(THUMB_H) + 2. * ROW_PAD).ceil();

    let row = flip_view(rect(EDGE, y, rw, row_h));
    add_sub(row, surface(rect(0., 0., rw, row_h), C_CARD, R_ROW));

    let thumb = art_view(rect(ROW_PAD, ROW_PAD, THUMB_W, THUMB_H), R_THUMB);
    add_sub(row, thumb);
    let glyph = glyph_in(rect(ROW_PAD, ROW_PAD, THUMB_W, THUMB_H), "play.rectangle", 28.);
    add_sub(row, glyph);
    bind(ui, ep.image_key, thumb, glyph);
    if let Some(frac) = progress_fraction(ep) {
        let track_y = ROW_PAD + THUMB_H - PROGRESS_H;
        let track = new_view(rect(ROW_PAD, track_y, THUMB_W, PROGRESS_H));
        let _: () = msg_send![layer_of(track), setBackgroundColor: cg_black(0.4)];
        add_sub(row, track);
        let fill = new_view(rect(ROW_PAD, track_y, THUMB_W * frac, PROGRESS_H));
        let _: () = msg_send![layer_of(fill), setBackgroundColor: cg(C_ACCENT)];
        add_sub(row, fill);
    }

    let t = label(&title, T_BODY_EM, C_TEXT, ALIGN_LEFT);
    let _: () = msg_send![t, setToolTip: ns(&title)];
    set_frame(t, rect(text_x, ROW_PAD, text_w, TITLE_H));
    add_sub(row, t);
    let meta = episode_meta(ep);
    if !meta.is_empty() {
        place_text(row, &meta, T_CALLOUT, C_TEXT2, rect(text_x, ROW_PAD + TITLE_H + SP1, text_w, META_H));
    }
    if let Some((v, h)) = overview {
        set_frame(v, rect(text_x, ROW_PAD + TITLE_H + SP1 + META_H + SP1, text_w, h));
        add_sub(row, v);
    }
    if ep.played {
        let g = glyph_in(rect(rw - ROW_PAD - check_w, ROW_PAD, check_w, check_w), "checkmark.circle.fill", 16.);
        let _: () = msg_send![g, setContentTintColor: color(C_OK)];
        add_sub(row, g);
    }

    let hit: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("") target: host action: sel!(detailEpisode:)];
    let _: () = msg_send![hit, setBordered: NO];
    let _: () = msg_send![hit, setTag: idx as isize];
    let _: () = msg_send![hit, setToolTip: ns(&title)];
    set_frame(hit, rect(0., 0., rw, row_h));
    add_sub(row, hit);

    add_sub(doc, row);
    y + row_h + SP2
}

/// Fraction watched for the progress bar: None when the episode is watched or unstarted.
fn progress_fraction(ep: &EpisodeRow) -> Option<f64> {
    let run = ep.run_time_ticks.filter(|r| *r > 0)?;
    if ep.played || ep.position_ticks <= 0 {
        return None;
    }
    Some((ep.position_ticks as f64 / run as f64).clamp(0., 1.))
}

/// Runtime, and minutes left when partly watched (ticks are 100 ns; 600 million per minute).
fn episode_meta(ep: &EpisodeRow) -> String {
    let mut parts: Vec<String> = Vec::new();
    let rt = fmt_runtime(ep.run_time_ticks);
    if !rt.is_empty() {
        parts.push(rt);
    }
    if let Some(run) = ep.run_time_ticks.filter(|r| *r > 0) {
        if !ep.played && ep.position_ticks > 0 {
            let left = (run - ep.position_ticks).max(0) / 600_000_000;
            if left > 0 {
                parts.push(format!("{}m left", left));
            }
        }
    }
    parts.join("  ·  ")
}

/// The first `max` characters of `text` with whitespace collapsed, cut at a word
/// boundary and ended with an ellipsis when it is longer than that.
pub fn overview_snippet(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let kept = match cut.rfind(' ') {
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_keeps_short_text_and_collapses_whitespace() {
        assert_eq!(overview_snippet("  A  quiet

start. ", 180), "A quiet start.");
        assert_eq!(overview_snippet("", 180), "");
    }

    #[test]
    fn snippet_cuts_at_a_word_with_ellipsis() {
        let long = "word ".repeat(100);
        let s = overview_snippet(&long, 50);
        assert!(s.ends_with('…'), "{s}");
        assert!(s.chars().count() <= 51, "{s}");
        assert!(!s.trim_end_matches('…').ends_with(' '), "{s}");
        assert!(s.trim_end_matches('…').split(' ').all(|w| w == "word" || w.is_empty()), "{s}");
    }

    #[test]
    fn snippet_handles_multibyte_text() {
        let s = overview_snippet(&"é".repeat(300), 10);
        assert_eq!(s.chars().count(), 11);
    }

    #[test]
    fn progress_only_for_started_unwatched_episodes() {
        let mut ep = EpisodeRow {
            id: "e".into(),
            number: Some(1),
            name: "n".into(),
            overview: None,
            run_time_ticks: Some(12_000_000_000),
            played: false,
            position_ticks: 3_000_000_000,
            image_key: None,
        };
        assert_eq!(progress_fraction(&ep), Some(0.25));
        assert_eq!(episode_meta(&ep), "20m  ·  15m left");
        ep.played = true;
        assert_eq!(progress_fraction(&ep), None);
        assert_eq!(episode_meta(&ep), "20m");
        ep.played = false;
        ep.position_ticks = 0;
        assert_eq!(progress_fraction(&ep), None);
        ep.run_time_ticks = None;
        assert_eq!(progress_fraction(&ep), None);
        assert_eq!(episode_meta(&ep), "");
    }
}

// ---- Introspection for the self-test (read-only) ---------------------------

/// Whether the page's artwork view for `key` has its image: None if the page has no such view.
pub fn art_filled(key: u64) -> Option<bool> {
    let ui = UI.get()?;
    let t = ui.detail.targets.iter().find(|t| t.key == key)?;
    unsafe {
        let layer: *mut Object = msg_send![t.art, layer];
        let c: *mut Object = msg_send![layer, contents];
        Some(!c.is_null())
    }
}

/// Number of artwork views laid out on the page (poster, backdrop, portraits).
pub fn target_count() -> usize {
    UI.get().map(|u| u.detail.targets.len()).unwrap_or(0)
}

/// The document is exactly as wide as the visible part of the scroll view.
pub fn layout_matches_clip() -> bool {
    let Some(ui) = UI.get() else { return false };
    unsafe {
        let doc = ui.detail.doc.get();
        let clip: *mut Object = msg_send![ui.detail.scroll.get(), contentView];
        let df: NSRect = msg_send![doc, frame];
        let cb: NSRect = msg_send![clip, bounds];
        (df.size.w - cb.size.w).abs() < 0.5 && df.size.h > 0.
    }
}
