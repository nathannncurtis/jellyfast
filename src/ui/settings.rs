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
    let w: *mut Object = msg_send![jr_window_class(), alloc];
    let w: *mut Object = msg_send![w,
        initWithContentRect: rect(0., 0., 460., 600.)
        styleMask: (1 | 2) as NSUInteger backing: 2usize defer: NO];
    let _: () = msg_send![w, setTitle: ns("Settings")];
    let _: () = msg_send![w, setReleasedWhenClosed: NO];
    ui.settings_win = Ptr::retained(w);
    let c: *mut Object = msg_send![w, contentView];
    let b = bounds_of(c);

    // Laid out top-down: `top` is the distance from the top edge, so rows can never land on each other.
    const X: f64 = 20.;
    const FULL: f64 = 420.;
    const HALF: f64 = 200.;
    const COL2: f64 = X + HALF + 20.;
    const LABEL_H: f64 = 16.;
    const FIELD_H: f64 = 24.;
    const POPUP_H: f64 = 26.;
    const GAP: f64 = 4.; // label to control
    const ROW_GAP: f64 = 16.;
    let at = |top: f64, h: f64| b.size.h - top - h; // AppKit y for a view `h` tall at `top`
    let label = |x: f64, top: f64, wd: f64, text: &str| -> *mut Object {
        let l = make_label(text, 12.0, false, false);
        set_frame(l, rect(x, at(top, LABEL_H), wd, LABEL_H));
        add_sub(c, l);
        l
    };
    let field = |x: f64, top: f64, wd: f64, text: &str, placeholder: &str, secure: bool| -> (*mut Object, *mut Object) {
        let l = label(x, top, wd, text);
        let f = text_field(placeholder, secure);
        set_frame(f, rect(x, at(top + LABEL_H + GAP, FIELD_H), wd, FIELD_H));
        add_sub(c, f);
        (l, f)
    };
    let popup = |x: f64, top: f64, wd: f64, items: &[&str]| -> *mut Object {
        let p: *mut Object = msg_send![class!(NSPopUpButton), alloc];
        let p: *mut Object = msg_send![p, initWithFrame: rect(x, at(top, POPUP_H), wd, POPUP_H) pullsDown: NO];
        let arr: *mut Object = msg_send![class!(NSMutableArray), array];
        for t in items {
            let _: () = msg_send![arr, addObject: ns(t)];
        }
        let _: () = msg_send![p, addItemsWithTitles: arr];
        add_sub(c, p);
        autorel(p)
    };
    let field_row = LABEL_H + GAP + FIELD_H + ROW_GAP;

    // ---- Jellyfin / playback ----
    let mut top = 20.;
    let (_, url_f) = field(X, top, FULL, "Server URL", "http://host:8096", false);
    ui.s_url = Ptr(url_f);
    top += field_row;
    let (_, sub_f) = field(X, top, HALF, "Subtitle language", "eng · blank = off", false);
    ui.s_sub_lang = Ptr(sub_f);
    let (_, audio_f) = field(COL2, top, HALF, "Audio language", "eng · blank = default", false);
    ui.s_audio_lang = Ptr(audio_f);
    top += field_row;
    label(X, top, HALF, "Preferred quality");
    label(COL2, top, HALF, "Appearance");
    ui.s_quality = Ptr(popup(X, top + LABEL_H + GAP, HALF, &["Original", "40 Mbps", "20 Mbps", "10 Mbps", "5 Mbps", "3 Mbps"]));
    ui.s_theme = Ptr(popup(COL2, top + LABEL_H + GAP, HALF, &["Match System", "Light", "Dark"]));
    top += LABEL_H + GAP + POPUP_H + 20.;
    add_sub(c, crate::ui::theme::hairline(rect(X, at(top, 1.), FULL, 1.)));
    top += 1. + 16.;

    // ---- Seerr ----
    let cb: *mut Object = msg_send![class!(NSButton),
        checkboxWithTitle: ns("Enable Seerr (request movies and shows)")
        target: nil() action: nil()];
    set_frame(cb, rect(X, at(top, 20.), FULL, 20.));
    add_sub(c, cb);
    ui.seerr.s_on = Ptr(cb);
    top += 20. + 12.;
    let (_, seerr_url_f) = field(X, top, FULL, "Seerr URL", "http://host:5055", false);
    ui.seerr.s_url = Ptr(seerr_url_f);
    top += field_row;

    // Sign-in mode: Jellyfin account (default) or API key. Label sits on the popup's text baseline.
    let ml = make_label("Sign in with", 12.0, false, false);
    set_frame(ml, rect(X, at(top + (POPUP_H - LABEL_H) / 2., LABEL_H), 90., LABEL_H));
    add_sub(c, ml);
    let mp = popup(X + 92., top, 220., &["Jellyfin account", "API key"]);
    let _: () = msg_send![mp, setTarget: ui.ctl.get()];
    let _: () = msg_send![mp, setAction: sel!(seerrModeChanged:)];
    ui.seerr.s_mode = Ptr(mp);
    top += POPUP_H + ROW_GAP;

    let (ul, uf) = field(X, top, FULL, "Seerr username (your Jellyfin username)", "username", false);
    ui.seerr.s_user = Ptr(uf);
    let (kl, kf) = field(X, top, FULL, "Seerr API key (Seerr > Settings > General)", "API key", true);
    ui.seerr.s_key = Ptr(kf);
    let (pl, pf) = field(X, top + field_row, FULL, "Seerr password (your Jellyfin password)", "password", true);
    ui.seerr.s_pass = Ptr(pf);
    ui.seerr.s_login_group = vec![Ptr(ul), Ptr(uf), Ptr(pl), Ptr(pf)];
    ui.seerr.s_key_group = vec![Ptr(kl), Ptr(kf)];
    top += 2. * field_row;

    let seerr_st = make_label("", 12.0, false, false);
    set_frame(seerr_st, rect(X, at(top, 32.), FULL, 32.));
    let _: () = msg_send![seerr_st, setTextColor: color_named("secondary")];
    let _: () = msg_send![seerr_st.cell(), setLineBreakMode: 0usize]; // wrap
    add_sub(c, seerr_st);
    ui.seerr.s_status = Ptr(seerr_st);
    crate::ui::seerr::apply_mode();

    let save: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Save")
        target: ui.ctl.get() action: sel!(saveSettings:)];
    set_frame(save, rect(b.size.w - 116., 16., 96., 32.));
    let _: () = msg_send![save, setKeyEquivalent: ns("\r")];
    add_sub(c, save);
    let cancel: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Cancel")
        target: ui.ctl.get() action: sel!(cancelSettings:)];
    set_frame(cancel, rect(b.size.w - 222., 16., 96., 32.));
    let _: () = msg_send![cancel, setKeyEquivalent: ns("\u{1b}")];
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
