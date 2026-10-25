#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::avkit::*;
use crate::ui::library::*;
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

// ---- Geometry & misc types (hand-rolled to avoid the deprecated
//      `cocoa` crate; ABI-identical to the C structs). -------------------
pub type CGFloat = f64;
pub(crate) type NSInteger = isize;
pub(crate) type NSUInteger = usize;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSSize {
    pub w: CGFloat,
    pub h: CGFloat,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}
pub fn rect(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat) -> NSRect {
    NSRect { origin: NSPoint { x, y }, size: NSSize { w, h } }
}
pub fn size(w: CGFloat, h: CGFloat) -> NSSize {
    NSSize { w, h }
}

/// CMTime must be ABI-identical to CoreMedia's (24 bytes; the uint8 flags
/// pads to 4 before the 8-aligned epoch).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u8,
    pub epoch: i64,
}
impl CMTime {
    pub fn seconds(self) -> f64 {
        if self.timescale == 0 {
            0.0
        } else {
            self.value as f64 / self.timescale as f64
        }
    }
}
extern "C" {
    pub(crate) fn CMTimeMakeWithSeconds(seconds: f64, scale: i32) -> CMTime;
}

// Framework links (so class lookups resolve and C symbols bind).
#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}
#[link(name = "AVKit", kind = "framework")]
extern "C" {}
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {}
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

// ---- Self-test isolation ---------------------------------------------------------------------
/// In self-test mode windows open far outside every screen and the app never takes
/// focus, so a headless run is invisible to whoever is at the machine.
/// `JELLYFAST_SELFTEST_ONSCREEN=1` opts out (for screenshot runs on a visible window).
pub fn offscreen_mode() -> bool {
    crate::app::selftest::enabled()
        && std::env::var("JELLYFAST_SELFTEST_ONSCREEN").map(|v| v != "1").unwrap_or(true)
}
/// Right edge of the union of all screens.
pub unsafe fn screens_max_x() -> f64 {
    let screens: *mut Object = msg_send![class!(NSScreen), screens];
    let n: NSUInteger = msg_send![screens, count];
    let mut max_x: f64 = 0.0;
    for i in 0..n {
        let s: *mut Object = msg_send![screens, objectAtIndex: i];
        let f: NSRect = msg_send![s, frame];
        max_x = max_x.max(f.origin.x + f.size.w);
    }
    max_x
}
/// Self-test only: move a window fully off every screen (size unchanged).
pub unsafe fn place_offscreen(win: *mut Object) {
    let x = screens_max_x() + 2000.0;
    let _: () = msg_send![win, setFrameOrigin: NSPoint { x, y: 100.0 }];
}
/// Center normally; off-screen in self-test mode.
pub unsafe fn place_window(win: *mut Object) {
    if offscreen_mode() {
        place_offscreen(win);
    } else {
        let _: () = msg_send![win, center];
    }
}
/// Window subclass that, in self-test mode, lets AppKit keep it off-screen.
extern "C" fn constrain_frame(this: &mut Object, _c: Sel, r: NSRect, screen: *mut Object) -> NSRect {
    unsafe {
        if offscreen_mode() {
            r
        } else {
            msg_send![super(this, class!(NSWindow)), constrainFrameRect: r toScreen: screen]
        }
    }
}
pub(crate) unsafe fn register_window_class() {
    let mut d = ClassDecl::new("JRWindow", class!(NSWindow)).unwrap();
    d.add_method(
        sel!(constrainFrameRect:toScreen:),
        constrain_frame as extern "C" fn(&mut Object, Sel, NSRect, *mut Object) -> NSRect,
    );
    d.register();
}
pub(crate) unsafe fn jr_window_class() -> *const Class {
    objc::runtime::objc_getClass(b"JRWindow\0".as_ptr() as *const std::os::raw::c_char) as *const Class
}

// ---- Tiny pointer RAII for process-lifetime globals --------------------
#[derive(Clone, Copy)]
pub struct Ptr(pub *mut Object);
unsafe impl Send for Ptr {}
impl Ptr {
    pub fn retained(p: *mut Object) -> Ptr {
        unsafe {
            let _: *mut Object = msg_send![p, retain];
        }
        Ptr(p)
    }
    pub fn get(&self) -> *mut Object {
        self.0
    }
}

// ---- String/nil helpers ------------------------------------------------
pub unsafe fn ns(s: &str) -> *mut Object {
    let c = CString::new(s).unwrap_or_default();
    msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
}
pub unsafe fn to_str(o: *mut Object) -> String {
    if o.is_null() {
        return String::new();
    }
    let p: *const std::os::raw::c_char = msg_send![o, UTF8String];
    if p.is_null() {
        String::new()
    } else {
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
pub unsafe fn nil() -> *mut Object {
    std::ptr::null_mut()
}
pub unsafe fn release_obj(p: *mut Object) {
    if !p.is_null() {
        let _: () = msg_send![p, release];
    }
}
pub(crate) unsafe fn menu_item(title: &str, action: Sel, key: &str) -> *mut Object {
    let alloc: *mut Object = msg_send![class!(NSMenuItem), alloc];
    let it: *mut Object = msg_send![alloc, initWithTitle: ns(title) action: action keyEquivalent: ns(key)];
    autorel(it)
}
pub(crate) unsafe fn autorel(p: *mut Object) -> *mut Object {
    let _: *mut Object = msg_send![p, autorelease];
    p
}
pub(crate) unsafe fn frame_of(v: *mut Object) -> NSRect {
    msg_send![v, frame]
}
pub(crate) unsafe fn set_frame(v: *mut Object, r: NSRect) {
    let _: () = msg_send![v, setFrame: r];
}
pub(crate) unsafe fn bounds_of(v: *mut Object) -> NSRect {
    msg_send![v, bounds]
}
pub(crate) unsafe fn add_sub(parent: *mut Object, child: *mut Object) {
    let _: () = msg_send![parent, addSubview: child];
}
pub(crate) unsafe fn set_hidden(v: *mut Object, h: bool) {
    let _: () = msg_send![v, setHidden: h as BOOL];
}
pub(crate) unsafe fn set_autoresize(v: *mut Object, mask: NSUInteger) {
    let _: () = msg_send![v, setAutoresizingMask: mask];
}
/// Layer of a view, making the view layer-backed first (`layer` is nil otherwise).
pub(crate) unsafe fn layer_of(v: *mut Object) -> *mut Object {
    let _: () = msg_send![v, setWantsLayer: YES];
    msg_send![v, layer]
}
pub(crate) unsafe fn color_named(name: &str) -> *mut Object {
    use crate::ui::theme::*;
    match name {
        "label" => color(C_TEXT),
        "secondary" => color(C_TEXT2),
        "tertiary" => color(C_TEXT3),
        "accent" => color(C_ACCENT),
        "red" => color(C_DANGER),
        "white" => color(C_ON_MEDIA),
        _ => color(C_TEXT),
    }
}
pub(crate) unsafe fn symbol_image(name: &str, point_size: CGFloat) -> *mut Object {
    use crate::ui::theme::*;
    if point_size <= 0.0 {
        return msg_send![class!(NSImage), imageWithSystemSymbolName: ns(name) accessibilityDescription: nil()];
    }
    symbol(name, point_size, Weight::Regular)
}

// ---- Global UI state (main thread only) --------------------------------
pub struct CellHandle {
    pub container: *mut Object,
    /// Clipped artwork layer (radius, placeholder colour, poster contents).
    pub art: *mut Object,
    /// Unclipped shadow host that also takes the hover transform.
    pub shadow: *mut Object,
    pub prog_track: *mut Object,
    pub prog_fill: *mut Object,
    pub prog_frac: f64,
    pub dim: *mut Object,
    pub circle: *mut Object,
    pub glyph: *mut Object,
    pub play_icon: *mut Object,
    pub badge: *mut Object,
    pub title: *mut Object,
    pub sub: *mut Object,
    pub btn: *mut Object,
    /// Skeleton text bars (null for real cells).
    pub skel_title: *mut Object,
    pub skel_sub: *mut Object,
    pub key: u64,
    pub meta: ListItem,
}

pub struct GridData {
    pub cells: Vec<CellHandle>,
    /// Poster height / width shared by every cell in the grid.
    pub ratio: f64,
}

pub struct PlayingNow {
    pub item_id: String,
    pub url: String,
    pub title: String,
    pub method: PlaybackMethod,
    pub resume_secs: f64,
    pub last_secs: f64,
    pub duration_secs: f64,
    pub paused: bool,
    pub attempts: u32,
    pub ticks_since_save: u8,
}

pub struct Ui {
    // channels
    pub ui_tx: UnboundedSender<UiCmd>,
    pub main_tx: UnboundedSender<MainMsg>,
    // config snapshot (worker owns the authoritative copy)
    pub cfg: Config,
    // objects (process-lifetime, retained via Ptr)
    pub app: Ptr,
    pub win: Ptr,
    pub ctl: Ptr,
    pub pump: Ptr,
    pub grid: Ptr,
    pub player: Ptr,
    pub avview: Ptr,
    pub pitem: Ptr,
    pub timer: Ptr,
    pub retry_timer: Ptr,
    // views
    pub bgfx: Ptr,
    pub sidebar: Ptr,
    pub side_stack: Ptr,
    pub search: Ptr,
    pub sort_popup: Ptr,
    pub header: Ptr,
    pub header_title: Ptr,
    pub header_count: Ptr,
    pub lib_back: Ptr,
    pub state_host: Ptr,
    pub state_icon: Ptr,
    pub state_body: Ptr,
    pub state_btn: Ptr,
    pub lib_scroll: Ptr,
    pub spinner: Ptr,
    pub status_label: Ptr,
    pub content_host: Ptr,
    pub player_host: Ptr,
    pub top_bar: Ptr,
    pub top_title: Ptr,
    pub top_sub: Ptr,
    pub toast_icon: Ptr,
    pub info_panel: Ptr,
    pub info_label: Ptr,
    pub info_item: Ptr,
    pub backdrop: Ptr,
    pub backdrop_poster: Ptr,
    pub backdrop_spin: Ptr,
    pub last_mouse: Option<std::time::Instant>,
    pub bar_shown: bool,
    pub chip: Ptr,
    pub chip_label: Ptr,
    pub back_btn: Ptr,
    pub login_card: Ptr,
    pub login_url: Ptr,
    pub login_user: Ptr,
    pub login_pass: Ptr,
    pub login_key: Ptr,
    pub login_btn: Ptr,
    pub login_err: Ptr,
    pub login_spin: Ptr,
    pub login_toggle: Ptr,
    pub login_remember: Ptr,
    pub settings_win: Ptr,
    pub s_url: Ptr,
    pub s_sub_lang: Ptr,
    pub s_audio_lang: Ptr,
    pub retry_pending: bool,
    pub s_quality: Ptr,
    pub s_theme: Ptr,
    pub s_lib_pop: Ptr,
    pub s_lib_show: Ptr,
    pub s_lib_up: Ptr,
    pub s_lib_down: Ptr,
    /// Settings: sidebar layout popup (separate libraries / one big library).
    pub s_layout: Ptr,
    pub seerr: crate::ui::seerr::SeerrUi,
    pub detail: crate::ui::detail::DetailUi,
    // Rust-side state
    pub rows: Vec<(*mut Object, String)>, // sidebar row buttons + view ids
    pub views: Vec<(String, String, Option<String>)>, // sidebar: visible, ordered
    pub all_views: Vec<(String, String, Option<String>)>, // every library from the server (+ Live TV)
    pub user_label: String,
    pub lib_edit: Vec<(String, String, Option<String>)>, // Settings working copy, full order
    pub lib_hidden: Vec<String>,                          // Settings working copy
    pub lib_sel: usize,
    pub items: Vec<ListItem>,
    pub poster_cache: HashMap<u64, *mut Object>, // +1 owned NSImage
    pub grid_data: Option<Box<GridData>>,
    pub playing: Option<PlayingNow>,
    pub sub_group: *mut Object,
    pub sub_opts: Vec<*mut Object>,
    pub audio_group: *mut Object,
    pub audio_opts: Vec<*mut Object>,
    pub sel_sub: usize,
    pub sel_audio: usize,
    pub mode_stack: Vec<(String, String, Option<String>)>, // for back nav
    pub selected_view: Option<usize>,
    pub side_dyn: Vec<*mut Object>, // header/rows/footer built by sidebar_rebuild
    pub cur_title: String,
    /// Home screen views (scroll, rows, cards, sidebar row).
    pub home: crate::ui::home::HomeUi,
    /// Unified-layout "Browse" sidebar row, and whether Browse is the screen showing.
    pub browse_row: Ptr,
    pub browse_active: bool,
    /// First-run onboarding card (see ui/onboarding.rs).
    pub onboard: crate::ui::onboarding::OnboardUi,
    /// Account name for the sidebar footer, from the last `Connected`.
    pub user_name: String,
    pub now: crate::ui::nowbar::NowBar,
    pub pbar: crate::ui::playerbar::PlayerBar,
    pub tint_cache: HashMap<u64, (f64, f64, f64)>, // artwork tint (h, s, b) by poster key
}
unsafe impl Send for Ui {}
// SAFETY: raw pointers above are dereferenced only on the main thread.
// The tokio worker communicates exclusively via the two channels.

/// Main-thread-only slot for the UI state. Written once in `bootstrap`, then
/// mutated only from the main thread (see the SAFETY MODEL above).
pub struct UiSlot(std::cell::UnsafeCell<Option<Ui>>);
unsafe impl Sync for UiSlot {}
impl UiSlot {
    pub fn set(&self, ui: Ui) -> Result<(), Ui> {
        unsafe {
            let slot = &mut *self.0.get();
            if slot.is_some() {
                return Err(ui);
            }
            *slot = Some(ui);
            Ok(())
        }
    }
    pub fn get(&self) -> Option<&'static Ui> {
        unsafe { (*self.0.get()).as_ref() }
    }
    pub fn get_mut(&self) -> Option<&'static mut Ui> {
        unsafe { (*self.0.get()).as_mut() }
    }
}
pub static UI: UiSlot = UiSlot(std::cell::UnsafeCell::new(None));
/// Pump object pointer, readable from any thread (set in bootstrap).
pub static PUMP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Sender worker-side code uses; main.rs forwards it to the UI and kicks the pump.
pub static WORKER_TX: OnceLock<UnboundedSender<MainMsg>> = OnceLock::new();
pub static MAIN_RX: OnceLock<Mutex<UnboundedReceiver<MainMsg>>> = OnceLock::new();
pub static GRID_CLASS: OnceLock<&'static Class> = OnceLock::new();
pub(crate) static CELL_CLASS: OnceLock<&'static Class> = OnceLock::new();


// ---- Main-thread pump (worker → UI) ------------------------------------
// SAFETY: closure pointer is transferred through performSelectorOnMainThread
// (deprecated but stable API; the block-based GCD alternative would drag in
// the `block` crate for no benefit). Box is reconstructed exactly once.
pub fn on_main(f: impl FnOnce() + Send + 'static) {
    unsafe {
        let pump = PUMP.load(std::sync::atomic::Ordering::SeqCst) as *mut Object;
        if pump.is_null() {
            return;
        }
        let b: Box<dyn FnOnce() + Send + 'static> = Box::new(f);
        let p = Box::into_raw(Box::new(b)) as *mut c_void;
        // Foundation retains the argument, so it must be a real object: wrap in NSValue.
        let v: *mut Object = msg_send![class!(NSValue), valueWithPointer: p as *const c_void];
        let _: () = msg_send![pump, performSelectorOnMainThread: sel!(jrCall:)
            withObject: v waitUntilDone: NO];
    }
}

pub(crate) extern "C" fn pump_call(_this: &mut Object, _cmd: Sel, boxed: *mut Object) {
    unsafe {
        if boxed.is_null() {
            return;
        }
        let p: *mut c_void = msg_send![boxed, pointerValue];
        let b = Box::from_raw(p as *mut Box<dyn FnOnce() + Send>);
        b();
    }
}

// ---- Controller class (targets for all menu/button actions + KVO) ------
pub(crate) unsafe fn register_classes() {
    // App delegate
    {
        let mut d = ClassDecl::new("JRAppDelegate", class!(NSObject)).unwrap();
        d.add_method(
            sel!(applicationDidFinishLaunching:),
            app_did_finish_launch as extern "C" fn(&mut Object, Sel, *mut Object),
        );
        d.add_method(
            sel!(applicationShouldTerminate:),
            app_should_terminate as extern "C" fn(&mut Object, Sel, *mut Object) -> NSInteger,
        );
        d.add_method(
            sel!(applicationShouldTerminateAfterLastWindowClosed:),
            crate::ui::playerbar::terminate_after_last_window as extern "C" fn(&mut Object, Sel, *mut Object) -> BOOL,
        );
        d.add_method(
            sel!(applicationSupportsSecureRestorableState:),
            always_yes_obj as extern "C" fn(&mut Object, Sel, *mut Object) -> BOOL,
        );
        d.register();
    }
    // Pump (main-thread closure trampoline)
    {
        let mut d = ClassDecl::new("JRPump", class!(NSObject)).unwrap();
        d.add_method(sel!(jrCall:), pump_call as extern "C" fn(&mut Object, Sel, *mut Object));
        d.register();
    }
    // Controller: menu/button/KVO/timer targets
    {
        let mut d = ClassDecl::new("JRCtl", class!(NSObject)).unwrap();
        for (s, f) in [
            (sel!(playPause:), play_pause as Extern0),
            (sel!(stopPlayback:), stop_playback as Extern0),
            (sel!(seekBack:), seek_back as Extern0),
            (sel!(seekFwd:), seek_fwd as Extern0),
            (sel!(volUp:), vol_up as Extern0),
            (sel!(volDown:), vol_down as Extern0),
            (sel!(mute:), mute_toggle as Extern0),
            (sel!(subCycle:), sub_cycle as Extern0),
            (sel!(audioCycle:), audio_cycle as Extern0),
            (sel!(setSpeed:), set_speed as Extern0),
            (sel!(fullscreen:), fullscreen as Extern0),
            (sel!(showSettings:), show_settings as Extern0),
            (sel!(signOut:), sign_out as Extern0),
            (sel!(refresh:), refresh as Extern0),
            (sel!(openSeerr:), crate::ui::seerr::open_seerr as Extern0),
            (sel!(showAbout:), show_about as Extern0),
            (sel!(seerrRequest:), crate::ui::seerr::seerr_request as Extern0),
            (sel!(seerrTab:), crate::ui::seerr::seerr_tab as Extern0),
            (sel!(seerrModeChanged:), crate::ui::seerr::seerr_mode_changed as Extern0),
            (sel!(goBack:), go_back as Extern0),
            (sel!(viewSelected:), view_selected as Extern0),
            (sel!(itemClicked:), item_clicked as Extern0),
            (sel!(homeItemClicked:), home_item_clicked as Extern0),
            (sel!(homeSelected:), crate::ui::home::home_selected as Extern0),
            (sel!(browseSelected:), crate::ui::home::browse_selected as Extern0),
            (sel!(openRequests:), crate::ui::seerr::open_requests as Extern0),
            (sel!(onboardPick:), crate::ui::onboarding::onboard_pick as Extern0),
            (sel!(onboardContinue:), crate::ui::onboarding::onboard_continue as Extern0),
            (sel!(markWatched:), mark_watched as Extern0),
            (sel!(markUnwatched:), mark_unwatched as Extern0),
            (sel!(openInWeb:), open_in_web as Extern0),
            (sel!(connectClicked:), connect_clicked as Extern0),
            (sel!(toggleApiKey:), toggle_api_key as Extern0),
            (sel!(saveSettings:), save_settings as Extern0),
            (sel!(cancelSettings:), cancel_settings as Extern0),
            (sel!(libSelected:), crate::ui::settings::lib_selected as Extern0),
            (sel!(libVisibleToggled:), crate::ui::settings::lib_visible_toggled as Extern0),
            (sel!(libMoveUp:), crate::ui::settings::lib_move_up as Extern0),
            (sel!(libMoveDown:), crate::ui::settings::lib_move_down as Extern0),
            (sel!(sortChanged:), sort_changed as Extern0),
            (sel!(controlTextDidChange:), text_changed as Extern0),
            (sel!(jrTick:), tick as Extern0),
            (sel!(jrSearchNow:), search_now as Extern0),
            (sel!(jrHideToast:), crate::ui::player::hide_toast as Extern0),
            (sel!(jrBackdropOff:), crate::ui::player::backdrop_off as Extern0),
            (sel!(togglePlaybackInfo:), crate::ui::player::toggle_info as Extern0),
            (sel!(jrRetry:), retry_now as Extern0),
            (sel!(jrNowRestart:), crate::ui::nowbar::restart as Extern0),
            (sel!(jrNowSkip:), crate::ui::nowbar::skip as Extern0),
            (sel!(jrNowScrub:), crate::ui::nowbar::scrub as Extern0),
            (sel!(jrNowVolume:), crate::ui::nowbar::volume as Extern0),
            (sel!(focusSearch:), crate::ui::shortcuts::focus_search as Extern0),
            (sel!(openSelected:), crate::ui::shortcuts::open_selected as Extern0),
            (sel!(focusNext:), crate::ui::shortcuts::focus_next as Extern0),
            (sel!(focusPrevious:), crate::ui::shortcuts::focus_previous as Extern0),
            (sel!(seerrShowTab:), crate::ui::shortcuts::seerr_show_tab as Extern0),
            (sel!(showShortcuts:), crate::ui::shortcuts::show_shortcuts as Extern0),
            (sel!(showSubtitleMenu:), crate::ui::playerbar::show_sub_menu as Extern0),
            (sel!(showAudioMenu:), crate::ui::playerbar::show_audio_menu as Extern0),
            (sel!(showSpeedMenu:), crate::ui::playerbar::show_speed_menu as Extern0),
            (sel!(pickTrack:), crate::ui::playerbar::pick_track as Extern0),
            (sel!(togglePiP:), crate::ui::playerbar::toggle_pip as Extern0),
        ] {
            d.add_method(s, f);
        }
        d.add_method(sel!(observeValueForKeyPath:ofObject:change:context:), kvo as KvoFn);
        d.add_method(
            sel!(validateMenuItem:),
            crate::ui::shortcuts::validate_menu_item as extern "C" fn(&mut Object, Sel, *mut Object) -> BOOL,
        );
        d.register();
    }
    crate::ui::seerr::register_seerr_classes();
    register_window_class();
    register_sidebar_row();
    register_header_class();
    crate::ui::detail::register_detail_classes();
    crate::ui::player::register_player_classes();
    crate::ui::playerbar::register_playerbar_classes();
    crate::ui::nowbar::register_nowbar_class();
    // Poster grid container
    {
        let mut d = ClassDecl::new("JRGridView", class!(NSView)).unwrap();
        d.add_ivar::<*mut c_void>("jrData");
        d.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
        d.add_method(sel!(layout), grid_layout as extern "C" fn(&mut Object, Sel));
        d.add_method(
            sel!(viewDidChangeEffectiveAppearance),
            grid_appearance_changed as extern "C" fn(&mut Object, Sel),
        );
        d.add_method(
            sel!(setFrameSize:),
            grid_set_frame as extern "C" fn(&mut Object, Sel, NSSize),
        );
        GRID_CLASS.set(d.register()).ok();
    }
    // Poster cell (hover)
    {
        let mut d = ClassDecl::new("JRCellView", class!(NSView)).unwrap();
        d.add_ivar::<*mut c_void>("jrCell");
        d.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
        d.add_method(sel!(layout), cell_layout as extern "C" fn(&mut Object, Sel));
        d.add_method(
            sel!(mouseEntered:),
            cell_hover as extern "C" fn(&mut Object, Sel, *mut Object),
        );
        d.add_method(
            sel!(mouseExited:),
            cell_unhover as extern "C" fn(&mut Object, Sel, *mut Object),
        );
        CELL_CLASS.set(d.register()).ok();
    }
}
pub(crate) type Extern0 = extern "C" fn(&mut Object, Sel, *mut Object);
pub(crate) type KvoFn = extern "C" fn(&mut Object, Sel, *mut Object, *mut Object, *mut Object, *mut c_void);

/// Refresh the playback-info panel text (time, resolution, bitrate, method). No-op while it is off.
pub(crate) unsafe fn update_chip() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    if !ui.cfg.settings.show_stats {
        return;
    }
    let pn = match ui.playing.as_ref() {
        Some(p) => p,
        None => return,
    };
    if ui.pitem.get().is_null() {
        return; // item being swapped (KVO can fire mid-replace)
    }
    let mut s = String::new();
    let t: CMTime = msg_send![ui.player.get(), currentTime];
    let d: CMTime = msg_send![ui.pitem.get(), duration];
    s.push_str(&format!(
        "{} / {}",
        crate::jellyfin::urls::fmt_clock(t.seconds()),
        crate::jellyfin::urls::fmt_clock(d.seconds())
    ));
    let sz: NSSize = msg_send![ui.pitem.get(), presentationSize];
    if sz.w > 0.0 {
        s.push_str(&format!("  ·  {}×{}", sz.w as i64, sz.h as i64));
    }
    let log: *mut Object = msg_send![ui.pitem.get(), accessLog];
    if !log.is_null() {
        let events: *mut Object = msg_send![log, events];
        if !events.is_null() {
            let n: NSUInteger = msg_send![events, count];
            if n > 0 {
                let ev: *mut Object = msg_send![events, objectAtIndex: n - 1];
                let br: f64 = msg_send![ev, indicatedBitrate];
                if br > 0.0 {
                    s.push_str(&format!("  ·  {:.1} Mb/s", br / 1_000_000.0));
                }
            }
        }
    }
    s.push_str(&format!("  ·  {}", crate::transcode::decision::method_label(pn.method)));
    set_text(ui.info_label.get(), &s);
    // Fit the pill to its text (the label truncates if the window is too narrow).
    let cell: *mut Object = msg_send![ui.info_label.get(), cell];
    let tsz: NSSize = msg_send![cell, cellSize];
    let w = (tsz.w.ceil() + 2. * crate::ui::theme::SP2 + 2.).min(bounds_of(ui.player_host.get()).size.w - 2. * crate::ui::theme::SP4);
    let f = frame_of(ui.info_panel.get());
    set_frame(ui.info_panel.get(), rect(f.origin.x, f.origin.y, w, f.size.h));
    set_frame(ui.info_label.get(), rect(crate::ui::theme::SP2, 5., w - 2. * crate::ui::theme::SP2, 16.));
}

pub(crate) extern "C" fn fullscreen(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if offscreen_mode() {
            ui.pbar.full_presses += 1;
            return;
        }
        let _: () = msg_send![ui.win.get(), toggleFullScreen: nil()];
    }
}
pub(crate) extern "C" fn show_settings(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _ = ui.main_tx.send(MainMsg::SettingsSnapshot(Box::new(ui.cfg.clone())));
    }
}
/// Whether the settings sheet is up. (Headless self-test shows it as a plain off-screen window:
/// AppKit drags an off-screen parent onto a screen to present a sheet.)
pub(crate) unsafe fn settings_open() -> bool {
    let ui = UI.get_mut().unwrap();
    if offscreen_mode() {
        let v: BOOL = msg_send![ui.settings_win.get(), isVisible];
        return v == YES;
    }
    let attached: *mut Object = msg_send![ui.win.get(), attachedSheet];
    !attached.is_null()
}
/// Standard About panel, with a credits line saying what this is and where the idea came from.
pub(crate) extern "C" fn show_about(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get().unwrap();
        let credits: *mut Object = msg_send![class!(NSAttributedString), alloc];
        let credits: *mut Object = msg_send![credits, initWithString: ns("A native client for Jellyfin. Inspired by Spotifast.")];
        let opts: *mut Object = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![opts, setObject: credits forKey: ns("Credits")];
        let _: () = msg_send![opts, setObject: ns("Jellyfast") forKey: ns("ApplicationName")];
        let _: () = msg_send![opts, setObject: ns(env!("CARGO_PKG_VERSION")) forKey: ns("ApplicationVersion")];
        let _: () = msg_send![ui.app.get(), orderFrontStandardAboutPanelWithOptions: opts];
        autorel(credits);
    }
}
pub(crate) unsafe fn show_settings_sheet() {
    let ui = UI.get_mut().unwrap();
    if settings_open() {
        return;
    }
    if offscreen_mode() {
        place_offscreen(ui.settings_win.get());
        let _: () = msg_send![ui.settings_win.get(), orderFront: nil()];
        return;
    }
    let _: () = msg_send![ui.win.get(), beginSheet: ui.settings_win.get() completionHandler: nil()];
}
pub(crate) extern "C" fn cancel_settings(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        close_settings();
    }
}
pub(crate) unsafe fn close_settings() {
    let ui = UI.get_mut().unwrap();
    set_text(ui.seerr.s_key.get(), "");
    set_text(ui.seerr.s_pass.get(), "");
    ui.seerr.pending = None;
    if offscreen_mode() {
        let _: () = msg_send![ui.settings_win.get(), orderOut: nil()];
    } else {
        let attached: *mut Object = msg_send![ui.win.get(), attachedSheet];
        if !attached.is_null() {
            let _: () = msg_send![ui.win.get(), endSheet: ui.settings_win.get()];
        }
    }
}
pub(crate) extern "C" fn save_settings(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let mut s = ui.cfg.settings.clone();
        s.subtitle_lang = read_field(ui.s_sub_lang.get());
        s.audio_lang = read_field(ui.s_audio_lang.get());
        let q: NSInteger = msg_send![ui.s_quality.get(), indexOfSelectedItem];
        s.max_bitrate_kbps = [0u64, 40_000, 20_000, 10_000, 5_000, 3_000][q.clamp(0, 5) as usize];
        let th: NSInteger = msg_send![ui.s_theme.get(), indexOfSelectedItem];
        s.theme = ["system", "light", "dark"][th.clamp(0, 2) as usize].to_string();
        // Compare display forms: the field shows a bare host, while the stored
        // URL carries a resolved scheme. The worker resolves and stores the URL.
        let new_url = read_field(ui.s_url.get()).trim().to_string();
        let url_changed = !new_url.is_empty()
            && ui.cfg.server_url.as_deref().map(crate::jellyfin::urls::bare_url)
                != Some(crate::jellyfin::urls::bare_url(&new_url));
        ui.cfg.settings = s;
        crate::ui::settings::commit_library_prefs();
        // The worker resolves and stores the URL (see server_url below); the
        // keeping-session save never overwrites the worker-owned session fields.
        crate::config::save_config_keeping_session(&ui.cfg);
        // Compare with the effective layout, so saving other settings never answers onboarding.
        let lay: NSInteger = msg_send![ui.s_layout.get(), indexOfSelectedItem];
        let layout = if lay == 1 { crate::config::LayoutMode::Unified } else { crate::config::LayoutMode::Separate };
        if layout != ui.cfg.layout_mode.unwrap_or_default() {
            crate::ui::shell::set_layout_mode(layout);
        }
        apply_theme();
        // SaveSettings replaces the worker's whole Settings, so it goes first;
        // ui.cfg.settings carries the last *confirmed* Seerr values. SaveSeerr
        // follows and wins for seerr_enabled/seerr_url.
        let _ = ui.ui_tx.send(UiCmd::SaveSettings {
            settings: ui.cfg.settings.clone(),
            reconnect: url_changed,
            server_url: if url_changed { Some(new_url) } else { None },
        });
        let seerr_on: NSInteger = msg_send![ui.seerr.s_on.get(), state];
        let seerr_on = seerr_on != 0;
        let seerr_url = read_field(ui.seerr.s_url.get()).trim().to_string();
        let mode: NSInteger = msg_send![ui.seerr.s_mode.get(), indexOfSelectedItem];
        let (user, pass, key) = if mode == 0 {
            (
                read_field(ui.seerr.s_user.get()).trim().to_string(),
                read_field_secure(ui.seerr.s_pass.get()),
                String::new(),
            )
        } else {
            (String::new(), String::new(), read_field_secure(ui.seerr.s_key.get()))
        };
        let secret_typed = !pass.is_empty() || !key.trim().is_empty();
        let cur = &ui.cfg.settings;
        let seerr_addr_changed = crate::jellyfin::urls::bare_url(&seerr_url) != crate::jellyfin::urls::bare_url(&cur.seerr_url);
        let seerr_dirty = seerr_on != cur.seerr_enabled
            || (seerr_on && (seerr_addr_changed || user != cur.seerr_username))
            || (!seerr_on && !seerr_url.is_empty() && seerr_addr_changed)
            || secret_typed;
        if seerr_on && mode == 0 && user.is_empty() && seerr_dirty {
            // Nothing to validate against; keep the sheet open and say why.
            set_text(ui.seerr.s_status.get(), "Enter your Jellyfin username to sign in to Seerr.");
            return;
        }
        if seerr_dirty {
            // Keep the sheet open: it shows the SeerrStatus reply.
            ui.seerr.pending = Some((seerr_on, seerr_url.clone(), user.clone()));
            set_text(ui.seerr.s_status.get(), if seerr_on { "Checking connection…" } else { "Saving…" });
            let _ = ui.ui_tx.send(UiCmd::SaveSeerr {
                enabled: seerr_on,
                url: seerr_url,
                username: user,
                password: if pass.is_empty() { None } else { Some(pass) },
                api_key: if key.trim().is_empty() { None } else { Some(key) },
            });
            // never keep secrets in the UI
            set_text(ui.seerr.s_key.get(), "");
            set_text(ui.seerr.s_pass.get(), "");
        } else {
            close_settings();
        }
    }
}
// Helper used above to keep the settings struct access symmetric; keeps
// the audio-lang field lookup in one place.

pub(crate) extern "C" fn sign_out(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _ = ui.ui_tx.send(UiCmd::SignOut);
    }
}
pub(crate) extern "C" fn refresh(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.home.active {
            crate::ui::home::reload();
            return;
        }
        if ui.browse_active {
            begin_load(false);
            let _ = ui.ui_tx.send(UiCmd::LoadBrowse);
            return;
        }
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
            begin_load(false);
            let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
        }
    }
}
pub(crate) extern "C" fn go_back(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.home.active {
            return;
        }
        if ui.mode_stack.len() > 1 {
            ui.mode_stack.pop();
            if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
                begin_load(false);
                let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
            }
        }
    }
}
/// Open library `idx` (an index into `ui.views`) and remember it as the last place.
pub(crate) unsafe fn select_view(idx: usize) {
    let ui = UI.get_mut().unwrap();
    if let Some((id, name, ct)) = ui.views.get(idx).cloned() {
        ui.mode_stack = vec![(id.clone(), name.clone(), ct.clone())];
        ui.selected_view = Some(idx);
        ui.browse_active = false;
        sidebar_refresh_selection();
        ui.cfg.last_view = Some(id.clone());
        crate::config::save_config_keeping_session(&ui.cfg);
        begin_load(true);
        let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
    }
}
pub(crate) extern "C" fn view_selected(_t: &mut Object, _c: Sel, sender: *mut Object) {
    crate::ui::detail::hide();
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        select_view(tag as usize);
    }
}
pub(crate) extern "C" fn item_clicked(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        if let Some(it) = ui.items.get(tag as usize).cloned() {
            activate(it);
        }
    }
}
pub(crate) extern "C" fn home_item_clicked(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        if let Some(it) = ui.home.flat.get(tag as usize).cloned() {
            activate(it);
        }
    }
}
/// Open a movie or series page, or play, list tracks or tune a channel.
unsafe fn activate(it: ListItem) {
    let ui = UI.get_mut().unwrap();
    set_busy(true);
    match it.item_type.as_str() {
        "Movie" | "Series" => {
            let _ = ui.ui_tx.send(UiCmd::OpenDetail { item_id: it.id.clone() });
        }
        "MusicAlbum" => {
            let _ = ui.ui_tx.send(UiCmd::Tracks { album_id: it.id.clone(), album_name: it.name.clone() });
        }
        "LiveTvChannel" => {
            let _ = ui.ui_tx.send(UiCmd::PlayChannel { channel_id: it.id.clone(), name: it.name.clone() });
        }
        _ => {
            let _ = ui.ui_tx.send(UiCmd::Play { item_id: it.id.clone() });
        }
    }
}
pub(crate) extern "C" fn mark_watched(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let ro: *mut Object = msg_send![sender, representedObject];
        let id = to_str(ro);
        if !id.is_empty() {
            let _ = ui.ui_tx.send(UiCmd::MarkPlayed { item_id: id, played: true });
        }
    }
}
pub(crate) extern "C" fn mark_unwatched(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let ro: *mut Object = msg_send![sender, representedObject];
        let id = to_str(ro);
        if !id.is_empty() {
            let _ = ui.ui_tx.send(UiCmd::MarkPlayed { item_id: id, played: false });
        }
    }
}
pub(crate) extern "C" fn open_in_web(_t: &mut Object, _c: Sel, sender: *mut Object) {
    // The self-test sets fake server URLs; never open a real browser tab for them.
    if std::env::var_os("JELLYFAST_SELFTEST").is_some() {
        return;
    }
    unsafe {
        let ui = UI.get_mut().unwrap();
        let ro: *mut Object = msg_send![sender, representedObject];
        let id = to_str(ro);
        if let Some(base) = ui.cfg.server_url.clone() {
            let url = crate::jellyfin::urls::build_web_item_url(&base, &id);
            let u: *mut Object = msg_send![class!(NSURL), URLWithString: ns(&url)];
            if !u.is_null() {
                let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
                let _: BOOL = msg_send![ws, openURL: u];
            }
        }
    }
}
pub(crate) extern "C" fn connect_clicked(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let url = read_field(ui.login_url.get());
        let user = read_field(ui.login_user.get());
        let pass = read_field_secure(ui.login_pass.get());
        let key_mode = is_hidden(ui.login_pass.get());
        let key = if key_mode { read_field_secure(ui.login_key.get()) } else { String::new() };
        let pass = if key_mode { String::new() } else { pass };
        login_status(""); // clear
        if url.is_empty() {
            login_error("Enter your server's address, like jellyfin.local:8096.");
            return;
        }
        if (key_mode && key.is_empty()) || (!key_mode && user.is_empty()) {
            login_error("Enter your username and password, or use an API key.");
            return;
        }
        login_connecting(&url);
        let remember_state: isize = msg_send![ui.login_remember.get(), state];
        let _ = ui.ui_tx.send(UiCmd::Connect {
            url,
            username: user,
            password: if pass.is_empty() { None } else { Some(pass) },
            api_key: if key.is_empty() { None } else { Some(key) },
            remember: remember_state == 1, // NSControlStateValueOn
        });
    }
}
pub(crate) extern "C" fn sort_changed(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
            begin_load(false);
            let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
        }
    }
}
pub(crate) extern "C" fn text_changed(_t: &mut Object, _c: Sel, note: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let obj: *mut Object = msg_send![note, object];
        if obj == ui.seerr.search.get() && !obj.is_null() {
            crate::ui::seerr::search_changed();
        } else if obj == ui.search.get() {
            // Debounce: keep the grid as is and search 200 ms after the last keystroke.
            let ctl = ui.ctl.get();
            let _: () = msg_send![class!(NSObject), cancelPreviousPerformRequestsWithTarget: ctl
                selector: sel!(jrSearchNow:) object: nil()];
            let _: () = msg_send![ctl, performSelector: sel!(jrSearchNow:) withObject: nil() afterDelay: 0.2f64];
        }
    }
}
pub(crate) extern "C" fn search_now(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let q = read_field(ui.search.get());
        begin_load(false);
        if q.trim().is_empty() {
            if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
                let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
            }
        } else {
            let _ = ui.ui_tx.send(UiCmd::Search { q });
        }
    }
}

// ---- App delegate callbacks --------------------------------------------
pub(crate) extern "C" fn app_did_finish_launch(_t: &mut Object, _c: Sel, _n: *mut Object) {
    unsafe {
        build_window_and_ui();
        // No connecting state here: a saved server URL alone is not a session. The
        // worker reports `MainMsg::Resuming` if it really resumes one.
        crate::app::controller::drain_main(); // anything queued before launch
        if crate::app::selftest::enabled() {
            crate::app::selftest::start();
        }
        // NOTE: QuitSaving is not sent on launch in practice — removed below.
    }
}
// The line above sending QuitSaving at launch is deliberately a no-op in
// the worker (matched arm ignores it when no session exists).

pub(crate) extern "C" fn app_should_terminate(
    _t: &mut Object,
    _c: Sel,
    _app: *mut Object,
) -> NSInteger {
    unsafe {
        // Clean shutdown: persist window frame + any final state.
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return 1,
        };
        let f: NSRect = msg_send![ui.win.get(), frame];
        ui.cfg.window = Some([f.origin.x, f.origin.y, f.size.w, f.size.h]);
        if let Some(pn) = ui.playing.as_ref() {
            let _ = ui.ui_tx.send(UiCmd::Stopped {
                item_id: pn.item_id.clone(),
                ticks: crate::jellyfin::urls::ticks_from_secs(pn.last_secs),
            });
        }
        crate::config::save_config_keeping_session(&ui.cfg);
        1 // NSTerminateNow (0 would cancel the quit)
    }
}

// ========================================================================
// SECTION: UI Construction (window, menus, sidebar, grid, login, settings)
// ========================================================================
// SPLIT: shell → src/ui/shell.rs, library grid/cells → src/ui/library.rs,
// login card → src/ui/login.rs, settings sheet → src/ui/settings.rs.
// All of these become safe functions taking typed view handles once `mod
// sys` is its own platform crate.

/// Legacy label helper kept while screens migrate to `theme::label`.
pub(crate) unsafe fn make_label(text: &str, size: CGFloat, bold: bool, center: bool) -> *mut Object {
    use crate::ui::theme::*;
    let t = TypeStyle { size, weight: if bold { Weight::Bold } else { Weight::Regular }, mono: false };
    label(text, t, C_TEXT, if center { ALIGN_CENTER } else { ALIGN_LEFT })
}
pub(crate) unsafe fn set_text(v: *mut Object, s: &str) {
    let _: () = msg_send![v, setStringValue: ns(s)];
}
pub(crate) unsafe fn read_field(f: *mut Object) -> String {
    to_str(msg_send![f, stringValue])
}
pub(crate) unsafe fn read_field_secure(f: *mut Object) -> String {
    read_field(f)
}
pub(crate) unsafe fn vev(material: i64, blending: i64) -> *mut Object {
    let v: *mut Object = msg_send![class!(NSVisualEffectView), alloc];
    let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 10., 10.)];
    let _: () = msg_send![v, setMaterial: material];
    let _: () = msg_send![v, setBlendingMode: blending];
    let _: () = msg_send![v, setState: 0i64]; // followsWindowActiveState
    autorel(v)
}
// Materials / blending modes (AppKit enum values, stable since 10.10):
pub(crate) const BLEND_BEHIND: i64 = 0;
pub(crate) const BLEND_WITHIN: i64 = 1;
pub(crate) const RESIZE_W: NSUInteger = 2;
pub(crate) const RESIZE_H: NSUInteger = 16;
pub(crate) const MARGIN_MINX: NSUInteger = 1;
pub(crate) const MARGIN_MINY: NSUInteger = 8;
pub(crate) const MARGIN_MAXX: NSUInteger = 4;
pub(crate) const MARGIN_MAXY: NSUInteger = 32;

pub(crate) unsafe fn text_field(placeholder: &str, secure: bool) -> *mut Object {
    let v: *mut Object = if secure {
        let v: *mut Object = msg_send![class!(NSSecureTextField), alloc];
        let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 260., 24.)];
        autorel(v)
    } else {
        let v: *mut Object = msg_send![class!(NSTextField), alloc];
        let v: *mut Object = msg_send![v, initWithFrame: rect(0., 0., 260., 24.)];
        autorel(v)
    };
    let _: () = msg_send![v.cell(), setPlaceholderString: ns(placeholder)];
    v
}
pub(crate) trait CellOf {
    unsafe fn cell(&self) -> *mut Object;
}
impl CellOf for *mut Object {
    unsafe fn cell(&self) -> *mut Object {
        msg_send![*self, cell]
    }
}

pub unsafe fn bootstrap(cfg: Config, ui_tx: UnboundedSender<UiCmd>, main_tx: UnboundedSender<MainMsg>) {
    register_classes();
    let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
    let _: BOOL = msg_send![app, setActivationPolicy: if offscreen_mode() { 1i64 } else { 0i64 }]; // regular app (accessory in headless self-test)

    let pump: *mut Object = {
        let cls = class!(NSObject); // JRPump registered above; look it up:
        let c = objc::runtime::objc_getClass(b"JRPump\0".as_ptr() as *const std::os::raw::c_char);
        let base: *mut Object = if c.is_null() { cls as *const Class as *mut Object } else { c as *mut Object };
        let o: *mut Object = msg_send![base, alloc];
        let o: *mut Object = msg_send![o, init];
        o // owned (+1), stored globally below
    };
    let ctl_cls = objc::runtime::objc_getClass(b"JRCtl\0".as_ptr() as *const std::os::raw::c_char);
    let ctl: *mut Object = {
        let o: *mut Object = msg_send![ctl_cls, alloc];
        let o: *mut Object = msg_send![o, init];
        o
    };
    let del_cls = objc::runtime::objc_getClass(b"JRAppDelegate\0".as_ptr() as *const std::os::raw::c_char);
    let del: *mut Object = {
        let o: *mut Object = msg_send![del_cls, alloc];
        let o: *mut Object = msg_send![o, init];
        o
    };
    let _: () = msg_send![app, setDelegate: del];

    let ui = Ui {
        ui_tx,
        main_tx,
        cfg,
        app: Ptr::retained(app),
        win: Ptr(nil()),
        ctl: Ptr::retained(ctl),
        pump: Ptr::retained(pump),
        grid: Ptr(nil()),
        player: Ptr(nil()),
        avview: Ptr(nil()),
        pitem: Ptr(nil()),
        timer: Ptr(nil()),
        retry_timer: Ptr(nil()),
        bgfx: Ptr(nil()),
        sidebar: Ptr(nil()),
        side_stack: Ptr(nil()),
        search: Ptr(nil()),
        sort_popup: Ptr(nil()),
        header: Ptr(nil()),
        header_title: Ptr(nil()),
        header_count: Ptr(nil()),
        lib_back: Ptr(nil()),
        state_host: Ptr(nil()),
        state_icon: Ptr(nil()),
        state_body: Ptr(nil()),
        state_btn: Ptr(nil()),
        lib_scroll: Ptr(nil()),
        spinner: Ptr(nil()),
        status_label: Ptr(nil()),
        content_host: Ptr(nil()),
        player_host: Ptr(nil()),
        top_bar: Ptr(nil()),
        top_title: Ptr(nil()),
        top_sub: Ptr(nil()),
        toast_icon: Ptr(nil()),
        info_panel: Ptr(nil()),
        info_label: Ptr(nil()),
        info_item: Ptr(nil()),
        backdrop: Ptr(nil()),
        backdrop_poster: Ptr(nil()),
        backdrop_spin: Ptr(nil()),
        last_mouse: None,
        bar_shown: false,
        chip: Ptr(nil()),
        chip_label: Ptr(nil()),
        back_btn: Ptr(nil()),
        login_card: Ptr(nil()),
        login_url: Ptr(nil()),
        login_user: Ptr(nil()),
        login_pass: Ptr(nil()),
        login_key: Ptr(nil()),
        login_btn: Ptr(nil()),
        login_err: Ptr(nil()),
        login_spin: Ptr(nil()),
        login_toggle: Ptr(nil()),
        login_remember: Ptr(nil()),
        settings_win: Ptr(nil()),
        s_url: Ptr(nil()),
        s_sub_lang: Ptr(nil()),
        s_audio_lang: Ptr(nil()),
        retry_pending: false,
        s_quality: Ptr(nil()),
        s_theme: Ptr(nil()),
        s_lib_pop: Ptr(nil()),
        s_lib_show: Ptr(nil()),
        s_lib_up: Ptr(nil()),
        s_lib_down: Ptr(nil()),
        s_layout: Ptr(nil()),
        seerr: crate::ui::seerr::SeerrUi::new(),
        detail: crate::ui::detail::DetailUi::new(),
        rows: vec![],
        views: vec![],
        all_views: vec![],
        user_label: String::new(),
        lib_edit: vec![],
        lib_hidden: vec![],
        lib_sel: 0,
        items: vec![],
        poster_cache: HashMap::new(),
        grid_data: None,
        playing: None,
        sub_group: nil(),
        sub_opts: vec![],
        audio_group: nil(),
        audio_opts: vec![],
        sel_sub: 0,
        sel_audio: 0,
        mode_stack: vec![],
        selected_view: None,
        side_dyn: vec![],
        cur_title: String::new(),
        home: crate::ui::home::HomeUi::new(),
        browse_row: Ptr(nil()),
        browse_active: false,
        onboard: crate::ui::onboarding::OnboardUi::new(),
        user_name: String::new(),
        now: crate::ui::nowbar::NowBar::new(),
        pbar: crate::ui::playerbar::PlayerBar::new(),
        tint_cache: HashMap::new(),
    };
    PUMP.store(pump as usize, std::sync::atomic::Ordering::SeqCst);
    let _ = UI.set(ui);
}

pub fn run() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.app.get(), run];
    }
}

// ---- misc sys helpers used elsewhere -------------------------------------
pub fn set_busy(b: bool) {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        set_hidden(ui.spinner.get(), !b);
        if b {
            let _: () = msg_send![ui.spinner.get(), startAnimation: nil()];
        } else {
            let _: () = msg_send![ui.spinner.get(), stopAnimation: nil()];
        }
    }
}

// Entry points used by main.rs (aliases for the names main/bridge expect).
pub fn run_app() {
    run();
}

pub fn install_main_rx(rx: UnboundedReceiver<MainMsg>) {
    let _ = MAIN_RX.set(Mutex::new(rx));
}

// Encodings for the geometry structs passed through msg_send! (CGFloat is f64).
unsafe impl objc::Encode for NSPoint {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
    }
}
unsafe impl objc::Encode for NSSize {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGSize=dd}") }
    }
}
unsafe impl objc::Encode for NSRect {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}
/// NSEdgeInsets { top, left, bottom, right }.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSEdgeInsets {
    pub top: CGFloat,
    pub left: CGFloat,
    pub bottom: CGFloat,
    pub right: CGFloat,
}
unsafe impl objc::Encode for NSEdgeInsets {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{NSEdgeInsets=dddd}") }
    }
}
unsafe impl objc::Encode for CMTime {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{?=qiIq}") }
    }
}
/// CMTimeRange: start then duration, each a CMTime (returned by NSValue's CMTimeRangeValue).
/// Note: this 48-byte struct comes back through plain msg_send, which is correct on arm64. An x86_64 build
/// would need objc_msgSend_stret for the large-struct return.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CMTimeRange {
    pub start: CMTime,
    pub duration: CMTime,
}
unsafe impl objc::Encode for CMTimeRange {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{?={?=qiIq}{?=qiIq}}") }
    }
}

// Named callbacks for the handful of "always YES" overrides (objc only accepts fn pointers).
extern "C" fn always_yes(_t: &mut Object, _c: Sel) -> BOOL {
    YES
}
extern "C" fn always_yes_obj(_t: &mut Object, _c: Sel, _o: *mut Object) -> BOOL {
    YES
}
