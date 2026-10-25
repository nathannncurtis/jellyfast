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
    use crate::ui::theme::*;
    let ui = UI.get_mut().unwrap();
    const WIN_W: f64 = 520.;
    let w: *mut Object = msg_send![jr_window_class(), alloc];
    let w: *mut Object = msg_send![w,
        initWithContentRect: rect(0., 0., WIN_W, 560.)
        styleMask: (1 | 2) as NSUInteger backing: 2usize defer: NO];
    let _: () = msg_send![w, setTitle: ns("Settings")];
    let _: () = msg_send![w, setReleasedWhenClosed: NO];
    ui.settings_win = Ptr::retained(w);
    let c: *mut Object = msg_send![w, contentView];

    // Two-column form laid out top-down from a running `top`; frames are applied once the
    // total height is known, so rows can never land on each other.
    const LABEL_X: f64 = SP6;
    const LABEL_W: f64 = 150.;
    const CTRL_X: f64 = LABEL_X + LABEL_W + SP3; // 186
    const CTRL_W: f64 = WIN_W - CTRL_X - SP6; // 310
    const ROW_H: f64 = 28.;
    let mut place: Vec<(*mut Object, NSRect)> = vec![]; // rect.origin.y holds `top`
    let mut top = SP5;
    let put = |place: &mut Vec<(*mut Object, NSRect)>, v: *mut Object, x: f64, top: f64, wd: f64, h: f64| {
        add_sub(c, v);
        place.push((v, rect(x, top, wd, h)));
    };
    let section = |place: &mut Vec<(*mut Object, NSRect)>, top: &mut f64, title: &str, first: bool| {
        if !first {
            *top += SP6;
        }
        put(place, label(title, T_BODY_EM, C_TEXT, ALIGN_LEFT), LABEL_X, *top, CTRL_X + CTRL_W - LABEL_X, 18.);
        *top += 18. + SP1;
        put(place, hairline(rect(0., 0., CTRL_X + CTRL_W - LABEL_X, 1.)), LABEL_X, *top, CTRL_X + CTRL_W - LABEL_X, 1.);
        *top += 1. + SP3;
    };
    // A labelled row: right-aligned label vertically centred on the control.
    let row = |place: &mut Vec<(*mut Object, NSRect)>, top: &mut f64, text: &str, ctl: *mut Object, ctl_h: f64, ctl_w: f64| -> *mut Object {
        let l = label(text, T_BODY, C_TEXT, ALIGN_RIGHT);
        put(place, l, LABEL_X, *top + (ROW_H - 16.) / 2., LABEL_W, 16.);
        put(place, ctl, CTRL_X, *top + (ROW_H - ctl_h) / 2., ctl_w, ctl_h);
        *top += ROW_H;
        l
    };
    // Help text sits under its control, never inside the label.
    let help = |place: &mut Vec<(*mut Object, NSRect)>, top: &mut f64, text: &str| -> *mut Object {
        let h = wrapping_label(text, T_CAPTION, C_TEXT2, ALIGN_LEFT);
        let cell: *mut Object = msg_send![h, cell];
        let need: NSSize = msg_send![cell, cellSizeForBounds: rect(0., 0., CTRL_W, 1000.)];
        let ht = need.h.ceil();
        put(place, h, CTRL_X, *top + SP1, CTRL_W, ht);
        *top += SP1 + ht;
        h
    };
    let popup = |items: &[&str]| -> *mut Object {
        let p: *mut Object = msg_send![class!(NSPopUpButton), alloc];
        let p: *mut Object = msg_send![p, initWithFrame: rect(0., 0., 200., 26.) pullsDown: NO];
        let arr: *mut Object = msg_send![class!(NSMutableArray), array];
        for t in items {
            let _: () = msg_send![arr, addObject: ns(t)];
        }
        let _: () = msg_send![p, addItemsWithTitles: arr];
        autorel(p)
    };

    section(&mut place, &mut top, "Server", true);
    let url_f = text_field("http://host:8096", false);
    row(&mut place, &mut top, "Server URL", url_f, 24., CTRL_W);
    ui.s_url = Ptr(url_f);

    section(&mut place, &mut top, "Playback", false);
    let q = popup(&["Original", "40 Mbps", "20 Mbps", "10 Mbps", "5 Mbps", "3 Mbps"]);
    row(&mut place, &mut top, "Preferred quality", q, 26., 160.);
    ui.s_quality = Ptr(q);
    top += SP3;
    let sub_f = text_field("eng", false);
    row(&mut place, &mut top, "Subtitle language", sub_f, 24., 160.);
    help(&mut place, &mut top, "Three-letter code, like eng. Leave empty to turn subtitles off.");
    ui.s_sub_lang = Ptr(sub_f);
    top += SP3;
    let audio_f = text_field("eng", false);
    row(&mut place, &mut top, "Audio language", audio_f, 24., 160.);
    help(&mut place, &mut top, "Leave empty to use the file's default.");
    ui.s_audio_lang = Ptr(audio_f);

    section(&mut place, &mut top, "Appearance", false);
    let th = popup(&["System", "Light", "Dark"]);
    row(&mut place, &mut top, "Theme", th, 26., 160.);
    ui.s_theme = Ptr(th);

    section(&mut place, &mut top, "Requests (Seerr)", false);
    let cb: *mut Object = msg_send![class!(NSButton),
        checkboxWithTitle: ns("Enable requests")
        target: nil() action: nil()];
    put(&mut place, cb, CTRL_X, top + (ROW_H - 18.) / 2., CTRL_W, 18.);
    top += ROW_H + SP3;
    ui.seerr.s_on = Ptr(cb);
    let seerr_url_f = text_field("http://host:5055", false);
    row(&mut place, &mut top, "Seerr URL", seerr_url_f, 24., CTRL_W);
    ui.seerr.s_url = Ptr(seerr_url_f);
    top += SP3;
    // Sign-in mode: Jellyfin account (default) or API key.
    let mp = popup(&["Jellyfin account", "API key"]);
    let _: () = msg_send![mp, setTarget: ui.ctl.get()];
    let _: () = msg_send![mp, setAction: sel!(seerrModeChanged:)];
    row(&mut place, &mut top, "Sign in with", mp, 26., 200.);
    ui.seerr.s_mode = Ptr(mp);
    top += SP3;
    // Login rows and the API-key row share the same slot; apply_mode shows one group.
    let slot = top;
    let uf = text_field("username", false);
    let ul = row(&mut place, &mut top, "Username", uf, 24., CTRL_W);
    let uh = help(&mut place, &mut top, "Your Jellyfin username and password.");
    top += SP3;
    let pf = text_field("password", true);
    let pl = row(&mut place, &mut top, "Password", pf, 24., CTRL_W);
    let login_end = top;
    top = slot;
    let kf = text_field("API key", true);
    let kl = row(&mut place, &mut top, "API key", kf, 24., CTRL_W);
    let kh = help(&mut place, &mut top, "In Seerr: Settings > General > API Key.");
    top = top.max(login_end);
    ui.seerr.s_user = Ptr(uf);
    ui.seerr.s_pass = Ptr(pf);
    ui.seerr.s_key = Ptr(kf);
    ui.seerr.s_login_group = vec![Ptr(ul), Ptr(uf), Ptr(uh), Ptr(pl), Ptr(pf)];
    ui.seerr.s_key_group = vec![Ptr(kl), Ptr(kf), Ptr(kh)];

    top += SP2;
    let seerr_st = wrapping_label("", T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    put(&mut place, seerr_st, CTRL_X, top, CTRL_W, 32.);
    top += 32.;
    ui.seerr.s_status = Ptr(seerr_st);

    // Buttons: bottom-right, 88 wide, SP3 apart, SP5 from the edges.
    top += SP5;
    let save: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Save")
        target: ui.ctl.get() action: sel!(saveSettings:)];
    let _: () = msg_send![save, setKeyEquivalent: ns("\r")];
    put(&mut place, save, WIN_W - SP5 - 88., top, 88., 32.);
    let cancel: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel")
        target: ui.ctl.get() action: sel!(cancelSettings:)];
    let _: () = msg_send![cancel, setKeyEquivalent: ns("\u{1b}")];
    put(&mut place, cancel, WIN_W - SP5 - 88. - SP3 - 88., top, 88., 32.);
    top += 32. + SP5;

    let _: () = msg_send![w, setContentSize: size(WIN_W, top)];
    for (v, r) in place {
        set_frame(v, rect(r.origin.x, top - r.origin.y - r.size.h, r.size.w, r.size.h));
    }
    crate::ui::seerr::apply_mode();
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
