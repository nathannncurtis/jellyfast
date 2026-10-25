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
    let win: *mut Object = msg_send![jr_window_class(), alloc];
    let win: *mut Object = msg_send![win,
        initWithContentRect: frame styleMask: style backing: 2usize defer: NO];
    let _: () = msg_send![win, setTitle: ns("Jellyfast")];
    let _: () = msg_send![win, setTitleVisibility: 1i64]; // hidden; the title still names the window in menus
    let _: () = msg_send![win, setTitlebarAppearsTransparent: YES];
    let _: () = msg_send![win, setReleasedWhenClosed: NO];
    let _: () = msg_send![win, setTabbingMode: 2i64]; // disallowed
    let _: () = msg_send![win, setMinSize: size(960., 600.)];
    place_window(win);
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

    // Header band: back, title, count, spinner, sort (laid out by `header_layout`).
    let pb = bounds_of(content_host);
    let hcls = *HEADER_CLASS.get().unwrap();
    let header: *mut Object = msg_send![hcls, alloc];
    let header: *mut Object = msg_send![header, initWithFrame:
        rect(SIDEBAR_W, pb.size.h - TITLEBAR_H - HEADER_H, pb.size.w - SIDEBAR_W, HEADER_H)];
    set_autoresize(header, RESIZE_W | MARGIN_MINY);
    add_sub(content_host, header);
    autorel(header);
    ui.header = Ptr(header);

    let back: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("")
        target: ui.ctl.get() action: sel!(goBack:)];
    let _: () = msg_send![back, setBordered: NO];
    let bi = symbol("chevron.left", 15., Weight::Medium);
    if !bi.is_null() {
        let _: () = msg_send![back, setImage: bi];
    }
    let _: () = msg_send![back, setImagePosition: 1isize]; // image only
    let _: () = msg_send![back, setContentTintColor: color(C_TEXT2)];
    let _: () = msg_send![back, setToolTip: ns("Back")];
    set_hidden(back, true);
    add_sub(header, back);
    ui.lib_back = Ptr(back);

    let ht = label("", T_DISPLAY, C_TEXT, ALIGN_LEFT);
    add_sub(header, ht);
    ui.header_title = Ptr(ht);
    let hc = label("", T_CALLOUT, C_TEXT2, ALIGN_LEFT);
    add_sub(header, hc);
    ui.header_count = Ptr(hc);

    // Small spinner: shown only while a view with items already on screen reloads.
    let sp: *mut Object = msg_send![class!(NSProgressIndicator), alloc];
    let sp: *mut Object = msg_send![sp, initWithFrame: rect(0., 0., 16., 16.)];
    let _: () = msg_send![sp, setIndeterminate: YES];
    let _: () = msg_send![sp, setStyle: 1isize]; // spinning
    let _: () = msg_send![sp, setControlSize: 1usize];
    let _: () = msg_send![sp, setUsesThreadedAnimation: YES];
    set_hidden(sp, true);
    add_sub(header, sp);
    autorel(sp);
    ui.spinner = Ptr(sp);

    let pop: *mut Object = msg_send![class!(NSPopUpButton), alloc];
    let pop: *mut Object = msg_send![pop, initWithFrame: rect(0., 0., 120., 24.) pullsDown: NO];
    let arr: *mut Object = msg_send![class!(NSMutableArray), array];
    for (t, _, _) in crate::app::library_query::SORT_CHOICES {
        let _: () = msg_send![arr, addObject: ns(t)];
    }
    let _: () = msg_send![pop, addItemsWithTitles: arr];
    let _: () = msg_send![pop, setTarget: ui.ctl.get()];
    let _: () = msg_send![pop, setAction: sel!(sortChanged:)];
    let _: () = msg_send![pop, setControlSize: 1usize];
    let _: () = msg_send![pop, setBordered: NO];
    let _: () = msg_send![pop, setFont: font(T_CALLOUT)];
    add_sub(header, pop);
    autorel(pop);
    ui.sort_popup = Ptr(pop);
    sync_sort_popup();

    // Grid scroll view
    let grid_area = rect(SIDEBAR_W, 0., pb.size.w - SIDEBAR_W, pb.size.h - TITLEBAR_H - HEADER_H - SP3);
    let sc: *mut Object = msg_send![class!(NSScrollView), alloc];
    let sc: *mut Object = msg_send![sc, initWithFrame: grid_area];
    let _: () = msg_send![sc, setHasVerticalScroller: YES];
    let _: () = msg_send![sc, setDrawsBackground: YES];
    let _: () = msg_send![sc, setBackgroundColor: color(C_CANVAS)];
    set_autoresize(sc, RESIZE_W | RESIZE_H);
    add_sub(content_host, sc);
    autorel(sc);
    ui.lib_scroll = Ptr(sc);
    crate::ui::home::build_scroll(content_host, grid_area);
    crate::ui::seerr::build_host(content_host, grid_area);

    let grid_cls = *GRID_CLASS.get().unwrap();
    let grid: *mut Object = msg_send![grid_cls, alloc];
    let grid: *mut Object = msg_send![grid, initWithFrame: rect(0., 0., pb.size.w - SIDEBAR_W, 10.)];
    // One GridData box, owned by ui.grid_data and also referenced by the grid's ivar.
    ui.grid_data = Some(Box::new(GridData { cells: vec![], ratio: POSTER_ASPECT }));
    let data: *mut GridData = &mut **ui.grid_data.as_mut().unwrap();
    (*grid).set_ivar::<*mut c_void>("jrData", data as *mut c_void);
    set_autoresize(grid, RESIZE_W);
    let _: *mut Object = msg_send![sc, setDocumentView: grid];
    autorel(grid);
    ui.grid = Ptr(grid);

    // Empty / error state, centred on the grid area (not the window).
    let host: *mut Object = msg_send![class!(NSView), alloc];
    let host: *mut Object = msg_send![host, initWithFrame: grid_area];
    set_autoresize(host, RESIZE_W | RESIZE_H);
    set_hidden(host, true);
    add_sub(content_host, host);
    autorel(host);
    ui.state_host = Ptr(host);
    let (sw, sh) = (360., 164.);
    let state: *mut Object = msg_send![class!(NSView), alloc];
    let state: *mut Object = msg_send![state, initWithFrame:
        rect((grid_area.size.w - sw) / 2., (grid_area.size.h - sh) / 2., sw, sh)];
    set_autoresize(state, MARGIN_MINX | MARGIN_MAXX | MARGIN_MINY | MARGIN_MAXY);
    add_sub(host, state);
    autorel(state);
    let si: *mut Object = msg_send![class!(NSImageView), alloc];
    let si: *mut Object = msg_send![si, initWithFrame: rect((sw - 40.) / 2., sh - 40., 40., 40.)];
    let _: () = msg_send![si, setContentTintColor: color(C_TEXT3)];
    add_sub(state, si);
    autorel(si);
    ui.state_icon = Ptr(si);
    let st = label("", T_TITLE, C_TEXT, ALIGN_CENTER);
    set_frame(st, rect(0., sh - 40. - SP3 - 26., sw, 26.));
    add_sub(state, st);
    ui.status_label = Ptr(st);
    let sbody = wrapping_label("", T_CALLOUT, C_TEXT2, ALIGN_CENTER);
    set_frame(sbody, rect(0., sh - 40. - SP3 - 26. - SP1 - 34., sw, 34.));
    add_sub(state, sbody);
    ui.state_body = Ptr(sbody);
    let sbtn: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Try Again")
        target: ui.ctl.get() action: sel!(refresh:)];
    let _: () = msg_send![sbtn, setBezelStyle: 1isize];
    set_frame(sbtn, rect((sw - 110.) / 2., 0., 110., 28.));
    set_hidden(sbtn, true);
    add_sub(state, sbtn);
    ui.state_btn = Ptr(sbtn);

    // --- Item detail page (hidden until opened; replaces the grid while open) ---
    crate::ui::detail::build(content_host, rect(SIDEBAR_W, 0., pb.size.w - SIDEBAR_W, pb.size.h - TITLEBAR_H));

    // --- Now-playing bar (audio), on top of the library ---
    crate::ui::nowbar::build(content_host, b);

    // --- First-run onboarding card, above everything on the library side ---
    crate::ui::onboarding::build(content_host, b);

    // --- Player host (AVKit) + chrome ---
    let pcls = *crate::ui::player::PLAYER_HOST_CLASS.get().unwrap();
    let ph: *mut Object = msg_send![pcls, alloc];
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
    crate::ui::player::build_player_chrome(ph, avv, b);

    // --- Login card ---
    let card_w = 380.;
    let card_h = 416. + 18. + SP3; // + "Remember me" row
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

    // "Remember me" checkbox, checked from the saved preference (default on).
    let rem_top = tog_top + 18. + SP3;
    let rem: *mut Object = msg_send![class!(NSButton), buttonWithTitle: ns("Remember me")
        target: nil() action: nil()];
    let _: () = msg_send![rem, setButtonType: 3usize]; // NSSwitchButton (checkbox)
    let _: () = msg_send![rem, setControlSize: 0usize]; // regular
    let _: () = msg_send![rem, setFont: font(T_CALLOUT)];
    let _: () = msg_send![rem, setState: if ui.cfg.remember_me { 1isize } else { 0isize }];
    set_frame(rem, rect(inner_x - 4., top(rem_top, 18.), 200., 18.));
    add_sub(cv, rem);
    ui.login_remember = Ptr(rem);

    let btn_top = rem_top + 18. + SP5;
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
        set_text(url_f, &crate::jellyfin::urls::bare_url(u));
    }
    if let Some(u) = &ui.cfg.username {
        set_text(user_f, u);
    }

    // --- Settings window (presented as a sheet) ---
    build_settings_window();

    // --- Menus (native menu bar: playback is keyboard-operable here) ---
    build_menus();

    let _: () = msg_send![win, makeKeyAndOrderFront: nil()];
    if !offscreen_mode() {
        #[allow(deprecated)]
        let _: () = msg_send![ui.app.get(), activateIgnoringOtherApps: YES];
    }

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
    // Self-test only: JELLYFAST_APPEARANCE=light|dark pins the appearance for screenshots.
    let forced = if crate::app::selftest::enabled() { std::env::var("JELLYFAST_APPEARANCE").ok() } else { None };
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

/// Native menu bar, generated from `ui::shortcuts::SHORTCUTS`.
pub(crate) unsafe fn build_menus() {
    crate::ui::shortcuts::build_menus_from_table();
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
    if !ui.home.row.get().is_null() {
        row_set_selected(ui.home.row.get(), ui.home.active);
    }
    if !ui.browse_row.get().is_null() {
        row_set_selected(ui.browse_row.get(), ui.browse_active);
    }
    if !ui.seerr.row.get().is_null() {
        row_set_selected(ui.seerr.row.get(), ui.seerr.active);
    }
    for (i, (row, _)) in ui.rows.iter().enumerate() {
        row_set_selected(*row, ui.selected_view == Some(i));
    }
}

/// Recompute the sidebar from `all_views` and the saved order/visibility, then rebuild it.
/// Keeps the selection on the same library when it is still shown.
pub(crate) unsafe fn apply_library_prefs() {
    let ui = UI.get_mut().unwrap();
    // `visible_views` keeps Live TV only when it is shown, so no extra Live TV row is added.
    ui.views = crate::app::libraries::visible_views(&ui.all_views, &ui.cfg.library_order, &ui.cfg.hidden_libraries);
    let user = ui.user_name.clone();
    let views = ui.views.clone();
    sidebar_rebuild(user, &views, false);
    let ui = UI.get_mut().unwrap();
    ui.selected_view = ui
        .mode_stack
        .first()
        .and_then(|m| ui.views.iter().position(|v| v.0 == m.0));
    sidebar_refresh_selection();
}

/// Apply a layout choice (onboarding or Settings): save it, tell the worker, and
/// rebuild the sidebar. A place the new layout no longer shows (a library in the
/// unified layout, Browse in the separate one) falls back to Home.
pub(crate) unsafe fn set_layout_mode(mode: crate::config::LayoutMode) {
    use crate::config::LayoutMode;
    let ui = UI.get_mut().unwrap();
    let changed = ui.cfg.layout_mode != Some(mode);
    ui.cfg.layout_mode = Some(mode);
    crate::config::save_config(&ui.cfg);
    let _ = ui.ui_tx.send(UiCmd::SetLayoutMode { mode });
    if !changed {
        return;
    }
    let user = ui.user_name.clone();
    let views = ui.views.clone();
    let has_live_tv = views.iter().any(|v| v.2.as_deref() == Some("livetv"));
    sidebar_rebuild(user, &views, has_live_tv);
    let lost_place = match mode {
        LayoutMode::Unified => ui.selected_view.is_some(),
        LayoutMode::Separate => ui.browse_active,
    };
    if lost_place {
        crate::ui::home::enter();
    } else {
        sidebar_refresh_selection();
    }
    crate::ui::home::layout_changed();
}

/// Rebuild the sidebar rows from `ui.views`. Callers set `ui.views` first.
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
    let views: Vec<(String, String, Option<String>)> = ui.views.clone();
    let unified = ui.cfg.layout_mode == Some(crate::config::LayoutMode::Unified);
    ui.user_name = user.clone();

    // Home sits above the library list.
    let home_row = sidebar_row("Home", "house", sel!(homeSelected:), -1);
    let _: () = msg_send![stack, addView: home_row inGravity: 1isize];
    ui.side_dyn.push(home_row);
    ui.home.row = Ptr(home_row);
    row_set_selected(home_row, ui.home.active);
    ui.browse_row = Ptr(nil());

    if unified {
        // One big library: Browse replaces the library list.
        let browse = sidebar_row("Browse", "square.grid.2x2", sel!(browseSelected:), -1);
        let _: () = msg_send![stack, addView: browse inGravity: 1isize];
        ui.side_dyn.push(browse);
        ui.browse_row = Ptr(browse);
        row_set_selected(browse, ui.browse_active);
    }
    // Requests: a full-size tab in both layouts, present once Seerr is set up.
    let configured = crate::ui::seerr::is_configured(&ui.cfg.settings);
    ui.seerr.in_sidebar = configured;
    ui.seerr.row = Ptr(nil());
    if configured {
        let req = sidebar_row("Requests", "tray.full", sel!(requestsSelected:), -1);
        let _: () = msg_send![stack, addView: req inGravity: 1isize];
        ui.side_dyn.push(req);
        ui.seerr.row = Ptr(req);
        row_set_selected(req, ui.seerr.active);
    }
    if unified {
        let r = sidebar_row("Search", "magnifyingglass", sel!(focusSearch:), -1);
        let _: () = msg_send![stack, addView: r inGravity: 1isize];
        ui.side_dyn.push(r);
    }
    if !unified {
        // Library list (separate layout): a header, then one row per library.
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
    }

    // Footer (bottom gravity): hairline, user row.
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
/// Connecting state for the login card: only shown once a connect (typed
/// credentials, or a resumed remembered session) is really under way.
pub fn login_connecting(url: &str) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.login_btn.get(), setEnabled: NO];
        set_text(ui.login_btn.get(), "Connecting…");
    }
    login_status(&format!("Connecting to {}…", host_of(url)));
}

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

// ---- Header band ----------------------------------------------------------------------
pub(crate) static HEADER_CLASS: OnceLock<&'static Class> = OnceLock::new();
pub(crate) unsafe fn register_header_class() {
    let mut d = ClassDecl::new("JRHeaderView", class!(NSView)).unwrap();
    d.add_method(sel!(layout), header_layout_cb as extern "C" fn(&mut Object, Sel));
    d.add_method(
        sel!(resizeSubviewsWithOldSize:),
        header_resize_cb as extern "C" fn(&mut Object, Sel, NSSize),
    );
    HEADER_CLASS.set(d.register()).ok();
}
extern "C" fn header_layout_cb(this: &mut Object, _c: Sel) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), layout];
        header_layout();
    }
}
extern "C" fn header_resize_cb(this: &mut Object, _c: Sel, old: NSSize) {
    unsafe {
        let _: () = msg_send![super(this, class!(NSView)), resizeSubviewsWithOldSize: old];
        header_layout();
    }
}
unsafe fn text_w(v: *mut Object) -> f64 {
    let cell: *mut Object = msg_send![v, cell];
    let s: NSSize = msg_send![cell, cellSize];
    s.w.ceil()
}

/// Back chevron, title (truncating), count, spinner and sort popup on one baseline row.
pub(crate) unsafe fn header_layout() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    let hdr = ui.header.get();
    if hdr.is_null() {
        return;
    }
    let w = bounds_of(hdr).size.w;
    let back_vis = !is_hidden(ui.lib_back.get());
    set_frame(ui.lib_back.get(), rect(EDGE, (HEADER_H - 28.) / 2., 28., 28.));
    let x0 = EDGE + if back_vis { 36. } else { 0. };
    // Sort popup, right-aligned.
    let _: () = msg_send![ui.sort_popup.get(), sizeToFit];
    let pf = frame_of(ui.sort_popup.get());
    let pw = pf.size.w.max(96.);
    let px = w - EDGE - pw;
    set_frame(ui.sort_popup.get(), rect(px, (HEADER_H - pf.size.h) / 2., pw, pf.size.h));
    let limit = px - SP3;
    let spin_vis = !is_hidden(ui.spinner.get());
    let spin_w = if spin_vis { 16. + SP2 } else { 0. };
    let cnt_txt = read_field(ui.header_count.get());
    let cw = if cnt_txt.is_empty() { 0. } else { text_w(ui.header_count.get()) };
    let natural = text_w(ui.header_title.get());
    let reserve = if cw > 0. { cw.min(160.) + SP2 } else { 0. } + spin_w;
    let tw = natural.min((limit - x0 - reserve).max(60.));
    set_frame(ui.header_title.get(), rect(x0, (HEADER_H - 32.) / 2., tw, 32.));
    let cx = x0 + tw + SP2;
    let cw = cw.min((limit - cx - spin_w).max(0.));
    set_frame(ui.header_count.get(), rect(cx, (HEADER_H - 32.) / 2. + 3., cw, 16.));
    set_frame(ui.spinner.get(), rect(cx + cw + if cw > 0. { SP2 } else { 0. }, (HEADER_H - 16.) / 2. + 1., 16., 16.));
}

/// "128 movies", "1 item"; the noun comes from the (single) item type.
pub fn count_label(items: &[ListItem]) -> String {
    let n = items.len();
    if n == 0 {
        return String::new();
    }
    let ty = items[0].item_type.as_str();
    let same = items.iter().all(|i| i.item_type == ty);
    let (one, many) = if same {
        match ty {
            "Movie" => ("movie", "movies"),
            "Series" => ("show", "shows"),
            "Episode" => ("episode", "episodes"),
            "MusicAlbum" => ("album", "albums"),
            "Audio" => ("track", "tracks"),
            "LiveTvChannel" => ("channel", "channels"),
            _ => ("item", "items"),
        }
    } else {
        ("item", "items")
    };
    format!("{} {}", n, if n == 1 { one } else { many })
}

pub(crate) unsafe fn set_count(text: &str, err: bool) {
    let ui = UI.get_mut().unwrap();
    set_text(ui.header_count.get(), text);
    set_label_color(ui.header_count.get(), if err { C_DANGER } else { C_TEXT2 });
    header_layout();
}

// ---- Empty / error / loading states ------------------------------------------------
pub(crate) unsafe fn hide_state() {
    let ui = UI.get_mut().unwrap();
    set_hidden(ui.state_host.get(), true);
    set_hidden(ui.status_label.get(), true);
}
/// Centred message in the grid area. `retry` adds the Try Again button.
pub(crate) unsafe fn show_state(symbol_name: &str, title: &str, body: &str, retry: bool) {
    let ui = UI.get_mut().unwrap();
    let img = symbol(symbol_name, 40., Weight::Regular);
    if !img.is_null() {
        let _: () = msg_send![ui.state_icon.get(), setImage: img];
    }
    set_text(ui.status_label.get(), title);
    set_text(ui.state_body.get(), &clip_msg(body, 90));
    set_hidden(ui.state_btn.get(), !retry);
    set_hidden(ui.state_host.get(), false);
    set_hidden(ui.status_label.get(), false);
}

/// Show the saved sort choice in the header popup (on launch, and whenever it changes).
pub(crate) unsafe fn sync_sort_popup() {
    let ui = UI.get_mut().unwrap();
    let index = crate::app::library_query::sort_index(ui.cfg.sort_by.as_deref());
    let _: () = msg_send![ui.sort_popup.get(), selectItemAtIndex: index as isize];
}

/// Called when a load begins. `first` = a different view (skeleton), otherwise keep
/// what is on screen and spin in the header.
pub(crate) unsafe fn begin_load(first: bool) {
    crate::ui::home::leave();
    crate::ui::seerr::leave();
    let ui = UI.get_mut().unwrap();
    ui.sort_applies = true;
    hide_state();
    if first || ui.items.is_empty() {
        ui.items.clear();
        set_count("", false);
        set_hidden(ui.spinner.get(), true);
        let _: () = msg_send![ui.spinner.get(), stopAnimation: nil()];
        grid_set_skeleton(12);
    } else {
        set_busy(true);
    }
}
