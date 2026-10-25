#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, Sel, NO, YES};
use objc::{sel, sel_impl};
use std::os::raw::c_void;

// ---- Grid maths (pure; unit-tested) --------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cols: usize,
    pub tile_w: f64,
    pub poster_h: f64,
    pub cell_h: f64,
}

/// `w` is the grid view's width; `ratio` is poster height / width.
pub fn grid_metrics(w: f64, ratio: f64) -> Metrics {
    let avail = (w - 2. * EDGE).max(TILE_MIN_W);
    let cols = (((avail + GRID_COL_GAP) / (TILE_MIN_W + GRID_COL_GAP)).floor() as usize).max(1);
    let tile_w = ((avail - GRID_COL_GAP * (cols as f64 - 1.)) / cols as f64).floor();
    let poster_h = (tile_w * ratio).round();
    Metrics { cols, tile_w, poster_h, cell_h: poster_h + CELL_TEXT_H }
}

/// Poster height / width for a whole collection: the most common `aspect`
/// (width / height) when present, otherwise by item type. One value per grid
/// keeps rows aligned.
pub fn grid_ratio(items: &[ListItem]) -> f64 {
    use std::collections::HashMap;
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for it in items {
        if let Some(a) = it.aspect.filter(|a| *a > 0.2 && *a < 4.0) {
            *counts.entry((a * 100.).round() as i64).or_default() += 1;
        }
    }
    if let Some((k, _)) = counts.iter().max_by_key(|(k, c)| (**c, **k)) {
        return (100. / *k as f64).clamp(0.5, 1.8);
    }
    match items.first().map(|i| i.item_type.as_str()) {
        Some("MusicAlbum") | Some("Audio") | Some("MusicArtist") => 1.0,
        Some("Episode") | Some("LiveTvChannel") => 9. / 16.,
        _ => POSTER_ASPECT,
    }
}

fn placeholder_symbol(item_type: &str) -> &'static str {
    match item_type {
        "Series" | "Episode" | "Season" => "tv",
        "MusicAlbum" | "Audio" => "music.note",
        "MusicArtist" | "Person" => "person.crop.square",
        "LiveTvChannel" => "antenna.radiowaves.left.and.right",
        _ => "film",
    }
}

// ---- Poster grid ---------------------------------------------------------------
pub(crate) struct CellData {
    shadow: *mut Object,
    dim: *mut Object,
    circle: *mut Object,
    play_icon: *mut Object,
}

unsafe fn new_layer() -> *mut Object {
    msg_send![class!(CALayer), layer]
}
unsafe fn add_layer(parent: *mut Object, child: *mut Object) {
    let _: () = msg_send![parent, addSublayer: child];
}
unsafe fn palette_symbol(name: &str, pt: f64, colors: &[*mut Object]) -> *mut Object {
    let img: *mut Object =
        msg_send![class!(NSImage), imageWithSystemSymbolName: ns(name) accessibilityDescription: nil()];
    if img.is_null() {
        return img;
    }
    let base: *mut Object = msg_send![class!(NSImageSymbolConfiguration),
        configurationWithPointSize: pt weight: Weight::Medium.value()];
    let arr: *mut Object = msg_send![class!(NSArray), arrayWithObjects: colors.as_ptr() count: colors.len()];
    let pal: *mut Object = msg_send![class!(NSImageSymbolConfiguration), configurationWithPaletteColors: arr];
    let cfg: *mut Object = msg_send![base, configurationByApplyingConfiguration: pal];
    let out: *mut Object = msg_send![img, imageWithSymbolConfiguration: cfg];
    if out.is_null() { img } else { out }
}

pub fn grid_set_items(items: &[ListItem]) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let grid = ui.grid.get();
        let scale: f64 = msg_send![ui.win.get(), backingScaleFactor];
        // Keep the same GridData box (the grid's ivar points at it); swap its cells.
        let mut gd = ui.grid_data.take().unwrap_or_else(|| Box::new(GridData { cells: vec![], ratio: POSTER_ASPECT }));
        {
            let d = &mut *gd;
            for c in &d.cells {
                free_cell_data(c.container);
                let _: () = msg_send![c.container, removeFromSuperview];
            }
            d.cells.clear();
            d.ratio = grid_ratio(items);
        }
        let m = grid_metrics(bounds_of(grid).size.w.max(200.), gd.ratio);
        tx_begin_instant();
        let mut cells = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            let cell_cls = *CELL_CLASS.get().unwrap();
            let container: *mut Object = msg_send![cell_cls, alloc];
            let container: *mut Object = msg_send![container, initWithFrame: rect(0., 0., m.tile_w, m.cell_h)];
            let root = layer_of(container);

            // Shadow host (no masking) + clipped art layer.
            let host = new_layer();
            let _: () = msg_send![host, setShadowColor: black_cg()];
            let _: () = msg_send![host, setShadowOpacity: SH_REST.opacity];
            let _: () = msg_send![host, setShadowRadius: SH_REST.radius];
            let _: () = msg_send![host, setShadowOffset: size(0., SH_REST.dy)];
            add_layer(root, host);
            let art = new_layer();
            set_corner(art, R_POSTER);
            let _: () = msg_send![art, setMasksToBounds: YES];
            let _: () = msg_send![art, setContentsGravity: ns("resizeAspectFill")];
            let _: () = msg_send![art, setContentsScale: scale];
            let _: () = msg_send![art, setBackgroundColor: cg(C_FILL_HOVER)];
            add_layer(host, art);

            // Progress: track + fill, clipped by the art layer.
            let frac = crate::jellyfin::urls::progress_fraction(
                it.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0),
                it.run_time_ticks,
            );
            let track = new_layer();
            let _: () = msg_send![track, setCornerRadius: 1.5f64];
            let _: () = msg_send![track, setBackgroundColor: cg_black(0.45)];
            let _: () = msg_send![track, setHidden: (frac <= 0.0) as objc::runtime::BOOL];
            add_layer(art, track);
            let fill = new_layer();
            let _: () = msg_send![fill, setCornerRadius: 1.5f64];
            let _: () = msg_send![fill, setBackgroundColor: cg(C_ACCENT)];
            add_layer(track, fill);

            // Hover dim + play affordance.
            let dim = new_layer();
            let _: () = msg_send![dim, setBackgroundColor: cg_black(0.18)];
            let _: () = msg_send![dim, setOpacity: 0.0f32];
            add_layer(art, dim);
            let circle = new_layer();
            let _: () = msg_send![circle, setCornerRadius: 22.0f64];
            let _: () = msg_send![circle, setBackgroundColor: cg_black(0.55)];
            let _: () = msg_send![circle, setOpacity: 0.0f32];
            add_layer(art, circle);

            // Placeholder glyph.
            let glyph: *mut Object = msg_send![class!(NSImageView), alloc];
            let glyph: *mut Object = msg_send![glyph, initWithFrame: rect(0., 0., 40., 40.)];
            let gi = symbol(placeholder_symbol(&it.item_type), 28., Weight::Regular);
            if !gi.is_null() {
                let _: () = msg_send![glyph, setImage: gi];
                let _: () = msg_send![glyph, setContentTintColor: color(C_TEXT3)];
            }
            add_sub(container, glyph);
            autorel(glyph);
            // Play icon (above the circle).
            let play: *mut Object = msg_send![class!(NSImageView), alloc];
            let play: *mut Object = msg_send![play, initWithFrame: rect(0., 0., 24., 24.)];
            let pi = symbol("play.fill", 18., Weight::Regular);
            if !pi.is_null() {
                let _: () = msg_send![play, setImage: pi];
                let _: () = msg_send![play, setContentTintColor: color(C_ON_MEDIA)];
            }
            let _: () = msg_send![play, setAlphaValue: 0.0f64];
            add_sub(container, play);
            autorel(play);
            // Watched badge.
            let badge: *mut Object = msg_send![class!(NSImageView), alloc];
            let badge: *mut Object = msg_send![badge, initWithFrame: rect(0., 0., 20., 20.)];
            let white = color(C_ON_MEDIA);
            let acc = color(C_ACCENT);
            let bimg = palette_symbol("checkmark.circle.fill", 16., &[white, acc]);
            if !bimg.is_null() {
                let _: () = msg_send![badge, setImage: bimg];
            }
            set_hidden(badge, !it.user_data.as_ref().map(|u| u.played).unwrap_or(false));
            add_sub(container, badge);
            autorel(badge);

            // Title + subtitle.
            let title = label(&it.name, T_BODY_EM, C_TEXT, ALIGN_LEFT);
            add_sub(container, title);
            let sub = label(&it.sub_label(), T_CAPTION, C_TEXT2, ALIGN_LEFT);
            add_sub(container, sub);

            // Click target over the whole cell.
            let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
                target: ui.ctl.get() action: sel!(itemClicked:)];
            let _: () = msg_send![btn, setBordered: NO];
            let _: () = msg_send![btn, setTag: i as NSInteger];
            let _: () = msg_send![btn, setToolTip: ns(&it.name)];
            add_sub(container, btn);
            // Context menu.
            let menu: *mut Object = msg_send![class!(NSMenu), alloc];
            let menu: *mut Object = msg_send![menu, initWithTitle: ns(&it.name)];
            let id_ns = ns(&it.id);
            let played = it.user_data.as_ref().map(|u| u.played).unwrap_or(false);
            let m0: *mut Object = menu_item("Play", sel!(itemClicked:), "");
            let _: () = msg_send![m0, setTarget: ui.ctl.get()];
            let _: () = msg_send![m0, setTag: i as NSInteger];
            let _: () = msg_send![menu, addItem: m0];
            let (t1, s1) = if played {
                ("Mark as Unwatched", sel!(markUnwatched:))
            } else {
                ("Mark as Watched", sel!(markWatched:))
            };
            let m1: *mut Object = menu_item(t1, s1, "");
            let _: () = msg_send![m1, setTarget: ui.ctl.get()];
            let _: () = msg_send![m1, setRepresentedObject: id_ns];
            let _: () = msg_send![menu, addItem: m1];
            let sep: *mut Object = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];
            let m2: *mut Object = menu_item("Open in Jellyfin Web", sel!(openInWeb:), "");
            let _: () = msg_send![m2, setTarget: ui.ctl.get()];
            let _: () = msg_send![m2, setRepresentedObject: id_ns];
            let _: () = msg_send![menu, addItem: m2];
            let _: () = msg_send![btn, setMenu: menu];
            autorel(menu);

            let cd = Box::into_raw(Box::new(CellData { shadow: host, dim, circle, play_icon: play }));
            (*container).set_ivar::<*mut c_void>("jrCell", cd as *mut c_void);
            let ta: *mut Object = msg_send![class!(NSTrackingArea), alloc];
            let ta: *mut Object = msg_send![ta,
                initWithRect: rect(0., 0., m.tile_w, m.poster_h)
                options: (0x01usize | 0x80usize | 0x200usize) owner: container userInfo: nil()];
            let _: () = msg_send![container, addTrackingArea: ta];
            autorel(ta);
            let _: () = msg_send![grid, addSubview: container];
            autorel(container);
            let key = poster_key(&it.id);
            let mut handle = CellHandle {
                container,
                art,
                shadow: host,
                prog_track: track,
                prog_fill: fill,
                prog_frac: frac,
                dim,
                circle,
                glyph,
                play_icon: play,
                badge,
                title,
                sub,
                btn,
                key,
                meta: it.clone(),
            };
            if let Some(img) = ui.poster_cache.get(&key) {
                show_art(&mut handle, *img, false);
            }
            cells.push(handle);
        }
        tx_end();
        gd.cells = cells;
        ui.grid_data = Some(gd);
        // Re-point the ivar users at the (same) box, then lay out.
        let _: () = msg_send![grid, setNeedsLayout: YES];
    }
}

unsafe fn show_art(c: &mut CellHandle, img: *mut Object, animate: bool) {
    let cg = cg_image(img);
    if cg.is_null() {
        return;
    }
    if animate {
        tx_begin(M_STD);
    } else {
        tx_begin_instant();
    }
    let _: () = msg_send![c.art, setContents: cg];
    tx_end();
    set_hidden(c.glyph, true);
}

/// Free the per-cell hover box and null the ivar (cell may outlive removal briefly).
unsafe fn free_cell_data(container: *mut Object) {
    let cd = *(*container).get_ivar::<*mut c_void>("jrCell") as *mut CellData;
    if !cd.is_null() {
        drop(Box::from_raw(cd));
        (*container).set_ivar::<*mut c_void>("jrCell", std::ptr::null_mut());
    }
}

pub(crate) extern "C" fn cell_layout(this: &mut Object, _cmd: Sel) {
    // Geometry is owned by grid_layout; the cell only forwards to NSView.
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
    }
}

unsafe fn hover(this: &mut Object, on: bool) {
    let cd = *(*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;
    if cd.is_null() {
        return;
    }
    let cd = &*cd;
    tx_begin(if on { M_FAST } else { M_STD });
    let sh = if on { SH_HOVER } else { SH_REST };
    let _: () = msg_send![cd.shadow, setShadowOpacity: sh.opacity];
    let _: () = msg_send![cd.shadow, setShadowRadius: sh.radius];
    let _: () = msg_send![cd.shadow, setShadowOffset: size(0., sh.dy)];
    if !reduce_motion() {
        let t = if on { CATransform3DMakeScale(1.03, 1.03, 1.0) } else { CATransform3DIdentity };
        let _: () = msg_send![cd.shadow, setTransform: t];
    }
    let o = if on { 1.0f32 } else { 0.0f32 };
    let _: () = msg_send![cd.dim, setOpacity: o];
    let _: () = msg_send![cd.circle, setOpacity: o];
    tx_end();
    let anim: *mut Object = msg_send![cd.play_icon, animator];
    anim_begin(if on { M_FAST } else { M_STD }, timing_ease_out());
    let _: () = msg_send![anim, setAlphaValue: if on { 1.0f64 } else { 0.0f64 }];
    anim_end();
}
pub(crate) extern "C" fn cell_hover(this: &mut Object, _cmd: Sel, _ev: *mut Object) {
    unsafe { hover(this, true) }
}
pub(crate) extern "C" fn cell_unhover(this: &mut Object, _cmd: Sel, _ev: *mut Object) {
    unsafe { hover(this, false) }
}

/// Position every layer and subview of a cell for the given metrics.
unsafe fn layout_cell(c: &CellHandle, m: &Metrics) {
    let (w, h) = (m.tile_w, m.poster_h);
    // Host: bounds + position so a hover transform never fights the frame.
    let _: () = msg_send![c.shadow, setBounds: rect(0., 0., w, h)];
    let _: () = msg_send![c.shadow, setPosition: NSPoint { x: w / 2., y: h / 2. }];
    set_shadow_path(c.shadow, w, h, R_POSTER);
    let _: () = msg_send![c.art, setFrame: rect(0., 0., w, h)];
    let _: () = msg_send![c.dim, setFrame: rect(0., 0., w, h)];
    let _: () = msg_send![c.circle, setFrame: rect(w / 2. - 22., h / 2. - 22., 44., 44.)];
    let tw = w - 2. * SP2;
    let _: () = msg_send![c.prog_track, setFrame: rect(SP2, h - SP2 - 3., tw, 3.)];
    let _: () = msg_send![c.prog_fill, setFrame: rect(0., 0., tw * c.prog_frac, 3.)];
    set_frame(c.glyph, rect(w / 2. - 20., h / 2. - 20., 40., 40.));
    set_frame(c.play_icon, rect(w / 2. - 12., h / 2. - 12., 24., 24.));
    set_frame(c.badge, rect(w - SP2 - 20., SP2, 20., 20.));
    set_frame(c.title, rect(0., h + SP2, w, 17.));
    set_frame(c.sub, rect(0., h + SP2 + 18., w, 14.));
    set_frame(c.btn, rect(0., 0., w, m.cell_h));
}

pub(crate) extern "C" fn grid_layout(this: &mut Object, _cmd: Sel) {
    unsafe {
        let data = *(*this).get_ivar::<*mut c_void>("jrData") as *mut GridData;
        if data.is_null() {
            return;
        }
        let b = bounds_of(this as *mut Object);
        let w = b.size.w.max(200.0);
        let d = &mut *data;
        let m = grid_metrics(w, d.ratio);
        tx_begin_instant();
        for (i, c) in d.cells.iter().enumerate() {
            let col = i % m.cols;
            let row = i / m.cols;
            let x = EDGE + col as f64 * (m.tile_w + GRID_COL_GAP);
            let y = SP2 + row as f64 * (m.cell_h + GRID_ROW_GAP);
            set_frame(c.container, rect(x, y, m.tile_w, m.cell_h));
            layout_cell(c, &m);
        }
        tx_end();
        let rows = (d.cells.len() + m.cols - 1) / m.cols;
        let content_h = SP2 + rows as f64 * (m.cell_h + GRID_ROW_GAP);
        if (content_h - b.size.h).abs() > 1.0 {
            let _: () = msg_send![this as *mut Object, setFrameSize: size(b.size.w, content_h)];
        }
    }
}
pub(crate) extern "C" fn grid_set_frame(this: &mut Object, _cmd: Sel, s: NSSize) {
    unsafe {
        // Forward to NSView's setFrameSize:, then request a layout pass.
        let _: () = msg_send![super(this, class!(NSView)), setFrameSize: s];
        let _: () = msg_send![this, setNeedsLayout: YES];
    }
}
/// Dynamic colours on layers are resolved once; re-resolve when the appearance flips.
pub(crate) extern "C" fn grid_appearance_changed(this: &mut Object, _cmd: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), viewDidChangeEffectiveAppearance];
        let data = *(*this).get_ivar::<*mut c_void>("jrData") as *mut GridData;
        if data.is_null() {
            return;
        }
        tx_begin_instant();
        for c in &(*data).cells {
            let _: () = msg_send![c.art, setBackgroundColor: cg(C_FILL_HOVER)];
            let _: () = msg_send![c.prog_fill, setBackgroundColor: cg(C_ACCENT)];
        }
        tx_end();
    }
}

pub fn poster_key(item_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    item_id.hash(&mut h);
    h.finish()
}

pub fn poster_ready(key: u64, bytes: &[u8]) {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        let data: *mut Object = msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr() length: bytes.len() as NSUInteger];
        if data.is_null() {
            return;
        }
        let img: *mut Object = msg_send![class!(NSImage), alloc];
        let img: *mut Object = msg_send![img, initWithData: data];
        if img.is_null() {
            return; // undecodable bytes: the placeholder stays
        }
        if cg_image(img).is_null() {
            release_obj(img);
            return;
        }
        if let Some(old) = ui.poster_cache.insert(key, img) {
            release_obj(old);
        }
        // Update any visible cells with this key.
        if let Some(d) = ui.grid_data.as_mut() {
            for c in d.cells.iter_mut() {
                if c.key == key {
                    show_art(c, img, true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_fill_the_width() {
        for w in [700., 960., 1040., 1500., 2400.] {
            let m = grid_metrics(w, 1.5);
            let used = 2. * EDGE + m.cols as f64 * m.tile_w + (m.cols as f64 - 1.) * GRID_COL_GAP;
            assert!(used <= w + 0.001, "{w}: used {used}");
            assert!(m.tile_w >= TILE_MIN_W - 0.001 || m.cols == 1);
            assert!(w - used < m.cols as f64 + 1.0, "{w}: slack {}", w - used);
            assert_eq!(m.cell_h, m.poster_h + CELL_TEXT_H);
        }
    }
    #[test]
    fn metrics_min_width_one_column() {
        let m = grid_metrics(100., 1.0);
        assert_eq!(m.cols, 1);
        assert_eq!(m.poster_h, m.tile_w);
    }
    fn li(ty: &str, aspect: Option<f64>) -> ListItem {
        serde_json::from_value(serde_json::json!({
            "Id": "x", "Name": "n", "Type": ty, "PrimaryImageAspectRatio": aspect
        }))
        .unwrap()
    }
    #[test]
    fn ratio_follows_majority_aspect_then_type() {
        let a = li("Movie", Some(0.667));
        let b = li("Movie", Some(1.0));
        assert!((grid_ratio(&[a.clone(), a.clone(), b]) - 1.5).abs() < 0.01);
        assert_eq!(grid_ratio(&[li("MusicAlbum", None)]), 1.0);
        assert!((grid_ratio(&[li("Episode", None)]) - 9. / 16.).abs() < 1e-9);
        assert_eq!(grid_ratio(&[]), POSTER_ASPECT);
    }
}
