//! First-run onboarding: a card over the library side asking how to browse.
//! It appears once, after the first successful login (while `layout_mode` is
//! unset), on top of Home. The choice is saved to config and can be changed
//! later in Settings. Closing it without choosing keeps the default (separate
//! libraries), so it never blocks the app.
#![allow(unsafe_code)]
use crate::config::LayoutMode;
use crate::platform::appkit::*;
use crate::ui::shell::set_layout_mode;
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, Sel, BOOL, NO};
use objc::{sel, sel_impl};

const CARD_W: f64 = 560.;
const PAD: f64 = SP8;
const TILE_H: f64 = 96.;
const TILE_GAP: f64 = SP3;
const ICON: f64 = 22.;
const TICK: f64 = 18.;
const BTN_W: f64 = 120.;
const BTN_H: f64 = 32.;

/// Index of each option inside `OnboardUi::picked` and the tile tags.
const PICK_SEPARATE: usize = 0;
const PICK_UNIFIED: usize = 1;

pub struct OnboardUi {
    /// Full-area dimmer that holds the card; hidden until shown.
    pub host: Ptr,
    /// The two option tiles (surfaces), restyled on selection.
    pub tiles: [Ptr; 2],
    /// Radio glyph on each tile.
    pub ticks: [Ptr; 2],
    /// Option highlighted right now (`PICK_SEPARATE` by default).
    pub picked: usize,
}

impl OnboardUi {
    pub fn new() -> OnboardUi {
        let null = Ptr(std::ptr::null_mut());
        OnboardUi { host: null, tiles: [null, null], ticks: [null, null], picked: PICK_SEPARATE }
    }
}

/// Copy for the two options: (symbol, title, body).
const OPTIONS: [(&str, &str, &str); 2] = [
    (
        "rectangle.stack",
        "Separate libraries",
        "Movies, Shows and each of your libraries get their own sidebar entry, as they do today.",
    ),
    (
        "square.grid.2x2",
        "One big library",
        "Like a streaming service: Home, with Movies and TV Shows tabs, plus one Browse grid for both.",
    ),
];

/// Build the hidden overlay over `content` (the library side). Call once at startup.
pub(crate) unsafe fn build(content: *mut Object, b: NSRect) {
    let ui = UI.get_mut().unwrap();
    let host: *mut Object = msg_send![class!(NSView), alloc];
    let host: *mut Object = msg_send![host, initWithFrame: b];
    set_autoresize(host, RESIZE_W | RESIZE_H);
    let layer = layer_of(host);
    let _: () = msg_send![layer, setBackgroundColor: crate::ui::theme::cg_black(0.42)];
    set_hidden(host, true);
    add_sub(content, host);
    autorel(host);
    ui.onboard.host = Ptr(host);

    // Card, centred. Built top-down; frames are applied once the height is known.
    let card_h = PAD + 26. + SP1 + 16. + SP5 + TILE_H + TILE_GAP + TILE_H + SP6 + BTN_H + PAD;
    let card = surface(rect((b.size.w - CARD_W) / 2., (b.size.h - card_h) / 2., CARD_W, card_h), C_CARD, R_CARD);
    let _: () = msg_send![card, setBorderWidth: 1.0f64];
    let _: () = msg_send![card, setBorderColor: color(C_SEPARATOR)];
    set_autoresize(card, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(host, card);
    let cv: *mut Object = msg_send![card, contentView];
    let inner_w = CARD_W - 2. * PAD;
    // y of a row whose top sits `top` below the card top, with height `h`.
    let y = |top: f64, h: f64| card_h - top - h;

    let title = label("How do you want to browse?", T_TITLE, C_TEXT, ALIGN_CENTER);
    set_frame(title, rect(PAD, y(PAD, 26.), inner_w, 26.));
    add_sub(cv, title);
    let sub = label("Pick a layout. You can change it anytime in Settings.", T_CALLOUT, C_TEXT2, ALIGN_CENTER);
    set_frame(sub, rect(PAD, y(PAD + 26. + SP1, 16.), inner_w, 16.));
    add_sub(cv, sub);

    let mut top = PAD + 26. + SP1 + 16. + SP5;
    for (i, (icon, head, body)) in OPTIONS.iter().enumerate() {
        let tile = surface(rect(PAD, y(top, TILE_H), inner_w, TILE_H), C_CANVAS, R_THUMB);
        set_autoresize(tile, RESIZE_W);
        add_sub(cv, tile);
        ui.onboard.tiles[i] = Ptr(tile);

        let tv: *mut Object = msg_send![class!(NSImageView), alloc];
        let tv: *mut Object = autorel(msg_send![tv, initWithFrame: rect(SP5, (TILE_H - ICON) / 2., ICON, ICON)]);
        let img = symbol(icon, ICON, Weight::Regular);
        if !img.is_null() {
            let _: () = msg_send![tv, setImage: img];
            let _: () = msg_send![tv, setContentTintColor: color(C_ACCENT)];
        }
        add_sub(tile, tv);

        let text_x = SP5 + ICON + SP4;
        let text_w = inner_w - text_x - SP5 - TICK - SP3;
        let head_l = label(head, T_HEADLINE, C_TEXT, ALIGN_LEFT);
        let body_l = wrapping_label(body, T_CALLOUT, C_TEXT2, ALIGN_LEFT);
        let cell: *mut Object = msg_send![body_l, cell];
        let need: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., text_w, 1000.)];
        let body_h = need.h.ceil();
        let block = 18. + SP1 + body_h;
        let by = (TILE_H - block) / 2.;
        set_frame(head_l, rect(text_x, by + body_h + SP1, text_w, 18.));
        set_frame(body_l, rect(text_x, by, text_w, body_h));
        add_sub(tile, head_l);
        add_sub(tile, body_l);

        let tk: *mut Object = msg_send![class!(NSImageView), alloc];
        let tk: *mut Object = autorel(msg_send![tk, initWithFrame: rect(inner_w - SP5 - TICK, (TILE_H - TICK) / 2., TICK, TICK)]);
        add_sub(tile, tk);
        ui.onboard.ticks[i] = Ptr(tk);

        // Transparent hit target over the whole tile (no title, no image).
        let hit: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
            target: ui.ctl.get() action: sel!(onboardPick:)];
        let _: () = msg_send![hit, setBordered: NO];
        let _: () = msg_send![hit, setTag: i as NSInteger];
        set_frame(hit, rect(0., 0., inner_w, TILE_H));
        set_autoresize(hit, RESIZE_W);
        add_sub(tile, hit);
        top += TILE_H + TILE_GAP;
    }
    // Continue, bottom-right of the card.
    let go: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Continue")
        target: ui.ctl.get() action: sel!(onboardContinue:)];
    let _: () = msg_send![go, setKeyEquivalent: ns("\r")];
    set_frame(go, rect(CARD_W - PAD - BTN_W, y(card_h - PAD - BTN_H, BTN_H), BTN_W, BTN_H));
    set_autoresize(go, MARGIN_MINX | MARGIN_MINY);
    add_sub(cv, go);

    apply_pick_style();
}

/// Restyle the tiles and radio glyphs for `picked`.
unsafe fn apply_pick_style() {
    let ui = UI.get_mut().unwrap();
    for i in 0..2 {
        let on = i == ui.onboard.picked;
        let tile = ui.onboard.tiles[i].get();
        let _: () = msg_send![tile, setBorderWidth: if on { 2.0f64 } else { 1.0f64 }];
        let _: () = msg_send![tile, setBorderColor: color(if on { C_ACCENT } else { C_SEPARATOR })];
        let tk = ui.onboard.ticks[i].get();
        let img = if on {
            symbol("checkmark.circle.fill", TICK, Weight::Regular)
        } else {
            symbol("circle", TICK, Weight::Regular)
        };
        if !img.is_null() {
            let _: () = msg_send![tk, setImage: img];
        }
        let _: () = msg_send![tk, setContentTintColor: color(if on { C_ACCENT } else { C_TEXT3 })];
    }
}

/// Whether the overlay is on screen.
pub(crate) unsafe fn visible() -> bool {
    let ui = UI.get().unwrap();
    if ui.onboard.host.get().is_null() {
        return false;
    }
    let h: BOOL = msg_send![ui.onboard.host.get(), isHidden];
    h == NO
}

/// Show the overlay with the default option highlighted.
pub(crate) unsafe fn show() {
    let ui = UI.get_mut().unwrap();
    if ui.onboard.host.get().is_null() {
        return;
    }
    ui.onboard.picked = PICK_SEPARATE;
    apply_pick_style();
    set_hidden(ui.onboard.host.get(), false);
}

unsafe fn hide() {
    let ui = UI.get_mut().unwrap();
    set_hidden(ui.onboard.host.get(), true);
}

/// Tile click: highlight that option.
pub(crate) extern "C" fn onboard_pick(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        ui.onboard.picked = if tag == 1 { PICK_UNIFIED } else { PICK_SEPARATE };
        apply_pick_style();
    }
}

/// Continue: save the highlighted layout and close the card.
pub(crate) extern "C" fn onboard_continue(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let mode = if ui.onboard.picked == PICK_UNIFIED { LayoutMode::Unified } else { LayoutMode::Separate };
        hide();
        set_layout_mode(mode);
    }
}
