#![allow(unsafe_code)]
use crate::*;
use crate::platform::appkit::*;
use crate::platform::avkit::*;
use crate::app::messages::UiCmd;
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
                // Land on the remembered place: its library if it still exists, else Home.
                let remembered = ui
                    .cfg
                    .last_view
                    .as_ref()
                    .and_then(|id| ui.views.iter().position(|v| &v.0 == id));
                match remembered {
                    Some(idx) => {
                        let (id, name, ct) = ui.views[idx].clone();
                        ui.mode_stack = vec![(id.clone(), name.clone(), ct.clone())];
                        ui.selected_view = Some(idx);
                        sidebar_refresh_selection();
                        begin_load(true);
                        let _ = ui.ui_tx.send(UiCmd::LoadView { view_id: id, name, collection_type: ct });
                        let _ = ui.ui_tx.send(UiCmd::LoadHome);
                    }
                    None => crate::ui::home::enter(), // also requests the Home rows
                }
            }
        }
        MainMsg::ConnectError(e) => unsafe {
            login_error(&e);
            show_login_or_library(false);
        },
        MainMsg::HomeLoaded(home) => unsafe {
            crate::ui::home::set_rows(home.rows);
        },
        MainMsg::ItemsLoaded { title, items, can_go_back } => unsafe {
            crate::ui::home::leave();
            let ui = UI.get_mut().unwrap();
            ui.items = items.clone();
            ui.cur_title = title.clone();
            set_text(ui.header_title.get(), &title);
            set_hidden(ui.lib_back.get(), !can_go_back);
            set_busy(false);
            set_count(&count_label(&items), false);
            if items.is_empty() {
                let q = read_field(ui.search.get());
                let body = if q.trim().is_empty() {
                    "This library is empty.".to_string()
                } else {
                    format!("No results for “{}”", q.trim())
                };
                show_state("tray", "No items yet", &body, false);
            } else {
                hide_state();
            }
            grid_set_items(&items);
            header_layout();
        },
        MainMsg::PosterReady { key, bytes } => unsafe {
            poster_ready(key, &bytes);
        },
        MainMsg::PlaybackReady(ps) => unsafe {
            start_playback(&ps);
        },
        MainMsg::PlaybackError(e) => unsafe {
            let ui = UI.get_mut().unwrap();
            set_busy(false);
            if ui.playing.is_some() {
                crate::ui::player::toast(&e, Some("exclamationmark.triangle"), true);
            } else if ui.items.is_empty() {
                grid_set_items(&[]);
                show_state("exclamationmark.triangle", "Couldn't load this library", &e, true);
            } else {
                set_count(&clip_msg(&e, 80), true);
            }
        },
        MainMsg::ServerUp => unsafe {
            retry_playback();
        },
        MainMsg::ServerDown => unsafe {
            let ui = UI.get_mut().unwrap();
            if let Some(pn) = ui.playing.as_mut() {
                pn.attempts += 1;
                if pn.attempts < 3 {
                    crate::ui::player::toast("Reconnecting…", Some("wifi.exclamationmark"), false);
                    schedule_retry(2.0);
                } else {
                    crate::ui::player::toast("Can't reach your server. Press ⌘R to retry.", Some("wifi.exclamationmark"), true);
                }
            }
        },
        MainMsg::SignedOut => unsafe {
            let ui = UI.get_mut().unwrap();
            ui.selected_view = None;
            sidebar_refresh_selection();
            show_login_or_library(false);
            login_status("You've signed out.");
        },
        MainMsg::SettingsSnapshot(cfg) => unsafe {
            settings_fill(&cfg);
            show_settings_sheet();
        },
        MainMsg::SeerrResults(v) => crate::ui::seerr::on_results(v),
        MainMsg::SeerrRequested { title } => crate::ui::seerr::on_requested(title),
        MainMsg::SeerrRequestList(v) => crate::ui::seerr::on_request_list(v),
        MainMsg::SeerrPoster { path, bytes } => crate::ui::seerr::on_poster(path, &bytes),
        MainMsg::SeerrError(m) => crate::ui::seerr::on_error(m),
        MainMsg::SeerrStatus { ok, message } => unsafe {
            crate::ui::seerr::on_save_status(ok, message);
        },
    }
}
