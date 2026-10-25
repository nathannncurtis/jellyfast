#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::settings::*;
use crate::ui::shell::*;
use objc::class;
use objc::declare::ClassDecl;
use objc::msg_send;
use objc::runtime::{Class, BOOL, NO, Object, Sel, YES};
use objc::{sel, sel_impl};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_void;
use std::sync::{Mutex, OnceLock};

// ---- Poster grid ---------------------------------------------------------
pub fn grid_set_items(items: &[ListItem]) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let grid = ui.grid.get();
        // Tear down old cells (remove releases them; see safety model).
        let old = ui.grid_data.take();
        if let Some(d) = old {
            for c in &d.cells {
                let _: () = msg_send![c.container, removeFromSuperview];
            }
            // Box drops here; contained raw pointers are dead by convention.
        }
        let mut cells = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            let cell_cls = *CELL_CLASS.get().unwrap();
            let container: *mut Object = msg_send![cell_cls, alloc];
            let container: *mut Object = msg_send![container, initWithFrame: rect(0., 0., 176., 300.)];
            let cw = 176.0;
            // Poster
            let poster: *mut Object = msg_send![class!(NSImageView), alloc];
            let poster: *mut Object = msg_send![poster, initWithFrame: rect(0., 0., cw, cw * 1.5)];
            let _: () = msg_send![poster, setImageScaling: 0i64]; // proportionally down
            add_sub(container, poster);
            autorel(poster);
            // Hover play icon
            let hover = symbol_image("play.circle.fill", 40.0);
            let hv: *mut Object = msg_send![class!(NSImageView), alloc];
            let hv: *mut Object = msg_send![hv, initWithFrame: rect(cw / 2. - 20., cw * 0.75 - 20., 40., 40.)];
            if !hover.is_null() {
                let _: () = msg_send![hv, setImage: hover];
            }
            let _: () = msg_send![hv, setAlphaValue: 0.0f64];
            add_sub(container, hv);
            autorel(hv);
            // Progress bar
            let prog: *mut Object = msg_send![class!(NSProgressIndicator), alloc];
            let prog: *mut Object = msg_send![prog, initWithFrame: rect(8., cw * 1.5 - 6., cw - 16., 4.)];
            let _: () = msg_send![prog, setStyle: 0i64]; // bar
            let _: () = msg_send![prog, setIndeterminate: NO];
            let _: () = msg_send![prog, setMinValue: 0.0f64];
            let _: () = msg_send![prog, setMaxValue: 1.0f64];
            let frac = crate::jellyfin::urls::progress_fraction(
                it.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0),
                it.run_time_ticks,
            );
            let _: () = msg_send![prog, setDoubleValue: frac];
            set_hidden(prog, frac <= 0.0);
            add_sub(container, prog);
            autorel(prog);
            // Watched badge
            let badge: *mut Object = msg_send![class!(NSImageView), alloc];
            let badge: *mut Object = msg_send![badge, initWithFrame: rect(6., cw * 1.5 - 24., 18., 18.)];
            let bimg = symbol_image("checkmark.circle.fill", 14.0);
            if !bimg.is_null() {
                let _: () = msg_send![badge, setImage: bimg];
                let _: () = msg_send![badge, setContentTintColor: color_named("accent")];
            }
            set_hidden(badge, !it.user_data.as_ref().map(|u| u.played).unwrap_or(false));
            add_sub(container, badge);
            autorel(badge);
            // Title + subtitle
            let title = make_label(&it.name, 12.5, false, false);
            set_frame(title, rect(0., cw * 1.5 + 6., cw, 16.));
            let _: () = msg_send![title.cell(), setLineBreakMode: 4usize]; // truncate tail
            add_sub(container, title);
            let sub = make_label(&it.sub_label(), 10.5, false, false);
            set_frame(sub, rect(0., cw * 1.5 + 22., cw, 13.));
            let _: () = msg_send![sub, setTextColor: color_named("secondary")];
            let _: () = msg_send![sub.cell(), setLineBreakMode: 4usize];
            add_sub(container, sub);
            // Click button (covers poster area)
            let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
                target: ui.ctl.get() action: sel!(itemClicked:)];
            set_frame(btn, rect(0., 0., cw, cw * 1.5));
            let _: () = msg_send![btn, setBordered: NO];
            let _: () = msg_send![btn, setTag: i as NSInteger];
            let _: () = msg_send![btn, setToolTip: ns(&it.name)];
            add_sub(container, btn);
            // Context menu (watched actions + open in web)
            let menu: *mut Object = msg_send![class!(NSMenu), alloc];
            let menu: *mut Object = msg_send![menu, initWithTitle: ns(&it.name)];
            let id_ns = ns(&it.id);
            let m1: *mut Object = menu_item("Mark as watched", sel!(markWatched:), "");
            let _: () = msg_send![m1, setTarget: ui.ctl.get()];
            let _: () = msg_send![m1, setRepresentedObject: id_ns];
            let _: () = msg_send![menu, addItem: m1];
            let m2: *mut Object = menu_item("Open in Jellyfin Web", sel!(openInWeb:), "");
            let _: () = msg_send![m2, setTarget: ui.ctl.get()];
            let _: () = msg_send![m2, setRepresentedObject: id_ns];
            let _: () = msg_send![menu, addItem: m2];
            let _: () = msg_send![btn, setMenu: menu];
            autorel(menu);
            // Cell ivar: hover view pointer
            let cd = Box::into_raw(Box::new(CellData { hover: hv }));
            (*container).set_ivar::<*mut c_void>("jrCell", cd as *mut c_void);
            // Tracking area for hover
            let ta: *mut Object = msg_send![class!(NSTrackingArea), alloc];
            let ta: *mut Object = msg_send![ta,
                initWithRect: rect(0., 0., cw, cw * 1.5)
                options: (16usize | 2usize) owner: container userInfo: nil()];
            let _: () = msg_send![container, addTrackingArea: ta];
            autorel(ta);
            let _: () = msg_send![ui.grid.get(), addSubview: container];
            autorel(container);
            cells.push(CellHandle {
                container,
                poster,
                title,
                sub,
                prog,
                badge,
                hover_icon: hv,
                key: poster_key(&it.id),
                meta: it.clone(),
            });
        }
        ui.grid_data = Some(Box::new(GridData { cells }));
        let _: () = msg_send![grid, setNeedsLayout: YES];
    }
}

pub(crate) struct CellData {
    hover: *mut Object,
}

pub(crate) extern "C" fn cell_layout(this: &mut Object, _cmd: Sel) {
    // Frames are set at build time and grid_layout positions the containers,
    // so the cell only forwards to NSView's own layout.
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
    }
}

// NOTE: cell_layout intentionally delegates to super; frames are set at
// build time and the grid positions containers in grid_layout.

pub(crate) extern "C" fn cell_hover(this: &mut Object, _cmd: Sel, _ev: *mut Object) {
    unsafe {
        let cd = *(*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;
        if !cd.is_null() {
            let anim: *mut Object = msg_send![(*cd).hover, animator];
            let _: () = msg_send![anim, setAlphaValue: 0.9f64];
        }
    }
}
pub(crate) extern "C" fn cell_unhover(this: &mut Object, _cmd: Sel, _ev: *mut Object) {
    unsafe {
        let cd = *(*this).get_ivar::<*mut c_void>("jrCell") as *mut CellData;
        if !cd.is_null() {
            let anim: *mut Object = msg_send![(*cd).hover, animator];
            let _: () = msg_send![anim, setAlphaValue: 0.0f64];
        }
    }
}

pub(crate) extern "C" fn grid_layout(this: &mut Object, _cmd: Sel) {
    unsafe {
        let data = *(*this).get_ivar::<*mut c_void>("jrData") as *mut GridData;
        if data.is_null() {
            return;
        }
        let b = bounds_of(this as *mut Object);
        let w = b.size.w.max(200.0);
        let gap = 18.0;
        let mut cols = ((w - gap) / (176.0 + gap)) as usize;
        if cols < 1 {
            cols = 1;
        }
        let tile_w = ((w - gap * (cols as f64 + 1.0)) / cols as f64).floor();
        let poster_h = tile_w * 1.5;
        let cell_h = poster_h + 42.0;
        let d = &mut *data;
        for (i, c) in d.cells.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = gap + col as f64 * (tile_w + gap);
            let y = gap + row as f64 * (cell_h + gap);
            set_frame(c.container, rect(x, y, tile_w, cell_h));
            // Scale poster-holding subviews to tile width.
            set_frame(c.poster, rect(0., 0., tile_w, poster_h));
            set_frame(c.hover_icon, rect(tile_w / 2. - 20., poster_h / 2. - 20., 40., 40.));
            set_frame(c.prog, rect(8., poster_h - 6., tile_w - 16., 4.));
            set_frame(c.badge, rect(6., poster_h - 24., 18., 18.));
            set_frame(c.title, rect(0., poster_h + 6., tile_w, 16.));
            set_frame(c.sub, rect(0., poster_h + 22., tile_w, 13.));
            // re-cover poster with the click button (stored in meta? keep in container children order)
        }
        let rows = (d.cells.len() + cols - 1) / cols;
        let content_h = gap + rows as f64 * (cell_h + gap);
        let cur_h = b.size.h;
        if (content_h - cur_h).abs() > 1.0 {
            let _: () = msg_send![this as *mut Object, setFrameSize: size(w, content_h)];
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
            return;
        }
        if let Some(old) = ui.poster_cache.insert(key, img) {
            release_obj(old);
        }
        // Update any visible cells with this key.
        if let Some(d) = ui.grid_data.as_mut() {
            for c in &d.cells {
                if c.key == key {
                    let _: () = msg_send![c.poster, setImage: img];
                }
            }
        }
    }
}
