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
    pub poster: *mut Object,
    pub title: *mut Object,
    pub sub: *mut Object,
    pub prog: *mut Object,
    pub badge: *mut Object,
    pub hover_icon: *mut Object,
    pub key: u64,
    pub meta: ListItem,
}

pub struct GridData {
    pub cells: Vec<CellHandle>,
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
    pub header_title: Ptr,
    pub lib_scroll: Ptr,
    pub spinner: Ptr,
    pub status_label: Ptr,
    pub content_host: Ptr,
    pub player_host: Ptr,
    pub top_bar: Ptr,
    pub top_title: Ptr,
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
    pub settings_win: Ptr,
    pub s_url: Ptr,
    pub s_sub_lang: Ptr,
    pub s_audio_lang: Ptr,
    pub retry_pending: bool,
    pub s_quality: Ptr,
    pub s_theme: Ptr,
    pub seerr: crate::ui::seerr::SeerrUi,
    // Rust-side state
    pub rows: Vec<(*mut Object, String)>, // sidebar row buttons + view ids
    pub views: Vec<(String, String, Option<String>)>,
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
    pub cur_title: String,
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
            always_yes_obj as extern "C" fn(&mut Object, Sel, *mut Object) -> BOOL,
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
            (sel!(seerrRequest:), crate::ui::seerr::seerr_request as Extern0),
            (sel!(seerrTab:), crate::ui::seerr::seerr_tab as Extern0),
            (sel!(seerrModeChanged:), crate::ui::seerr::seerr_mode_changed as Extern0),
            (sel!(goBack:), go_back as Extern0),
            (sel!(viewSelected:), view_selected as Extern0),
            (sel!(itemClicked:), item_clicked as Extern0),
            (sel!(markWatched:), mark_watched as Extern0),
            (sel!(openInWeb:), open_in_web as Extern0),
            (sel!(connectClicked:), connect_clicked as Extern0),
            (sel!(saveSettings:), save_settings as Extern0),
            (sel!(cancelSettings:), cancel_settings as Extern0),
            (sel!(sortChanged:), sort_changed as Extern0),
            (sel!(controlTextDidChange:), text_changed as Extern0),
            (sel!(jrTick:), tick as Extern0),
            (sel!(jrRetry:), retry_now as Extern0),
        ] {
            d.add_method(s, f);
        }
        d.add_method(sel!(observeValueForKeyPath:ofObject:change:context:), kvo as KvoFn);
        d.register();
    }
    crate::ui::seerr::register_seerr_classes();
    // Poster grid container
    {
        let mut d = ClassDecl::new("JRGridView", class!(NSView)).unwrap();
        d.add_ivar::<*mut c_void>("jrData");
        d.add_method(sel!(isFlipped), always_yes as extern "C" fn(&mut Object, Sel) -> BOOL);
        d.add_method(sel!(layout), grid_layout as extern "C" fn(&mut Object, Sel));
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

pub(crate) unsafe fn update_chip() {
    let ui = match UI.get_mut() {
        Some(u) => u,
        None => return,
    };
    let pn = match ui.playing.as_ref() {
        Some(p) => p,
        None => return,
    };
    if ui.pitem.get().is_null() {
        return; // item being swapped (KVO can fire mid-replace)
    }
    let mut s = String::new();
    let tcs: NSInteger = msg_send![ui.player.get(), timeControlStatus];
    if tcs == 1 {
        s.push_str("Buffering…");
    } else {
        // position / duration
        let t: CMTime = msg_send![ui.player.get(), currentTime];
        let d: CMTime = msg_send![ui.pitem.get(), duration];
        let dur = d.seconds();
        s.push_str(&format!(
            "{} / {}",
            crate::jellyfin::urls::fmt_clock(t.seconds()),
            crate::jellyfin::urls::fmt_clock(dur)
        ));
        // resolution
        let sz: NSSize = msg_send![ui.pitem.get(), presentationSize];
        if sz.w > 0.0 {
            s.push_str(&format!("  ·  {}×{}", sz.w as i64, sz.h as i64));
        }
        // bitrate from access log (last event)
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
    }
    set_chip(&s, false);
}

pub(crate) unsafe fn set_chip(text: &str, is_err: bool) {
    let ui = UI.get_mut().unwrap();
    set_text(ui.chip_label.get(), text);
    let col = if is_err { color_named("red") } else { color_named("label") };
    let _: () = msg_send![ui.chip_label.get(), setTextColor: col];
}

pub(crate) extern "C" fn fullscreen(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _: () = msg_send![ui.win.get(), toggleFullScreen: nil()];
    }
}
pub(crate) extern "C" fn show_settings(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let _ = ui.main_tx.send(MainMsg::SettingsSnapshot(Box::new(ui.cfg.clone())));
    }
}
pub(crate) unsafe fn show_settings_sheet() {
    let ui = UI.get_mut().unwrap();
    let attached: *mut Object = msg_send![ui.win.get(), attachedSheet];
    if !attached.is_null() {
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
    let attached: *mut Object = msg_send![ui.win.get(), attachedSheet];
    if !attached.is_null() {
        let _: () = msg_send![ui.win.get(), endSheet: ui.settings_win.get()];
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
        let new_url = read_field(ui.s_url.get());
        let url_changed = match &ui.cfg.server_url {
            Some(u) => u != &new_url && !new_url.is_empty(),
            None => !new_url.is_empty(),
        };
        ui.cfg.settings = s;
        if url_changed {
            ui.cfg.server_url = Some(new_url);
        }
        crate::config::save_config(&ui.cfg);
        apply_theme();
        // SaveSettings replaces the worker's whole Settings, so it goes first;
        // ui.cfg.settings carries the last *confirmed* Seerr values. SaveSeerr
        // follows and wins for seerr_enabled/seerr_url.
        let _ = ui.ui_tx.send(UiCmd::SaveSettings {
            settings: ui.cfg.settings.clone(),
            reconnect: url_changed,
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
        let seerr_dirty = seerr_on != cur.seerr_enabled
            || (seerr_on && (seerr_url != cur.seerr_url || user != cur.seerr_username))
            || (!seerr_on && !seerr_url.is_empty() && seerr_url != cur.seerr_url)
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
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
            let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
        }
    }
}
pub(crate) extern "C" fn go_back(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.mode_stack.len() > 1 {
            ui.mode_stack.pop();
            if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
                let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
            }
        }
    }
}
pub(crate) extern "C" fn view_selected(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        if let Some((id, name, ct)) = ui.views.get(tag as usize).cloned() {
            ui.mode_stack = vec![(id.clone(), name.clone(), ct.clone())];
            set_hidden(ui.spinner.get(), false);
            set_hidden(ui.status_label.get(), true);
            let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
        }
    }
}
pub(crate) extern "C" fn item_clicked(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let tag: NSInteger = msg_send![sender, tag];
        let it = match ui.items.get(tag as usize) {
            Some(i) => i.clone(),
            None => return,
        };
        set_hidden(ui.spinner.get(), false);
        match it.item_type.as_str() {
            "Series" => {
                let _ = ui.ui_tx.send(UiCmd::Episodes { series_id: it.id.clone(), series_name: it.name.clone() });
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
pub(crate) extern "C" fn open_in_web(_t: &mut Object, _c: Sel, sender: *mut Object) {
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
        let key = read_field_secure(ui.login_key.get());
        login_error(""); // clear
        if url.is_empty() {
            login_error("Enter your server address (e.g. http://192.168.1.10:8096).");
            return;
        }
        if user.is_empty() && key.is_empty() {
            login_error("Enter a username + password, or an API key.");
            return;
        }
        let _: () = msg_send![ui.login_btn.get(), setEnabled: NO];
        set_text(ui.login_btn.get(), "Connecting…");
        let _ = ui.ui_tx.send(UiCmd::Connect {
            url,
            username: user,
            password: if pass.is_empty() { None } else { Some(pass) },
            api_key: if key.is_empty() { None } else { Some(key) },
        });
    }
}
pub(crate) extern "C" fn sort_changed(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
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
            let q = read_field(ui.search.get());
            if q.trim().is_empty() {
                if let Some((id, name, ct)) = ui.mode_stack.last().cloned() {
                    let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
                }
            } else {
                let _ = ui.ui_tx.send(UiCmd::Search { q });
            }
        }
    }
}

// ---- App delegate callbacks --------------------------------------------
pub(crate) extern "C" fn app_did_finish_launch(_t: &mut Object, _c: Sel, _n: *mut Object) {
    unsafe {
        build_window_and_ui();
        let ui = UI.get_mut().unwrap();
        // If a server + token are stored, the worker is already auto-connecting;
        // reflect that state in the login card.
        if ui.cfg.server_url.is_some() {
            set_text(ui.login_err.get(), "Connecting to your server…");
        }
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
        crate::config::save_config(&ui.cfg);
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
pub(crate) const MAT_SIDEBAR: i64 = 7;
pub(crate) const MAT_POPOVER: i64 = 6;
pub(crate) const MAT_HUD: i64 = 13;
pub(crate) const MAT_UNDER_WINDOW: i64 = 21;
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
    let _: BOOL = msg_send![app, setActivationPolicy: 0i64]; // regular app

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
        header_title: Ptr(nil()),
        lib_scroll: Ptr(nil()),
        spinner: Ptr(nil()),
        status_label: Ptr(nil()),
        content_host: Ptr(nil()),
        player_host: Ptr(nil()),
        top_bar: Ptr(nil()),
        top_title: Ptr(nil()),
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
        settings_win: Ptr(nil()),
        s_url: Ptr(nil()),
        s_sub_lang: Ptr(nil()),
        s_audio_lang: Ptr(nil()),
        retry_pending: false,
        s_quality: Ptr(nil()),
        s_theme: Ptr(nil()),
        seerr: crate::ui::seerr::SeerrUi::new(),
        rows: vec![],
        views: vec![],
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
        cur_title: String::new(),
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
unsafe impl objc::Encode for CMTime {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{?=qiIq}") }
    }
}

// Named callbacks for the handful of "always YES" overrides (objc only accepts fn pointers).
extern "C" fn always_yes(_t: &mut Object, _c: Sel) -> BOOL {
    YES
}
extern "C" fn always_yes_obj(_t: &mut Object, _c: Sel, _o: *mut Object) -> BOOL {
    YES
}
