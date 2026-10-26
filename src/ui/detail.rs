//! Item detail page for movies and series, in the library's content area.
//!
//! It replaces the grid while open and Back returns to it. The page is a
//! flipped document inside a vertical scroll view. The backdrop runs full
//! bleed under the transparent titlebar, with the title logo (or the name, set
//! large) and the facts on it, then the primary Play or Resume button, round
//! icon actions, and the progress of the Next Up episode. Below the backdrop
//! come the tagline, the overview, the series options, the season picker and
//! episode rows (Next Up highlighted), a Cast & Crew row and More Like This.
//! The document is rebuilt whenever the item, the width or the appearance
//! changes. Artwork views are registered as targets and filled from
//! `Ui.poster_cache` when their key arrives. All state lives in `Ui.detail`;
//! the views are only touched on the main thread.
//!
//! A series adds the season section: a season popup and the selected season's
//! episodes as rows (thumbnail, number and title, runtime, overview snippet,
//! watched or progress state). A row plays its episode.
#![allow(unsafe_code)]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::*;
use crate::jellyfin::urls::{fmt_runtime, progress_fraction};
use crate::platform::appkit::*;
use crate::ui::theme::*;

const HERO_MIN: f64 = 340.;
const HERO_MAX: f64 = 560.;
/// Clear space under the hero text, where the backdrop fades into the canvas.
const HERO_FOOT: f64 = 56.;
/// The titlebar band at the top of the page, where the Back button sits.
const HERO_TOP: f64 = TITLEBAR_H;
/// Widest the hero text runs before it wraps.
const HERO_TEXT_MAX: f64 = 640.;
/// Widest a paragraph on the canvas runs (reading width).
const TEXT_MAX: f64 = 760.;
const LOGO_H: f64 = 120.;
const LOGO_W: f64 = 480.;
const PRIMARY_H: f64 = 40.;
const ICON_D: f64 = 40.;
const CHIP_H: f64 = 20.;
const PROGRESS_H: f64 = 3.;
const CAST_W: f64 = 128.;
const CAST_GAP: f64 = SP4;
const AVATAR: f64 = 104.;
/// Avatar, gap, two name lines, gap, two role lines.
const CAST_H: f64 = AVATAR + SP2 + 32. + SP1 + 30.;
const SIM_W: f64 = 150.;
const SIM_H: f64 = 225.;
/// Poster, gap, name line, gap, year line.
const SIM_CELL_H: f64 = SIM_H + SP2 + 18. + SP1 + 15.;
const HEAD_H: f64 = 30.;
const THUMB_W: f64 = 208.;
const THUMB_H: f64 = 117.;
const ROW_PAD: f64 = SP3;
const TITLE_H: f64 = 18.;
const META_H: f64 = 16.;
/// Overview snippet length for episode rows, in characters.
const SNIPPET_CHARS: usize = 180;

/// Hero title styles, largest first. The name takes the first that fits two lines.
const T_HERO: TypeStyle = TypeStyle { size: 44., weight: Weight::Bold, mono: false };
const T_HERO_M: TypeStyle = TypeStyle { size: 36., weight: Weight::Bold, mono: false };
const T_HERO_S: TypeStyle = TypeStyle { size: 30., weight: Weight::Bold, mono: false };

/// An artwork view waiting for its image. Raw pointers: valid only until the
/// next `render`, which clears the list before it removes those views.
#[derive(Clone, Copy)]
pub struct Target {
    key: u64,
    art: *mut AnyObject,
    glyph: *mut AnyObject,
    /// A title logo: once its image arrives, the view narrows to the image's shape.
    fit: bool,
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

extern "C" fn always_yes(_t: *mut AnyObject, _c: Sel) -> Bool {
    Bool::YES
}

/// Forward to NSView, then re-render the page when its width changed.
extern "C" fn doc_set_frame(this: *mut AnyObject, _cmd: Sel, s: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        let Some(ui) = UI.get_mut() else { return };
        if ui.detail.open && this == ui.detail.doc.get() && (s.w - ui.detail.laid_w).abs() > 0.5 {
            render(ui);
        }
    }
}

/// Colours on layers are resolved once, so re-render when the appearance flips.
extern "C" fn doc_appearance_changed(this: *mut AnyObject, _cmd: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), viewDidChangeEffectiveAppearance];
        let Some(ui) = UI.get_mut() else { return };
        if ui.detail.open && this == ui.detail.doc.get() {
            render(ui);
        }
    }
}

extern "C" fn on_back(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    hide();
}

/// Play or Resume: the Next Up episode for a series, the item itself otherwise.
extern "C" fn on_play(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    let Some(ui) = UI.get_mut() else { return };
    if let Some(item) = ui.detail.item.as_ref() {
        let id = play_id(item);
        crate::platform::appkit::set_busy(true);
        let _ = ui.ui_tx.send(UiCmd::Play { item_id: id });
    }
}

/// Favourite heart: flips the flag on the page and tells the server.
extern "C" fn on_favorite(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(item) = ui.detail.item.as_mut() else { return };
        item.is_favorite = !item.is_favorite;
        let (id, favorite) = (item.id.clone(), item.is_favorite);
        let _ = ui.ui_tx.send(UiCmd::SetFavorite { item_id: id, favorite });
        render(ui);
    }
}

/// Trailer: opens the first remote trailer in the browser.
extern "C" fn on_trailer(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    let Some(ui) = UI.get() else { return };
    if let Some(url) = ui.detail.item.as_ref().and_then(|i| i.trailer_urls.first()) {
        open_link(url);
    }
}

/// Hands a web link to the browser. Only http and https links; the self-test never opens one.
fn open_link(url: &str) {
    if std::env::var_os("JELLYFAST_SELFTEST").is_some() {
        return;
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    unsafe {
        let u: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns(url)];
        if !u.is_null() {
            let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let _: Bool = msg_send![ws, openURL: u];
        }
    }
}

/// A similar title's poster was clicked (its tag is the index): open that title's page.
extern "C" fn on_similar(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let tag: isize = msg_send![sender, tag];
        let id = ui
            .detail
            .item
            .as_ref()
            .and_then(|i| usize::try_from(tag).ok().and_then(|n| i.similar.get(n)))
            .map(|s| s.id.clone());
        if let Some(item_id) = id {
            let _ = ui.ui_tx.send(UiCmd::OpenDetail { item_id });
        }
    }
}

/// Mark watched or unwatched.
extern "C" fn on_watched(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
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

/// The series page's "Skip intros automatically": saved for this show on this Mac only.
extern "C" fn on_skip_intros(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
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

/// Cast button: Play On menu for the open movie, episode or series.
extern "C" fn on_cast(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(item) = ui.detail.item.as_ref() else { return };
        let series = item.item_type == "Series";
        let cast_item = crate::ui::cast::CastItem {
            id: item.id.clone(),
            title: item.name.clone(),
            subtitle: meta_line(item),
            // A series starts at its Next Up episode, so no saved position applies.
            start_secs: if series { 0. } else { crate::jellyfin::urls::secs_from_ticks(item.resume_ticks) },
            run_secs: crate::jellyfin::urls::secs_from_ticks(item.run_time_ticks.unwrap_or(0)),
            series,
        };
        crate::ui::cast::open_menu(sender, cast_item);
    }
}

/// The season popup changed: load the chosen season's episodes.
extern "C" fn on_season(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
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
extern "C" fn on_episode(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
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

/// Menu actions for the open page (⌘↩, ⌘D, ⌘T, ⇧⌘W). The menus grey them out when no page is open.
pub(crate) extern "C" fn key_play(t: *mut AnyObject, c: Sel, s: *mut AnyObject) {
    on_play(t, c, s)
}
pub(crate) extern "C" fn key_favorite(t: *mut AnyObject, c: Sel, s: *mut AnyObject) {
    on_favorite(t, c, s)
}
pub(crate) extern "C" fn key_trailer(t: *mut AnyObject, c: Sel, s: *mut AnyObject) {
    on_trailer(t, c, s)
}
pub(crate) extern "C" fn key_watched(t: *mut AnyObject, c: Sel, s: *mut AnyObject) {
    on_watched(t, c, s)
}

/// Classes used by the page. Called once at startup, with the other classes.
pub(crate) unsafe fn register_detail_classes() {
    let mut h = ClassBuilder::new(c"JRDetailHost", class!(NSView)).unwrap();
    h.add_method(sel!(isFlipped), always_yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    h.add_method(sel!(detailBack:), on_back as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailPlay:), on_play as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailFavorite:), on_favorite as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailTrailer:), on_trailer as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailSimilar:), on_similar as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailWatched:), on_watched as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailCast:), on_cast as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailSeason:), on_season as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailEpisode:), on_episode as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.add_method(sel!(detailSkipIntros:), on_skip_intros as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject));
    h.register();
    let mut d = ClassBuilder::new(c"JRDetailDoc", class!(NSView)).unwrap();
    d.add_method(sel!(isFlipped), always_yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    d.add_method(sel!(setFrameSize:), doc_set_frame as extern "C" fn(*mut AnyObject, Sel, NSSize));
    d.add_method(
        sel!(viewDidChangeEffectiveAppearance),
        doc_appearance_changed as extern "C" fn(*mut AnyObject, Sel),
    );
    d.register();
    let mut f = ClassBuilder::new(c"JRDetailFlip", class!(NSView)).unwrap();
    f.add_method(sel!(isFlipped), always_yes as extern "C" fn(*mut AnyObject, Sel) -> Bool);
    f.register();
}

/// Create the (hidden) page inside the content host. `frame` is the library area,
/// taken to the top of the window so the backdrop runs under the titlebar.
pub(crate) unsafe fn build(content_host: *mut AnyObject, frame: NSRect) {
    let ui = UI.get_mut().unwrap();
    let host_cls = AnyClass::get(c"JRDetailHost").unwrap();
    let host: *mut AnyObject = msg_send![host_cls, alloc];
    let host: *mut AnyObject = msg_send![host, initWithFrame: frame];
    set_autoresize(host, RESIZE_W | RESIZE_H);
    set_hidden(host, true);
    add_sub(content_host, host);
    autorel(host);
    ui.detail.host = Ptr(host);

    let sc: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let sc: *mut AnyObject = msg_send![sc, initWithFrame: rect(0., 0., frame.size.w, frame.size.h)];
    let _: () = msg_send![sc, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![sc, setDrawsBackground: Bool::YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    add_sub(host, sc);
    autorel(sc);
    ui.detail.scroll = Ptr(sc);

    let doc_cls = AnyClass::get(c"JRDetailDoc").unwrap();
    let doc: *mut AnyObject = msg_send![doc_cls, alloc];
    let doc: *mut AnyObject = msg_send![doc, initWithFrame: rect(0., 0., frame.size.w, 10.)];
    set_autoresize(doc, RESIZE_W);
    let _: () = msg_send![sc, setDocumentView: doc];
    autorel(doc);
    ui.detail.doc = Ptr(doc);

    // Back sits in the titlebar band, over the backdrop, on a dark glass pill.
    let back: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns("Back"), target: host, action: sel!(detailBack:)];
    let _: () = msg_send![back, setBordered: Bool::NO];
    let _: () = msg_send![back, setWantsLayer: Bool::YES];
    let chev = symbol("chevron.left", 13., Weight::Medium);
    if !chev.is_null() {
        let _: () = msg_send![back, setImage: chev];
    }
    let _: () = msg_send![back, setImagePosition: 7isize]; // leading
    let _: () = msg_send![back, setContentTintColor: ns_white(1.0)];
    let _: () = msg_send![back, setFont: font(T_CALLOUT)];
    let _: () = msg_send![back, sizeToFit];
    let f: NSRect = msg_send![back, frame];
    let bw = f.size.w.ceil() + SP3;
    let bh = 28.;
    set_frame(back, rect(EDGE - SP2, (TITLEBAR_H - bh) / 2., bw, bh));
    set_corner(layer_of(back), bh / 2.);
    let _: () = msg_send![layer_of(back), setBackgroundColor: cg_rgba(0., 0., 0., 0.4)];
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
    let Some(ui) = UI.get_mut() else { return };
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
            if t.fit {
                fit_logo(t.art, img);
            }
        }
    }
}

unsafe fn set_art(art: *mut AnyObject, glyph: *mut AnyObject, img: *mut AnyObject) {
    let cg = cg_image(img);
    if cg.is_null() {
        return;
    }
    let layer = layer_of(art);
    tx_begin_instant();
    let _: () = msg_send![layer, setContents: cg as *mut AnyObject];
    tx_end();
    if !glyph.is_null() {
        set_hidden(glyph, true);
    }
}

/// Narrow a title logo to the shape of its image, keeping the box height, so it sits flush left.
unsafe fn fit_logo(art: *mut AnyObject, img: *mut AnyObject) {
    let s: NSSize = msg_send![img, size];
    if s.w <= 0. || s.h <= 0. {
        return;
    }
    let f = frame_of(art);
    let w = (f.size.h * s.w / s.h).min(f.size.w).max(1.);
    set_frame(art, rect(f.origin.x, f.origin.y, w, f.size.h));
}

/// Register an artwork view for `key` and fill it now if the image is cached.
unsafe fn bind(ui: &mut Ui, key: Option<u64>, art: *mut AnyObject, glyph: *mut AnyObject) {
    bind_with(ui, key, art, glyph, false);
}

unsafe fn bind_with(ui: &mut Ui, key: Option<u64>, art: *mut AnyObject, glyph: *mut AnyObject, fit: bool) {
    let Some(key) = key else { return };
    ui.detail.targets.push(Target { key, art, glyph, fit });
    if let Some(img) = ui.poster_cache.get(&key).copied() {
        set_art(art, glyph, img);
        if fit {
            fit_logo(art, img);
        }
    }
}

unsafe fn new_view(frame: NSRect) -> *mut AnyObject {
    let v: *mut AnyObject = msg_send![class!(NSView), alloc];
    let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
    autorel(v)
}

unsafe fn flip_view(frame: NSRect) -> *mut AnyObject {
    let cls = AnyClass::get(c"JRDetailFlip").unwrap();
    let v: *mut AnyObject = msg_send![cls, alloc];
    let v: *mut AnyObject = msg_send![v, initWithFrame: frame];
    autorel(v)
}

/// Layer-backed rounded rectangle that shows artwork (aspect-fill) over a fill colour.
unsafe fn art_view(frame: NSRect, radius: f64) -> *mut AnyObject {
    let v = new_view(frame);
    let layer = layer_of(v);
    set_corner(layer, radius);
    let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
    let _: () = msg_send![layer, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![layer, setBackgroundColor: cg(C_FILL_SEL)];
    v
}

/// Centred placeholder glyph inside `frame`.
unsafe fn glyph_in(frame: NSRect, name: &str, pt: f64) -> *mut AnyObject {
    let g: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let g: *mut AnyObject = msg_send![g, initWithFrame: rect(
        frame.origin.x + (frame.size.w - pt) / 2., frame.origin.y + (frame.size.h - pt) / 2., pt, pt)];
    let img = symbol(name, pt, Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![g, setImage: img];
    }
    let _: () = msg_send![g, setContentTintColor: color(C_TEXT3)];
    autorel(g)
}

/// Height a wrapping label needs at `width`.
unsafe fn text_height(v: *mut AnyObject, width: f64) -> f64 {
    let cell: *mut AnyObject = msg_send![v, cell];
    let sz: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., width, 1.0e6)];
    sz.h.ceil()
}

/// Height of `n` lines of `t`, from the font's metrics.
unsafe fn lines_height(t: TypeStyle, n: usize) -> f64 {
    let f = font(t);
    let asc: f64 = msg_send![f, ascender];
    let desc: f64 = msg_send![f, descender];
    let lead: f64 = msg_send![f, leading];
    (n as f64 * (asc - desc + lead)).ceil()
}

/// Height of two lines of `t`, from the font's metrics.
unsafe fn two_lines(t: TypeStyle) -> f64 {
    lines_height(t, 2)
}

/// `text` shortened to fit `lines` wrapped lines of `t` at `width`, ending in an ellipsis when
/// it is cut. Word wrap alone would cut the last line mid-word, and truncation alone would stop
/// the text wrapping, so the words are counted out until they fit.
unsafe fn fit_lines(text: &str, t: TypeStyle, width: f64, lines: usize) -> String {
    let limit = lines_height(t, lines) + 1.;
    let fits = |s: &str| text_height(wrapping_label(s, t, C_TEXT, ALIGN_LEFT), width) <= limit;
    if fits(text) {
        return text.to_string();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    // The longest run of leading words that still fits with an ellipsis after it.
    let (mut lo, mut hi) = (0usize, words.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if fits(&format!("{}…", words[..mid].join(" "))) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}…", words[..lo].join(" "))
}

/// Limit a wrapping label to `lines` lines. Words wrap across those lines; callers that
/// might overflow pass the text through `fit_lines` first so it ends in an ellipsis.
unsafe fn clamp_lines(v: *mut AnyObject, lines: isize) {
    let _: () = msg_send![v, setMaximumNumberOfLines: lines];
    let cell: *mut AnyObject = msg_send![v, cell];
    let _: () = msg_send![cell, setLineBreakMode: 0usize]; // word wrapping
}

unsafe fn place_text(doc: *mut AnyObject, text: &str, t: TypeStyle, c: Col, frame: NSRect) {
    let l = label(text, t, c, ALIGN_LEFT);
    set_frame(l, frame);
    add_sub(doc, l);
}

/// White with the given alpha (the hero sits on artwork, so its text ignores the appearance).
unsafe fn ns_white(a: f64) -> *mut AnyObject {
    msg_send![class!(NSColor), colorWithWhite: 1.0f64, alpha: a]
}

/// An sRGB colour as a CoreGraphics colour, for layers.
unsafe fn cg_rgba(r: f64, g: f64, b: f64, a: f64) -> *mut CGColor {
    let c: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: r, green: g, blue: b, alpha: a];
    msg_send![c, CGColor]
}

/// Hero text colour: white, at `a` opacity.
unsafe fn on_hero(v: *mut AnyObject, a: f64) {
    let _: () = msg_send![v, setTextColor: ns_white(a)];
}

/// Clear-to-solid gradient layer over `frame`. `stops` pairs a colour with its
/// location, from the start point to the end point.
unsafe fn gradient(frame: NSRect, stops: &[(*mut CGColor, f64)], start: NSPoint, end: NSPoint) -> *mut AnyObject {
    let g: *mut AnyObject = msg_send![class!(CAGradientLayer), layer];
    let _: () = msg_send![g, setFrame: frame];
    let cols: Vec<*mut AnyObject> = stops.iter().map(|s| s.0 as *mut AnyObject).collect();
    let locs: Vec<*mut AnyObject> = stops
        .iter()
        .map(|s| msg_send![class!(NSNumber), numberWithDouble: s.1])
        .collect();
    let arr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: cols.as_ptr(), count: cols.len()];
    let locs_arr: *mut AnyObject = msg_send![class!(NSArray), arrayWithObjects: locs.as_ptr(), count: locs.len()];
    let _: () = msg_send![g, setColors: arr];
    let _: () = msg_send![g, setLocations: locs_arr];
    let _: () = msg_send![g, setStartPoint: start];
    let _: () = msg_send![g, setEndPoint: end];
    g
}

/// Dark scrims over the backdrop: from the left for the text, and a fade into the
/// canvas at the foot. The fade is the canvas colour itself, so it meets the page.
unsafe fn add_scrims(layer: *mut AnyObject, w: f64, h: f64) {
    let frame = rect(0., 0., w, h);
    let left = gradient(
        frame,
        &[(cg_rgba(0., 0., 0., 0.7), 0.), (cg_rgba(0., 0., 0., 0.), 1.)],
        NSPoint { x: 0., y: 0.5 },
        NSPoint { x: 0.85, y: 0.5 },
    );
    let _: () = msg_send![layer, addSublayer: left];
    let canvas_solid: *mut CGColor = msg_send![color(C_CANVAS), CGColor];
    let foot = gradient(
        frame,
        &[
            (cg_rgba(0., 0., 0., 0.), 0.),
            (cg_rgba(0., 0., 0., 0.9), 0.8),
            (canvas_solid, 1.),
        ],
        NSPoint { x: 0., y: 1. },
        NSPoint { x: 0., y: 0. },
    );
    let _: () = msg_send![layer, addSublayer: foot];
}

/// Title text for the hero: the largest style whose two lines hold the name at `w`.
/// Returns the label and its height.
unsafe fn hero_title(text: &str, w: f64) -> (*mut AnyObject, f64) {
    for t in [T_HERO, T_HERO_M, T_HERO_S] {
        let v = wrapping_label(text, t, C_TEXT, ALIGN_LEFT);
        let h = text_height(v, w);
        if h as f64 <= two_lines(t) + 1. {
            on_hero(v, 1.);
            return (v, h);
        }
    }
    // Still too long at the smallest size: two lines, ending in an ellipsis.
    let v = wrapping_label(&fit_lines(text, T_HERO_S, w, 2), T_HERO_S, C_TEXT, ALIGN_LEFT);
    clamp_lines(v, 2);
    on_hero(v, 1.);
    (v, two_lines(T_HERO_S))
}

/// The hero's facts in streaming order: year, rating, runtime, genres, scores.
fn hero_facts(item: &DetailView) -> String {
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
    if !item.genres.is_empty() {
        parts.push(item.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
    }
    if let Some(c) = item.community_rating {
        parts.push(format!("★ {:.1}", c));
    }
    if let Some(c) = item.critic_rating {
        parts.push(format!("Critics {:.0}%", c));
    }
    parts.join("  ·  ")
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

/// "S2:E4", or "E4" when the season is unknown, or "" when there is no number.
fn episode_code(season: Option<u32>, number: Option<u32>) -> String {
    match (season, number) {
        (Some(s), Some(n)) => format!("S{}:E{}", s, n),
        (None, Some(n)) => format!("E{}", n),
        _ => String::new(),
    }
}

/// Minutes left of a partly watched item, as "23m left" (None when under a minute or unknown).
fn minutes_left(pos_ticks: i64, run_ticks: Option<i64>) -> Option<String> {
    let run = run_ticks.filter(|r| *r > 0)?;
    let m = (run - pos_ticks).max(0) / 600_000_000;
    (m > 0).then(|| format!("{}m left", m))
}

/// The primary button: its title, and the watched fraction for a resume bar.
/// A series uses its Next Up episode (none gives no button). A movie plays, or resumes.
fn primary_for(item: &DetailView) -> Option<(String, Option<f64>)> {
    let (code, pos, run) = if item.item_type == "Series" {
        let n = item.next_up.as_ref()?;
        (episode_code(n.season, n.number), n.position_ticks, n.run_time_ticks)
    } else {
        (String::new(), if item.resume { item.resume_ticks } else { 0 }, item.run_time_ticks)
    };
    let resuming = pos > 0;
    let mut title = if resuming { "Resume".to_string() } else { "Play".to_string() };
    if !code.is_empty() {
        title = format!("{} {}", title, code);
    }
    if resuming {
        if let Some(left) = minutes_left(pos, run) {
            title = format!("{} · {}", title, left);
        }
    }
    let progress = (resuming && run.is_some_and(|r| r > 0)).then(|| progress_fraction(pos, run));
    Some((title, progress))
}

/// The item a Play or Resume starts: the Next Up episode of a series, else the item.
fn play_id(item: &DetailView) -> String {
    match (item.item_type.as_str(), item.next_up.as_ref()) {
        ("Series", Some(n)) => n.id.clone(),
        _ => item.id.clone(),
    }
}

/// A chip of quality or audio information: small text in an outline.
unsafe fn chip(text: &str) -> (*mut AnyObject, f64) {
    let l = label(text, T_CAPTION_EM, C_TEXT, ALIGN_CENTER);
    on_hero(l, 0.92);
    let _: () = msg_send![l, sizeToFit];
    let f: NSRect = msg_send![l, frame];
    let w = f.size.w.ceil() + 2. * SP2;
    let v = new_view(rect(0., 0., w, CHIP_H));
    let layer = layer_of(v);
    set_corner(layer, 4.);
    let _: () = msg_send![layer, setBorderWidth: 1.0f64];
    let _: () = msg_send![layer, setBorderColor: cg_rgba(1., 1., 1., 0.6)];
    set_frame(l, rect(SP2, (CHIP_H - f.size.h) / 2., f.size.w, f.size.h));
    add_sub(v, l);
    (v, w)
}

/// The main hero button: white, with black text, sized to its title and symbol.
unsafe fn primary_button(title: &str, target: *mut AnyObject) -> (*mut AnyObject, f64) {
    let b: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(title), target: target, action: sel!(detailPlay:)];
    let _: () = msg_send![b, setBordered: Bool::NO];
    let _: () = msg_send![b, setWantsLayer: Bool::YES];
    let img = symbol("play.fill", 13., Weight::Semibold);
    if !img.is_null() {
        let _: () = msg_send![b, setImage: img];
    }
    let _: () = msg_send![b, setImagePosition: 7isize];
    let _: () = msg_send![b, setFont: font(T_BODY_EM)];
    let black: *mut AnyObject = msg_send![class!(NSColor), colorWithSRGBRed: 0.0f64, green: 0.0f64, blue: 0.0f64, alpha: 1.0f64];
    let _: () = msg_send![b, setContentTintColor: black];
    let _: () = msg_send![b, setKeyEquivalent: ns("\r")];
    let _: () = msg_send![b, sizeToFit];
    let f: NSRect = msg_send![b, frame];
    let w = (f.size.w + 2. * SP6).ceil();
    set_frame(b, rect(0., 0., w, PRIMARY_H));
    let layer = layer_of(b);
    set_corner(layer, PRIMARY_H / 2.);
    let _: () = msg_send![layer, setBackgroundColor: cg_rgba(1., 1., 1., 1.)];
    (b, w)
}

/// A round glass icon button (no title); `tip` is its tooltip and accessibility label.
unsafe fn icon_button(name: &str, tip: &str, action: Sel, target: *mut AnyObject) -> *mut AnyObject {
    let b: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: target, action: action];
    let _: () = msg_send![b, setBordered: Bool::NO];
    let _: () = msg_send![b, setWantsLayer: Bool::YES];
    set_icon(b, name);
    let _: () = msg_send![b, setImagePosition: 1isize]; // image only
    let _: () = msg_send![b, setContentTintColor: ns_white(1.0)];
    let _: () = msg_send![b, setToolTip: ns(tip)];
    let _: () = msg_send![b, setAccessibilityLabel: ns(tip)];
    set_frame(b, rect(0., 0., ICON_D, ICON_D));
    let layer = layer_of(b);
    set_corner(layer, ICON_D / 2.);
    let _: () = msg_send![layer, setBackgroundColor: cg_rgba(1., 1., 1., 0.16)];
    b
}

/// Swap the symbol of a button (the heart, the watched tick) without rebuilding it.
unsafe fn set_icon(b: *mut AnyObject, name: &str) {
    let img = symbol(name, 16., Weight::Medium);
    if !img.is_null() {
        let _: () = msg_send![b, setImage: img];
    }
}

/// Lay the whole page out into the document for `ui.detail.item` at width `w`.
unsafe fn render(ui: &mut Ui) {
    let Some(item) = ui.detail.item.clone() else { return };
    let doc = ui.detail.doc.get();
    let clip: *mut AnyObject = msg_send![ui.detail.scroll.get(), contentView];
    let w = bounds_of(clip).size.w.max(320.);
    ui.detail.laid_w = w;
    ui.detail.targets.clear();
    let none: *mut AnyObject = msg_send![class!(NSArray), array];
    let _: () = msg_send![doc, setSubviews: none];
    let host = ui.detail.host.get();
    let h = build_page(ui, doc, host, &item, w);
    set_frame(doc, rect(0., 0., w, h));
}

unsafe fn build_page(ui: &mut Ui, doc: *mut AnyObject, host: *mut AnyObject, item: &DetailView, w: f64) -> f64 {
    let rw = (w - 2. * EDGE).max(160.);
    let hero_w = rw.min(HERO_TEXT_MAX);
    let text_w = rw.min(TEXT_MAX);
    let is_series = item.item_type == "Series";

    // Hero text, measured first so the backdrop can grow to hold it.
    let (title, title_h) = hero_title(&item.name, hero_w);
    let has_logo = item.logo_key.is_some();
    let title_block = if has_logo { LOGO_H } else { title_h };
    let facts = wrapping_label(&hero_facts(item), T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    on_hero(facts, 0.85);
    let facts_h = text_height(facts, hero_w);
    let chips: Vec<(*mut AnyObject, f64)> = item.badges.iter().map(|b| chip(b)).collect();
    let chips_h = if chips.is_empty() { 0. } else { SP3 + CHIP_H };
    let primary = primary_for(item);
    let resume_bar = primary.as_ref().and_then(|p| p.1).is_some();
    let row_h = PRIMARY_H + if resume_bar { SP2 + PROGRESS_H } else { 0. };
    let block_h = title_block + SP4 + facts_h as f64 + chips_h + SP6 + row_h;
    let hero_h = (w * 0.5).round().clamp(HERO_MIN, HERO_MAX).max(block_h + HERO_FOOT + HERO_TOP + SP4);

    // Full-bleed backdrop (the poster stands in when there is no backdrop), with its scrims.
    let hero = new_view(rect(0., 0., w, hero_h));
    add_sub(doc, hero);
    let hero_layer = layer_of(hero);
    let _: () = msg_send![hero_layer, setMasksToBounds: Bool::YES];
    let _: () = msg_send![hero_layer, setContentsGravity: ns("resizeAspectFill")];
    let _: () = msg_send![hero_layer, setBackgroundColor: cg(C_FILL_SEL)];
    bind(ui, item.backdrop_key.or(item.poster_key), hero, std::ptr::null_mut());
    add_scrims(hero_layer, w, hero_h);

    // Title: the fallback name first, then the logo over it, which hides the name once it arrives.
    let mut y = hero_h - HERO_FOOT - block_h;
    set_frame(title, rect(EDGE, y, hero_w, title_h));
    add_sub(doc, title);
    if has_logo {
        let logo = new_view(rect(EDGE, y, hero_w.min(LOGO_W), LOGO_H));
        let _: () = msg_send![layer_of(logo), setContentsGravity: ns("resizeAspect")];
        add_sub(doc, logo);
        bind_with(ui, item.logo_key, logo, title, true);
    }
    y += title_block + SP4;
    set_frame(facts, rect(EDGE, y, hero_w, facts_h as f64));
    add_sub(doc, facts);
    y += facts_h as f64;
    if !chips.is_empty() {
        y += SP3;
        let mut cx = EDGE;
        for (c, cw) in chips {
            set_frame(c, rect(cx, y, cw, CHIP_H));
            add_sub(doc, c);
            cx += cw + SP2;
        }
        y += CHIP_H;
    }
    y += SP6;

    // Actions: the primary Play or Resume, then round icon buttons.
    let mut bx = EDGE;
    if let Some((title, progress)) = primary.as_ref() {
        let (b, bw) = primary_button(title, host);
        set_frame(b, rect(bx, y, bw, PRIMARY_H));
        add_sub(doc, b);
        if let Some(frac) = progress {
            let track_y = y + PRIMARY_H + SP2;
            let track = new_view(rect(bx, track_y, bw, PROGRESS_H));
            set_corner(layer_of(track), PROGRESS_H / 2.);
            let _: () = msg_send![layer_of(track), setBackgroundColor: cg_rgba(1., 1., 1., 0.3)];
            add_sub(doc, track);
            let fill = new_view(rect(bx, track_y, bw * frac, PROGRESS_H));
            set_corner(layer_of(fill), PROGRESS_H / 2.);
            let _: () = msg_send![layer_of(fill), setBackgroundColor: cg(C_ACCENT)];
            add_sub(doc, fill);
        }
        bx += bw + SP3;
    }
    let trailer = icon_button("play.rectangle", "Watch the trailer", sel!(detailTrailer:), host);
    if item.trailer_urls.is_empty() {
        let _: () = msg_send![trailer, setEnabled: Bool::NO];
        let _: () = msg_send![trailer, setToolTip: ns("No trailer for this title")];
    }
    let fav_name = if item.is_favorite { "heart.fill" } else { "heart" };
    let fav_tip = if item.is_favorite { "Remove from Favorites" } else { "Add to Favorites" };
    let fav = icon_button(fav_name, fav_tip, sel!(detailFavorite:), host);
    let watched_name = if item.played { "checkmark.circle.fill" } else { "checkmark.circle" };
    let watched_tip = if item.played { "Mark as unwatched" } else { "Mark as watched" };
    let watched = icon_button(watched_name, watched_tip, sel!(detailWatched:), host);
    let cast = icon_button("tv", "Cast to a device", sel!(detailCast:), host);
    for b in [trailer, fav, watched, cast] {
        set_frame(b, rect(bx, y, ICON_D, ICON_D));
        add_sub(doc, b);
        bx += ICON_D + SP2;
    }

    // Below the hero: tagline, overview, then the options for a series.
    let mut y = hero_h + SP6;
    if let Some(t) = item.tagline.as_deref() {
        let v = wrapping_label(t, T_HEADLINE, C_TEXT, ALIGN_LEFT);
        let th = text_height(v, text_w) as f64;
        set_frame(v, rect(EDGE, y, text_w, th));
        add_sub(doc, v);
        y += th + SP3;
    }
    if let Some(ov) = item.overview.as_deref() {
        let v = wrapping_label(ov, T_BODY, C_TEXT2, ALIGN_LEFT);
        let th = text_height(v, text_w) as f64;
        set_frame(v, rect(EDGE, y, text_w, th));
        add_sub(doc, v);
        y += th + SP5;
    }
    if is_series {
        let on = crate::config::skip_intros_for(&item.id, ui.cfg.settings.skip_intros_default);
        let cb: *mut AnyObject = msg_send![class!(NSButton), checkboxWithTitle: ns("Skip intros automatically"), target: host, action: sel!(detailSkipIntros:)];
        let _: () = msg_send![cb, setState: on as NSInteger];
        let _: () = msg_send![cb, setToolTip: ns("Skip intros as soon as they start, for this show on this Mac.")];
        let _: () = msg_send![cb, sizeToFit];
        let f: NSRect = msg_send![cb, frame];
        set_frame(cb, rect(EDGE, y, f.size.w, f.size.h));
        add_sub(doc, cb);
        y += f.size.h;
    }
    let mut sy = y + SP8;

    if is_series {
        sy = build_seasons(ui, doc, host, w, sy) + SP8;
    }

    // Cast & Crew: one horizontally scrolling row of round portraits.
    if !item.people.is_empty() {
        sy = section_head(doc, "Cast & Crew", w, sy);
        let n = item.people.len();
        let inner_w = EDGE * 2. + n as f64 * CAST_W + (n as f64 - 1.) * CAST_GAP;
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
            let name = wrapping_label(&fit_lines(&p.name, T_CALLOUT, CAST_W, 2), T_CALLOUT, C_TEXT, ALIGN_CENTER);
            clamp_lines(name, 2);
            let _: () = msg_send![name, setToolTip: ns(&p.name)];
            set_frame(name, rect(0., AVATAR + SP2, CAST_W, 32.));
            add_sub(cell, name);
            let role_text = fit_lines(p.role.as_deref().unwrap_or(""), T_CAPTION, CAST_W, 2);
            let role = wrapping_label(&role_text, T_CAPTION, C_TEXT2, ALIGN_CENTER);
            clamp_lines(role, 2);
            set_frame(role, rect(0., AVATAR + SP2 + 32. + SP1, CAST_W, 30.));
            add_sub(cell, role);
        }
        scroll_row(doc, w, sy, CAST_H + SP4, inner);
        sy += CAST_H + SP4 + SP8;
    }

    // More Like This: a row of similar posters; a click opens that title.
    if !item.similar.is_empty() {
        sy = section_head(doc, "More Like This", w, sy);
        let n = item.similar.len();
        let inner_w = EDGE * 2. + n as f64 * SIM_W + (n as f64 - 1.) * CAST_GAP;
        let inner = flip_view(rect(0., 0., inner_w, SIM_CELL_H));
        for (i, s) in item.similar.iter().enumerate() {
            let x = EDGE + i as f64 * (SIM_W + CAST_GAP);
            let cell = flip_view(rect(x, 0., SIM_W, SIM_CELL_H));
            add_sub(inner, cell);
            let art = art_view(rect(0., 0., SIM_W, SIM_H), R_POSTER);
            add_sub(cell, art);
            let g = glyph_in(rect(0., 0., SIM_W, SIM_H), "film", 36.);
            add_sub(cell, g);
            bind(ui, s.image_key, art, g);
            let name = label(&s.name, T_CALLOUT, C_TEXT, ALIGN_LEFT);
            let _: () = msg_send![name, setToolTip: ns(&s.name)];
            set_frame(name, rect(0., SIM_H + SP2, SIM_W, 18.));
            add_sub(cell, name);
            let year = s.year.map(|y| y.to_string()).unwrap_or_default();
            place_text(cell, &year, T_CAPTION, C_TEXT2, rect(0., SIM_H + SP2 + 18. + SP1, SIM_W, 15.));
            // A transparent button over the whole cell opens that title; added last so it sits on top.
            let hit: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: host, action: sel!(detailSimilar:)];
            let _: () = msg_send![hit, setBordered: Bool::NO];
            let _: () = msg_send![hit, setTag: i as isize];
            let _: () = msg_send![hit, setToolTip: ns(&s.name)];
            set_frame(hit, rect(0., 0., SIM_W, SIM_CELL_H));
            add_sub(cell, hit);
        }
        scroll_row(doc, w, sy, SIM_CELL_H + SP4, inner);
        sy += SIM_CELL_H + SP4 + SP8;
    }
    sy
}

/// A section heading at `y`. Returns the y below it.
unsafe fn section_head(doc: *mut AnyObject, title: &str, w: f64, y: f64) -> f64 {
    let head = label(title, T_TITLE, C_TEXT, ALIGN_LEFT);
    set_frame(head, rect(EDGE, y, w - 2. * EDGE, HEAD_H));
    add_sub(doc, head);
    y + HEAD_H + SP4
}

/// A horizontally scrolling row at `y` holding `inner`, the row's document view.
unsafe fn scroll_row(doc: *mut AnyObject, w: f64, y: f64, row_h: f64, inner: *mut AnyObject) {
    let row: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let row: *mut AnyObject = autorel(msg_send![row, initWithFrame: rect(0., y, w, row_h)]);
    let _: () = msg_send![row, setHasHorizontalScroller: Bool::YES];
    let _: () = msg_send![row, setHasVerticalScroller: Bool::NO];
    let _: () = msg_send![row, setDrawsBackground: Bool::NO];
    set_autoresize(row, RESIZE_W);
    let _: () = msg_send![row, setDocumentView: inner];
    add_sub(doc, row);
}

/// Season picker and the selected season's episodes. Returns the y below them.
unsafe fn build_seasons(ui: &mut Ui, doc: *mut AnyObject, host: *mut AnyObject, w: f64, mut y: f64) -> f64 {
    let st = ui.detail.seasons.clone();
    let next_id = ui.detail.item.as_ref().and_then(|i| i.next_up.as_ref()).map(|n| n.id.clone());
    let content_w = w - 2. * EDGE;
    let head_h = HEAD_H;
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
        set_frame(v, rect(EDGE, y, content_w, th as f64));
        add_sub(doc, v);
        return y + th as f64 + SP8;
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
                let next = next_id.as_deref() == Some(ep.id.as_str());
                y = episode_row(ui, doc, host, ep, i, w, y, next);
            }
            y
        }
    }
}

/// Season popup: lists the seasons; choosing one loads its episodes.
unsafe fn season_popup(st: &SeasonsUi, host: *mut AnyObject) -> *mut AnyObject {
    let pop: *mut AnyObject = msg_send![class!(NSPopUpButton), alloc];
    let pop: *mut AnyObject = msg_send![pop, initWithFrame: rect(0., 0., 160., 24.), pullsDown: Bool::NO];
    let names: *mut AnyObject = msg_send![class!(NSMutableArray), array];
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

/// One episode row: a 16:9 thumbnail (progress bar underneath), "E3 · Title", runtime,
/// overview snippet, and a check when watched. The Next Up episode gets a tinted
/// row with an accent bar and "Up next". A transparent button over the whole row
/// plays it; it is added last so it sits on top. Returns the y below the row.
unsafe fn episode_row(
    ui: &mut Ui,
    doc: *mut AnyObject,
    host: *mut AnyObject,
    ep: &EpisodeRow,
    idx: usize,
    w: f64,
    y: f64,
    next: bool,
) -> f64 {
    let rw = w - 2. * EDGE;
    let text_x = ROW_PAD + THUMB_W + SP4;
    let check_w = if ep.played { 18. } else { 0. };
    let text_w = (rw - text_x - ROW_PAD - check_w - SP2).max(120.);
    let title = match ep.number {
        Some(n) => format!("E{} · {}", n, ep.name),
        None => ep.name.clone(),
    };
    let snippet = ep.overview.as_deref().map(|o| overview_snippet(o, SNIPPET_CHARS)).filter(|s| !s.is_empty());
    let overview = snippet.as_ref().map(|t| {
        let v = wrapping_label(t, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
        (v, text_height(v, text_w) as f64)
    });
    let overview_h = overview.map(|(_, h)| SP1 + h).unwrap_or(0.);
    let text_h = TITLE_H + SP1 + META_H + overview_h;
    let row_h = (text_h.max(THUMB_H) + 2. * ROW_PAD).ceil();

    let row = flip_view(rect(EDGE, y, rw, row_h));
    if next {
        add_sub(row, surface(rect(0., 0., rw, row_h), C_FILL_ACTIVE, R_ROW));
        let bar = new_view(rect(0., ROW_PAD, 3., row_h - 2. * ROW_PAD));
        let _: () = msg_send![layer_of(bar), setBackgroundColor: cg(C_ACCENT)];
        add_sub(row, bar);
    } else {
        add_sub(row, surface(rect(0., 0., rw, row_h), C_CARD, R_ROW));
    }

    let thumb = art_view(rect(ROW_PAD, ROW_PAD, THUMB_W, THUMB_H), R_THUMB);
    add_sub(row, thumb);
    let glyph = glyph_in(rect(ROW_PAD, ROW_PAD, THUMB_W, THUMB_H), "play.rectangle", 28.);
    add_sub(row, glyph);
    bind(ui, ep.image_key, thumb, glyph);
    if let Some(frac) = progress_fraction_of(ep) {
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
    let mut meta = episode_meta(ep);
    if next {
        meta = if meta.is_empty() { "Up next".to_string() } else { format!("Up next  ·  {}", meta) };
    }
    if !meta.is_empty() {
        let m = label(&meta, T_CALLOUT, if next { C_ACCENT } else { C_TEXT2 }, ALIGN_LEFT);
        set_frame(m, rect(text_x, ROW_PAD + TITLE_H + SP1, text_w, META_H));
        add_sub(row, m);
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

    let hit: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: host, action: sel!(detailEpisode:)];
    let _: () = msg_send![hit, setBordered: Bool::NO];
    let _: () = msg_send![hit, setTag: idx as isize];
    let _: () = msg_send![hit, setToolTip: ns(&title)];
    set_frame(hit, rect(0., 0., rw, row_h));
    add_sub(row, hit);

    add_sub(doc, row);
    y + row_h + SP2
}

/// Fraction watched for the progress bar: None when the episode is watched or unstarted.
fn progress_fraction_of(ep: &EpisodeRow) -> Option<f64> {
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

    fn movie() -> DetailView {
        DetailView {
            id: "m".into(),
            name: "Movie".into(),
            item_type: "Movie".into(),
            year: Some(2010),
            official_rating: Some("PG-13".into()),
            community_rating: Some(7.8),
            run_time_ticks: Some(72_000_000_000),
            genres: vec!["Drama".into(), "Mystery".into(), "Thriller".into(), "Action".into()],
            overview: None,
            played: false,
            resume: false,
            resume_ticks: 0,
            poster_key: None,
            backdrop_key: None,
            logo_key: None,
            people: vec![],
            trailer_urls: vec![],
            tagline: None,
            critic_rating: Some(91.4),
            badges: vec![],
            is_favorite: false,
            similar: vec![],
            next_up: None,
        }
    }

    fn next(pos: i64) -> NextUpEpisode {
        NextUpEpisode { id: "e4".into(), name: "E".into(), season: Some(2), number: Some(4), position_ticks: pos, run_time_ticks: Some(36_000_000_000) }
    }

    #[test]
    fn snippet_keeps_short_text_and_collapses_whitespace() {
        assert_eq!(overview_snippet("  A  quiet\n\nstart. ", 180), "A quiet start.");
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
        assert_eq!(progress_fraction_of(&ep), Some(0.25));
        assert_eq!(episode_meta(&ep), "20m  ·  15m left");
        ep.played = true;
        assert_eq!(progress_fraction_of(&ep), None);
        assert_eq!(episode_meta(&ep), "20m");
        ep.played = false;
        ep.position_ticks = 0;
        assert_eq!(progress_fraction_of(&ep), None);
        ep.run_time_ticks = None;
        assert_eq!(progress_fraction_of(&ep), None);
        assert_eq!(episode_meta(&ep), "");
    }

    #[test]
    fn episode_codes_read_like_streaming_apps() {
        assert_eq!(episode_code(Some(1), Some(1)), "S1:E1");
        assert_eq!(episode_code(None, Some(3)), "E3");
        assert_eq!(episode_code(Some(2), None), "");
    }

    #[test]
    fn minutes_left_needs_a_whole_minute() {
        assert_eq!(minutes_left(60 * 600_000_000 - 1, Some(60 * 600_000_000)), None);
        assert_eq!(minutes_left(0, Some(60 * 600_000_000)), Some("60m left".into()));
        assert_eq!(minutes_left(37 * 600_000_000, Some(60 * 600_000_000)), Some("23m left".into()));
        assert_eq!(minutes_left(5, None), None);
    }

    #[test]
    fn movie_button_plays_or_resumes_with_a_bar() {
        let mut m = movie();
        assert_eq!(primary_for(&m), Some(("Play".into(), None)));
        m.resume = true;
        m.resume_ticks = 37 * 600_000_000;
        let (title, bar) = primary_for(&m).unwrap();
        assert_eq!(title, "Resume · 83m left");
        assert!((bar.unwrap() - 37. / 120.).abs() < 1e-9);
    }

    #[test]
    fn series_button_uses_next_up() {
        let mut s = movie();
        s.item_type = "Series".into();
        assert_eq!(primary_for(&s), None);
        s.next_up = Some(next(0));
        assert_eq!(primary_for(&s), Some(("Play S2:E4".into(), None)));
        assert_eq!(play_id(&s), "e4");
        s.next_up = Some(next(37 * 600_000_000));
        let (title, bar) = primary_for(&s).unwrap();
        assert_eq!(title, "Resume S2:E4 · 23m left");
        assert!(bar.is_some());
    }

    #[test]
    fn hero_facts_run_in_streaming_order() {
        let m = movie();
        assert_eq!(hero_facts(&m), "2010  ·  PG-13  ·  2h 00m  ·  Drama, Mystery, Thriller  ·  ★ 7.8  ·  Critics 91%");
        let mut bare = movie();
        bare.year = None;
        bare.official_rating = None;
        bare.run_time_ticks = None;
        bare.genres.clear();
        bare.community_rating = None;
        bare.critic_rating = None;
        assert_eq!(hero_facts(&bare), "");
    }
}

// ---- Introspection for the self-test (read-only) ---------------------------

/// Whether the page's artwork view for `key` has its image: None if the page has no such view.
pub fn art_filled(key: u64) -> Option<bool> {
    let ui = UI.get()?;
    let t = ui.detail.targets.iter().find(|t| t.key == key)?;
    unsafe {
        let layer: *mut AnyObject = msg_send![t.art, layer];
        let c: *mut AnyObject = msg_send![layer, contents];
        Some(!c.is_null())
    }
}

/// Number of artwork views laid out on the page (backdrop, logo, portraits, similar posters).
pub fn target_count() -> usize {
    UI.get().map(|u| u.detail.targets.len()).unwrap_or(0)
}

/// The document is exactly as wide as the visible part of the scroll view.
pub fn layout_matches_clip() -> bool {
    let Some(ui) = UI.get() else { return false };
    unsafe {
        let doc = ui.detail.doc.get();
        let clip: *mut AnyObject = msg_send![ui.detail.scroll.get(), contentView];
        let df: NSRect = msg_send![doc, frame];
        let cb: NSRect = msg_send![clip, bounds];
        (df.size.w - cb.size.w).abs() < 0.5 && df.size.h > 0.
    }
}
