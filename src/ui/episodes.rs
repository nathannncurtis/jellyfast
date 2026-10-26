//! The player's Episodes panel: the current season's episodes in a side panel, each with its
//! 16:9 thumbnail, number, title and watched progress. Picking one jumps to it. It opens from the
//! Episodes button or E, sits between the top bar and the bottom bar, and pushes the Up Next card
//! and the skip button left while it is open (see `right_inset`).
#![allow(unsafe_code)]
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use crate::app::messages::SeasonEpisode;
use crate::platform::appkit::*;
use crate::ui::skip::{art_placeholder, hud_panel};
use crate::ui::theme::*;

/// Panel width. Other overlays move left by this plus `SP4` while the panel is open.
pub const PANEL_W: f64 = 340.;
/// Panel corner radius, matching the skip overlays.
const PANEL_R: f64 = 12.;
/// Header band with the season title and the close button.
const HEAD_H: f64 = 56.;
/// One episode row: a 16:9 thumbnail with padding around it.
const ROW_H: f64 = 96.;
const THUMB_W: f64 = 128.;
const THUMB_H: f64 = 72.;
/// Progress strip under the thumbnail.
const PROGRESS_H: f64 = 3.;
/// Gap between the panel and the window edges and the bars.
const INSET: f64 = SP4;
/// Text column: thumbnail, then a gap, then the text up to the panel's right padding.
const TEXT_X: f64 = SP3 + THUMB_W + SP3;

/// Episodes panel state, kept with the player chrome.
pub struct EpisodesUi {
    pub panel: Ptr,
    pub scroll: Ptr,
    pub title: Ptr,
    /// Season episodes of the current item, in order.
    pub items: Vec<SeasonEpisode>,
    /// Item id of the episode playing now (its row shows the "Playing" tag).
    pub current: String,
    /// Season number of the list, for the header.
    pub season: Option<u32>,
    /// Thumbnails still waiting for their still: (poster key, image view, placeholder glyph).
    pub thumbs: Vec<(u64, Ptr, Ptr)>,
    /// The panel is on screen.
    pub open: bool,
}

impl EpisodesUi {
    pub fn new() -> EpisodesUi {
        let n = || Ptr(std::ptr::null_mut());
        EpisodesUi { panel: n(), scroll: n(), title: n(), items: Vec::new(), current: String::new(), season: None, thumbs: Vec::new(), open: false }
    }
}

/// Builds the panel above the bottom bar, hidden until it is opened.
pub(crate) unsafe fn build(ph: *mut AnyObject) {
    let ui = UI.get_mut().unwrap();
    let panel = hud_panel(PANEL_R);
    set_autoresize(panel, MARGIN_MINX | MARGIN_MINY | MARGIN_MAXY);
    set_hidden(panel, true);
    add_sub(ph, panel);
    ui.episodes.panel = Ptr(panel);

    let title = label("", T_HEADLINE, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(title, rect(SP4, HEAD_H - SP4 - 24., PANEL_W - 2. * SP4 - 32., 24.));
    add_sub(panel, title);
    ui.episodes.title = Ptr(title);

    let close: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: sel!(toggleEpisodes:)];
    let _: () = msg_send![close, setBordered: Bool::NO];
    let img = symbol("xmark", 14., Weight::Semibold);
    if !img.is_null() {
        let _: () = msg_send![close, setImage: img];
    }
    let _: () = msg_send![close, setImagePosition: 1isize];
    let _: () = msg_send![close, setContentTintColor: color(C_ON_MEDIA2)];
    let _: () = msg_send![close, setToolTip: ns("Close episodes (E)")];
    let _: () = msg_send![close, setFocusRingType: 1usize];
    set_frame(close, rect(PANEL_W - SP4 - 24., HEAD_H - SP4 - 24., 24., 24.));
    add_sub(panel, close);

    let sv: *mut AnyObject = msg_send![class!(NSScrollView), alloc];
    let sv: *mut AnyObject = msg_send![sv, initWithFrame: rect(0., 0., PANEL_W, 10.)];
    let _: () = msg_send![sv, setHasVerticalScroller: Bool::YES];
    let _: () = msg_send![sv, setAutohidesScrollers: Bool::YES];
    let _: () = msg_send![sv, setDrawsBackground: Bool::NO];
    let _: () = msg_send![sv, setBorderType: 0usize];
    set_autoresize(sv, RESIZE_W | RESIZE_H);
    add_sub(panel, sv);
    autorel(sv);
    ui.episodes.scroll = Ptr(sv);
    layout();
}

/// Sets the season's episodes for the item now playing. Starting an item closes the panel.
pub(crate) unsafe fn set_items(items: Vec<SeasonEpisode>, current: &str, season: Option<u32>) {
    let ui = UI.get_mut().unwrap();
    ui.episodes.items = items;
    ui.episodes.current = current.to_string();
    ui.episodes.season = season;
    if ui.episodes.open {
        close();
    }
}

/// Places the panel: right-aligned, from just above the bottom bar to just under the top bar.
pub(crate) unsafe fn layout() {
    let ui = match UI.get() {
        Some(u) => u,
        None => return,
    };
    let p = ui.episodes.panel.get();
    if p.is_null() {
        return;
    }
    let host = bounds_of(ui.player_host.get()).size;
    let h = (host.h - PLAYER_BAR_H - PLAYER_CTRL_H - 2. * SP2).max(120.);
    set_frame(p, rect(host.w - INSET - PANEL_W, PLAYER_CTRL_H + SP2, PANEL_W, h));
    let sv = ui.episodes.scroll.get();
    set_frame(sv, rect(0., 0., PANEL_W, h - HEAD_H));
    set_frame(ui.episodes.title.get(), rect(SP4, h - SP4 - 24., PANEL_W - 2. * SP4 - 32., 24.));
    let close = find_close(p);
    if !close.is_null() {
        set_frame(close, rect(PANEL_W - SP4 - 24., h - SP4 - 24., 24., 24.));
    }
}

/// The close button is the only button directly in the panel besides the scroll view's.
unsafe fn find_close(panel: *mut AnyObject) -> *mut AnyObject {
    let subs: *mut AnyObject = msg_send![panel, subviews];
    let n: NSUInteger = msg_send![subs, count];
    for i in 0..n {
        let v: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        let is: Bool = msg_send![v, isKindOfClass: class!(NSButton)];
        if is == Bool::YES {
            return v;
        }
    }
    nil()
}

/// Width the Up Next card and the skip button give up to the panel while it is open.
pub(crate) unsafe fn right_inset() -> f64 {
    match UI.get() {
        Some(ui) if ui.episodes.open => PANEL_W + INSET,
        _ => 0.,
    }
}

/// Whether the panel is on screen.
pub(crate) unsafe fn is_open() -> bool {
    UI.get().map(|u| u.episodes.open).unwrap_or(false)
}

/// Shows the panel, rebuilt for the current season, with the playing episode in view.
pub(crate) unsafe fn open() {
    let ui = UI.get_mut().unwrap();
    if ui.episodes.panel.get().is_null() || ui.episodes.items.is_empty() {
        return;
    }
    ui.episodes.open = true;
    rebuild();
    layout();
    set_hidden(ui.episodes.panel.get(), false);
    let p = ui.episodes.panel.get();
    let _: () = msg_send![p, setAlphaValue: 0.0f64];
    crate::ui::player::fade(p, 1.0, M_STD);
    crate::ui::skip::place_now();
}

/// Hides the panel.
pub(crate) unsafe fn close() {
    let ui = UI.get_mut().unwrap();
    if !ui.episodes.open {
        return;
    }
    ui.episodes.open = false;
    let p = ui.episodes.panel.get();
    crate::ui::player::fade(p, 0.0, M_FAST);
    set_hidden(p, true);
    crate::ui::skip::place_now();
}

/// Rows for every episode of the season. The document view is non-flipped, so row 0 sits at the top.
unsafe fn rebuild() {
    let ui = UI.get_mut().unwrap();
    let sv = ui.episodes.scroll.get();
    let n = ui.episodes.items.len();
    // At least the visible height, so a short season sits at the top of the panel rather than the bottom.
    let visible = frame_of(sv).size.h.max(ROW_H);
    let doc_h = (n as f64 * ROW_H).max(visible);
    let doc: *mut AnyObject = msg_send![class!(NSView), alloc];
    let doc: *mut AnyObject = autorel(msg_send![doc, initWithFrame: rect(0., 0., PANEL_W, doc_h)]);
    let title = match ui.episodes.season {
        Some(s) => format!("Season {}", s),
        None => "Episodes".to_string(),
    };
    set_text(ui.episodes.title.get(), &title);

    let rows: Vec<SeasonEpisode> = ui.episodes.items.clone();
    let current = ui.episodes.current.clone();
    let mut waiting = Vec::new();
    for (i, ep) in rows.iter().enumerate() {
        let y = doc_h - (i as f64 + 1.) * ROW_H;
        let is_now = ep.id == current;
        if let Some(w) = add_row(doc, i, ep, is_now, y) {
            waiting.push(w);
        }
    }
    ui.episodes.thumbs = waiting;
    let _: () = msg_send![sv, setDocumentView: doc];

    // Scroll so the playing episode is the first row in view, or the list starts at the top.
    let cur = rows.iter().position(|e| e.id == current).unwrap_or(0);
    let top_of_cur = (cur as f64) * ROW_H;
    let max_y = (doc_h - visible).max(0.);
    let y = (doc_h - top_of_cur - ROW_H - 0.).min(max_y).max(0.);
    let clip: *mut AnyObject = msg_send![sv, contentView];
    let _: () = msg_send![clip, scrollToPoint: NSPoint { x: 0., y }];
    let _: () = msg_send![sv, reflectScrolledClipView: clip];
}

/// One row: thumbnail with progress strip, number, title, runtime, and a transparent button over it.
/// Returns the thumbnail's key, view and placeholder when its still is not cached yet.
unsafe fn add_row(doc: *mut AnyObject, index: usize, ep: &SeasonEpisode, is_now: bool, y: f64) -> Option<(u64, Ptr, Ptr)> {
    let ui = UI.get().unwrap();
    let row: *mut AnyObject = msg_send![class!(NSView), alloc];
    let row: *mut AnyObject = autorel(msg_send![row, initWithFrame: rect(0., y, PANEL_W, ROW_H)]);
    let _: () = msg_send![row, setWantsLayer: Bool::YES];
    if is_now {
        let l = layer_of(row);
        set_corner(l, R_THUMB);
        let _: () = msg_send![l, setBackgroundColor: cg_a(C_ON_MEDIA, 0.12)];
    }
    add_sub(doc, row);

    let ty = (ROW_H - THUMB_H) / 2.;
    let thumb: *mut AnyObject = msg_send![class!(NSImageView), alloc];
    let thumb: *mut AnyObject = autorel(msg_send![thumb, initWithFrame: rect(SP3, ty, THUMB_W, THUMB_H)]);
    let _: () = msg_send![thumb, setImageScaling: 3isize];
    set_corner(layer_of(thumb), R_THUMB);
    let _: () = msg_send![layer_of(thumb), setMasksToBounds: Bool::YES];
    let _: () = msg_send![layer_of(thumb), setBackgroundColor: cg_a(C_ON_MEDIA, 0.08)];
    // A film glyph on the card until the still is in the cache; it is hidden once the still is set.
    let glyph = art_placeholder(rect(0., 0., THUMB_W, THUMB_H), "film", 22.);
    add_sub(thumb, glyph);
    let mut waiting = None;
    if let Some(key) = ep.image_key {
        match ui.poster_cache.get(&key).copied() {
            Some(img) => {
                let _: () = msg_send![thumb, setImage: img];
                set_hidden(glyph, true);
            }
            None => waiting = Some((key, Ptr(thumb), Ptr(glyph))),
        }
    }
    add_sub(row, thumb);

    // Progress strip along the bottom of the thumbnail: the watched share, white on a faint track.
    let track: *mut AnyObject = msg_send![class!(NSView), alloc];
    let track: *mut AnyObject = autorel(msg_send![track, initWithFrame: rect(SP3, ty, THUMB_W, PROGRESS_H)]);
    let _: () = msg_send![track, setWantsLayer: Bool::YES];
    let _: () = msg_send![layer_of(track), setBackgroundColor: cg_a(C_ON_MEDIA, 0.28)];
    add_sub(row, track);
    if ep.progress > 0.0 {
        let fill: *mut AnyObject = msg_send![class!(NSView), alloc];
        let fill: *mut AnyObject = autorel(msg_send![fill, initWithFrame: rect(SP3, ty, THUMB_W * ep.progress.clamp(0., 1.), PROGRESS_H)]);
        let _: () = msg_send![fill, setWantsLayer: Bool::YES];
        let _: () = msg_send![layer_of(fill), setBackgroundColor: cg_a(C_ON_MEDIA, 1.0)];
        add_sub(row, fill);
    }

    let tx = TEXT_X;
    let tw = PANEL_W - tx - SP3 - 4.;
    let num = match ep.number {
        Some(n) => format!("Episode {}", n),
        None => "Episode".to_string(),
    };
    let kicker = label(&if is_now { format!("{} · Playing", num) } else { num }, T_CAPTION_EM, if is_now { C_ON_MEDIA } else { C_ON_MEDIA2 }, ALIGN_LEFT);
    set_frame(kicker, rect(tx, ROW_H / 2. + SP1, tw, 14.));
    add_sub(row, kicker);
    let name = label(&ep.name, T_BODY_EM, C_ON_MEDIA, ALIGN_LEFT);
    set_frame(name, rect(tx, ROW_H / 2. - SP1 - 18., tw, 18.));
    add_sub(row, name);
    let mins = (ep.runtime_secs / 60.).round().max(0.) as u64;
    let detail = if mins > 0 { format!("{} min", mins) } else { String::new() };
    let rt = label(&detail, T_CAPTION, C_ON_MEDIA2, ALIGN_LEFT);
    set_frame(rt, rect(tx, ROW_H / 2. - SP1 - 18. - SP1 - 14., tw, 14.));
    add_sub(row, rt);

    // Whole-row target: a clear button that jumps to this episode.
    let hit: *mut AnyObject = msg_send![class!(NSButton), buttonWithTitle: ns(""), target: ui.ctl.get(), action: sel!(jrEpisodePick:)];
    let _: () = msg_send![hit, setBordered: Bool::NO];
    let _: () = msg_send![hit, setTag: index as isize];
    let _: () = msg_send![hit, setFocusRingType: 1usize];
    let _: () = msg_send![hit, setToolTip: ns(&format!("Play {}", ep.name))];
    set_frame(hit, rect(0., 0., PANEL_W, ROW_H));
    add_sub(row, hit);
    waiting
}

/// A thumbnail's still arrived: fill every row waiting for it and drop its placeholder.
pub(crate) unsafe fn image_ready(key: u64) {
    let ui = match UI.get() {
        Some(u) => u,
        None => return,
    };
    let Some(img) = ui.poster_cache.get(&key).copied() else { return };
    for (k, thumb, glyph) in ui.episodes.thumbs.iter() {
        if *k == key {
            let _: () = msg_send![thumb.get(), setImage: img];
            set_hidden(glyph.get(), true);
        }
    }
}

/// Opens or closes the panel (E, the Episodes button, the close button).
pub(crate) extern "C" fn toggle(_t: *mut AnyObject, _c: Sel, _s: *mut AnyObject) {
    unsafe {
        if is_open() {
            close();
        } else {
            open();
        }
    }
}

/// A row was picked: save the current position, then play that episode.
pub(crate) extern "C" fn pick(_t: *mut AnyObject, _c: Sel, sender: *mut AnyObject) {
    unsafe {
        let tag: isize = msg_send![sender, tag];
        let ui = UI.get_mut().unwrap();
        let Some(ep) = ui.episodes.items.get(tag as usize).cloned() else { return };
        if ep.id == ui.episodes.current {
            close();
            return;
        }
        let Some(pn) = ui.playing.as_ref() else { return };
        let item_id = pn.item_id.clone();
        let ticks = crate::jellyfin::urls::ticks_from_secs(crate::ui::skip::position());
        let _ = ui.ui_tx.send(crate::app::messages::UiCmd::Stopped { item_id, ticks });
        let _ = ui.ui_tx.send(crate::app::messages::UiCmd::Play { item_id: ep.id });
        close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_leaves_room_for_the_overlays() {
        // The panel stays narrower than a phone-sized slice of the picture and leaves its text room.
        assert!(PANEL_W < 400.);
        assert!(TEXT_X + 100. < PANEL_W);
    }
}
