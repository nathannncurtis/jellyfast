#![allow(unsafe_code)]
use crate::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::ui::shell::*;
use crate::ui::library::*;
use crate::ui::settings::*;
use objc::msg_send;
use objc::runtime::{BOOL, NO, Object, Sel, YES};
use objc::{sel, sel_impl};

/// Drain the MainMsg queue on the main thread; called via on_main.
pub(crate) fn drain_main() {
    let rx = MAIN_RX.get();
    if let Some(m) = rx {
        if let Ok(mut guard) = m.lock() {
            while let Ok(msg) = guard.try_recv() {
                handle_main(msg);
            }
        }
    }
}

// ========================================================================
// SECTION: Application Controller (message pump + menu/KVO handlers)
// ========================================================================
// SPLIT: → src/app/controller.rs. `handle_main` is the UI-side reducer;
// each arm is a candidate method on a future `Controller` struct.

pub(crate) fn handle_main(msg: MainMsg) {
    match msg {
        MainMsg::Connected { user_name, views, has_live_tv } => {
            unsafe {
                let ui = UI.get_mut().unwrap();
                ui.views = views.clone();
                sidebar_rebuild(user_name, &views, has_live_tv);
                show_login_or_library(true);
            }
        }
        MainMsg::ConnectError(e) => unsafe {
            login_error(&e);
            show_login_or_library(false);
        },
        MainMsg::ItemsLoaded { title, items, can_go_back } => unsafe {
            let ui = UI.get_mut().unwrap();
            ui.items = items.clone();
            ui.cur_title = title.clone();
            set_text(ui.header_title.get(), &title);
            let _: BOOL = msg_send![ui.back_btn.get(), setHidden: (!can_go_back) as BOOL];
            set_hidden(ui.spinner.get(), true);
            set_hidden(ui.status_label.get(), items.is_empty());
            if items.is_empty() {
                set_text(ui.status_label.get(), "Nothing here.");
            }
            grid_set_items(&items);
        },
        MainMsg::PosterReady { key, bytes } => unsafe {
            poster_ready(key, &bytes);
        },
        MainMsg::PlaybackReady(ps) => unsafe {
            start_playback(&ps);
        },
        MainMsg::PlaybackError(e) => unsafe {
            set_chip(&format!("⚠︎ {}", e), true);
            set_hidden(UI.get_mut().unwrap().spinner.get(), true);
        },
        MainMsg::ServerUp => unsafe {
            retry_playback();
        },
        MainMsg::ServerDown => unsafe {
            let ui = UI.get_mut().unwrap();
            if let Some(pn) = ui.playing.as_mut() {
                pn.attempts += 1;
                if pn.attempts < 3 {
                    set_chip("Reconnecting…", false);
                    schedule_retry(2.0);
                } else {
                    set_chip("Stream failed — server unreachable. Press ⌘R to try again.", true);
                }
            }
        },
        MainMsg::SignedOut => unsafe {
            show_login_or_library(false);
            set_text(UI.get_mut().unwrap().login_err.get(), "Signed out.");
        },
        MainMsg::SettingsSnapshot(cfg) => unsafe {
            settings_fill(&cfg);
            show_settings_sheet();
        },
        // TODO(seerr-ui): route the Seerr messages to the Seerr window.
        _ => {}
    }
}
