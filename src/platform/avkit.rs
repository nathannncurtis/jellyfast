#![allow(unsafe_code)]
use crate::*;
use crate::jellyfin::models::*;
use crate::app::controller::*;
use crate::app::messages::*;
use crate::platform::appkit::*;
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

// ---- KVO / timer / actions ---------------------------------------------
pub(crate) extern "C" fn kvo(
    _this: &mut Object,
    _cmd: Sel,
    key_path: *mut Object,
    obj: *mut Object,
    _change: *mut Object,
    _ctx: *mut c_void,
) {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        let kp = to_str(key_path);
        match kp.as_str() {
            "status" => {
                let st: NSInteger = msg_send![obj, status];
                if st == 2 {
                    // AVPlayerItemFailedToPlayToEndTime — probe server, retry
                    if let Some(pn) = ui.playing.as_ref() {
                        crate::ui::player::toast("Network interruption. Checking your server…", Some("wifi.exclamationmark"), true);
                        let _ = ui.ui_tx.send(UiCmd::ProbeServer);
                        let _ = pn; // retry uses stored URL + current position
                    }
                }
            }
            "timeControlStatus" => {
                let s: NSInteger = msg_send![obj, timeControlStatus];
                if let Some(pn) = ui.playing.as_mut() {
                    pn.paused = s != 2;
                }
                if s == 2 {
                    crate::ui::player::backdrop_hide();
                }
                update_chip();
            }
            "presentationSize" => update_chip(),
            "externalPlaybackActive" => {
                // AirPlay started or stopped: the video is on another screen.
                let on: BOOL = msg_send![obj, externalPlaybackActive];
                crate::ui::player::set_external(on == YES);
            }
            _ => {}
        }
    }
}

pub(crate) extern "C" fn tick(_this: &mut Object, _cmd: Sel, _t: *mut Object) {
    unsafe {
        let ui = match UI.get_mut() {
            Some(u) => u,
            None => return,
        };
        drain_main(); // safety net in case a pump kick was missed
        crate::ui::cast::poll_tick(); // the remote bar refreshes its Play On session (own clock)
        if ui.playing.is_none() || ui.pitem.get().is_null() {
            return;
        }
        crate::ui::nowbar::update(); // every tick (0.25 s while the bar shows)
        crate::ui::playerbar::update(); // video transport bar, same cadence
        if !crate::ui::nowbar::slow_due() {
            return;
        }
        update_chip();
        crate::ui::player::hide_bars_if_idle();
        ensure_media_groups();
        let pn = match ui.playing.as_mut() {
            Some(p) => p,
            None => return,
        };
        let t: CMTime = msg_send![ui.player.get(), currentTime];
        let d: CMTime = msg_send![ui.pitem.get(), duration];
        pn.last_secs = t.seconds();
        if d.seconds() > 0.0 {
            pn.duration_secs = d.seconds();
        }
        pn.ticks_since_save += 1;
        if pn.ticks_since_save >= 10 {
            pn.ticks_since_save = 0;
            let _ = ui.ui_tx.send(UiCmd::Progress {
                item_id: pn.item_id.clone(),
                ticks: crate::jellyfin::urls::ticks_from_secs(pn.last_secs),
                paused: pn.paused,
            });
        }
        // Auto mark-watched near the end.
        if pn.duration_secs > 0.0 && pn.last_secs >= pn.duration_secs - 0.5 {
            let id = pn.item_id.clone();
            let _ = ui.ui_tx.send(UiCmd::Stopped {
                item_id: id.clone(),
                ticks: crate::jellyfin::urls::ticks_from_secs(pn.duration_secs),
            });
            let _ = ui.ui_tx.send(UiCmd::MarkPlayed { item_id: id, played: true });
            stop_playback_ui();
        }
    }
}

pub(crate) unsafe fn schedule_retry(delay: f64) {
    let ui = UI.get_mut().unwrap();
    if !ui.retry_timer.get().is_null() {
        let _: () = msg_send![ui.retry_timer.get(), invalidate];
        release_obj(ui.retry_timer.get());
        ui.retry_timer = Ptr(nil());
    }
    let t: *mut Object = msg_send![class!(NSTimer),
        scheduledTimerWithTimeInterval: delay target: ui.ctl.get()
        selector: sel!(jrRetry:) userInfo: nil() repeats: NO];
    ui.retry_timer = Ptr::retained(t);
}
pub(crate) extern "C" fn retry_now(_this: &mut Object, _cmd: Sel, _t: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        release_obj(ui.retry_timer.get()); // fired non-repeating timer is invalid; drop our +1
        ui.retry_timer = Ptr(nil());
        let _ = ui.ui_tx.send(UiCmd::ProbeServer);
    }
}

pub(crate) unsafe fn ensure_media_groups() {
    let ui = UI.get_mut().unwrap();
    if !ui.sub_group.is_null() || ui.playing.is_none() {
        return;
    }
    let asset: *mut Object = msg_send![ui.pitem.get(), asset];
    if asset.is_null() {
        return;
    }
    // NOTE: mediaSelectionGroupForMediaCharacteristic may block briefly on
    // first call; we only call it once, after the item is playing.
    let leg = ns("AVMediaCharacteristicLegible");
    let aud = ns("AVMediaCharacteristicAudible");
    let g1: *mut Object = msg_send![asset, mediaSelectionGroupForMediaCharacteristic: leg];
    let g2: *mut Object = msg_send![asset, mediaSelectionGroupForMediaCharacteristic: aud];
    for (group, opts_slot) in [(g1, 0usize), (g2, 1usize)] {
        if group.is_null() {
            continue;
        }
        let opts: *mut Object = msg_send![group, options];
        let n: NSUInteger = msg_send![opts, count];
        let mut vec = Vec::new();
        for i in 0..n {
            let o: *mut Object = msg_send![opts, objectAtIndex: i];
            let _: *mut Object = msg_send![o, retain];
            vec.push(o);
        }
        if opts_slot == 0 {
            ui.sub_group = Ptr::retained(group).get();
            ui.sub_opts = vec;
            ui.sel_sub = 0;
            apply_lang_pref(0);
        } else {
            ui.audio_group = Ptr::retained(group).get();
            ui.audio_opts = vec;
            ui.sel_audio = 0;
            apply_lang_pref(1);
        }
    }
}

pub(crate) unsafe fn apply_lang_pref(which: usize) {
    let ui = UI.get_mut().unwrap();
    let want = if which == 0 {
        ui.cfg.settings.subtitle_lang.clone()
    } else {
        ui.cfg.settings.audio_lang.clone()
    };
    if want.is_empty() {
        return;
    }
    let (group, opts) = if which == 0 {
        (ui.sub_group, &ui.sub_opts)
    } else {
        (ui.audio_group, &ui.audio_opts)
    };
    if group.is_null() {
        return;
    }
    for (i, o) in opts.iter().enumerate() {
        let lang = to_str(msg_send![*o, extendedLanguageTag]).to_ascii_lowercase();
        let disp = to_str(msg_send![*o, displayName]).to_ascii_lowercase();
        if lang.starts_with(&want.to_ascii_lowercase()) || disp.starts_with(&want.to_ascii_lowercase())
        {
            let _: () = msg_send![ui.pitem.get(),
                selectMediaOption: *o inMediaSelectionGroup: group];
            if which == 0 {
                ui.sel_sub = i + 1;
            } else {
                ui.sel_audio = i + 1;
            }
            break;
        }
    }
}

// ---- Menu & button actions ---------------------------------------------
pub(crate) extern "C" fn play_pause(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.playing.is_none() {
            return;
        }
        let rate: f32 = msg_send![ui.player.get(), rate];
        if rate > 0.0 {
            let _: () = msg_send![ui.player.get(), pause];
        } else {
            let def: f32 = msg_send![ui.player.get(), defaultRate];
            let _: () = msg_send![ui.player.get(), setRate: if def > 0.0 { def } else { 1.0f32 }];
        }
    }
}
pub(crate) extern "C" fn stop_playback(_t: &mut Object, _c: Sel, _s: *mut Object) {
    stop_for_cast();
}
/// Stops local playback and reports the stop, before a cast hands the item to another device.
pub fn stop_for_cast() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if let Some(pn) = ui.playing.as_ref() {
            let _ = ui.ui_tx.send(UiCmd::Stopped {
                item_id: pn.item_id.clone(),
                ticks: crate::jellyfin::urls::ticks_from_secs(pn.last_secs),
            });
        }
    }
    stop_playback_ui();
}

pub fn stop_playback_ui() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        crate::ui::player::set_external(false);
        if ui.playing.is_none() {
            return; // nothing observed / nothing to tear down
        }
        if !ui.pitem.get().is_null() {
            for kp in ["status", "presentationSize"] {
                let _: () = msg_send![ui.pitem.get(),
                    removeObserver: ui.ctl.get() forKeyPath: ns(kp)];
            }
        }
        // The timeControlStatus observer exists exactly while `playing` is Some.
        let _: () = msg_send![ui.player.get(), removeObserver: ui.ctl.get()
            forKeyPath: ns("timeControlStatus")];
        clear_media_groups();
        let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: nil()];
        release_obj(ui.pitem.get());
        ui.pitem = Ptr(nil());
        ui.playing = None;
        let content: *mut Object = msg_send![ui.win.get(), contentView];
        crate::ui::theme::crossfade(content, crate::ui::theme::M_EMPH);
        set_hidden(ui.player_host.get(), true);
        set_hidden(ui.content_host.get(), false);
        crate::ui::nowbar::hide();
        crate::ui::player::toast("", None, false);
        crate::ui::player::backdrop_hide();
        let _: () = msg_send![class!(NSCursor), setHiddenUntilMouseMoves: NO];
        let _: () = msg_send![ui.win.get(), setTitle: ns("Jellyfast")];
        let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.search.get()];
        crate::ui::cast::layout(); // after the player host is hidden: the strip may come back
    }
}
pub(crate) unsafe fn clear_media_groups() {
    let ui = UI.get_mut().unwrap();
    for o in ui.sub_opts.drain(..) {
        release_obj(o);
    }
    for o in ui.audio_opts.drain(..) {
        release_obj(o);
    }
    release_obj(ui.sub_group);
    release_obj(ui.audio_group);
    ui.sub_group = nil();
    ui.audio_group = nil();
}

pub(crate) unsafe fn seek_by(delta: f64) {
    let ui = UI.get_mut().unwrap();
    if ui.playing.is_none() {
        return;
    }
    let t: CMTime = msg_send![ui.player.get(), currentTime];
    let target = (t.seconds() + delta).clamp(0.0, ui.playing.as_ref().unwrap().duration_secs.max(1.0));
    let tm = CMTimeMakeWithSeconds(target, 600);
    let _: () = msg_send![ui.player.get(), seekToTime: tm];
}
pub(crate) extern "C" fn seek_back(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        seek_by(-10.0)
    }
}
pub(crate) extern "C" fn seek_fwd(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        seek_by(10.0)
    }
}
pub(crate) unsafe fn bump_volume(d: f64) {
    let ui = UI.get_mut().unwrap();
    if ui.playing.is_none() {
        return;
    }
    let v: f32 = msg_send![ui.player.get(), volume];
    let nv = (v + d as f32).clamp(0.0, 1.0);
    let _: () = msg_send![ui.player.get(), setVolume: nv];
    let mut cfg = ui.cfg.clone();
    cfg.volume = nv as f64;
    ui.cfg = cfg;
    crate::config::save_config_keeping_session(&ui.cfg);
}
pub(crate) extern "C" fn vol_up(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        bump_volume(0.05)
    }
}
pub(crate) extern "C" fn vol_down(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        bump_volume(-0.05)
    }
}
pub(crate) extern "C" fn mute_toggle(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.playing.is_none() {
            return;
        }
        let m: BOOL = msg_send![ui.player.get(), isMuted];
        let _: () = msg_send![ui.player.get(), setMuted: if m == NO { YES } else { NO }];
    }
}
pub(crate) unsafe fn cycle(which: usize) {
    let ui = UI.get_mut().unwrap();
    let (group, opts, cur) = if which == 0 {
        (ui.sub_group, ui.sub_opts.len(), ui.sel_sub)
    } else {
        (ui.audio_group, ui.audio_opts.len(), ui.sel_audio)
    };
    if group.is_null() || opts == 0 {
        return;
    }
    // Index 0 == "Off" (nil selection); 1.. = options.
    let next = (cur + 1) % (opts + 1);
    set_track(which, next);
}
/// Selects track `idx` of the subtitle (`which` 0) or audio (`which` 1) group: 0 is Off, i+1 is option i.
/// Shared by the keyboard cycle and the transport bar's menus.
pub(crate) unsafe fn set_track(which: usize, idx: usize) {
    let ui = UI.get_mut().unwrap();
    let (group, count) = if which == 0 {
        (ui.sub_group, ui.sub_opts.len())
    } else {
        (ui.audio_group, ui.audio_opts.len())
    };
    if group.is_null() || idx > count {
        return;
    }
    let opt = if idx == 0 {
        nil()
    } else if which == 0 {
        ui.sub_opts[idx - 1]
    } else {
        ui.audio_opts[idx - 1]
    };
    let _: () = msg_send![ui.pitem.get(), selectMediaOption: opt inMediaSelectionGroup: group];
    if which == 0 {
        ui.sel_sub = idx;
    } else {
        ui.sel_audio = idx;
    }
    crate::ui::player::toast(
        &format!("{}: {}", if which == 0 { "Subtitles" } else { "Audio" },
            if idx == 0 { "Off".to_string() } else {
                to_str(msg_send![opt, displayName])
            }),
        Some(if which == 0 { "captions.bubble" } else { "speaker.wave.2" }),
        false,
    );
}
pub(crate) extern "C" fn sub_cycle(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        cycle(0)
    }
}
pub(crate) extern "C" fn audio_cycle(_t: &mut Object, _c: Sel, _s: *mut Object) {
    unsafe {
        cycle(1)
    }
}
pub(crate) extern "C" fn set_speed(_t: &mut Object, _c: Sel, sender: *mut Object) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        if ui.playing.is_none() {
            return;
        }
        let tag: NSInteger = msg_send![sender, tag];
        let v = tag as f32 / 100.0;
        let _: () = msg_send![ui.player.get(), setDefaultRate: v];
        let rate: f32 = msg_send![ui.player.get(), rate];
        if rate > 0.0 {
            let _: () = msg_send![ui.player.get(), setRate: v];
        }
        crate::ui::player::toast(&format!("Speed: {}×", v), Some("gauge.with.dots.needle.67percent"), false);
    }
}

// ---- Playback ------------------------------------------------------------
pub fn start_playback(ps: &PlaybackStart) {
    unsafe {
        let ui = UI.get_mut().unwrap();
        // Tear down previous item observation.
        if !ui.pitem.get().is_null() {
            for kp in ["status", "presentationSize"] {
                let _: () = msg_send![ui.pitem.get(),
                    removeObserver: ui.ctl.get() forKeyPath: ns(kp)];
            }
            release_obj(ui.pitem.get());
            ui.pitem = Ptr(nil());
        }
        clear_media_groups();
        let url: *mut Object = msg_send![class!(NSURL), URLWithString: ns(&ps.url)];
        let item: *mut Object = if url.is_null() {
            nil()
        } else {
            msg_send![class!(AVPlayerItem), playerItemWithURL: url]
        };
        if item.is_null() {
            let _ = ui.main_tx.send(MainMsg::PlaybackError("could not create player item".into()));
            return;
        }
        let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: item];
        ui.pitem = Ptr::retained(item);
        for kp in ["status", "presentationSize"] {
            let _: () = msg_send![item, addObserver: ui.ctl.get()
                forKeyPath: ns(kp) options: 3usize context: nil()]; // new+old
        }
        if ui.playing.is_none() {
            let _: () = msg_send![ui.player.get(), addObserver: ui.ctl.get()
                forKeyPath: ns("timeControlStatus") options: 1usize context: nil()];
        }
        let _: () = msg_send![ui.player.get(), setVolume: ui.cfg.volume.clamp(0.0, 1.0) as f32];
        let _: () = msg_send![ui.player.get(), setDefaultRate: 1.0f32];
        let _: () = msg_send![ui.player.get(), setRate: 1.0f32];

        ui.playing = Some(PlayingNow {
            item_id: ps.item_id.clone(),
            url: ps.url.clone(),
            title: ps.title.clone(),
            method: ps.method,
            resume_secs: ps.resume_secs,
            last_secs: 0.0,
            duration_secs: ps.run_time_secs,
            paused: false,
            attempts: 0,
            ticks_since_save: 0,
        });
        // Upper-left header: the title (episode or movie name) and, under it, the header line.
        let header_sub = ps.header_subtitle();
        set_text(ui.top_title.get(), &ps.title);
        set_text(ui.top_sub.get(), header_sub.as_deref().unwrap_or(""));
        {
            // One line vertically centred when there is no subtitle.
            let has_sub = header_sub.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
            let f = frame_of(ui.top_title.get());
            let y = if has_sub { 26. } else { (crate::ui::theme::PLAYER_BAR_H - 18.) / 2. };
            set_frame(ui.top_title.get(), rect(f.origin.x, y, f.size.w, 18.));
        }
        let sub = ps.subtitle.clone().unwrap_or_default();
        if !sub.is_empty() {
            let _: () = msg_send![ui.win.get(), setTitle: ns(&format!("{} — {}", ps.title, sub))];
        } else {
            let _: () = msg_send![ui.win.get(), setTitle: ns(&ps.title)];
        }
        if ps.media_kind == MediaKind::Audio {
            // Audio: same AVPlayer, no video view. The library stays up with the bar below it.
            let content: *mut Object = msg_send![ui.win.get(), contentView];
            if !is_hidden(ui.player_host.get()) {
                // Coming from video: swap the player view out.
                crate::ui::theme::crossfade(content, crate::ui::theme::M_EMPH);
                set_hidden(ui.player_host.get(), true);
                set_hidden(ui.content_host.get(), false);
                crate::ui::player::backdrop_hide();
            }
            crate::ui::player::toast("", None, false);
            let _: () = msg_send![class!(NSCursor), setHiddenUntilMouseMoves: NO];
            crate::ui::nowbar::show(ps);
        } else {
            crate::ui::nowbar::hide();
            let content: *mut Object = msg_send![ui.win.get(), contentView];
            crate::ui::theme::crossfade(content, crate::ui::theme::M_EMPH);
            set_hidden(ui.content_host.get(), true);
            set_hidden(ui.player_host.get(), false);
            let _: () = msg_send![ui.win.get(), makeFirstResponder: ui.avview.get()];
            crate::ui::player::toast("", None, false);
            ui.bar_shown = false;
            crate::ui::player::show_bars();
            crate::ui::playerbar::on_start();
            crate::ui::player::apply_info_visibility();
            {
                use crate::ui::theme::*;
                let poster = ps.image_key.and_then(|k| ui.poster_cache.get(&k).copied()).unwrap_or(nil());
                let tint: *mut Object = msg_send![class!(NSColor), colorWithWhite: 0.13f64 alpha: 1.0f64];
                crate::ui::player::backdrop_show(tint, poster);
            }
        }
        if ps.resume_secs > 1.0 {
            let tm = CMTimeMakeWithSeconds(ps.resume_secs, 600);
            let _: () = msg_send![ui.player.get(), seekToTime: tm];
        }
        let _ = ui.ui_tx.send(UiCmd::Progress {
            item_id: ps.item_id.clone(),
            ticks: crate::jellyfin::urls::ticks_from_secs(ps.resume_secs),
            paused: false,
        });
        crate::ui::cast::layout();
    }
}

pub fn retry_playback() {
    unsafe {
        let ui = UI.get_mut().unwrap();
        let pn = match ui.playing.as_ref() {
            Some(p) => p,
            None => return,
        };
        if pn.attempts >= 3 {
            return;
        }
        let url = pn.url.clone();
        let resume = pn.last_secs;
        let item_id = pn.item_id.clone();
        let title = pn.title.clone();
        let method = pn.method;
        drop_replace_item(&url, resume);
        if let Some(pn) = ui.playing.as_mut() {
            pn.attempts += 1;
        }
        crate::ui::player::toast("Back online. Resuming…", Some("wifi"), false);
        let _ = ui.ui_tx.send(UiCmd::Progress {
            item_id,
            ticks: crate::jellyfin::urls::ticks_from_secs(resume),
            paused: false,
        });
        let _ = title;
        let _ = method;
    }
}

pub(crate) unsafe fn drop_replace_item(url: &str, resume: f64) {
    let ui = UI.get_mut().unwrap();
    let u: *mut Object = msg_send![class!(NSURL), URLWithString: ns(url)];
    if u.is_null() {
        let _ = ui.main_tx.send(MainMsg::PlaybackError("invalid stream URL".into()));
        return;
    }
    if !ui.pitem.get().is_null() {
        for kp in ["status", "presentationSize"] {
            let _: () = msg_send![ui.pitem.get(),
                removeObserver: ui.ctl.get() forKeyPath: ns(kp)];
        }
    }
    let item: *mut Object = msg_send![class!(AVPlayerItem), playerItemWithURL: u];
    clear_media_groups();
    let _: () = msg_send![ui.player.get(), replaceCurrentItemWithPlayerItem: item];
    if !ui.pitem.get().is_null() {
        release_obj(ui.pitem.get());
    }
    ui.pitem = Ptr::retained(item);
    for kp in ["status", "presentationSize"] {
        let _: () = msg_send![item, addObserver: ui.ctl.get()
            forKeyPath: ns(kp) options: 3usize context: nil()];
    }
    if resume > 1.0 {
        let tm = CMTimeMakeWithSeconds(resume, 600);
        let _: () = msg_send![ui.player.get(), seekToTime: tm];
    }
}
