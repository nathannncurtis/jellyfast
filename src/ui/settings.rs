#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::app::libraries::{move_by, ordered_views};
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
    release_obj(w); // Ptr::retained took its own +1; drop the alloc/init +1 (same as seerr.rs)
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
    let url_f = text_field("host:8096", false);
    row(&mut place, &mut top, "Server URL", url_f, 24., CTRL_W);
    ui.s_url = Ptr(url_f);

    section(&mut place, &mut top, "Browsing", false);
    let lay = popup(&["Separate libraries", "One big library"]);
    row(&mut place, &mut top, "Sidebar", lay, 26., 200.);
    help(&mut place, &mut top, "One big library shows Home, Browse, Search and Requests, with movies and shows mixed.");
    ui.s_layout = Ptr(lay);

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
    let th = popup(&["Follow system"]);
    row(&mut place, &mut top, "Theme", th, 26., 200.);
    let _: () = msg_send![th, setTarget: ui.ctl.get()];
    let _: () = msg_send![th, setAction: sel!(themePicked:)];
    ui.s_theme = Ptr(th);
    let note = wrapping_label("", T_CAPTION, C_TEXT2, ALIGN_LEFT);
    put(&mut place, note, CTRL_X, top, CTRL_W, 30.);
    top += 30. + SP2;
    ui.s_theme_note = Ptr(note);
    // Follow system only: a light and a dark picker. Hidden (and the rows below shifted)
    // by `set_follow_rows` when a single theme is chosen.
    let sub_top0 = top;
    let sub_start = place.len();
    let lp = popup(&["Jellyfast Light"]);
    row(&mut place, &mut top, "Light mode theme", lp, 26., 200.);
    ui.s_light = Ptr(lp);
    top += SP2;
    let dp = popup(&["Jellyfast Dark"]);
    row(&mut place, &mut top, "Dark mode theme", dp, 26., 200.);
    ui.s_dark = Ptr(dp);
    top += SP2;
    let sub_end = place.len();
    let sub_h = top - sub_top0;
    // Theme folder: always visible, under the pickers.
    let folder_btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Open themes folder")
        target: ui.ctl.get() action: sel!(openThemesFolder:)];
    let _: () = msg_send![folder_btn, setBezelStyle: 1isize];
    put(&mut place, folder_btn, CTRL_X, top, 160., 24.);
    let reload_btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Reload themes")
        target: ui.ctl.get() action: sel!(reloadThemes:)];
    let _: () = msg_send![reload_btn, setBezelStyle: 1isize];
    put(&mut place, reload_btn, CTRL_X + 170., top, 120., 24.);
    top += 24. + SP3;

    // Libraries: pick one, then reorder it or hide it. Filled from the server views on open.
    section(&mut place, &mut top, "Libraries", false);
    let lp = popup(&["No libraries yet"]);
    let _: () = msg_send![lp, setTarget: ui.ctl.get()];
    let _: () = msg_send![lp, setAction: sel!(libSelected:)];
    let lib_row_top = top;
    row(&mut place, &mut top, "Library", lp, 26., 200.);
    ui.s_lib_pop = Ptr(lp);
    let arrow = |name: &str, tip: &str, action: objc::runtime::Sel| -> *mut Object {
        let b: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
            target: ui.ctl.get() action: action];
        let _: () = msg_send![b, setBordered: NO];
        let img = symbol(name, 12., Weight::Semibold);
        if !img.is_null() {
            let _: () = msg_send![b, setImage: img];
        }
        let _: () = msg_send![b, setImagePosition: 1isize]; // image only
        let _: () = msg_send![b, setContentTintColor: color(C_TEXT2)];
        let _: () = msg_send![b, setToolTip: ns(tip)];
        b
    };
    let up = arrow("chevron.up", "Move up", sel!(libMoveUp:));
    let down = arrow("chevron.down", "Move down", sel!(libMoveDown:));
    put(&mut place, up, CTRL_X + 200. + SP2, lib_row_top + (ROW_H - 24.) / 2., 24., 24.);
    put(&mut place, down, CTRL_X + 200. + SP2 + 24. + SP1, lib_row_top + (ROW_H - 24.) / 2., 24., 24.);
    ui.s_lib_up = Ptr(up);
    ui.s_lib_down = Ptr(down);
    let show: *mut Object = msg_send![class!(NSButton),
        checkboxWithTitle: ns("Show in sidebar")
        target: ui.ctl.get() action: sel!(libVisibleToggled:)];
    put(&mut place, show, CTRL_X, top + (ROW_H - 18.) / 2., CTRL_W, 18.);
    top += ROW_H;
    ui.s_lib_show = Ptr(show);
    help(&mut place, &mut top, "Pick a library, then use the arrows to move it. Hidden libraries stay on the server.");

    section(&mut place, &mut top, "Requests (Seerr)", false);
    let cb: *mut Object = msg_send![class!(NSButton),
        checkboxWithTitle: ns("Enable requests")
        target: nil() action: nil()];
    put(&mut place, cb, CTRL_X, top + (ROW_H - 18.) / 2., CTRL_W, 18.);
    top += ROW_H + SP3;
    ui.seerr.s_on = Ptr(cb);
    let seerr_url_f = text_field("host:5055", false);
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
    ui.s_follow = FollowLayout {
        items: place
            .iter()
            .enumerate()
            .map(|(i, (v, r))| LayoutItem {
                v: Ptr(*v),
                t: r.origin.y,
                x: r.origin.x,
                w: r.size.w,
                h: r.size.h,
                sub: i >= sub_start && i < sub_end,
                below: i >= sub_end,
            })
            .collect(),
        sub_h,
        base_h: top,
        cur_h: top,
    };
    for (v, r) in place {
        set_frame(v, rect(r.origin.x, top - r.origin.y - r.size.h, r.size.w, r.size.h));
    }
    crate::ui::seerr::apply_mode();
}

pub(crate) unsafe fn settings_fill(cfg: &Config) {
    let ui = UI.get_mut().unwrap();
    if let Some(u) = &cfg.server_url {
        set_text(ui.s_url.get(), &crate::jellyfin::urls::bare_url(u));
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
    fill_theme_popups(cfg);
    let l_idx = if cfg.layout_mode == Some(crate::config::LayoutMode::Unified) { 1isize } else { 0 };
    let _: () = msg_send![ui.s_layout.get(), selectItemAtIndex: l_idx];
    let _: () = msg_send![ui.seerr.s_on.get(), setState: cfg.settings.seerr_enabled as NSInteger];
    set_text(ui.seerr.s_url.get(), &crate::jellyfin::urls::bare_url(&cfg.settings.seerr_url));
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
    ui.lib_edit = ordered_views(&ui.all_views, &cfg.library_order);
    ui.lib_hidden = cfg.hidden_libraries.clone();
    ui.lib_sel = 0;
    lib_refresh();
}

/// Repaint the Libraries controls from the Settings working copy (`ui.lib_edit`).
pub(crate) unsafe fn lib_refresh() {
    let ui = UI.get_mut().unwrap();
    let pop = ui.s_lib_pop.get();
    let _: () = msg_send![pop, removeAllItems];
    let n = ui.lib_edit.len();
    let enable = |v: *mut Object, on: bool| {
        let _: () = msg_send![v, setEnabled: if on { YES } else { NO }];
    };
    if n == 0 {
        let _: () = msg_send![pop, addItemWithTitle: ns("No libraries yet")];
        enable(pop, false);
        enable(ui.s_lib_show.get(), false);
        enable(ui.s_lib_up.get(), false);
        enable(ui.s_lib_down.get(), false);
        let _: () = msg_send![ui.s_lib_show.get(), setState: 0isize];
        return;
    }
    for (id, name, _) in &ui.lib_edit {
        let title = if ui.lib_hidden.contains(id) { format!("{name} (hidden)") } else { name.clone() };
        let _: () = msg_send![pop, addItemWithTitle: ns(&title)];
    }
    ui.lib_sel = ui.lib_sel.min(n - 1);
    let _: () = msg_send![pop, selectItemAtIndex: ui.lib_sel as NSInteger];
    let shown = !ui.lib_hidden.contains(&ui.lib_edit[ui.lib_sel].0);
    let _: () = msg_send![ui.s_lib_show.get(), setState: shown as NSInteger];
    enable(pop, true);
    enable(ui.s_lib_show.get(), true);
    enable(ui.s_lib_up.get(), ui.lib_sel > 0);
    enable(ui.s_lib_down.get(), ui.lib_sel + 1 < n);
}

pub(crate) extern "C" fn lib_selected(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let i: NSInteger = msg_send![ui.s_lib_pop.get(), indexOfSelectedItem];
        ui.lib_sel = i.max(0) as usize;
        lib_refresh();
    }
}

/// Checkbox: show or hide the selected library. The last visible library cannot be hidden,
/// so the box snaps back instead.
pub(crate) extern "C" fn lib_visible_toggled(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.lib_sel >= ui.lib_edit.len() {
            return;
        }
        let on: NSInteger = msg_send![ui.s_lib_show.get(), state];
        let id = ui.lib_edit[ui.lib_sel].0.clone();
        if on != 0 {
            ui.lib_hidden.retain(|h| h != &id);
        } else {
            let others = ui.lib_edit.iter().filter(|v| v.0 != id && !ui.lib_hidden.contains(&v.0)).count();
            if others > 0 {
                ui.lib_hidden.push(id);
            }
        }
        lib_refresh();
    }
}

pub(crate) extern "C" fn lib_move_up(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { lib_move(true) }
}

pub(crate) extern "C" fn lib_move_down(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { lib_move(false) }
}

unsafe fn lib_move(up: bool) {
    let ui = UI.get_mut().unwrap();
    if ui.lib_sel < ui.lib_edit.len() {
        ui.lib_sel = move_by(&mut ui.lib_edit, ui.lib_sel, up);
    }
    lib_refresh();
}

/// Called by Save before the config is written. Applies the Libraries working copy to
/// `ui.cfg`, tells the worker when it changed, and rebuilds the sidebar. An empty working
/// copy (never connected, so no libraries) leaves the saved preferences alone.
pub(crate) unsafe fn commit_library_prefs() {
    let ui = UI.get_mut().unwrap();
    if ui.lib_edit.is_empty() {
        return;
    }
    let order: Vec<String> = ui.lib_edit.iter().map(|v| v.0.clone()).collect();
    let hidden = ui.lib_hidden.clone();
    let saved: Vec<String> = ordered_views(&ui.all_views, &ui.cfg.library_order).iter().map(|v| v.0.clone()).collect();
    if order == saved && hidden == ui.cfg.hidden_libraries {
        return;
    }
    ui.cfg.library_order = order.clone();
    ui.cfg.hidden_libraries = hidden.clone();
    let _ = ui.ui_tx.send(UiCmd::SaveLibraries { order, hidden });
    crate::ui::shell::apply_library_prefs();
}

/// One Settings row as laid out, so the Follow-system pickers can be shown or hidden
/// and everything below them moved without rebuilding the window.
pub(crate) struct LayoutItem {
    pub v: Ptr,
    /// Distance from the top of the content view.
    pub t: f64,
    pub x: f64,
    pub w: f64,
    pub h: f64,
    /// One of the two Follow-system picker rows (label or popup).
    pub sub: bool,
    /// Sits below the picker rows, so it moves up when they are hidden.
    pub below: bool,
}

#[derive(Default)]
pub(crate) struct FollowLayout {
    pub items: Vec<LayoutItem>,
    pub sub_h: f64,
    pub base_h: f64,
    pub cur_h: f64,
}

/// Append a saved selection the popup does not list (a missing palette), so the picker
/// still shows what is in effect.
fn with_saved(mut items: Vec<(String, String)>, selected: &str, cache: &std::collections::BTreeMap<String, String>) -> Vec<(String, String)> {
    if selected.is_empty() || items.iter().any(|(id, _)| id == selected) {
        return items;
    }
    let name = crate::ui::palettes::find(selected, cache)
        .map(|(t, _)| t.name)
        .unwrap_or_else(|| format!("{selected} (missing)"));
    items.push((selected.to_string(), name));
    items
}

/// Fill a popup from `(id, name)` pairs (an empty id is a separator) and select `selected`.
unsafe fn fill_popup(p: *mut Object, items: &[(String, String)], ids: &mut Vec<String>, selected: &str) {
    let _: () = msg_send![p, removeAllItems];
    ids.clear();
    let mut idx = 0usize;
    for (id, name) in items {
        if id.is_empty() {
            let menu: *mut Object = msg_send![p, menu];
            let sep: *mut Object = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];
        } else {
            let _: () = msg_send![p, addItemWithTitle: ns(name)];
            if id == selected {
                idx = ids.len();
            }
        }
        ids.push(id.clone());
    }
    let _: () = msg_send![p, selectItemAtIndex: idx as NSInteger];
}

/// Repopulate the Theme, Light-mode and Dark-mode pickers from the palette registry,
/// select the saved choices, show the fallback note, and show or hide the Follow-system
/// pickers. Called on open, after Reload themes and after Save.
pub(crate) unsafe fn fill_theme_popups(cfg: &Config) {
    let ui = UI.get_mut().unwrap();
    let cache = &cfg.palette_cache;
    let cur = if cfg.settings.theme.is_empty() { "system".to_string() } else { cfg.settings.theme.clone() };
    let top = with_saved(crate::ui::palettes::theme_choices(), &cur, cache);
    let mut theme_ids = std::mem::take(&mut ui.s_theme_ids);
    fill_popup(ui.s_theme.get(), &top, &mut theme_ids, &cur);
    ui.s_theme_ids = theme_ids;

    let light_sel = if cfg.settings.system_light.is_empty() { "light".to_string() } else { cfg.settings.system_light.clone() };
    let dark_sel = if cfg.settings.system_dark.is_empty() { "dark".to_string() } else { cfg.settings.system_dark.clone() };
    let light = with_saved(crate::ui::palettes::mode_choices(false), &light_sel, cache);
    let mut light_ids = std::mem::take(&mut ui.s_light_ids);
    fill_popup(ui.s_light.get(), &light, &mut light_ids, &light_sel);
    ui.s_light_ids = light_ids;
    let dark = with_saved(crate::ui::palettes::mode_choices(true), &dark_sel, cache);
    let mut dark_ids = std::mem::take(&mut ui.s_dark_ids);
    fill_popup(ui.s_dark.get(), &dark, &mut dark_ids, &dark_sel);
    ui.s_dark_ids = dark_ids;

    let note = crate::ui::shell::THEME_NOTE.lock().map(|n| n.clone()).unwrap_or_default();
    set_text(ui.s_theme_note.get(), &note);
    set_follow_rows(cur == "system");
}

/// Show or hide the Follow-system picker rows. Hiding moves the rows below them up and
/// shrinks the window from the top, so the layout stays tight in both states.
pub(crate) unsafe fn set_follow_rows(show: bool) {
    let ui = UI.get_mut().unwrap();
    let f = &mut ui.s_follow;
    if f.items.is_empty() {
        return;
    }
    let delta = if show { 0. } else { -f.sub_h };
    let new_h = f.base_h + delta;
    for it in &f.items {
        let t = if it.below { it.t + delta } else { it.t };
        if it.sub {
            let _: () = msg_send![it.v.get(), setHidden: if show { NO } else { YES }];
        }
        set_frame(it.v.get(), rect(it.x, new_h - t - it.h, it.w, it.h));
    }
    let dh = new_h - f.cur_h;
    if dh.abs() > 0.5 {
        let win = ui.settings_win.get();
        let fr: NSRect = msg_send![win, frame];
        let nf = rect(fr.origin.x, fr.origin.y - dh, fr.size.w, fr.size.h + dh);
        let _: () = msg_send![win, setFrame: nf display: YES animate: NO];
    }
    f.cur_h = new_h;
}

/// Theme popup action: Follow system shows the Light/Dark mode pickers.
pub(crate) extern "C" fn theme_picked(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let i: NSInteger = msg_send![ui.s_theme.get(), indexOfSelectedItem];
        let id = ui.s_theme_ids.get(i.max(0) as usize).cloned().unwrap_or_default();
        set_follow_rows(id == "system");
    }
}

/// Selected id of a theme-style popup, or `fallback` when nothing valid is selected.
pub(crate) unsafe fn popup_choice(p: *mut Object, ids: &[String], fallback: &str) -> String {
    let i: NSInteger = msg_send![p, indexOfSelectedItem];
    ids.get(i.max(0) as usize).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| fallback.to_string())
}
