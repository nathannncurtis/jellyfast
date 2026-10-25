#![allow(unsafe_code)]
//! One table drives everything keyboard-related: the native menu bar, the key
//! equivalents, the enable rules that keep plain keys out of text fields, and the
//! Keyboard Shortcuts help window (⌘/). Self-test checks the menus against it.
use crate::*;
use crate::platform::appkit::*;
use crate::ui::theme::*;
use objc::class;
use objc::msg_send;
use objc::runtime::{Object, Sel, BOOL, NO, YES};
use objc::{sel, sel_impl};
use std::cell::RefCell;

pub(crate) const CMD: NSUInteger = 1 << 20;
pub(crate) const ALT: NSUInteger = 1 << 19;
pub(crate) const CTRL: NSUInteger = 1 << 18;
pub(crate) const SHIFT: NSUInteger = 1 << 17;

/// One keyboard shortcut. `menu` picks the menu it lives in ("Speed" is the
/// Playback Speed submenu); `key` is the NSMenuItem key equivalent, empty for none.
pub(crate) struct Shortcut {
    pub menu: &'static str,
    pub title: &'static str,
    pub action: &'static str,
    pub key: &'static str,
    pub mods: NSUInteger,
    pub tag: NSInteger,
}

const fn s(
    menu: &'static str,
    title: &'static str,
    action: &'static str,
    key: &'static str,
    mods: NSUInteger,
    tag: NSInteger,
) -> Shortcut {
    Shortcut { menu, title, action, key, mods, tag }
}

const LEFT: &str = "\u{F702}";
const RIGHT: &str = "\u{F703}";
const UP: &str = "\u{F700}";
const DOWN: &str = "\u{F701}";

pub(crate) const SHORTCUTS: &[Shortcut] = &[
    // Application
    s("App", "About Jellyfast", "showAbout:", "", 0, 0),
    s("App", "Settings…", "showSettings:", ",", CMD, 0),
    s("App", "Quit Jellyfast", "terminate:", "q", CMD, 0),
    // Playback
    s("Controls", "Play / Pause", "playPause:", " ", 0, 0),
    s("Controls", "Stop", "stopPlayback:", ".", CMD, 0),
    s("Controls", "Show Playback Info", "togglePlaybackInfo:", "i", CMD, 0),
    s("Controls", "Back 10s", "seekBack:", LEFT, 0, 0),
    s("Controls", "Forward 10s", "seekFwd:", RIGHT, 0, 0),
    s("Controls", "Volume Up", "volUp:", UP, 0, 0),
    s("Controls", "Volume Down", "volDown:", DOWN, 0, 0),
    s("Controls", "Mute", "mute:", "m", 0, 0),
    s("Controls", "Next Subtitle Track", "subCycle:", "c", 0, 0),
    s("Controls", "Next Audio Track", "audioCycle:", "a", 0, 0),
    s("Controls", "Enter / Exit Full Screen", "fullscreen:", "f", 0, 0),
    // Playback speed (submenu of Controls)
    s("Speed", "0.5×", "setSpeed:", "1", CTRL | CMD, 50),
    s("Speed", "0.75×", "setSpeed:", "2", CTRL | CMD, 75),
    s("Speed", "1×", "setSpeed:", "3", CTRL | CMD, 100),
    s("Speed", "1.25×", "setSpeed:", "4", CTRL | CMD, 125),
    s("Speed", "1.5×", "setSpeed:", "5", CTRL | CMD, 150),
    s("Speed", "2×", "setSpeed:", "6", CTRL | CMD, 200),
    // Browse
    s("Go", "Search", "focusSearch:", "f", CMD, 0),
    s("Go", "Open Selected Item", "openSelected:", "\r", 0, 0),
    s("Go", "Next Item", "focusNext:", "\t", 0, 0),
    s("Go", "Previous Item", "focusPrevious:", "\u{19}", 0, 0),
    s("Go", "Library 1", "viewSelected:", "1", CMD, 0),
    s("Go", "Library 2", "viewSelected:", "2", CMD, 1),
    s("Go", "Library 3", "viewSelected:", "3", CMD, 2),
    s("Go", "Library 4", "viewSelected:", "4", CMD, 3),
    s("Go", "Library 5", "viewSelected:", "5", CMD, 4),
    s("Go", "Library 6", "viewSelected:", "6", CMD, 5),
    s("Go", "Library 7", "viewSelected:", "7", CMD, 6),
    s("Go", "Library 8", "viewSelected:", "8", CMD, 7),
    s("Go", "Library 9", "viewSelected:", "9", CMD, 8),
    s("Go", "Back", "goBack:", "[", CMD, 0),
    s("Go", "Refresh", "refresh:", "r", CMD, 0),
    s("Go", "Sign Out", "signOut:", "l", ALT, 0),
    // Requests (Seerr)
    s("Go", "Request Something New…", "openSeerr:", "k", ALT | CMD, 0),
    s("Go", "Seerr: Discover", "seerrShowTab:", "1", ALT | CMD, 0),
    s("Go", "Seerr: My Requests", "seerrShowTab:", "2", ALT | CMD, 1),
    // Help
    s("Help", "Keyboard Shortcuts", "showShortcuts:", "/", CMD, 0),
];

/// Actions that exist without a key equivalent on purpose.
pub(crate) const EXEMPT_ACTIONS: &[&str] = &["showAbout:"];

/// Actions that only make sense while something is playing; they grey out when idle.
const PLAYBACK_ACTIONS: &[&str] = &[
    "playPause:", "stopPlayback:", "seekBack:", "seekFwd:", "volUp:", "volDown:", "mute:",
    "subCycle:", "audioCycle:", "setSpeed:",
];

/// Human label for a key equivalent, e.g. "⌘F", "⌃⌘2", "←", "⇧⇥", "Space".
pub(crate) fn key_label(key: &str, mods: NSUInteger) -> String {
    let mut out = String::new();
    if mods & CTRL != 0 {
        out.push('⌃');
    }
    if mods & ALT != 0 {
        out.push('⌥');
    }
    if mods & SHIFT != 0 {
        out.push('⇧');
    }
    if mods & CMD != 0 {
        out.push('⌘');
    }
    let glyph: String = match key {
        " " => "Space".into(),
        LEFT => "←".into(),
        RIGHT => "→".into(),
        UP => "↑".into(),
        DOWN => "↓".into(),
        "\r" => "Return".into(),
        "\t" => "⇥".into(),
        "\u{19}" => {
            out.push('⇧');
            "⇥".into()
        }
        k => k.to_uppercase(),
    };
    out.push_str(&glyph);
    out
}

/// Rows for the help window, in table order. Library and speed entries are listed once as ranges.
pub(crate) enum HelpRow {
    Heading(&'static str),
    Line(String, &'static str),
}

pub(crate) fn help_rows() -> Vec<HelpRow> {
    let mut out = Vec::new();
    let mut last = "";
    for e in SHORTCUTS {
        if e.key.is_empty() {
            continue;
        }
        if (e.action == "viewSelected:" && e.tag != 0) || (e.action == "setSpeed:" && e.tag != 50) {
            continue;
        }
        if e.menu != last {
            last = e.menu;
            out.push(HelpRow::Heading(match e.menu {
                "App" => "Application",
                "Controls" => "Playback",
                "Speed" => "Playback Speed",
                "Go" => "Browse and Requests",
                _ => "Help",
            }));
        }
        let line = match e.action {
            "viewSelected:" => (format!("{}–9", key_label("1", CMD)), "Switch to a library, in sidebar order"),
            "setSpeed:" => (format!("{}–6", key_label("1", CTRL | CMD)), "Playback speed, 0.5× to 2×"),
            _ => (key_label(e.key, e.mods), e.title),
        };
        out.push(HelpRow::Line(line.0, line.1));
    }
    out
}

/// Whether an item with this action should be enabled right now. Plain keys (no ⌘ ⌥ ⌃)
/// yield to any text editor, so typing in search, login or settings is never hijacked.
pub(crate) unsafe fn item_enabled(action: &str, tag: NSInteger, mods: NSUInteger) -> bool {
    let ui = match UI.get() {
        Some(u) => u,
        None => return false,
    };
    if mods & (CMD | ALT | CTRL) == 0 && text_editing() {
        return false;
    }
    if PLAYBACK_ACTIONS.contains(&action) && ui.playing.is_none() {
        return false;
    }
    match action {
        "viewSelected:" => (tag as usize) < ui.views.len(),
        "focusSearch:" => {
            let hidden: BOOL = msg_send![ui.search.get(), isHiddenOrHasHiddenAncestor];
            hidden == NO
        }
        "openSelected:" => main_window_active() && crate::ui::library::kb_focus_index().is_some(),
        "focusNext:" | "focusPrevious:" => main_window_active() && grid_has_items() && !settings_open(),
        "seerrShowTab:" => crate::ui::seerr::is_configured(&ui.cfg.settings),
        _ => true,
    }
}

unsafe fn key_window() -> *mut Object {
    let ui = UI.get().unwrap();
    let k: *mut Object = msg_send![ui.app.get(), keyWindow];
    if k.is_null() { ui.win.get() } else { k }
}

/// True while the first responder of the key window is a text editor.
unsafe fn text_editing() -> bool {
    let w = key_window();
    let fr: *mut Object = msg_send![w, firstResponder];
    if fr.is_null() {
        return false;
    }
    let is_text: BOOL = msg_send![fr, isKindOfClass: class!(NSText)];
    is_text == YES
}

/// Keyboard grid navigation only runs when the main window is the one in front.
unsafe fn main_window_active() -> bool {
    let ui = UI.get().unwrap();
    let k: *mut Object = msg_send![ui.app.get(), keyWindow];
    k.is_null() || k == ui.win.get()
}

unsafe fn grid_has_items() -> bool {
    let ui = UI.get().unwrap();
    ui.grid_data.as_ref().map_or(false, |g| !g.cells.is_empty())
}

/// Menu validation: greys out items that cannot act right now, and lets the key fall
/// through to the focused control when they are disabled.
pub(crate) extern "C" fn validate_menu_item(_t: &mut Object, _c: Sel, item: *mut Object) -> BOOL {
    unsafe {
        let action: Sel = msg_send![item, action];
        if action.as_ptr().is_null() {
            return YES;
        }
        let mods: NSUInteger = msg_send![item, keyEquivalentModifierMask];
        let tag: NSInteger = msg_send![item, tag];
        if item_enabled(action.name(), tag, mods) { YES } else { NO }
    }
}

pub(crate) extern "C" fn focus_search(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let k = key_window();
        let field = if !ui.seerr.win.get().is_null() && k == ui.seerr.win.get() {
            ui.seerr.search.get()
        } else {
            ui.search.get()
        };
        let _: BOOL = msg_send![k, makeFirstResponder: field];
        let _: () = msg_send![field, selectText: nil()];
    }
}

pub(crate) extern "C" fn open_selected(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { crate::ui::library::kb_open() }
}
pub(crate) extern "C" fn focus_next(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { crate::ui::library::kb_move(1) }
}
pub(crate) extern "C" fn focus_previous(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { crate::ui::library::kb_move(-1) }
}
pub(crate) extern "C" fn seerr_show_tab(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let tag: NSInteger = msg_send![sender, tag];
        crate::ui::seerr::open_window();
        crate::ui::seerr::select_mode(tag == 1);
    }
}

thread_local! {
    static HELP_WIN: RefCell<Option<Ptr>> = const { RefCell::new(None) };
}

/// ⌘/: show (or bring forward) the Keyboard Shortcuts window, built from the table.
pub(crate) extern "C" fn show_shortcuts(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe { show_shortcuts_window() }
}

pub(crate) unsafe fn show_shortcuts_window() {
    if let Some(w) = HELP_WIN.with(|h| h.borrow().as_ref().map(|p| p.get())) {
        let _: () = msg_send![w, makeKeyAndOrderFront: nil()];
        return;
    }
    const W: f64 = 440.;
    const PAD: f64 = 24.;
    const KEY_W: f64 = 150.;
    let rows = help_rows();
    let mut h = 2. * PAD;
    let mut headings = 0usize;
    for r in &rows {
        h += match r {
            HelpRow::Heading(_) => {
                headings += 1;
                30.
            }
            HelpRow::Line(..) => 22.,
        };
    }
    h += 8. * headings.saturating_sub(1) as f64; // gap before every heading but the first
    let w: *mut Object = msg_send![jr_window_class(), alloc];
    let w: *mut Object = msg_send![w,
        initWithContentRect: rect(0., 0., W, h)
        styleMask: (1 | 2) as NSUInteger backing: 2usize defer: NO];
    let w = autorel(w);
    let _: () = msg_send![w, setTitle: ns("Keyboard Shortcuts")];
    let _: () = msg_send![w, setReleasedWhenClosed: NO];
    let c: *mut Object = msg_send![w, contentView];
    let mut top = h - PAD;
    for r in &rows {
        match r {
            HelpRow::Heading(t) => {
                if top < h - PAD {
                    top -= 8.;
                }
                let l = label(t, TypeStyle { size: 11., weight: Weight::Bold, mono: false }, C_TEXT2, ALIGN_LEFT);
                set_frame(l, rect(PAD, top - 20., W - 2. * PAD, 16.));
                add_sub(c, l);
                top -= 30.;
            }
            HelpRow::Line(keys, title) => {
                let k = label(keys, TypeStyle { size: 13., weight: Weight::Regular, mono: true }, C_TEXT, ALIGN_RIGHT);
                set_frame(k, rect(PAD, top - 20., KEY_W, 17.));
                add_sub(c, k);
                let t = label(title, TypeStyle { size: 13., weight: Weight::Regular, mono: false }, C_TEXT, ALIGN_LEFT);
                set_frame(t, rect(PAD + KEY_W + 16., top - 20., W - 2. * PAD - KEY_W - 16., 17.));
                add_sub(c, t);
                top -= 22.;
            }
        }
    }
    HELP_WIN.with(|hw| *hw.borrow_mut() = Some(Ptr::retained(w)));
    place_window(w);
    let _: () = msg_send![w, makeKeyAndOrderFront: nil()];
}

/// Close the help window (self-test uses this so later steps start clean).
pub(crate) unsafe fn hide_shortcuts_window() {
    if let Some(w) = HELP_WIN.with(|h| h.borrow().as_ref().map(|p| p.get())) {
        let _: () = msg_send![w, orderOut: nil()];
    }
}
pub(crate) unsafe fn shortcuts_window_visible() -> bool {
    match HELP_WIN.with(|h| h.borrow().as_ref().map(|p| p.get())) {
        Some(w) => {
            let v: BOOL = msg_send![w, isVisible];
            v == YES
        }
        None => false,
    }
}

/// Build the native menu bar from the table. Speed items go into the Playback Speed submenu.
pub(crate) unsafe fn build_menus_from_table() {
    let ui = UI.get_mut().unwrap();
    let ctl = ui.ctl.get();
    let main_menu: *mut Object = msg_send![class!(NSMenu), alloc];
    let main_menu: *mut Object = msg_send![main_menu, initWithTitle: ns("MainMenu")];
    let _: () = msg_send![ui.app.get(), setMainMenu: main_menu];
    autorel(main_menu);

    let top = |title: &str| -> *mut Object {
        let item: *mut Object = msg_send![class!(NSMenuItem), alloc];
        let item: *mut Object = msg_send![item, initWithTitle: ns(title) action: nil() keyEquivalent: ns("")];
        autorel(item);
        let menu: *mut Object = msg_send![class!(NSMenu), alloc];
        let menu: *mut Object = msg_send![menu, initWithTitle: ns(title)];
        let _: () = msg_send![item, setSubmenu: menu];
        let _: () = msg_send![main_menu, addItem: item];
        autorel(menu)
    };
    let add = |menu: *mut Object, e: &Shortcut| -> *mut Object {
        let it = menu_item(e.title, Sel::register(e.action), e.key);
        if e.action != "terminate:" {
            let _: () = msg_send![it, setTarget: ctl];
        }
        let _: () = msg_send![it, setKeyEquivalentModifierMask: e.mods];
        let _: () = msg_send![it, setTag: e.tag];
        let _: () = msg_send![menu, addItem: it];
        it
    };
    let sep = || -> *mut Object { msg_send![class!(NSMenuItem), separatorItem] };

    let app_menu = top("Jellyfast");
    for e in SHORTCUTS.iter().filter(|e| e.menu == "App") {
        if e.action == "terminate:" {
            let _: () = msg_send![app_menu, addItem: sep()];
        }
        add(app_menu, e);
        if e.action == "showAbout:" {
            let _: () = msg_send![app_menu, addItem: sep()];
        }
    }

    let ctl_menu = top("Controls");
    for e in SHORTCUTS.iter().filter(|e| e.menu == "Controls") {
        let it = add(ctl_menu, e);
        if e.action == "togglePlaybackInfo:" {
            ui.info_item = Ptr(it);
            let _: () = msg_send![it, setState: ui.cfg.settings.show_stats as isize];
        }
    }
    {
        let item: *mut Object = msg_send![class!(NSMenuItem), alloc];
        let item: *mut Object = msg_send![item, initWithTitle: ns("Playback Speed") action: nil() keyEquivalent: ns("")];
        autorel(item);
        let speed_menu: *mut Object = msg_send![class!(NSMenu), alloc];
        let speed_menu: *mut Object = msg_send![speed_menu, initWithTitle: ns("Playback Speed")];
        autorel(speed_menu);
        let _: () = msg_send![item, setSubmenu: speed_menu];
        let _: () = msg_send![ctl_menu, addItem: sep()];
        let _: () = msg_send![ctl_menu, addItem: item];
        for e in SHORTCUTS.iter().filter(|e| e.menu == "Speed") {
            add(speed_menu, e);
        }
    }

    let go_menu = top("Go");
    for e in SHORTCUTS.iter().filter(|e| e.menu == "Go") {
        if e.action == "goBack:" || e.action == "openSeerr:" || e.action == "focusNext:" {
            let _: () = msg_send![go_menu, addItem: sep()];
        }
        add(go_menu, e);
    }

    let help_menu = top("Help");
    for e in SHORTCUTS.iter().filter(|e| e.menu == "Help") {
        add(help_menu, e);
    }
}

/// Self-test: every leaf menu item with an action has a key equivalent (or is exempt),
/// no key combination is bound twice, and the menus match the table exactly.
pub(crate) unsafe fn menu_key_problems() -> Vec<String> {
    let ui = UI.get().unwrap();
    let main: *mut Object = msg_send![ui.app.get(), mainMenu];
    let mut problems = Vec::new();
    let mut seen: Vec<(String, NSUInteger, String)> = Vec::new();
    let mut leaves = 0usize;
    let mut keyed = 0usize;
    walk_menu(main, &mut |item| {
        let action: Sel = msg_send![item, action];
        if action.as_ptr().is_null() {
            return;
        }
        leaves += 1;
        let title = to_str(msg_send![item, title]);
        let key = to_str(msg_send![item, keyEquivalent]);
        let mods: NSUInteger = msg_send![item, keyEquivalentModifierMask];
        let name = action.name();
        if key.is_empty() {
            if !EXEMPT_ACTIONS.contains(&name) {
                problems.push(format!("“{}” ({}) has no shortcut", title, name));
            }
            return;
        }
        keyed += 1;
        if let Some((_, _, other)) = seen.iter().find(|(k, m, _)| *k == key && *m == mods) {
            problems.push(format!("“{}” duplicates “{}” ({})", title, other, key_label(&key, mods)));
        } else {
            seen.push((key, mods, title));
        }
    });
    let table_keyed = SHORTCUTS.iter().filter(|e| !e.key.is_empty()).count();
    if leaves != SHORTCUTS.len() || keyed != table_keyed {
        problems.push(format!(
            "menus have {} items ({} keyed) but the table has {} ({} keyed)",
            leaves,
            keyed,
            SHORTCUTS.len(),
            table_keyed
        ));
    }
    problems
}

unsafe fn walk_menu(menu: *mut Object, f: &mut dyn FnMut(*mut Object)) {
    let n: NSInteger = msg_send![menu, numberOfItems];
    for i in 0..n {
        let item: *mut Object = msg_send![menu, itemAtIndex: i];
        let sub: *mut Object = msg_send![item, submenu];
        if !sub.is_null() {
            walk_menu(sub, f);
        } else {
            f(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_key_combinations() {
        let mut seen = std::collections::HashSet::new();
        for e in SHORTCUTS.iter().filter(|e| !e.key.is_empty()) {
            assert!(seen.insert((e.key, e.mods)), "duplicate shortcut: {}", key_label(e.key, e.mods));
        }
    }

    #[test]
    fn only_exempt_actions_lack_a_key() {
        for e in SHORTCUTS.iter().filter(|e| e.key.is_empty()) {
            assert!(EXEMPT_ACTIONS.contains(&e.action), "{} has no shortcut", e.action);
        }
    }

    #[test]
    fn labels_read_like_macos() {
        assert_eq!(key_label("f", CMD), "⌘F");
        assert_eq!(key_label("1", CTRL | CMD), "⌃⌘1");
        assert_eq!(key_label(LEFT, 0), "←");
        assert_eq!(key_label(" ", 0), "Space");
        assert_eq!(key_label("\u{19}", 0), "⇧⇥");
        assert_eq!(key_label("k", ALT | CMD), "⌥⌘K");
    }
}
