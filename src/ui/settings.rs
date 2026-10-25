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
        initWithContentRect: rect(0., 0., 460., 620.)
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
    let audio_f = mk(204.0, "Preferred audio language (e.g. eng, empty = default)", "eng");
    ui.s_audio_lang = Ptr(audio_f);
    let seerr_url_f = mk(296.0, "Seerr URL", "http://host:5055");
    ui.seerr.s_url = Ptr(seerr_url_f);

    // Sign-in mode: Jellyfin account (default) or API key.
    let ml = make_label("Sign in with", 12.0, false, false);
    set_frame(ml, rect(20., b.size.h - 360. - 16., 90., 14.));
    add_sub(c, ml);
    let mp: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let mp: *mut Object = msg_send![mp, initWithFrame: rect(112., b.size.h - 360. - 22., 220., 26.) pullsDown: NO];
    let arr: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in ["Jellyfin account", "API key"] {
        let _: () = msg_send![arr, addObject: ns(t)];
    }
    let _: () = msg_send![mp, addItemsWithTitles: arr];
    let _: () = msg_send![mp, setTarget: ui.ctl.get()];
    let _: () = msg_send![mp, setAction: sel!(seerrModeChanged:)];
    add_sub(c, mp);
    autorel(mp);
    ui.seerr.s_mode = Ptr(mp);

    let field_at = |y: f64, label: &str, placeholder: &str, secure: bool| -> (*mut Object, *mut Object) {
        let l = make_label(label, 12.0, false, false);
        set_frame(l, rect(20., b.size.h - y - 16., 300., 14.));
        add_sub(c, l);
        let f = text_field(placeholder, secure);
        set_frame(f, rect(20., b.size.h - y - 40., 420., 24.));
        add_sub(c, f);
        (l, f)
    };
    let (ul, uf) = field_at(396.0, "Seerr username (your Jellyfin username)", "username", false);
    ui.seerr.s_user = Ptr(uf);
    let (pl, pf) = field_at(452.0, "Seerr password (your Jellyfin password)", "password", true);
    ui.seerr.s_pass = Ptr(pf);
    let (kl, kf) = field_at(396.0, "Seerr API key (Seerr > Settings > General)", "API key", true);
    ui.seerr.s_key = Ptr(kf);
    ui.seerr.s_login_group = vec![Ptr(ul), Ptr(uf), Ptr(pl), Ptr(pf)];
    ui.seerr.s_key_group = vec![Ptr(kl), Ptr(kf)];

    let seerr_st = make_label("", 12.0, false, false);
    set_frame(seerr_st, rect(20., b.size.h - 548., 420., 32.));
    let _: () = msg_send![seerr_st, setTextColor: color_named("secondary")];
    let _: () = msg_send![seerr_st.cell(), setLineBreakMode: 0usize]; // wrap
    add_sub(c, seerr_st);
    ui.seerr.s_status = Ptr(seerr_st);
    crate::ui::seerr::apply_mode();

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
        checkboxWithTitle: ns("Enable Seerr (request movies and shows)")
        target: nil() action: nil()];
    set_frame(cb, rect(20., b.size.h - 268., 420., 20.));
    add_sub(c, cb);
    ui.seerr.s_on = Ptr(cb);

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
    set_text(ui.s_audio_lang.get(), &cfg.settings.audio_lang);
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
    let _: () = msg_send![ui.seerr.s_on.get(), setState: cfg.settings.seerr_enabled as NSInteger];
    set_text(ui.seerr.s_url.get(), &cfg.settings.seerr_url);
    // Secrets are never prefilled. A saved URL implies a stored credential
    // (saving requires one), so say it is unchanged rather than asking the
    // Keychain from the main thread.
    set_text(ui.seerr.s_key.get(), "");
    set_text(ui.seerr.s_pass.get(), "");
    let saved = !cfg.settings.seerr_url.trim().is_empty();
    let login_mode = cfg.settings.seerr_username.trim().is_empty() == false || !saved;
    let _: () = msg_send![ui.seerr.s_mode.get(), selectItemAtIndex: if login_mode { 0isize } else { 1isize }];
    let uname = if !cfg.settings.seerr_username.trim().is_empty() {
        cfg.settings.seerr_username.clone()
    } else {
        cfg.username.clone().unwrap_or_default() // prefill from the Jellyfin login
    };
    set_text(ui.seerr.s_user.get(), &uname);
    let ph = |s: bool, d: &'static str| if s { "unchanged (leave blank to keep)" } else { d };
    let _: () = msg_send![ui.seerr.s_pass.get().cell(), setPlaceholderString: ns(ph(saved && login_mode, "password"))];
    let _: () = msg_send![ui.seerr.s_key.get().cell(), setPlaceholderString: ns(ph(saved && !login_mode, "API key"))];
    crate::ui::seerr::apply_mode();
    set_text(ui.seerr.s_status.get(), "");
    ui.seerr.pending = None;
}
