#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::library::*;
use crate::ui::settings::*;
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

pub(crate) unsafe fn build_window_and_ui() {
    let ui = UI.get_mut().unwrap();
    let mut frame = rect(0., 0., 1280., 800.);
    if let Some([x, y, w, h]) = ui.cfg.window {
        if w > 400.0 && h > 300.0 {
            frame = rect(x, y, w, h);
        }
    }
    let style: NSUInteger = 1 | 2 | 4 | 8 | (1 << 15); // titled|closable|mini|resizable|fullSizeContent
    let win: *mut Object = msg_send![class!(NSWindow), alloc];
    let win: *mut Object = msg_send![win,
        initWithContentRect: frame styleMask: style backing: 2usize defer: NO];
    let _: () = msg_send![win, setTitle: ns("Jellyfin")];
    let _: () = msg_send![win, setTitlebarAppearsTransparent: YES];
    let _: () = msg_send![win, setReleasedWhenClosed: NO];
    let _: () = msg_send![win, setTabbingMode: 2i64]; // disallowed
    let _: () = msg_send![win, setMinSize: size(960., 600.)];
    let _: () = msg_send![win, setOpaque: NO];
    let clear: *mut Object = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![win, setBackgroundColor: clear];
    let _: () = msg_send![win, center];
    ui.win = Ptr::retained(win);

    let content: *mut Object = msg_send![win, contentView];
    let b = bounds_of(content);

    // Background vibrancy (whole-window glass; adapts to light/dark and to
    // macOS 15's material rendering automatically).
    let bg = vev(MAT_UNDER_WINDOW, BLEND_BEHIND);
    set_frame(bg, b);
    set_autoresize(bg, RESIZE_W | RESIZE_H);
    add_sub(content, bg);
    ui.bgfx = Ptr(bg);

    // --- Content host (library side) ---
    let content_host: *mut Object = msg_send![class!(NSView), alloc];
    let content_host: *mut Object = msg_send![content_host, initWithFrame: b];
    set_autoresize(content_host, RESIZE_W | RESIZE_H);
    add_sub(content, content_host);
    autorel(content_host);
    ui.content_host = Ptr(content_host);

    // Sidebar
    let sb = vev(MAT_SIDEBAR, BLEND_BEHIND);
    set_frame(sb, rect(0., 0., 240., b.size.h));
    set_autoresize(sb, RESIZE_H | MARGIN_MAXX);
    add_sub(content_host, sb);
    ui.sidebar = Ptr(sb);

    let stack: *mut Object = msg_send![class!(NSStackView), alloc];
    let stack: *mut Object = msg_send![stack, initWithFrame: rect(12., 12., 216., b.size.h - 24.)];
    let _: () = msg_send![stack, setOrientation: 1i64]; // vertical
    let _: () = msg_send![stack, setAlignment: 0i64]; // leading
    let _: () = msg_send![stack, setSpacing: 6.0f64];
    set_autoresize(stack, RESIZE_H | RESIZE_W);
    add_sub(sb, stack);
    autorel(stack);
    ui.side_stack = Ptr(stack);

    let title = make_label("Jellyfin", 17.0, true, false);
    add_arranged(stack, title, 12.0, 6.0);

    // Search field
    let sf: *mut Object = msg_send![class!(NSSearchField), alloc];
    let sf: *mut Object = msg_send![sf, initWithFrame: rect(0., 0., 216., 28.)];
    let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search libraries")];
    let _: () = msg_send![sf, setTarget: ui.ctl.get()];
    let _: () = msg_send![sf, setDelegate: ui.ctl.get()]; // controlTextDidChange: -> Search
    let _: () = msg_send![sf, setSendsWholeSearchString: NO];
    add_arranged(stack, sf, 0.0, 4.0);
    autorel(sf);
    ui.search = Ptr(sf);

    // Libraries get injected by sidebar_rebuild(); spacer + buttons below.
    let settings_btn: *mut Object = msg_send![class!(NSButton),
        buttonWithTitle: ns("Settings…") target: ui.ctl.get() action: sel!(showSettings:)];
    add_arranged(stack, settings_btn, 0.0, 2.0);
    let searx_btn: *mut Object = msg_send![class!(NSButton),
        buttonWithTitle: ns("Find something new (SearXNG)") target: ui.ctl.get() action: sel!(openSearx:)];
    add_arranged(stack, searx_btn, 0.0, 2.0);

    // Header row (title + sort)
    let ht = make_label("Library", 20.0, true, false);
    set_frame(ht, rect(264., b.size.h - 52., 500., 26.));
    set_autoresize(ht, MARGIN_MINY);
    add_sub(content_host, ht);
    ui.header_title = Ptr(ht);

    let pop: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let pop: *mut Object = msg_send![pop, initWithFrame: rect(0., 0., 150., 26.) pullsDown: NO];
    let arr: *mut Object = msg_send![class!(NSMutableArray), array];
    for t in ["Name (A–Z)", "Newest first", "Highest rated", "Recently played"] {
        let _: () = msg_send![arr, addObject: ns(t)];
    }
    let _: () = msg_send![pop, addItemsWithTitles: arr];
    let _: () = msg_send![pop, setTarget: ui.ctl.get()];
    let _: () = msg_send![pop, setAction: sel!(sortChanged:)];
    let pb = bounds_of(content_host);
    set_frame(pop, rect(pb.size.w - 170., pb.size.h - 54., 150., 26.));
    set_autoresize(pop, MARGIN_MINX | MARGIN_MINY);
    add_sub(content_host, pop);
    autorel(pop);
    ui.sort_popup = Ptr(pop);

    // Grid scroll view
    let sc: *mut Object = msg_send![class!(NSScrollView), alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: rect(256., 0., pb.size.w - 256. - 12., pb.size.h - 60.)];
    let _: () = msg_send![sc, setHasVerticalScroller: YES];
    let _: () = msg_send![sc, setDrawsBackground: NO];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    add_sub(content_host, sc);
    autorel(sc);
    ui.lib_scroll = Ptr(sc);

    let grid_cls = *GRID_CLASS.get().unwrap();
    let grid: *mut Object = msg_send![grid_cls, alloc];
    let grid: *mut Object = msg_send![grid, initWithFrame: rect(0., 0., pb.size.w - 256. - 28., 10.)];
    // One GridData box, owned by ui.grid_data and also referenced by the grid's ivar.
    ui.grid_data = Some(Box::new(GridData { cells: vec![] }));
    let data: *mut GridData = &mut **ui.grid_data.as_mut().unwrap();
    (*grid).set_ivar::<*mut c_void>("jrData", data as *mut c_void);
    let _: *mut Object = msg_send![sc, setDocumentView: grid];
    autorel(grid);
    ui.grid = Ptr(grid);

    // Spinner + status label
    let sp: *mut Object = msg_send![class!(NSProgressIndicator), alloc];
    let sp: *mut Object = msg_send![sp, initWithFrame: rect(pb.size.w / 2. - 12., pb.size.h / 2., 24., 24.)];
    let _: () = msg_send![sp, setIndeterminate: YES];
    let _: () = msg_send![sp, setStyle: 1i64]; // spinning
    let _: () = msg_send![sp, setUsesThreadedAnimation: YES];
    set_autoresize(sp, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(content_host, sp);
    autorel(sp);
    ui.spinner = Ptr(sp);
    let _: () = msg_send![sp, startAnimation: nil()];

    let st = make_label("", 13.0, false, true);
    set_frame(st, rect(260., pb.size.h / 2. - 30., pb.size.w - 280., 20.));
    set_autoresize(st, RESIZE_W | MARGIN_MINY | MARGIN_MAXY);
    let _: () = msg_send![st, setTextColor: color_named("secondary")];
    add_sub(content_host, st);
    ui.status_label = Ptr(st);

    // --- Player host (AVKit) + top bar + chip ---
    let ph: *mut Object = msg_send![class!(NSView), alloc];
    let ph: *mut Object = msg_send![ph, initWithFrame: b];
    set_autoresize(ph, RESIZE_W | RESIZE_H);
    add_sub(content, ph);
    autorel(ph);
    ui.player_host = Ptr(ph);
    set_hidden(ph, true);

    let avv: *mut Object = msg_send![class!(AVPlayerView), alloc];
    let avv: *mut Object = msg_send![avv, initWithFrame: b];
    set_autoresize(avv, RESIZE_W | RESIZE_H);
    let _: () = msg_send![avv, setShowsFullScreenToggleButton: YES];
    let _: () = msg_send![avv, setShowsFrameSteppingButtons: NO];
    let _: () = msg_send![avv, setShowsSharingServiceButton: NO];
    let _: () = msg_send![avv, setAllowsPictureInPicturePlayback: YES];
    add_sub(ph, avv);
    autorel(avv);
    ui.avview = Ptr(avv);

    let player: *mut Object = autorel(msg_send![class!(AVPlayer), new]); // empty until something plays
    // about:blank won't play; we replace the item on every real playback.
    ui.player = Ptr::retained(player);
    let _: () = msg_send![avv, setPlayer: player];
    let _: () = msg_send![player, setVolume: ui.cfg.volume.clamp(0.0, 1.0) as f32];

    let bar = vev(MAT_HUD, BLEND_WITHIN);
    let _: () = msg_send![bar, setAlphaValue: 0.92f64];
    set_frame(bar, rect(0., b.size.h - 44., b.size.w, 44.));
    set_autoresize(bar, RESIZE_W | MARGIN_MINY);
    add_sub(ph, bar);
    ui.top_bar = Ptr(bar);
    let l = layer_of(bar);
    let _: () = msg_send![l, setMasksToBounds: YES];

    let back: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("‹  Back")
        target: ui.ctl.get() action: sel!(goBack:)];
    set_frame(back, rect(12., 8., 72., 28.));
    let _: () = msg_send![back, setBordered: NO];
    add_sub(bar, back);
    ui.back_btn = Ptr(back);

    let tt = make_label("", 14.0, true, false);
    set_frame(tt, rect(96., 12., b.size.w - 560., 20.));
    set_autoresize(tt, RESIZE_W);
    let _: () = msg_send![tt, setTextColor: color_named("white")];
    add_sub(bar, tt);
    ui.top_title = Ptr(tt);

    let chip = vev(MAT_HUD, BLEND_WITHIN);
    let _: () = msg_send![chip, setAlphaValue: 0.8f64];
    set_frame(chip, rect(b.size.w - 356., b.size.h - 78., 340., 24.));
    set_autoresize(chip, MARGIN_MINX | MARGIN_MINY);
    let cl = layer_of(chip);
    let _: () = msg_send![cl, setCornerRadius: 8.0f64];
    let _: () = msg_send![cl, setMasksToBounds: YES];
    add_sub(ph, chip);
    ui.chip = Ptr(chip);
    let chip_l = make_label("", 11.5, false, true);
    set_frame(chip_l, rect(6., 4., 328., 16.));
    let _: () = msg_send![chip_l, setTextColor: color_named("white")];
    add_sub(chip, chip_l);
    ui.chip_label = Ptr(chip_l);

    // --- Login card ---
    let card = vev(MAT_POPOVER, BLEND_WITHIN);
    let card_w = 340.;
    let card_h = 320.;
    set_frame(card, rect(b.size.w / 2. - card_w / 2., b.size.h / 2. - card_h / 2., card_w, card_h));
    set_autoresize(card, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    let cl = layer_of(card);
    let _: () = msg_send![cl, setCornerRadius: 14.0f64];
    let _: () = msg_send![cl, setMasksToBounds: YES];
    add_sub(content, card);
    ui.login_card = Ptr(card);

    let t = make_label("Sign in to Jellyfin", 17.0, true, false);
    set_frame(t, rect(24., card_h - 44., card_w - 48., 24.));
    add_sub(card, t);

    let url_f = text_field("Server URL (http://host:8096)", false);
    set_frame(url_f, rect(24., card_h - 96., card_w - 48., 26.));
    add_sub(card, url_f);
    ui.login_url = Ptr(url_f);
    let user_f = text_field("Username", false);
    set_frame(user_f, rect(24., card_h - 132., card_w - 48., 26.));
    add_sub(card, user_f);
    ui.login_user = Ptr(user_f);
    let pass_f = text_field("Password", true);
    set_frame(pass_f, rect(24., card_h - 168., card_w - 48., 26.));
    add_sub(card, pass_f);
    ui.login_pass = Ptr(pass_f);
    let key_f = text_field("API key (alternative to password)", false);
    set_frame(key_f, rect(24., card_h - 204., card_w - 48., 26.));
    add_sub(card, key_f);
    ui.login_key = Ptr(key_f);

    let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Connect")
        target: ui.ctl.get() action: sel!(connectClicked:)];
    set_frame(btn, rect(24., card_h - 244., card_w - 48., 32.));
    let _: () = msg_send![btn, setKeyEquivalent: ns("\r")];
    add_sub(card, btn);
    ui.login_btn = Ptr(btn);

    let err = make_label("", 12.0, false, false);
    set_frame(err, rect(24., 20., card_w - 48., 44.));
    let _: () = msg_send![err, setTextColor: color_named("red")];
    add_sub(card, err);
    ui.login_err = Ptr(err);

    // Restore remembered URL/user.
    if let Some(u) = &ui.cfg.server_url {
        set_text(url_f, u);
    }
    if let Some(u) = &ui.cfg.username {
        set_text(user_f, u);
    }

    // --- Settings window (presented as a sheet) ---
    build_settings_window();

    // --- Menus (native menu bar: playback is keyboard-operable here) ---
    build_menus();

    let _: () = msg_send![win, makeKeyAndOrderFront: nil()];
    #[allow(deprecated)]
    let _: () = msg_send![ui.app.get(), activateIgnoringOtherApps: YES];

    // 1s UI timer: chip/progress/autosave (target = controller).
    let timer: *mut Object = msg_send![class!(NSTimer),
        scheduledTimerWithTimeInterval: 1.0f64 target: ui.ctl.get()
        selector: sel!(jrTick:) userInfo: nil() repeats: YES];
    ui.timer = Ptr(timer);

    apply_theme();
}

pub(crate) unsafe fn add_arranged(stack: *mut Object, v: *mut Object, _top: CGFloat, _pad: CGFloat) {
    let _: () = msg_send![stack, addArrangedSubview: v];
}

pub(crate) unsafe fn apply_theme() {
    let ui = UI.get_mut().unwrap();
    let appearance: *mut Object = match ui.cfg.settings.theme.as_str() {
        "light" => {
            let a: *mut Object =
                msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameAqua")];
            a
        }
        "dark" => {
            let a: *mut Object =
                msg_send![class!(NSAppearance), appearanceNamed: ns("NSAppearanceNameDarkAqua")];
            a
        }
        _ => nil(),
    };
    let _: () = msg_send![ui.app.get(), setAppearance: appearance];
}

pub(crate) unsafe fn build_menus() {
    let ui = UI.get_mut().unwrap();
    let main_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let main_menu: *mut Object = msg_send![main_menu, initWithTitle: ns("MainMenu")];
    let _: () = msg_send![ui.app.get(), setMainMenu: main_menu];
    autorel(main_menu);

    // App menu
    let app_item: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let app_item: *mut Object = msg_send![app_item,
        initWithTitle: ns("JellyfinMac") action: nil() keyEquivalent: ns("")];
    let app_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let app_menu: *mut Object = msg_send![app_menu, initWithTitle: ns("JellyfinMac")];
    let _: () = msg_send![app_item, setSubmenu: app_menu];
    let _: () = msg_send![main_menu, addItem: app_item];
    autorel(app_item);
    let about: *mut Object = menu_item("About JellyfinMac", sel!(orderFrontStandardAboutPanel:), "");
    let _: () = msg_send![app_menu, addItem: about];
    let _: () = msg_send![app_menu, addItem: separator()];
    let settings_i: *mut Object = menu_item("Settings…", sel!(showSettings:), ",");
    let _: () = msg_send![settings_i, setTarget: ui.ctl.get()];
    let _: () = msg_send![app_menu, addItem: settings_i];
    let _: () = msg_send![app_menu, addItem: separator()];
    let quit: *mut Object = menu_item("Quit JellyfinMac", sel!(terminate:), "q");
    let _: () = msg_send![app_menu, addItem: quit];

    // Controls menu (playback is fully operable from the keyboard here).
    let ctl_item: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let ctl_item: *mut Object = msg_send![ctl_item,
        initWithTitle: ns("Controls") action: nil() keyEquivalent: ns("")];
    let ctl_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let ctl_menu: *mut Object = msg_send![ctl_menu, initWithTitle: ns("Controls")];
    let _: () = msg_send![ctl_item, setSubmenu: ctl_menu];
    let _: () = msg_send![main_menu, addItem: ctl_item];
    autorel(ctl_item);
    let add = |menu: *mut Object, title: &str, action: Sel, key: &str, mask: NSUInteger, tag: NSInteger| unsafe {
        let it: *mut Object = menu_item(title, action, key);
        let _: () = msg_send![it, setTarget: ui.ctl.get()];
        let _: () = msg_send![it, setKeyEquivalentModifierMask: mask];
        let _: () = msg_send![it, setTag: tag];
        let _: () = msg_send![menu, addItem: it];
    };
    const CMD: NSUInteger = 1 << 20;
    const ALT: NSUInteger = 1 << 19;
    const CTRL_CMD: NSUInteger = (1 << 18) | (1 << 20);
    add(ctl_menu, "Play / Pause", sel!(playPause:), " ", 0usize, 0);
    add(ctl_menu, "Stop", sel!(stopPlayback:), ".", CMD, 0);
    add(ctl_menu, "Back 10s", sel!(seekBack:), "\u{F702}", CMD, 0);
    add(ctl_menu, "Forward 10s", sel!(seekFwd:), "\u{F703}", CMD, 0);
    add(ctl_menu, "Volume Up", sel!(volUp:), "\u{F700}", CMD, 0);
    add(ctl_menu, "Volume Down", sel!(volDown:), "\u{F701}", CMD, 0);
    add(ctl_menu, "Mute", sel!(mute:), "m", ALT, 0);
    add(ctl_menu, "Next Subtitle Track", sel!(subCycle:), "s", ALT, 0);
    add(ctl_menu, "Next Audio Track", sel!(audioCycle:), "a", ALT, 0);
    add(ctl_menu, "Enter / Exit Full Screen", sel!(fullscreen:), "f", CTRL_CMD, 0);
    // Speed submenu
    let speed_item: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let speed_item: *mut Object = msg_send![speed_item,
        initWithTitle: ns("Playback Speed") action: nil() keyEquivalent: ns("")];
    let speed_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let speed_menu: *mut Object = msg_send![speed_menu, initWithTitle: ns("Playback Speed")];
    let _: () = msg_send![speed_item, setSubmenu: speed_menu];
    let _: () = msg_send![ctl_menu, addItem: speed_item];
    autorel(speed_item);
    for (label, tag) in [("0.5×", 50), ("0.75×", 75), ("1×", 100), ("1.25×", 125), ("1.5×", 150), ("2×", 200)] {
        add(speed_menu, label, sel!(setSpeed:), "", 0usize, tag);
    }

    // Go menu
    let go_item: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let go_item: *mut Object = msg_send![go_item,
        initWithTitle: ns("Go") action: nil() keyEquivalent: ns("")];
    let go_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let go_menu: *mut Object = msg_send![go_menu, initWithTitle: ns("Go")];
    let _: () = msg_send![go_item, setSubmenu: go_menu];
    let _: () = msg_send![main_menu, addItem: go_item];
    autorel(go_item);
    add(go_menu, "Refresh", sel!(refresh:), "r", CMD, 0);
    add(go_menu, "Back", sel!(goBack:), "[", CMD, 0);
    add(go_menu, "Sign Out", sel!(signOut:), "l", ALT, 0);
    add(go_menu, "Find something new (SearXNG)", sel!(openSearx:), "k", ALT, 0);
}

pub(crate) unsafe fn separator() -> *mut Object {
    msg_send![class!(NSMenuItem), separatorItem]
}

// ---- Sidebar ------------------------------------------------------------
pub(crate) unsafe fn sidebar_rebuild(user: String, views: &[(String, String, Option<String>)], has_live_tv: bool) {
    let ui = UI.get_mut().unwrap();
    let ui_tx = ui.ui_tx.clone();
    // Clear previous rows (stack retains arranged subviews; removing them
    // releases them — we never held our own +1).
    for (btn, _) in ui.rows.drain(..) {
        let _: () = msg_send![ui.side_stack.get(), removeArrangedSubview: btn];
        let _: () = msg_send![btn, removeFromSuperview];
    }
    let user_l = make_label(&format!("Signed in as {}", user), 11.0, false, false);
    let _: () = msg_send![user_l, setTextColor: color_named("secondary")];
    add_arranged(ui.side_stack.get(), user_l, 0.0, 2.0);
    ui.rows.push((user_l, String::new())); // removed with the rows on the next rebuild
    // Live TV needs a views entry so the button's tag resolves in viewSelected:.
    if has_live_tv && !views.iter().any(|v| v.2.as_deref() == Some("livetv")) {
        ui.views.push(("livetv".into(), "Live TV".into(), Some("livetv".into())));
    }
    let views: Vec<(String, String, Option<String>)> = ui.views.clone();

    for (i, (id, name, ct)) in views.iter().enumerate() {
        let symbol = match ct.as_deref() {
            Some("movies") => "film",
            Some("tvshows") => "tv",
            Some("music") => "music.note",
            Some("livetv") => "antenna.radiowaves.left.and.right",
            _ => "square.stack",
        };
        let btn: *mut Object = msg_send![class!(NSButton),
            buttonWithTitle: ns(name) target: ui.ctl.get() action: sel!(viewSelected:)];
        let _: () = msg_send![btn, setBordered: NO];
        let _: () = msg_send![btn, setTag: i as NSInteger];
        let img = symbol_image(symbol, 13.0);
        if !img.is_null() {
            let _: () = msg_send![btn, setImage: img];
            let _: () = msg_send![btn, setImagePosition: 2i64]; // imageLeft
        }
        add_arranged(ui.side_stack.get(), btn, 0.0, 0.0);
        ui.rows.push((btn, id.clone()));
    }
    let _ = ui_tx;
}

// ---- View switching ------------------------------------------------------
pub fn show_login_or_library(connected: bool) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_hidden(ui.content_host.get(), !connected);
        set_hidden(ui.login_card.get(), connected);
        if connected {
            let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.search.get()];
        } else {
            let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];
            set_text(ui.login_btn.get(), "Connect");
        }
    }
}

pub fn login_error(msg: &str) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_err.get(), msg);
        let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];
        set_text(ui.login_btn.get(), "Connect");
        set_hidden(ui.spinner.get(), true);
    }
}
