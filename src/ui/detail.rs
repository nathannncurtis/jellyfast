//! Item detail page for movies and series, in the library's content area.
//!
//! It replaces the grid while open and Back returns to it. The page is a
//! flipped document inside a vertical scroll view: backdrop with a fade, the
//! poster overlapping it, title and facts, action buttons, the overview, and
//! a horizontally scrolling Cast & Crew row. The document is rebuilt whenever
//! the item, the width or the appearance changes. Artwork views are registered
//! as targets and filled from `Ui.poster_cache` when their key arrives. All
//! state lives in `Ui.detail`; the views are only touched on the main thread.
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
    laid_w: f64,
    targets: Vec<Target>,
}

impl DetailUi {
    pub fn new() -> Self {
        DetailUi {
            host: Ptr(std::ptr::null_mut()),
            scroll: Ptr(std::ptr::null_mut()),
            doc: Ptr(std::ptr::null_mut()),
            item: None,
            open: false,
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

extern "C" fn on_episodes(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let Some(ui) = UI.get_mut() else { return };
        let Some(item) = ui.detail.item.as_ref() else { return };
        let (id, name) = (item.id.clone(), item.name.clone());
        close(ui);
        crate::platform::appkit::set_busy(true);
        let _ = ui.ui_tx.send(UiCmd::Episodes { series_id: id, series_name: name });
    }
}

/// Classes used by the page. Called once at startup, with the other classes.
pub(crate) unsafe fn register_detail_classes() {
    let mut h = ClassDecl::new("JRDetailHost", class!(NSView)).unwrap();
    h.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
    h.add_method(sel!(detailBack:), on_back as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailPlay:), on_play as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailWatched:), on_watched as extern "C" fn(&mut Object, Sel, *mut Object));
    h.add_method(sel!(detailEpisodes:), on_episodes as extern "C" fn(&mut Object, Sel, *mut Object));
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
        ui.detail.item = Some(view);
        ui.detail.open = true;
        ui.detail.laid_w = 0.;
        set_hidden(ui.header.get(), true);
        set_hidden(ui.lib_scroll.get(), true);
        set_hidden(ui.state_host.get(), true);
        set_hidden(ui.detail.host.get(), false);
        render(ui);
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
    ui.detail.targets.clear();
    set_hidden(ui.detail.host.get(), true);
    set_hidden(ui.header.get(), false);
    set_hidden(ui.lib_scroll.get(), false);
}

pub fn is_open() -> bool {
    UI.get().map(|u| u.detail.open).unwrap_or(false)
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
    bx += bw + SP2;
    if is_series {
        let (b, bw) = action_button("Episodes", "list.bullet", sel!(detailEpisodes:), host, true);
        set_frame(b, rect(bx, y, bw, BUTTON_H));
        add_sub(doc, b);
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
