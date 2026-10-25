#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::theme::*;
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
    let _: () = msg_send![win, setTitle: ns("Jellyfast")];
    let _: () = msg_send![win, setTitleVisibility: 1i64]; // hidden; the title still names the window in menus
    let _: () = msg_send![win, setTitlebarAppearsTransparent: YES];
    let _: () = msg_send![win, setReleasedWhenClosed: NO];
    let _: () = msg_send![win, setTabbingMode: 2i64]; // disallowed
    let _: () = msg_send![win, setMinSize: size(960., 600.)];
    let _: () = msg_send![win, center];
    ui.win = Ptr::retained(win);

    let content: *mut Object = msg_send![win, contentView];
    let b = bounds_of(content);
    let _: () = msg_send![content, setWantsLayer: YES]; // crossfade() needs a layer

    // Login backdrop only: the main window itself is opaque.
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
    // Opaque canvas under header + grid (the sidebar is the only translucent surface).
    let canvas = surface(rect(SIDEBAR_W, 0., b.size.w - SIDEBAR_W, b.size.h), C_CANVAS, 0.);
    set_autoresize(canvas, RESIZE_W | RESIZE_H);
    add_sub(content_host, canvas);

    let sb = vev(MAT_SIDEBAR, BLEND_BEHIND);
    set_frame(sb, rect(0., 0., SIDEBAR_W, b.size.h));
    set_autoresize(sb, RESIZE_H | MARGIN_MAXX);
    add_sub(content_host, sb);
    ui.sidebar = Ptr(sb);

    let stack: *mut Object = msg_send![class!(NSStackView), alloc];
    let stack: *mut Object =
        msg_send![stack, initWithFrame: rect(SP3, 0., SIDEBAR_W - 2. * SP3, b.size.h - TITLEBAR_H)];
    let _: () = msg_send![stack, setOrientation: 1isize]; // vertical
    let _: () = msg_send![stack, setAlignment: 7isize]; // NSLayoutAttributeWidth: rows fill the width
    let _: () = msg_send![stack, setSpacing: 2.0f64];
    let _: () = msg_send![stack, setDetachesHiddenViews: YES];
    let _: () = msg_send![stack, setEdgeInsets: NSEdgeInsets { top: 0., left: 0., bottom: SP3, right: 0. }];
    set_autoresize(stack, RESIZE_H | RESIZE_W);
    add_sub(sb, stack);
    autorel(stack);
    ui.side_stack = Ptr(stack);

    // Search field (the only static sidebar row; libraries and footer are rebuilt).
    let sf: *mut Object = msg_send![class!(NSSearchField), alloc];
    let sf: *mut Object = msg_send![sf, initWithFrame: rect(0., 0., SIDEBAR_W - 2. * SP3, SIDEBAR_ROW_H)];
    let _: () = msg_send![sf.cell(), setPlaceholderString: ns("Search")];
    let _: () = msg_send![sf, setControlSize: 3usize];
    let _: () = msg_send![sf, setTarget: ui.ctl.get()];
    let _: () = msg_send![sf, setDelegate: ui.ctl.get()]; // controlTextDidChange: -> Search
    let _: () = msg_send![sf, setSendsWholeSearchString: NO];
    fix_height(sf, SIDEBAR_ROW_H);
    let _: () = msg_send![stack, addView: sf inGravity: 1isize];
    let _: () = msg_send![stack, setCustomSpacing: SP4 afterView: sf];
    autorel(sf);
    ui.search = Ptr(sf);

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
    let _: () = msg_send![sc, setDrawsBackground: YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
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
    set_hidden(sp, true);

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
    let card_w = 380.;
    let card_h = 416.;
    let card = surface(
        rect(b.size.w / 2. - card_w / 2., b.size.h / 2. - card_h / 2., card_w, card_h),
        C_CARD,
        R_CARD,
    );
    let _: () = msg_send![card, setBorderWidth: 1.0f64];
    let _: () = msg_send![card, setBorderColor: color(C_SEPARATOR)];
    set_autoresize(card, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    let _: () = msg_send![card, setWantsLayer: YES];
    let shadow: *mut Object = msg_send![class!(NSShadow), alloc];
    let shadow: *mut Object = autorel(msg_send![shadow, init]);
    let _: () = msg_send![shadow, setShadowBlurRadius: SH_CARD.radius];
    let _: () = msg_send![shadow, setShadowOffset: size(0., SH_CARD.dy)];
    let _: () = msg_send![shadow, setShadowColor: color_a_black_fn(SH_CARD.opacity as f64)];
    let _: () = msg_send![card, setShadow: shadow];
    add_sub(content, card);
    ui.login_card = Ptr(card);
    let cv: *mut Object = msg_send![card, contentView];

    let inner_x = SP8;
    let inner_w = card_w - 2. * SP8;
    let top = |t: f64, h: f64| card_h - t - h; // y from a distance below the card top

    let mark = symbol("play.rectangle.on.rectangle.fill", 36., Weight::Regular);
    let mv: *mut Object = msg_send![class!(NSImageView), alloc];
    let mv: *mut Object = autorel(msg_send![mv, initWithFrame: rect(inner_x, top(SP8, 44.), inner_w, 44.)]);
    if !mark.is_null() {
        let _: () = msg_send![mv, setImage: mark];
        let _: () = msg_send![mv, setContentTintColor: color(C_ACCENT)];
    }
    add_sub(cv, mv);
    let t = label("Jellyfast", T_TITLE, C_TEXT, ALIGN_CENTER);
    set_frame(t, rect(inner_x, top(SP8 + 44. + SP3, 26.), inner_w, 26.));
    add_sub(cv, t);
    let sub = label("Connect to your Jellyfin server", T_CALLOUT, C_TEXT2, ALIGN_CENTER);
    set_frame(sub, rect(inner_x, top(SP8 + 44. + SP3 + 26. + SP1, 16.), inner_w, 16.));
    add_sub(cv, sub);

    let fields_top = SP8 + 44. + SP3 + 26. + SP1 + 16. + SP6; // 148
    let fh = 28.;
    let large_field = |ph: &str, secure: bool, row: f64| -> *mut Object {
        let f = text_field(ph, secure);
        let _: () = msg_send![f, setControlSize: 3usize]; // large
        let _: () = msg_send![f, setFont: font(T_BODY)];
        set_frame(f, rect(inner_x, top(fields_top + row * (fh + SP3), fh), inner_w, fh));
        add_sub(cv, f);
        f
    };
    let url_f = large_field("jellyfin.local:8096", false, 0.);
    ui.login_url = Ptr(url_f);
    let user_f = large_field("Username", false, 1.);
    ui.login_user = Ptr(user_f);
    let pass_f = large_field("Password", true, 2.);
    ui.login_pass = Ptr(pass_f);
    let key_f = large_field("API key", true, 2.);
    set_hidden(key_f, true);
    ui.login_key = Ptr(key_f);

    let tog_top = fields_top + 3. * fh + 2. * SP3 + SP2;
    let tog: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Use an API key instead")
        target: ui.ctl.get() action: sel!(toggleApiKey:)];
    let _: () = msg_send![tog, setBordered: NO];
    let _: () = msg_send![tog, setFont: font(T_CALLOUT)];
    let _: () = msg_send![tog, setContentTintColor: color(C_ACCENT)];
    let _: () = msg_send![tog, setAlignment: ALIGN_LEFT as isize];
    set_frame(tog, rect(inner_x - 4., top(tog_top, 18.), 220., 18.));
    add_sub(cv, tog);
    ui.login_toggle = Ptr(tog);

    let btn_top = tog_top + 18. + SP5;
    let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Connect")
        target: ui.ctl.get() action: sel!(connectClicked:)];
    let _: () = msg_send![btn, setBezelStyle: 1isize]; // push
    let _: () = msg_send![btn, setControlSize: 3usize];
    let _: () = msg_send![btn, setKeyEquivalent: ns("\r")];
    set_frame(btn, rect(inner_x, top(btn_top, 32.), inner_w, 32.));
    add_sub(cv, btn);
    ui.login_btn = Ptr(btn);

    let st_top = btn_top + 32. + SP3;
    let lsp: *mut Object = msg_send![class!(NSProgressIndicator), alloc];
    let lsp: *mut Object = autorel(msg_send![lsp, initWithFrame: rect(inner_x, top(st_top, 16.), 16., 16.)]);
    let _: () = msg_send![lsp, setStyle: 1isize]; // spinning
    let _: () = msg_send![lsp, setControlSize: 1usize]; // small
    let _: () = msg_send![lsp, setIndeterminate: YES];
    let _: () = msg_send![lsp, setUsesThreadedAnimation: YES];
    set_hidden(lsp, true);
    add_sub(cv, lsp);
    ui.login_spin = Ptr(lsp);

    let err = wrapping_label("", T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    set_frame(err, rect(inner_x, top(st_top, 48.), inner_w, 48.));
    add_sub(cv, err);
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
    // Launch state: only the login card (the library chrome is hidden until a
    // session exists); `app_did_finish_launch` turns it into a connecting state.
    show_login_or_library(false);
}

pub(crate) unsafe fn add_arranged(stack: *mut Object, v: *mut Object, _top: CGFloat, _pad: CGFloat) {
    let _: () = msg_send![stack, addArrangedSubview: v];
}

pub(crate) unsafe fn apply_theme() {
    let ui = UI.get_mut().unwrap();
    // Self-test only: JELLYFIN_MAC_APPEARANCE=light|dark pins the appearance for screenshots.
    let forced = if crate::app::selftest::enabled() { std::env::var("JELLYFIN_MAC_APPEARANCE").ok() } else { None };
    let theme = forced.unwrap_or_else(|| ui.cfg.settings.theme.clone());
    let appearance: *mut Object = match theme.as_str() {
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
        initWithTitle: ns("Jellyfast") action: nil() keyEquivalent: ns("")];
    let app_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let app_menu: *mut Object = msg_send![app_menu, initWithTitle: ns("Jellyfast")];
    let _: () = msg_send![app_item, setSubmenu: app_menu];
    let _: () = msg_send![main_menu, addItem: app_item];
    autorel(app_item);
    let about: *mut Object = menu_item("About Jellyfast", sel!(orderFrontStandardAboutPanel:), "");
    let _: () = msg_send![app_menu, addItem: about];
    let _: () = msg_send![app_menu, addItem: separator()];
    let settings_i: *mut Object = menu_item("Settings…", sel!(showSettings:), ",");
    let _: () = msg_send![settings_i, setTarget: ui.ctl.get()];
    let _: () = msg_send![app_menu, addItem: settings_i];
    let _: () = msg_send![app_menu, addItem: separator()];
    let quit: *mut Object = menu_item("Quit Jellyfast", sel!(terminate:), "q");
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
    add(go_menu, "Request Something New…", sel!(openSeerr:), "k", ALT, 0);
}

pub(crate) unsafe fn separator() -> *mut Object {
    msg_send![class!(NSMenuItem), separatorItem]
}

// ---- Sidebar ------------------------------------------------------------
/// Pin a view's height (arranged stack views have no autoresizing constraints).
pub(crate) unsafe fn fix_height(v: *mut Object, h: f64) {
    let _: () = msg_send![v, setTranslatesAutoresizingMaskIntoConstraints: NO];
    let anchor: *mut Object = msg_send![v, heightAnchor];
    let c: *mut Object = msg_send![anchor, constraintEqualToConstant: h];
    let _: () = msg_send![c, setActive: YES];
}

pub(crate) struct RowData {
    bg: *mut Object,
    selected: bool,
}
pub(crate) static ROW_CLASS: OnceLock<&'static Class> = OnceLock::new();

pub(crate) unsafe fn register_sidebar_row() {
    let mut d = ClassDecl::new("JRSidebarRow", class!(NSView)).unwrap();
    d.add_ivar::<*mut c_void>("jrRow");
    d.add_method(sel!(isFlipped), row_flipped as extern "C" fn(&mut Object, Sel) -> BOOL);
    d.add_method(sel!(mouseEntered:), row_enter as extern "C" fn(&mut Object, Sel, *mut Object));
    d.add_method(sel!(mouseExited:), row_exit as extern "C" fn(&mut Object, Sel, *mut Object));
    ROW_CLASS.set(d.register()).ok();
}
extern "C" fn row_flipped(_t: &mut Object, _c: Sel) -> BOOL {
    YES
}
unsafe fn row_data(this: *mut Object) -> *mut RowData {
    *(*this).get_ivar::<*mut c_void>("jrRow") as *mut RowData
}
unsafe fn row_paint(rd: &RowData, hover: bool) {
    if rd.selected {
        let _: () = msg_send![rd.bg, setFillColor: color(C_FILL_SEL)];
        let _: () = msg_send![rd.bg, setTransparent: NO];
    } else if hover {
        let _: () = msg_send![rd.bg, setFillColor: color(C_FILL_HOVER)];
        let _: () = msg_send![rd.bg, setTransparent: NO];
    } else {
        let _: () = msg_send![rd.bg, setTransparent: YES];
    }
}
extern "C" fn row_enter(this: &mut Object, _c: Sel, _e: *mut Object) {
    unsafe {
        let rd = row_data(this);
        if !rd.is_null() {
            row_paint(&*rd, true);
        }
    }
}
extern "C" fn row_exit(this: &mut Object, _c: Sel, _e: *mut Object) {
    unsafe {
        let rd = row_data(this);
        if !rd.is_null() {
            row_paint(&*rd, false);
        }
    }
}
pub(crate) unsafe fn row_set_selected(row: *mut Object, selected: bool) {
    let rd = row_data(row);
    if !rd.is_null() {
        (*rd).selected = selected;
        row_paint(&*rd, false);
    }
}
pub(crate) unsafe fn row_free(row: *mut Object) {
    let rd = row_data(row);
    if !rd.is_null() {
        drop(Box::from_raw(rd));
        (*row).set_ivar::<*mut c_void>("jrRow", std::ptr::null_mut());
    }
}

/// One 28 pt sidebar row: selection/hover pill, symbol, label, full-row hit button.
pub(crate) unsafe fn sidebar_row(title: &str, symbol_name: &str, action: Sel, tag: NSInteger) -> *mut Object {
    let w = SIDEBAR_W - 2. * SP3;
    let cls = *ROW_CLASS.get().unwrap();
    let row: *mut Object = msg_send![cls, alloc];
    let row: *mut Object = msg_send![row, initWithFrame: rect(0., 0., w, SIDEBAR_ROW_H)];
    let bg = surface(rect(0., 0., w, SIDEBAR_ROW_H), C_FILL_SEL, R_ROW);
    let _: () = msg_send![bg, setTransparent: YES];
    set_autoresize(bg, RESIZE_W | RESIZE_H);
    add_sub(row, bg);
    let img = symbol(symbol_name, 15., Weight::Medium);
    let iv: *mut Object = msg_send![class!(NSImageView), alloc];
    let iv: *mut Object = autorel(msg_send![iv, initWithFrame: rect(SP2, 4., 20., 20.)]);
    if !img.is_null() {
        let _: () = msg_send![iv, setImage: img];
        let _: () = msg_send![iv, setContentTintColor: color(C_ACCENT)];
    }
    add_sub(row, iv);
    let l = label(title, T_BODY, C_TEXT, ALIGN_LEFT);
    set_frame(l, rect(36., 6., w - 36. - SP2, 16.));
    set_autoresize(l, RESIZE_W);
    add_sub(row, l);
    let btn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
        target: nil() action: action];
    let ui = UI.get_mut().unwrap();
    let _: () = msg_send![btn, setTarget: ui.ctl.get()];
    let _: () = msg_send![btn, setBordered: NO];
    let _: () = msg_send![btn, setTag: tag];
    set_frame(btn, rect(0., 0., w, SIDEBAR_ROW_H));
    set_autoresize(btn, RESIZE_W | RESIZE_H);
    add_sub(row, btn);
    let rd = Box::into_raw(Box::new(RowData { bg, selected: false }));
    (*row).set_ivar::<*mut c_void>("jrRow", rd as *mut c_void);
    let ta: *mut Object = msg_send![class!(NSTrackingArea), alloc];
    let ta: *mut Object = msg_send![ta,
        initWithRect: rect(0., 0., w, SIDEBAR_ROW_H)
        options: (0x01usize | 0x80usize | 0x200usize) owner: row userInfo: nil()];
    let _: () = msg_send![row, addTrackingArea: ta];
    autorel(ta);
    fix_height(row, SIDEBAR_ROW_H);
    autorel(row)
}

/// Refresh the selection pill on every library row from `ui.selected_view`.
pub(crate) unsafe fn sidebar_refresh_selection() {
    let ui = UI.get_mut().unwrap();
    for (i, (row, _)) in ui.rows.iter().enumerate() {
        row_set_selected(*row, ui.selected_view == Some(i));
    }
}

pub(crate) unsafe fn sidebar_rebuild(user: String, views: &[(String, String, Option<String>)], has_live_tv: bool) {
    let ui = UI.get_mut().unwrap();
    let stack = ui.side_stack.get();
    // Drop everything built last time (header, rows, footer) so nothing piles up.
    for (row, _) in ui.rows.drain(..) {
        row_free(row);
    }
    for v in ui.side_dyn.drain(..) {
        if is_row(v) {
            row_free(v);
        }
        let _: () = msg_send![stack, removeArrangedSubview: v];
        let _: () = msg_send![v, removeFromSuperview];
    }
    // Live TV needs a views entry so the button's tag resolves in viewSelected:.
    if has_live_tv && !views.iter().any(|v| v.2.as_deref() == Some("livetv")) {
        ui.views.push(("livetv".into(), "Live TV".into(), Some("livetv".into())));
    }
    let views: Vec<(String, String, Option<String>)> = ui.views.clone();

    // Section header
    let hdr_box: *mut Object = msg_send![class!(NSView), alloc];
    let hdr_box: *mut Object = autorel(msg_send![hdr_box, initWithFrame: rect(0., 0., SIDEBAR_W - 2. * SP3, 20.)]);
    let hdr = label("Library", T_CAPTION_EM, C_TEXT3, ALIGN_LEFT);
    set_frame(hdr, rect(SP2, 2., SIDEBAR_W - 2. * SP3 - SP2, 16.));
    set_autoresize(hdr, RESIZE_W);
    add_sub(hdr_box, hdr);
    fix_height(hdr_box, 20.);
    let _: () = msg_send![stack, addView: hdr_box inGravity: 1isize];
    ui.side_dyn.push(hdr_box);

    for (i, (id, name, ct)) in views.iter().enumerate() {
        let symbol_name = match ct.as_deref() {
            Some("movies") => "film",
            Some("tvshows") => "tv",
            Some("music") => "music.note",
            Some("livetv") => "antenna.radiowaves.left.and.right",
            Some("boxsets") => "rectangle.stack",
            _ => "square.stack",
        };
        let row = sidebar_row(name, symbol_name, sel!(viewSelected:), i as NSInteger);
        let _: () = msg_send![stack, addView: row inGravity: 1isize];
        ui.rows.push((row, id.clone()));
        ui.side_dyn.push(row);
        row_set_selected(row, ui.selected_view == Some(i));
    }

    // Footer (bottom gravity): Request, hairline, user row.
    let req = sidebar_row("Request…", "plus.magnifyingglass", sel!(openSeerr:), 0);
    let _: () = msg_send![stack, addView: req inGravity: 3isize];
    ui.side_dyn.push(req);
    let hl = hairline(rect(0., 0., SIDEBAR_W - 2. * SP3, 1.));
    fix_height(hl, 1.);
    let _: () = msg_send![stack, addView: hl inGravity: 3isize];
    ui.side_dyn.push(hl);
    let urow = user_row(&user);
    let _: () = msg_send![stack, addView: urow inGravity: 3isize];
    ui.side_dyn.push(urow);
}

unsafe fn is_row(v: *mut Object) -> bool {
    let cls = *ROW_CLASS.get().unwrap();
    let b: BOOL = msg_send![v, isKindOfClass: cls];
    b == YES
}

/// Avatar circle with the user's initial, the name, and a gear that opens Settings.
unsafe fn user_row(user: &str) -> *mut Object {
    let w = SIDEBAR_W - 2. * SP3;
    let h = 36.;
    let row: *mut Object = msg_send![class!(NSView), alloc];
    let row: *mut Object = autorel(msg_send![row, initWithFrame: rect(0., 0., w, h)]);
    let av = surface(rect(SP2, (h - 22.) / 2., 22., 22.), C_FILL_SEL, 11.);
    let initial: String = user.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let il = label(&initial, T_CAPTION_EM, C_TEXT, ALIGN_CENTER);
    set_frame(il, rect(0., 3., 22., 14.));
    let avc: *mut Object = msg_send![av, contentView];
    add_sub(avc, il);
    add_sub(row, av);
    let nl = label(user, T_BODY, C_TEXT, ALIGN_LEFT);
    set_frame(nl, rect(SP2 + 22. + SP2, (h - 16.) / 2., w - (SP2 + 22. + SP2) - 22. - SP2 - SP1, 16.));
    set_autoresize(nl, RESIZE_W);
    add_sub(row, nl);
    let ui = UI.get_mut().unwrap();
    let gear: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
        target: ui.ctl.get() action: sel!(showSettings:)];
    let _: () = msg_send![gear, setBordered: NO];
    let gi = symbol("gearshape", 15., Weight::Medium);
    if !gi.is_null() {
        let _: () = msg_send![gear, setImage: gi];
    }
    let _: () = msg_send![gear, setImagePosition: 1isize]; // image only
    let _: () = msg_send![gear, setContentTintColor: color(C_TEXT2)];
    let _: () = msg_send![gear, setToolTip: ns("Settings")];
    set_frame(gear, rect(w - 22. - SP2, (h - 22.) / 2., 22., 22.));
    set_autoresize(gear, MARGIN_MINX);
    add_sub(row, gear);
    fix_height(row, h);
    row
}

// ---- View switching ------------------------------------------------------
pub fn show_login_or_library(connected: bool) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let content: *mut Object = msg_send![ui.win.get(), contentView];
        crate::ui::theme::crossfade(content, crate::ui::theme::M_EMPH);
        set_hidden(ui.content_host.get(), !connected);
        set_hidden(ui.login_card.get(), connected);
        set_hidden(ui.bgfx.get(), connected || reduce_transparency());
        if connected {
            let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.search.get()];
        } else {
            let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];
            set_text(ui.login_btn.get(), "Connect");
        }
    }
}

pub(crate) extern "C" fn toggle_api_key(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let key_mode = !is_hidden(ui.login_pass.get());
        set_hidden(ui.login_pass.get(), key_mode);
        set_hidden(ui.login_key.get(), !key_mode);
        set_text(
            ui.login_toggle.get(),
            if key_mode { "Use a password instead" } else { "Use an API key instead" },
        );
        let f = if key_mode { ui.login_key.get() } else { ui.login_pass.get() };
        let _: BOOL = msg_send![ui.win.get(), makeFirstResponder: f];
    }
}
pub(crate) unsafe fn is_hidden(v: *mut Object) -> bool {
    let h: BOOL = msg_send![v, isHidden];
    h == YES
}

/// Neutral progress/info text on the login card (secondary colour).
pub fn login_status(msg: &str) {
    use crate::ui::theme::*;
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_err.get(), &clip_msg(msg, 96));
        set_label_color(ui.login_err.get(), C_TEXT2);
        login_layout_status(!msg.is_empty() && msg.ends_with('…'));
    }
}

/// Failure text on the login card (danger colour); re-enables Connect.
pub fn login_error(msg: &str) {
    use crate::ui::theme::*;
    unsafe {
        let ui = UI.get_mut().unwrap();
        set_text(ui.login_err.get(), &clip_msg(msg, 140));
        set_label_color(ui.login_err.get(), C_DANGER);
        login_layout_status(false);
        let _: () = msg_send![ui.login_btn.get(), setEnabled: YES];
        set_text(ui.login_btn.get(), "Connect");
        set_busy(false);
    }
}

/// Three wrapped lines fit the status area; longer text is cut with an ellipsis instead of clipping.
pub fn clip_msg(msg: &str, max: usize) -> String {
    if msg.chars().count() > max {
        format!("{}…", msg.chars().take(max - 1).collect::<String>())
    } else {
        msg.to_string()
    }
}

/// Show the inline spinner (and indent the text next to it) or not.
unsafe fn login_layout_status(spinning: bool) {
    let ui = UI.get_mut().unwrap();
    let f = frame_of(ui.login_err.get());
    let x = if spinning { SP8 + 16. + SP2 } else { SP8 };
    set_frame(ui.login_err.get(), rect(x, f.origin.y, 380. - SP8 - x, f.size.h));
    set_hidden(ui.login_spin.get(), !spinning);
    if spinning {
        let _: () = msg_send![ui.login_spin.get(), startAnimation: nil()];
    } else {
        let _: () = msg_send![ui.login_spin.get(), stopAnimation: nil()];
    }
}

/// "http://192.168.1.10:8096/x" -> "192.168.1.10:8096" (display only).
pub fn host_of(url: &str) -> String {
    let t = url.trim();
    let t = t.split("://").nth(1).unwrap_or(t);
    t.split('/').next().unwrap_or(t).to_string()
}

#[cfg(test)]
mod tests {
    use super::{clip_msg, host_of};
    #[test]
    fn clip_msg_limits_length() {
        assert_eq!(clip_msg("short", 10), "short");
        assert_eq!(clip_msg(&"x".repeat(50), 10).chars().count(), 10);
    }
    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("http://192.168.1.10:8096/web"), "192.168.1.10:8096");
        assert_eq!(host_of(" https://j.example.com "), "j.example.com");
        assert_eq!(host_of("jellyfin.local:8096"), "jellyfin.local:8096");
    }
}

unsafe fn color_a_black_fn(a: f64) -> *mut Object {
    let b: *mut Object = msg_send![class!(NSColor), blackColor];
    msg_send![b, colorWithAlphaComponent: a]
}
