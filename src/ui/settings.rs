#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::library::*;
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

pub(crate) unsafe fn build_settings_window() {
    let ui = UI.get_mut().unwrap();
    let w: *mut Object = msg_send![class!(NSWindow), alloc];
    let w: *mut Object = msg_send![w,
        initWithContentRect: rect(0., 0., 460., 430.)
        styleMask: (1 | 2) as NSUInteger backing: 2usize defer: NO];
    let _: () = msg_send![w, setTitle: ns("Settings")];
    let _: () = msg_send![w, setReleasedWhenClosed: NO];
    ui.settings_win = Ptr::retained(w);
    let c: *mut Object = msg_send![w, contentView];
    let b = bounds_of(c);

    let mk = |y: f64, label: &str, placeholder: &str| -> *mut Object {
        let l = make_label(label, 12.0, false, false);
        set_frame(l, rect(20., b.size.h - y - 16., 200., 14.));
        add_sub(c, l);
        let f = text_field(placeholder, false);
        set_frame(f, rect(20., b.size.h - y - 40., 420., 24.));
        add_sub(c, f);
        f
    };
    let url_f = mk(24.0, "Server URL", "http://host:8096");
    ui.s_url = Ptr(url_f);
    let sub_f = mk(84.0, "Preferred subtitle language (e.g. eng, empty = off)", "eng");
    ui.s_sub_lang = Ptr(sub_f);
    let searx_f = mk(204.0, "SearXNG instance URL", "https://searx.example.com");
    ui.s_searx_url = Ptr(searx_f);

    let ql = make_label("Preferred quality", 12.0, false, false);
    set_frame(ql, rect(20., b.size.h - 144. - 16., 200., 14.));
    add_sub(c, ql);
    let q: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let q: *mut Object = msg_send![q, initWithFrame: rect(20., b.size.h - 168., 220., 26.) pullsDown: NO];
    let arr: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in ["Original", "40 Mbps", "20 Mbps", "10 Mbps", "5 Mbps", "3 Mbps"] {
        let _: () = msg_send![arr, addObject: ns(t)];
    }
    let _: () = msg_send![q, addItemsWithTitles: arr];
    add_sub(c, q);
    autorel(q);
    ui.s_quality = Ptr(q);

    let tl = make_label("Appearance", 12.0, false, false);
    set_frame(tl, rect(260., b.size.h - 144. - 16., 200., 14.));
    add_sub(c, tl);
    let th: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let th: *mut Object = msg_send![th, initWithFrame: rect(260., b.size.h - 168., 180., 26.) pullsDown: NO];
    let arr: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in ["Match System", "Light", "Dark"] {
        let _: () = msg_send![arr, addObject: ns(t)];
    }
    let _: () = msg_send![th, addItemsWithTitles: arr];
    add_sub(c, th);
    autorel(th);
    ui.s_theme = Ptr(th);

    let cb: *mut Object = msg_send![class!(NSButton),
        checkboxWithTitle: ns("Enable SearXNG search (opens your browser)")
        target: nil() action: nil()];
    set_frame(cb, rect(20., b.size.h - 280., 420., 20.));
    add_sub(c, cb);
    ui.s_searx_on = Ptr(cb);

    let save: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Save")
        target: ui.ctl.get() action: sel!(saveSettings:)];
    set_frame(save, rect(b.size.w - 116., 16., 96., 32.));
    let _: () = msg_send![save, setKeyEquivalent: ns("\r")];
    add_sub(c, save);
    let cancel: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel")
        target: ui.ctl.get() action: sel!(cancelSettings:)];
    set_frame(cancel, rect(b.size.w - 222., 16., 96., 32.));
    add_sub(c, cancel);
}

pub(crate) unsafe fn settings_fill(cfg: &Config) {
    let ui = UI.get_mut().unwrap();
    if let Some(u) = &cfg.server_url {
        set_text(ui.s_url.get(), u);
    }
    set_text(ui.s_sub_lang.get(), &cfg.settings.subtitle_lang);
    let q_idx = match cfg.settings.max_bitrate_kbps {
        0 => 0usize,
        40_000 => 1,
        20_000 => 2,
        10_000 => 3,
        5_000 => 4,
        _ => 5,
    };
    let _: () = msg_send![ui.s_quality.get(), selectItemAtIndex: q_idx as NSInteger];
    let t_idx = match cfg.settings.theme.as_str() {
        "light" => 1usize,
        "dark" => 2,
        _ => 0,
    };
    let _: () = msg_send![ui.s_theme.get(), selectItemAtIndex: t_idx as NSInteger];
    let _: () = msg_send![ui.s_searx_on.get(), setState: cfg.settings.searx_enabled as NSInteger];
    set_text(ui.s_searx_url.get(), &cfg.settings.searx_url);
}
