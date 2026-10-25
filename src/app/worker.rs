use crate::*;
use std::collections::VecDeque;
use crate::app::messages::*;
use crate::jellyfin::client::JellyfinClient;
use crate::jellyfin::models::{DetailItem, Episode, Person, Season};
use crate::jellyfin::urls::{build_art_url, build_image_url};
use crate::config::{load_config, save_config, Config};
use crate::seerr::client::{SeerrClient, SeerrError};

pub(crate) async fn worker(mut rx: UnboundedReceiver<UiCmd>, main_tx: UnboundedSender<MainMsg>, cfg: Config) {
    let mut client: Option<JellyfinClient> = None;
    let mut cfg = cfg;
    // Lazily built from settings + Keychain; invalidated when settings change.
    let mut seerr: Option<SeerrClient> = None;
    // Live stream opened by the current channel playback; closed on the next one / stop.
    let mut live_stream: Option<String> = None;

    // Poster disk cache: ~/Library/Caches/Jellyfast/posters
    let poster_dir = crate::config::cache_dir().join("posters");
    let _ = std::fs::create_dir_all(&poster_dir);

    // Silent auto-connect from persisted server + keychain token, only when
    // "Remember me" was on. Otherwise nothing may persist: drop any leftover token.
    if !cfg.remember_me {
        if let Some(url) = cfg.server_url.clone() {
            crate::config::delete_token(&url);
        }
    } else if let (Some(url), Some(uid)) = (cfg.server_url.clone(), cfg.user_id.clone()) {
        if let Some(token) = crate::config::load_token(&url) {
            match crate::jellyfin::client::JellyfinClient::new(&url, token.clone(), uid.clone(), &cfg.device_id) {
                Ok(c) => match connect_flow(&c, &cfg.username.clone().unwrap_or_else(|| uid.clone()), &main_tx).await {
                    Ok(()) => client = Some(c),
                    Err(e) => {
                        let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                    }
                },
                Err(e) => {
                    let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                }
            }
        } else {
            let _ = main_tx.send(MainMsg::ConnectError(String::new()));
        }
    }

    let http_poster = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .ok();

    let mut backlog: VecDeque<UiCmd> = VecDeque::new();
    loop {
        let cmd = match backlog.pop_front() {
            Some(c) => c,
            None => match rx.recv().await {
                Some(c) => c,
                None => break,
            },
        };
        match cmd {
            UiCmd::QuitSaving => { /* marker no-op (see sys note); real saves happen on terminate */ }
            UiCmd::Connect { url, username, password, api_key, remember } => {
                match connect_cmd(&url, &username, password, api_key, remember, &mut cfg, &main_tx).await {
                    Ok(c) => client = Some(c),
                    Err(e) => {
                        let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                    }
                }
            }
            UiCmd::LoadView { view_id, name, collection_type } => {
                if let Some(c) = &client {
                    let live = collection_type.as_deref() == Some("livetv") || view_id == "livetv";
                    let res = if live {
                        c.livetv_channels().await.map(|v| (name.clone(), v))
                    } else {
                        let sort = sort_from_popup(&cfg);
                        let mut q = crate::jellyfin::models::ItemQuery {
                            parent_id: Some(view_id.clone()),
                            ..Default::default()
                        };
                        q.sort_by = sort.0;
                        q.sort_order = sort.1;
                        match collection_type.as_deref() {
                            Some("movies") => q.types = Some("Movie".into()),
                            Some("tvshows") => q.types = Some("Series".into()),
                            Some("music") => q.types = Some("MusicAlbum".into()),
                            _ => {}
                        }
                        c.items(&q).await.map(|v| (name.clone(), v))
                    };
                    match res {
                        Ok((title, items)) => {
                            fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;
                            let _ = main_tx.send(MainMsg::ItemsLoaded {
                                title,
                                items,
                                can_go_back: false,
                            });
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
                        }
                    }
                }
            }
            UiCmd::Search { q } => {
                // Debounce: wait, then keep only the newest search.
                tokio::time::sleep(Duration::from_millis(250)).await;
                let mut latest = q;
                let mut held = Vec::new();
                while let Ok(c2) = rx.try_recv() {
                    match c2 {
                        UiCmd::Search { q } => latest = q,
                        other => held.push(other),
                    }
                }
                // Non-search commands drained above run after this search, in arrival order.
                backlog.extend(held);
                if let Some(c) = &client {
                    if !latest.trim().is_empty() {
                        let mut qy = crate::jellyfin::models::ItemQuery {
                            search: Some(latest.clone()),
                            ..Default::default()
                        };
                        qy.limit = 100;
                        match c.items(&qy).await {
                            Ok(items) => {
                                fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;
                                let _ = main_tx.send(MainMsg::ItemsLoaded {
                                    title: format!("Results for “{}”", latest),
                                    items,
                                    can_go_back: true,
                                });
                            }
                            Err(e) => {
                                let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
                            }
                        }
                    }
                }
            }
            UiCmd::Episodes { series_id, series_name } => {
                if let Some(c) = &client {
                    let mut q = crate::jellyfin::models::ItemQuery {
                        series_id: Some(series_id.clone()),
                        types: Some("Episode".into()),
                        ..Default::default()
                    };
                    q.sort_by = "ParentIndexNumber,IndexNumber".into();
                    q.sort_order = "Ascending".into();
                    match c.items(&q).await {
                        Ok(items) => {
                            fetch_posters(&c, &main_tx, &items, &poster_dir, http_poster.as_ref()).await;
                            let _ = main_tx.send(MainMsg::ItemsLoaded {
                                title: series_name,
                                items,
                                can_go_back: true,
                            });
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
                        }
                    }
                }
            }
            UiCmd::Tracks { album_id, album_name } => {
                if let Some(c) = &client {
                    let q = crate::jellyfin::models::ItemQuery {
                        parent_id: Some(album_id.clone()),
                        types: Some("Audio".into()),
                        ..Default::default()
                    };
                    match c.items(&q).await {
                        Ok(items) => {
                            let _ = main_tx.send(MainMsg::ItemsLoaded {
                                title: album_name,
                                items,
                                can_go_back: true,
                            });
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
                        }
                    }
                }
            }
            UiCmd::OpenDetail { item_id } => {
                if let Some(c) = &client {
                    match c.item_detail(&item_id).await {
                        Ok(d) => {
                            let (view, jobs) = plan_detail(c, &d);
                            let _ = main_tx.send(MainMsg::DetailLoaded(Box::new(view)));
                            fetch_art(jobs, &main_tx, &poster_dir, http_poster.as_ref());
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::DetailError(format!("{}", e)));
                        }
                    }
                }
            }
            UiCmd::Seasons { series_id } => {
                if let Some(c) = &client {
                    match c.seasons(&series_id).await {
                        Ok(seasons) => {
                            // Next Up only picks the starting season; without it the first one is shown.
                            let next = c.next_up(&series_id).await.ok().flatten();
                            let selected = default_season(&seasons, next.as_ref());
                            let seasons = seasons
                                .into_iter()
                                .map(|s| SeasonEntry { id: s.id, name: s.name })
                                .collect();
                            let _ = main_tx.send(MainMsg::SeasonsLoaded { series_id, seasons, selected });
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::SeasonsError { series_id, message: format!("{}", e) });
                        }
                    }
                }
            }
            UiCmd::SeasonEpisodes { series_id, season_id } => {
                if let Some(c) = &client {
                    match c.season_episodes(&series_id, &season_id).await {
                        Ok(eps) => {
                            let (rows, jobs) = plan_episodes(c, &eps);
                            let _ = main_tx.send(MainMsg::SeasonEpisodesLoaded { series_id, season_id, episodes: rows });
                            fetch_art(jobs, &main_tx, &poster_dir, http_poster.as_ref());
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::SeasonsError { series_id, message: format!("{}", e) });
                        }
                    }
                }
            }
            UiCmd::Play { item_id } => {
                if let Some(c) = &client {
                    match c.full_item(&item_id).await {
                        Ok(full) if full.item_type == "TvChannel" => {
                            play_channel(c, &cfg, &main_tx, &mut live_stream, full.id.clone(), full.name.clone()).await;
                        }
                        Ok(full) => {
                            let ms = full.media_sources.first().cloned();
                            let (method, _kbps) =
                                crate::transcode::decision::build_stream_plan(ms.as_ref(), cfg.settings.max_bitrate_kbps);
                            let resume = full
                                .user_data
                                .as_ref()
                                .map(|u| u.position_ticks)
                                .unwrap_or(0);
                            let resume_secs = crate::jellyfin::urls::secs_from_ticks(resume);
                            let played = full.user_data.as_ref().map(|u| u.played).unwrap_or(false);
                            let resume_secs = if played { 0.0 } else { resume_secs };
                            let url = match (&method, &ms) {
                                (
                                    crate::transcode::decision::PlaybackMethod::DirectPlay,
                                    Some(ms),
                                ) => crate::jellyfin::urls::build_direct_url(
                                    &c.base,
                                    &full.id,
                                    &ms.id,
                                    ms.container.as_deref(),
                                    &c.token,
                                ),
                                (crate::transcode::decision::PlaybackMethod::ServerHls, Some(ms)) => {
                                    crate::jellyfin::urls::build_hls_url(
                                        &c.base,
                                        &full.id,
                                        &ms.id,
                                        &c.token,
                                        cfg.settings.max_bitrate_kbps,
                                        &cfg.settings.subtitle_lang,
                                    )
                                }
                                _ => crate::jellyfin::urls::build_hls_url(
                                    &c.base,
                                    &full.id,
                                    &full.id,
                                    &c.token,
                                    cfg.settings.max_bitrate_kbps,
                                    &cfg.settings.subtitle_lang,
                                ),
                            };
                            let play_method = match method {
                                crate::transcode::decision::PlaybackMethod::DirectPlay => "DirectPlay",
                                crate::transcode::decision::PlaybackMethod::ServerHls => "Transcode",
                            };
                            c.report_start(&full.id, crate::jellyfin::urls::ticks_from_secs(resume_secs), play_method)
                                .await;
                            let mut hist = crate::config::load_history();
                            hist = crate::config::push_history(
                                hist,
                                crate::config::HistoryEntry {
                                    item_id: full.id.clone(),
                                    name: full.name.clone(),
                                    at_ms: now_ms(),
                                },
                            );
                            let _ = main_tx.send(MainMsg::PlaybackReady(PlaybackStart {
                                url,
                                item_id: full.id.clone(),
                                title: full.name.clone(),
                                subtitle: if full.item_type == "Audio" {
                                    let parts: Vec<&str> = [full.album_artist.as_deref(), full.album.as_deref()]
                                        .into_iter()
                                        .flatten()
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    if parts.is_empty() { None } else { Some(parts.join(" · ")) }
                                } else {
                                    full.series_name.clone()
                                },
                                resume_secs,
                                method,
                                run_time_secs: crate::jellyfin::urls::secs_from_ticks(
                                    full.run_time_ticks.unwrap_or(0),
                                ),
                                image_key: Some(crate::ui::library::poster_key(&full.id)),
                                media_kind: if full.item_type == "Audio" {
                                    crate::app::messages::MediaKind::Audio
                                } else {
                                    crate::app::messages::MediaKind::Video
                                },
                            }));
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
                        }
                    }
                }
            }
            UiCmd::PlayChannel { channel_id, name } => {
                if let Some(c) = &client {
                    play_channel(c, &cfg, &main_tx, &mut live_stream, channel_id, name).await;
                }
            }
            UiCmd::Progress { item_id, ticks, paused } => {
                if let Some(c) = &client {
                    c.report_progress(&item_id, ticks, paused).await;
                }
            }
            UiCmd::Stopped { item_id, ticks } => {
                if let Some(c) = &client {
                    if let Some(id) = live_stream.take() {
                        c.close_live_stream(&id).await;
                    }
                    c.report_stopped(&item_id, ticks).await;
                }
            }
            UiCmd::MarkPlayed { item_id, played } => {
                if let Some(c) = &client {
                    if let Err(e) = c.set_played(&item_id, played).await {
                        tracing::warn!("set_played failed: {}", e);
                    }
                }
            }
            UiCmd::SeerrSearch { q } => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let mut latest = q;
                let mut held = Vec::new();
                while let Ok(c2) = rx.try_recv() {
                    match c2 {
                        UiCmd::SeerrSearch { q } => latest = q,
                        other => held.push(other),
                    }
                }
                backlog.extend(held);
                match seerr_client(&cfg, &mut seerr) {
                    Err(m) => {
                        let _ = main_tx.send(MainMsg::SeerrError(m));
                    }
                    Ok(_) if latest.trim().is_empty() => {
                        let _ = main_tx.send(MainMsg::SeerrResults(Vec::new()));
                    }
                    Ok(c) => match c.search(&latest, 1).await {
                        Ok(page) => {
                            let paths = page.results.iter().filter_map(|r| r.poster_path.clone()).collect();
                            let _ = main_tx.send(MainMsg::SeerrResults(page.results));
                            fetch_seerr_posters(&c, &main_tx, paths, &poster_dir);
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::SeerrError(e.to_string()));
                        }
                    },
                }
            }
            UiCmd::SeerrRequest { media_type, tmdb_id, title } => match seerr_client(&cfg, &mut seerr) {
                Err(m) => {
                    let _ = main_tx.send(MainMsg::SeerrError(m));
                }
                Ok(c) => match c.request(&media_type, tmdb_id).await {
                    Ok(_) => {
                        let _ = main_tx.send(MainMsg::SeerrRequested { title });
                    }
                    Err(SeerrError::AlreadyRequested(_)) => {
                        let _ = main_tx.send(MainMsg::SeerrError(format!("“{}” has already been requested.", title)));
                    }
                    Err(e) => {
                        let _ = main_tx.send(MainMsg::SeerrError(e.to_string()));
                    }
                },
            },
            UiCmd::SeerrRequests => match seerr_client(&cfg, &mut seerr) {
                Err(m) => {
                    let _ = main_tx.send(MainMsg::SeerrError(m));
                }
                Ok(c) => match c.requests(20, 0).await {
                    Ok(v) => {
                        let paths = v.iter().filter_map(|r| r.poster_path.clone()).collect();
                        let _ = main_tx.send(MainMsg::SeerrRequestList(v));
                        fetch_seerr_posters(&c, &main_tx, paths, &poster_dir);
                    }
                    Err(e) => {
                        let _ = main_tx.send(MainMsg::SeerrError(e.to_string()));
                    }
                },
            },
            UiCmd::SaveSeerr { enabled, url, username, password, api_key } => {
                seerr = None;
                let msg = save_seerr(&mut cfg, enabled, &url, &username, password, api_key).await;
                let _ = main_tx.send(msg);
            }
            UiCmd::SaveLibraries { order, hidden } => {
                cfg.library_order = order;
                cfg.hidden_libraries = hidden;
                crate::config::save_config(&cfg);
            }
            UiCmd::SaveSettings { settings, reconnect, server_url } => {
                seerr = None;
                // Settings are saved before the URL resolves on purpose: a mistyped
                // server host must not discard the other edits. If the URL fails to
                // resolve, the old URL stays in effect, the error is reported, and
                // no reconnect is attempted (the `continue` below).
                cfg.settings = settings;
                crate::config::save_config(&cfg);
                if let Some(raw) = server_url {
                    match crate::jellyfin::client::JellyfinClient::resolve(&raw).await {
                        Ok((base, _)) => {
                            cfg.server_url = Some(base.clone());
                            crate::config::save_config(&cfg);
                            let _ = main_tx.send(MainMsg::ServerUrl(base));
                        }
                        Err(e) => {
                            let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                            continue;
                        }
                    }
                }
                if reconnect {
                    if let Some(url) = cfg.server_url.clone() {
                        if let Some(token) = crate::config::load_token(&url) {
                            let uid = cfg.user_id.clone().unwrap_or_default();
                            let name = cfg.username.clone().unwrap_or_else(|| uid.clone());
                            match crate::jellyfin::client::JellyfinClient::new(
                                &url,
                                token.clone(),
                                uid,
                                &cfg.device_id,
                            ) {
                                Ok(c) => match connect_flow(&c, &name, &main_tx).await {
                                    Ok(()) => client = Some(c),
                                    Err(e) => {
                                        let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                                    }
                                },
                                Err(e) => {
                                    let _ = main_tx.send(MainMsg::ConnectError(format!("{}", e)));
                                }
                            }
                        }
                    }
                }
            }
            UiCmd::SignOut => {
                if let Some(url) = &cfg.server_url {
                    crate::config::delete_token(url);
                }
                cfg.server_url = None;
                cfg.username = None;
                cfg.user_id = None;
                client = None; // no further server calls after sign-out
                crate::config::save_config(&cfg);
                let _ = main_tx.send(MainMsg::SignedOut);
            }
            UiCmd::ProbeServer => {
                if let Some(c) = &client {
                    match crate::jellyfin::client::JellyfinClient::public_info(&c.base).await {
                        Ok(_) => {
                            let _ = main_tx.send(MainMsg::ServerUp);
                        }
                        Err(_) => {
                            let _ = main_tx.send(MainMsg::ServerDown);
                        }
                    }
                } else {
                    let _ = main_tx.send(MainMsg::ServerDown);
                }
            }
        }
    }
}

/// Live TV: PlaybackInfo with an HLS-capable device profile, then play the
/// returned TranscodingUrl (AVPlayer cannot play a raw MPEG-TS stream).
async fn play_channel(
    c: &JellyfinClient,
    cfg: &Config,
    main_tx: &UnboundedSender<MainMsg>,
    live_stream: &mut Option<String>,
    channel_id: String,
    name: String,
) {
    // Release any previously opened live stream first.
    if let Some(id) = live_stream.take() {
        c.close_live_stream(&id).await;
    }
    match c.playback_info(&channel_id, cfg.settings.max_bitrate_kbps * 1000).await {
        Ok(info) => {
            let ms = info.media_sources.into_iter().next();
            match ms.as_ref().and_then(|m| m.transcoding_url.clone()) {
                Some(rel) => {
                    *live_stream = ms.as_ref().and_then(|m| m.live_stream_id.clone());
                    let url = c.absolute_media_url(&rel);
                    c.report_start(&channel_id, 0, "Transcode").await;
                    let _ = main_tx.send(MainMsg::PlaybackReady(PlaybackStart {
                        url,
                        item_id: channel_id.clone(),
                        title: name,
                        subtitle: Some("Live".into()),
                        resume_secs: 0.0,
                        method: crate::transcode::decision::PlaybackMethod::ServerHls,
                        run_time_secs: 0.0,
                        image_key: Some(crate::ui::library::poster_key(&channel_id)),
                        media_kind: crate::app::messages::MediaKind::Video,
                    }));
                }
                None => {
                    if let Some(id) = ms.and_then(|m| m.live_stream_id) {
                        c.close_live_stream(&id).await;
                    }
                    let _ = main_tx.send(MainMsg::PlaybackError(format!("“{}” is not available right now.", name)));
                }
            }
        }
        Err(e) => {
            let _ = main_tx.send(MainMsg::PlaybackError(format!("{}", e)));
        }
    }
}

/// Build (or reuse) the Seerr client. Err = user-facing reason; no network I/O.
fn seerr_client(cfg: &Config, cache: &mut Option<SeerrClient>) -> Result<SeerrClient, String> {
    if !cfg.settings.seerr_enabled {
        return Err("Seerr is turned off. Enable it in Settings to search and request.".into());
    }
    let raw = cfg.settings.seerr_url.trim();
    let user = cfg.settings.seerr_username.trim().to_string();
    if raw.is_empty() {
        return Err("Seerr URL is not set. Add it in Settings.".into());
    }
    let url = crate::jellyfin::urls::normalize_base_url(raw).map_err(|e| format!("Invalid Seerr URL: {}", e))?;
    if let Some(c) = cache {
        if c.base == url {
            return Ok(c.clone());
        }
    }
    let c = if user.is_empty() {
        let key = crate::config::load_seerr_key(&url)
            .ok_or_else(|| "No Seerr API key saved. Add it in Settings.".to_string())?;
        SeerrClient::new(&url, &key)
    } else {
        let pw = crate::config::load_seerr_login(&url)
            .ok_or_else(|| "No Seerr password saved. Add it in Settings.".to_string())?;
        SeerrClient::with_login(&url, &user, &pw)
    }
    .map_err(|e| e.to_string())?;
    *cache = Some(c.clone());
    Ok(c)
}

/// Handle SaveSeerr: validate before persisting anything when enabling.
async fn save_seerr(
    cfg: &mut Config,
    enabled: bool,
    url: &str,
    username: &str,
    password: Option<String>,
    api_key: Option<String>,
) -> MainMsg {
    let fail = |m: String| MainMsg::SeerrStatus { ok: false, message: m };
    let key_in = api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let pw_in = password.filter(|p| !p.is_empty());
    let user = username.trim().to_string();
    let use_login = !user.is_empty();
    let trimmed = url.trim();
    if !enabled {
        if !trimmed.is_empty() {
            // Not probed while disabled: keep the address as typed (scheme-less
            // stays scheme-less) and resolve it when Seerr is turned on.
            let n = crate::jellyfin::urls::parse_server_input(trimmed)
                .map(|p| match p.scheme {
                    Some(sc) => format!("{}://{}", sc, p.rest),
                    None => p.rest,
                })
                .unwrap_or_else(|_| trimmed.to_string());
            if let Some(k) = &key_in {
                crate::config::store_seerr_key(&n, k);
            }
            if let Some(p) = &pw_in {
                crate::config::store_seerr_login(&n, p);
            }
            cfg.settings.seerr_url = n;
            cfg.settings.seerr_username = user;
        }
        cfg.settings.seerr_enabled = false;
        crate::config::save_config(cfg);
        return MainMsg::SeerrStatus { ok: true, message: "Seerr is turned off.".into() };
    }
    // Check for a credential before any network I/O. Secrets are keyed by the
    // resolved address; ones saved while Seerr was off are keyed by the typed one.
    let typed = crate::jellyfin::urls::bare_url(trimmed);
    let candidates = match crate::jellyfin::urls::candidate_bases(trimmed) {
        Ok(c) => c,
        Err(e) => return fail(format!("Invalid Seerr URL: {}", e)),
    };
    let keys_for = || candidates.iter().map(String::as_str).chain(std::iter::once(typed.as_str()));
    let stored_pw = keys_for().find_map(crate::config::load_seerr_login);
    let stored_key = keys_for().find_map(crate::config::load_seerr_key);
    if use_login && pw_in.is_none() && stored_pw.is_none() {
        return fail("Enter your Seerr password (the one you use to sign in with your Jellyfin account).".into());
    }
    if !use_login && key_in.is_none() && stored_key.is_none() {
        return fail("Enter your Seerr API key (Seerr > Settings > General), or sign in with your Jellyfin account.".into());
    }
    let n = match SeerrClient::resolve(trimmed).await {
        Ok((base, _)) => base,
        Err(e) => return fail(e.to_string()),
    };
    let client = if use_login {
        SeerrClient::with_login(&n, &user, &pw_in.clone().or(stored_pw).unwrap_or_default())
    } else {
        SeerrClient::new(&n, &key_in.clone().or(stored_key).unwrap_or_default())
    };
    let client = match client {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };
    match client.verify().await {
        Ok((st, me)) => {
            if use_login {
                if let Some(p) = &pw_in {
                    crate::config::store_seerr_login(&n, p);
                }
            } else if let Some(k) = &key_in {
                crate::config::store_seerr_key(&n, k);
            }
            cfg.settings.seerr_enabled = true;
            cfg.settings.seerr_url = n;
            cfg.settings.seerr_username = user;
            crate::config::save_config(cfg);
            MainMsg::SeerrStatus {
                ok: true,
                message: format!("Connected to Seerr {} as {}.", st.version, me.display_name),
            }
        }
        Err(e) => fail(e.to_string()),
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn sort_from_popup(cfg: &Config) -> (String, String) {
    // The popup index is not directly visible to the worker; settings could
    // carry it — for now we honor the UI default (SortName) and read the
    // popup choice through the settings struct when modularized.
    let _ = cfg;
    ("SortName".into(), "Ascending".into())
}

pub(crate) async fn connect_cmd(
    url: &str,
    username: &str,
    password: Option<String>,
    api_key: Option<String>,
    remember: bool,
    cfg: &mut Config,
    main_tx: &UnboundedSender<MainMsg>,
) -> Result<JellyfinClient, crate::jellyfin::client::JFError> {
    use crate::jellyfin::client::JFError;
    // Step 1: verify it's Jellyfin.
    let (url, info) = JellyfinClient::resolve(url).await?;
    let url = url.as_str();
    tracing::info!(server_version = ?info.version, "server identified");

    let (token, user_id, user_name) = if let Some(pw) = password {
        let (resp, base) = JellyfinClient::authenticate(url, username, &pw, &cfg.device_id).await?;
        (resp.access_token, resp.user.id, resp.user.name)
    } else if let Some(k) = api_key {
        // API-key path: validate by hitting an authed endpoint.
        let probe = JellyfinClient::new(url, k.clone(), String::new(), &cfg.device_id)?;
        let uid = probe.first_user_id().await?;
        (k, uid, username.to_string())
    } else {
        return Err(JFError::Auth("provide a password or an API key".into()));
    };

    let c = JellyfinClient::new(url, token.clone(), user_id.clone(), &cfg.device_id)?;
    connect_flow(&c, &user_name, main_tx).await?;
    // Persist only after a fully successful connect. The choice itself is saved
    // either way, so the next launch knows whether to sign in automatically.
    let previous = cfg.server_url.take();
    cfg.remember_me = remember;
    if remember {
        cfg.server_url = Some(normalize(url));
        cfg.username = Some(user_name.clone());
        cfg.user_id = Some(user_id.clone());
        crate::config::store_token(&normalize(url), &token);
    } else {
        // Remember me off: no token and no session identity survive this sign-in.
        crate::config::delete_token(&normalize(url));
        if let Some(prev) = previous {
            crate::config::delete_token(&prev);
        }
        cfg.username = None;
        cfg.user_id = None;
    }
    crate::config::save_config(cfg);
    // The UI keeps its own copy of the config; without this, Settings shows a
    // stale or empty Server URL until relaunch, and the next save looks changed.
    let _ = main_tx.send(MainMsg::ServerUrl(normalize(url)));
    Ok(c)
}

pub(crate) fn normalize(u: &str) -> String {
    crate::jellyfin::urls::normalize_base_url(u).unwrap_or_else(|_| u.to_string())
}

pub(crate) async fn connect_flow(
    c: &JellyfinClient,
    user_name: &str,
    main_tx: &UnboundedSender<MainMsg>,
) -> Result<(), crate::jellyfin::client::JFError> {
    let views = c.views().await?;
    let mut v: Vec<(String, String, Option<String>)> = views
        .into_iter()
        .map(|x| (x.id, x.name, x.collection_type))
        .collect();
    // Probe live TV (cheap, 1 item) — sidebar shows the section only if present.
    let has_live = c.livetv_channels().await.map(|i| !i.is_empty()).unwrap_or(false);
    // Jellyfin 10.9+/12 already lists Live TV as a user view; avoid a duplicate.
    let already = v.iter().any(|x| x.2.as_deref() == Some("livetv"));
    if has_live && !already {
        v.push(("livetv".into(), "Live TV".into(), Some("livetv".into())));
    }
    let _ = main_tx.send(MainMsg::Connected {
        user_name: user_name.to_string(),
        views: v,
        has_live_tv: has_live,
    });
    Ok(())
}

pub(crate) async fn fetch_posters(
    c: &JellyfinClient,
    main_tx: &UnboundedSender<MainMsg>,
    items: &[crate::jellyfin::models::ListItem],
    poster_dir: &std::path::Path,
    http: Option<&reqwest::Client>,
) {
    // Spawn a bounded set of concurrent poster fetches (first 200 items).
    let jobs: Vec<(u64, String)> = items
        .iter()
        .take(200)
        .filter(|it| it.has_image())
        .map(|it| (sys_key(&it.id), build_image_url(&c.base, &it.id, &c.token, 400)))
        .collect();
    fetch_art(jobs, main_tx, poster_dir, http);
}

/// Download artwork jobs `(key, url)` concurrently, through the disk cache in
/// `poster_dir`. Each image arrives as `PosterReady { key, bytes }`.
pub(crate) fn fetch_art(
    jobs: Vec<(u64, String)>,
    main_tx: &UnboundedSender<MainMsg>,
    poster_dir: &std::path::Path,
    http: Option<&reqwest::Client>,
) {
    let Some(http) = http.cloned() else {
        return;
    };
    for (key, url) in jobs {
        let tx = main_tx.clone();
        let dir = poster_dir.to_path_buf();
        let http = http.clone();
        tokio::spawn(async move {
            let path = dir.join(format!("{:016x}.img", key));
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => Some(b),
                Err(_) => match http.get(url).send().await {
                    Ok(r) if r.status().is_success() => match r.bytes().await {
                        Ok(b) => {
                            let _ = tokio::fs::write(&path, &b).await;
                            Some(b.to_vec())
                        }
                        Err(_) => None,
                    },
                    _ => None,
                },
            };
            if let Some(b) = bytes {
                let _ = tx.send(MainMsg::PosterReady { key, bytes: b });
            }
        });
    }
}

/// Cast and crew shown on the detail page (the first this many of `People`).
const MAX_PEOPLE: usize = 30;

/// Turn a detail response into the page's view plus the artwork to fetch
/// as `(key, url)` jobs. Artwork is requested only where the server has it.
pub(crate) fn plan_detail(c: &JellyfinClient, d: &DetailItem) -> (DetailView, Vec<(u64, String)>) {
    let mut jobs = Vec::new();
    let poster_key = d.image_tags.as_ref().and_then(|t| t.primary.as_ref()).map(|_| sys_key(&d.id));
    if let Some(k) = poster_key {
        jobs.push((k, build_image_url(&c.base, &d.id, &c.token, 400)));
    }
    let backdrop_key = d.backdrop_tags.first().map(|_| sys_key(&format!("{}#backdrop", d.id)));
    if let Some(k) = backdrop_key {
        jobs.push((k, build_art_url(&c.base, &d.id, "Backdrop", 1280, &c.token)));
    }
    let people = d
        .people
        .iter()
        .take(MAX_PEOPLE)
        .map(|p: &Person| {
            let image_key = match p.image_tag.as_ref() {
                Some(_) if !p.id.is_empty() => {
                    let k = sys_key(&p.id);
                    jobs.push((k, build_art_url(&c.base, &p.id, "Primary", 200, &c.token)));
                    Some(k)
                }
                _ => None,
            };
            let role = p
                .role
                .clone()
                .filter(|r| !r.trim().is_empty())
                .or_else(|| p.kind.clone().filter(|k| !k.trim().is_empty()));
            DetailPerson { name: p.name.clone(), role, image_key }
        })
        .collect();
    let played = d.user_data.as_ref().map(|u| u.played).unwrap_or(false);
    let position = d.user_data.as_ref().map(|u| u.position_ticks).unwrap_or(0);
    let view = DetailView {
        id: d.id.clone(),
        name: d.name.clone(),
        item_type: d.item_type.clone(),
        year: d.year,
        official_rating: d.official_rating.clone().filter(|r| !r.trim().is_empty()),
        community_rating: d.community_rating,
        run_time_ticks: d.run_time_ticks,
        genres: d.genres.clone(),
        overview: d.overview.clone().filter(|o| !o.trim().is_empty()),
        played,
        resume: !played && position > 0,
        poster_key,
        backdrop_key,
        people,
    };
    (view, jobs)
}

/// Index of the season that holds `next_up` (matched by season id, then by season number),
/// or the first season when there is no Next Up episode or no matching season.
pub(crate) fn default_season(seasons: &[Season], next_up: Option<&Episode>) -> usize {
    let Some(next) = next_up else { return 0 };
    if let Some(id) = next.season_id.as_deref() {
        if let Some(i) = seasons.iter().position(|s| s.id == id) {
            return i;
        }
    }
    if let Some(n) = next.parent_index {
        if let Some(i) = seasons.iter().position(|s| s.index == Some(n)) {
            return i;
        }
    }
    0
}

/// Episode rows for the series page plus their thumbnail jobs `(key, url)`.
pub(crate) fn plan_episodes(c: &JellyfinClient, eps: &[Episode]) -> (Vec<EpisodeRow>, Vec<(u64, String)>) {
    let mut jobs = Vec::new();
    let rows = eps
        .iter()
        .map(|e| {
            let image_key = e.has_image().then(|| sys_key(&e.id));
            if image_key.is_some() {
                jobs.push((sys_key(&e.id), build_image_url(&c.base, &e.id, &c.token, 400)));
            }
            let ud = e.user_data.clone().unwrap_or_default();
            EpisodeRow {
                id: e.id.clone(),
                number: e.number,
                name: e.name.clone(),
                overview: e.overview.clone().filter(|o| !o.trim().is_empty()),
                run_time_ticks: e.run_time_ticks,
                played: ud.played,
                position_ticks: ud.position_ticks,
                image_key,
            }
        })
        .collect();
    (rows, jobs)
}

/// Fetch thumbnails for Seerr results through the user's Seerr server (never TMDB),
/// with the same on-disk cache as library posters. Each arrives as `SeerrPoster`.
pub(crate) fn fetch_seerr_posters(
    c: &SeerrClient,
    main_tx: &UnboundedSender<MainMsg>,
    mut paths: Vec<String>,
    poster_dir: &std::path::Path,
) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    for path in paths.into_iter().take(40) {
        let (c, tx, dir) = (c.clone(), main_tx.clone(), poster_dir.to_path_buf());
        tokio::spawn(async move {
            let file = dir.join(format!("seerr-{:016x}.img", sys_key(&path)));
            let bytes = match tokio::fs::read(&file).await {
                Ok(b) if !b.is_empty() => Some(b),
                _ => match c.poster(&path).await {
                    Ok(b) => {
                        let _ = tokio::fs::write(&file, &b).await;
                        Some(b)
                    }
                    Err(_) => None,
                },
            };
            if let Some(bytes) = bytes {
                let _ = tx.send(MainMsg::SeerrPoster { path, bytes });
            }
        });
    }
}

pub(crate) fn sys_key(item_id: &str) -> u64 {
    // Matches crate::sys::poster_key (same DefaultHasher on the same binary).
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    item_id.hash(&mut h);
    h.finish()
}

// ============================================================================
// Headless end-to-end tests: worker + mock Jellyfin HTTP server
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_env::TestEnv;
    use crate::transcode::decision::PlaybackMethod;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ---------------------------------------------------------------- mock
    #[derive(Clone, Debug)]
    struct Req {
        method: String,
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
        body: String,
    }

    struct Resp {
        status: u16,
        body: Vec<u8>,
    }
    fn json(v: serde_json::Value) -> Resp {
        Resp { status: 200, body: v.to_string().into_bytes() }
    }
    fn status(code: u16) -> Resp {
        Resp { status: code, body: Vec::new() }
    }

    #[derive(Default)]
    struct MockCfg {
        live: bool,
        images: bool,
        items_status: Option<u16>,
        not_jellyfin: bool,
    }

    struct Mock {
        addr: std::net::SocketAddr,
        log: Arc<Mutex<Vec<Req>>>,
        cfg: Arc<Mutex<MockCfg>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Mock {
        fn url(&self) -> String {
            format!("http://{}", self.addr)
        }
        fn reqs(&self) -> Vec<Req> {
            self.log.lock().unwrap().clone()
        }
        fn find(&self, method: &str, path_prefix: &str) -> Vec<Req> {
            self.reqs()
                .into_iter()
                .filter(|r| r.method == method && r.path.starts_with(path_prefix))
                .collect()
        }
        fn stop(&self) {
            self.task.abort();
        }
    }

    fn pct_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    async fn start_mock() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log: Arc<Mutex<Vec<Req>>> = Default::default();
        let cfg: Arc<Mutex<MockCfg>> = Default::default();
        let (l2, c2) = (log.clone(), cfg.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (log, cfg) = (l2.clone(), c2.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let head_end = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let mut lines = head.split("\r\n");
                    let mut first = lines.next().unwrap().split(' ');
                    let method = first.next().unwrap().to_string();
                    let target = first.next().unwrap().to_string();
                    let mut headers = HashMap::new();
                    for l in lines {
                        if let Some((k, v)) = l.split_once(':') {
                            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                        }
                    }
                    let clen: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                    while buf.len() < head_end + clen {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                    let (path, q) = target.split_once('?').unwrap_or((target.as_str(), ""));
                    let query = q
                        .split('&')
                        .filter(|p| !p.is_empty())
                        .map(|p| {
                            let (k, v) = p.split_once('=').unwrap_or((p, ""));
                            (pct_decode(k), pct_decode(v))
                        })
                        .collect();
                    let req = Req { method, path: pct_decode(path), query, headers, body };
                    log.lock().unwrap().push(req.clone());
                    let resp = route(&req, &cfg.lock().unwrap());
                    let reason = if resp.status < 300 { "OK" } else { "ERR" };
                    let head = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp.status,
                        reason,
                        resp.body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&resp.body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Mock { addr, log, cfg, task }
    }

    fn item(id: &str, name: &str, ty: &str, images: bool) -> serde_json::Value {
        let mut v = serde_json::json!({
            "Id": id, "Name": name, "Type": ty, "ProductionYear": 2020,
            "RunTimeTicks": 72_000_000_000i64,
            "UserData": {"Played": false, "PlaybackPositionTicks": 0, "PlayCount": 0},
            "ServerId": "srv"
        });
        if images {
            v["ImageTags"] = serde_json::json!({"Primary": "tag1"});
            v["PrimaryImageAspectRatio"] = serde_json::json!(0.667);
        }
        v
    }

    fn media(container: &str, vcodec: &str, acodec: &str, bitrate: u64) -> serde_json::Value {
        serde_json::json!([{
            "Id": "ms-1", "Container": container, "Bitrate": bitrate,
            "MediaStreams": [
                {"Type": "Video", "Codec": vcodec, "Index": 0},
                {"Type": "Audio", "Codec": acodec, "Language": "eng", "Index": 1}
            ]
        }])
    }

    fn route(r: &Req, c: &MockCfg) -> Resp {
        let seg: Vec<&str> = r.path.trim_matches('/').split('/').collect();
        let token = r.headers.get("x-emby-token").map(String::as_str);
        let authed = token == Some("tok123")
            || token == Some("apikey-1")
            || r.query.get("api_key").map(String::as_str) == Some("tok123");
        match (r.method.as_str(), seg.as_slice()) {
            ("GET", ["System", "Info", "Public"]) => {
                if c.not_jellyfin {
                    json(serde_json::json!({"hello": "world"}))
                } else {
                    json(serde_json::json!({"ProductName": "Jellyfin Server", "Version": "10.9.11", "Id": "srvid", "ServerName": "mock"}))
                }
            }
            ("POST", ["Users", "AuthenticateByName"]) => {
                let b: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                if b["Username"] == "alice" && b["Pw"] == "pw" {
                    json(serde_json::json!({
                        "AccessToken": "tok123", "ServerId": "srvid",
                        "User": {"Id": "u1", "Name": "Alice Display", "HasPassword": true}
                    }))
                } else {
                    status(401)
                }
            }
            ("GET", ["Users"]) => {
                if token == Some("apikey-1") {
                    json(serde_json::json!([{"Id": "u-first", "Name": "admin"}, {"Id": "u-2", "Name": "bob"}]))
                } else {
                    status(401)
                }
            }
            _ if !authed => status(401),
            ("GET", ["Users", _, "Views"]) => json(serde_json::json!({"Items": [
                {"Id": "v-mov", "Name": "Movies", "CollectionType": "movies"},
                {"Id": "v-tv", "Name": "Shows", "CollectionType": "tvshows"},
                {"Id": "v-mus", "Name": "Music", "CollectionType": "music"},
            ], "TotalRecordCount": 3})),
            ("POST", ["Items", id, "PlaybackInfo"]) => json(serde_json::json!({
                "PlaySessionId": "ps1",
                "MediaSources": [{
                    "Id": "lms-1", "Container": "hls", "LiveStreamId": "ls-1",
                    "SupportsTranscoding": true,
                    "TranscodingUrl": format!("/videos/{}/master.m3u8?MediaSourceId=lms-1&ApiKey=tok123&LiveStreamId=ls-1", id),
                    "MediaStreams": []
                }]
            })),
            ("POST", ["LiveStreams", "Close"]) => json(serde_json::json!({})),
            ("GET", ["LiveTv", "Channels"]) => {
                if c.live {
                    json(serde_json::json!({"Items": [item("ch1", "Channel One", "TvChannel", false)]}))
                } else {
                    json(serde_json::json!({"Items": [], "TotalRecordCount": 0}))
                }
            }
            ("GET", ["Users", _, "Items"]) => {
                if let Some(s) = c.items_status {
                    return status(s);
                }
                let img = c.images;
                if let Some(term) = r.query.get("SearchTerm") {
                    return json(serde_json::json!({"Items": [item("s1", &format!("Found {}", term), "Movie", img)]}));
                }
                match r.query.get("IncludeItemTypes").map(String::as_str) {
                    Some("Movie") => json(serde_json::json!({"Items": [item("m1", "Movie One", "Movie", img), item("m2", "Movie Two", "Movie", img)]})),
                    Some("Series") => json(serde_json::json!({"Items": [item("sr1", "Show One", "Series", img)]})),
                    Some("MusicAlbum") => json(serde_json::json!({"Items": [item("al1", "Album One", "MusicAlbum", img)]})),
                    Some("Episode") => json(serde_json::json!({"Items": [item("e1", "Pilot", "Episode", false), item("e2", "Second", "Episode", false)]})),
                    Some("Audio") => json(serde_json::json!({"Items": [item("t1", "Track One", "Audio", false)]})),
                    _ => json(serde_json::json!({"Items": [item("x1", "Anything", "Folder", img)]})),
                }
            }
            ("GET", ["Users", _, "Items", id]) => {
                let mut v = item(id, "Some Movie", "Movie", false);
                v["MediaSources"] = media("mp4", "h264", "aac", 4_000_000);
                match *id {
                    "missing" => return status(404),
                    "expired" => return status(401),
                    "mkv" => v["MediaSources"] = media("mkv", "h264", "aac", 4_000_000),
                    "hibr" => v["MediaSources"] = media("mp4", "h264", "aac", 20_000_000),
                    "dts" => v["MediaSources"] = media("mp4", "h264", "dts", 4_000_000),
                    "nosrc" => v["MediaSources"] = serde_json::json!([]),
                    "resume" => v["UserData"] = serde_json::json!({"Played": false, "PlaybackPositionTicks": 6_000_000_000i64}),
                    "played" => v["UserData"] = serde_json::json!({"Played": true, "PlaybackPositionTicks": 6_000_000_000i64}),
                    "ep1" => {
                        v["Type"] = "Episode".into();
                        v["SeriesName"] = "Show One".into();
                    }
                    _ => {}
                }
                if id.starts_with("det-") && *id != "det-bare" {
                    v["Name"] = "Detail Movie".into();
                    v["Overview"] = "A detailed overview.".into();
                    v["Genres"] = serde_json::json!(["Drama", "Thriller"]);
                    v["CommunityRating"] = serde_json::json!(7.8);
                    v["OfficialRating"] = "PG-13".into();
                    v["ImageTags"] = serde_json::json!({"Primary": "tag1"});
                    v["BackdropImageTags"] = serde_json::json!(["bd1"]);
                    v["People"] = serde_json::json!([
                        {"Id": "p-actor", "Name": "Ada Actor", "Role": "Lead", "Type": "Actor", "PrimaryImageTag": "pt"},
                        {"Id": "p-dir", "Name": "Dan Director", "Role": "", "Type": "Director"},
                        {"Id": "p-noimg", "Name": "No Image", "Role": "Extra", "Type": "Actor"}
                    ]);
                    match *id {
                        "det-series" => v["Type"] = "Series".into(),
                        "det-resume" => v["UserData"] = serde_json::json!({"Played": false, "PlaybackPositionTicks": 6_000_000_000i64}),
                        "det-played" => v["UserData"] = serde_json::json!({"Played": true, "PlaybackPositionTicks": 0}),
                        _ => {}
                    }
                }
                json(v)
            }
            ("GET", ["Shows", "NextUp"]) => {
                let items = match r.query.get("SeriesId").map(String::as_str) {
                    Some("sr1") => vec![serde_json::json!({"Id": "nx1", "Name": "Next", "Type": "Episode", "IndexNumber": 3, "ParentIndexNumber": 2, "SeasonId": "sea-b"})],
                    Some("sr2") => vec![serde_json::json!({"Id": "nx2", "Name": "Next", "Type": "Episode", "IndexNumber": 1, "ParentIndexNumber": 2})],
                    Some("sr3") => return status(500),
                    _ => vec![],
                };
                json(serde_json::json!({"Items": items}))
            }
            ("GET", ["Shows", sid, "Seasons"]) => match *sid {
                "noseasons" => json(serde_json::json!({"Items": []})),
                "seasons-500" => status(500),
                _ => json(serde_json::json!({"Items": [
                    {"Id": "sea-a", "Name": "Season 1", "IndexNumber": 1, "Type": "Season"},
                    {"Id": "sea-b", "Name": "Season 2", "IndexNumber": 2, "Type": "Season"}
                ]})),
            },
            ("GET", ["Shows", _, "Episodes"]) => {
                let ep = |id: &str, n: u32, season: &str, played: bool, pos: i64, image: bool| {
                    let mut v = serde_json::json!({
                        "Id": id, "Name": format!("Episode {}", n), "Type": "Episode", "IndexNumber": n,
                        "ParentIndexNumber": 1, "SeasonId": season, "Overview": format!("About episode {}.", n),
                        "RunTimeTicks": 27_000_000_000i64,
                        "UserData": {"Played": played, "PlaybackPositionTicks": pos, "PlayCount": 0}
                    });
                    if image {
                        v["ImageTags"] = serde_json::json!({"Primary": "tag1"});
                    }
                    v
                };
                match r.query.get("seasonId").map(String::as_str) {
                    Some("sea-a") => json(serde_json::json!({"Items": [ep("a1", 1, "sea-a", false, 0, false), ep("a2", 2, "sea-a", false, 0, false)]})),
                    Some("sea-b") => json(serde_json::json!({"Items": [
                        ep("b1", 1, "sea-b", true, 0, false),
                        ep("b2", 2, "sea-b", false, 6_000_000_000, true),
                        ep("b3", 3, "sea-b", false, 0, false)
                    ]})),
                    Some("sea-empty") => json(serde_json::json!({"Items": []})),
                    _ => status(500),
                }
            }
            ("GET", ["Items", _, "Images", _]) => Resp { status: 200, body: vec![0x89, b'P', b'N', b'G', 1, 2, 3] },
            ("POST", ["Sessions", "Playing", ..]) => status(204),
            ("POST", ["Users", _, "PlayedItems", id]) | ("DELETE", ["Users", _, "PlayedItems", id]) => {
                if *id == "boom" {
                    status(500)
                } else {
                    json(serde_json::json!({"Played": true}))
                }
            }
            _ => status(404),
        }
    }

    // ------------------------------------------------------------- harness
    struct H {
        tx: UnboundedSender<UiCmd>,
        rx: UnboundedReceiver<MainMsg>,
        task: tokio::task::JoinHandle<()>,
    }
    impl H {
        fn start(cfg: Config) -> H {
            let (tx, crx) = tokio::sync::mpsc::unbounded_channel();
            let (mtx, rx) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(worker(crx, mtx, cfg));
            H { tx, rx, task }
        }
        fn send(&self, c: UiCmd) {
            self.tx.send(c).unwrap();
        }
        async fn next_raw(&mut self) -> MainMsg {
            tokio::time::timeout(Duration::from_secs(10), self.rx.recv())
                .await
                .expect("timed out waiting for MainMsg")
                .expect("main channel closed")
        }
        /// Next message, ignoring asynchronous Seerr thumbnail deliveries and the
        /// ServerUrl side-notification that follows a successful connect (tests
        /// that care about it read it with `next_raw`).
        async fn next(&mut self) -> MainMsg {
            loop {
                match self.next_raw().await {
                    MainMsg::SeerrPoster { .. } | MainMsg::ServerUrl(_) => continue,
                    m => return m,
                }
            }
        }
        /// Next message, skipping asynchronous PosterReady notifications.
        async fn next_nonposter(&mut self) -> MainMsg {
            loop {
                match self.next().await {
                    MainMsg::PosterReady { .. } => continue,
                    m => return m,
                }
            }
        }
        /// Ensure everything queued so far was processed, return remaining msgs.
        async fn drain_via_probe(&mut self) -> Vec<MainMsg> {
            self.send(UiCmd::ProbeServer);
            let mut v = vec![];
            loop {
                match self.next_nonposter().await {
                    MainMsg::ServerUp | MainMsg::ServerDown => return v,
                    m => v.push(m),
                }
            }
        }
    }

    fn base_cfg() -> Config {
        Config { device_id: "dev-test".into(), ..Default::default() }
    }

    fn connect_pw(url: &str) -> UiCmd {
        UiCmd::Connect { url: url.into(), username: "alice".into(), password: Some("pw".into()), api_key: None, remember: true }
    }

    /// Worker connected to a fresh mock via the password path.
    async fn connected() -> (TestEnv, Mock, H) {
        let env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(connect_pw(&m.url()));
        assert!(matches!(h.next().await, MainMsg::Connected { .. }));
        (env, m, h)
    }

    fn items_of(m: MainMsg) -> (String, Vec<String>, bool) {
        match m {
            MainMsg::ItemsLoaded { title, items, can_go_back } => {
                (title, items.into_iter().map(|i| i.name).collect(), can_go_back)
            }
            o => panic!("expected ItemsLoaded, got {:?}", o),
        }
    }

    fn playback(m: MainMsg) -> PlaybackStart {
        match m {
            MainMsg::PlaybackReady(p) => p,
            o => panic!("expected PlaybackReady, got {:?}", o),
        }
    }

    // --------------------------------------------------------------- connect
    #[tokio::test]
    async fn connect_password_sends_display_name_and_persists() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(connect_pw(&format!("{}/", m.url())));
        match h.next().await {
            MainMsg::Connected { user_name, views, has_live_tv } => {
                assert_eq!(user_name, "Alice Display");
                assert!(!has_live_tv);
                assert_eq!(views.len(), 3);
                assert_eq!(views[0], ("v-mov".into(), "Movies".into(), Some("movies".into())));
            }
            o => panic!("{:?}", o),
        }
        let c = load_config();
        assert_eq!(c.server_url.as_deref(), Some(m.url().as_str()));
        assert_eq!(c.username.as_deref(), Some("Alice Display"));
        assert_eq!(c.user_id.as_deref(), Some("u1"));
        assert_eq!(crate::config::load_token(&m.url()).as_deref(), Some("tok123"));
        let auth = m.find("POST", "/Users/AuthenticateByName");
        assert_eq!(auth.len(), 1);
        assert!(auth[0].headers["authorization"].contains("DeviceId=\"dev-test\""));
    }

    #[tokio::test]
    async fn connect_reports_resolved_url_to_ui() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        // Bare host (no scheme, trailing slash): the mock is plain http, so
        // resolution must fall back to http and the UI must get the resolved base.
        let bare = m.url().trim_start_matches("http://").to_string();
        h.send(connect_pw(&format!("{}/", bare)));
        assert!(matches!(h.next_raw().await, MainMsg::Connected { .. }));
        match h.next_raw().await {
            MainMsg::ServerUrl(u) => assert_eq!(u, m.url()),
            o => panic!("expected ServerUrl after connect, got {:?}", o),
        }
    }

    #[tokio::test]
    async fn connect_with_live_tv_adds_section() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        m.cfg.lock().unwrap().live = true;
        let mut h = H::start(base_cfg());
        h.send(connect_pw(&m.url()));
        match h.next().await {
            MainMsg::Connected { views, has_live_tv, .. } => {
                assert!(has_live_tv);
                assert_eq!(views.len(), 4);
                assert_eq!(views[3], ("livetv".into(), "Live TV".into(), Some("livetv".into())));
            }
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn connect_api_key_path() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(UiCmd::Connect { url: m.url(), username: "me".into(), password: None, api_key: Some("apikey-1".into()), remember: true });
        match h.next().await {
            MainMsg::Connected { user_name, views, .. } => {
                assert_eq!(user_name, "me");
                assert_eq!(views.len(), 3);
            }
            o => panic!("{:?}", o),
        }
        assert_eq!(load_config().user_id.as_deref(), Some("u-first"));
        assert_eq!(crate::config::load_token(&m.url()).as_deref(), Some("apikey-1"));
        // Views were fetched for the first user with the API key.
        assert_eq!(m.find("GET", "/Users/u-first/Views").len(), 1);
    }

    #[tokio::test]
    async fn connect_error_paths_persist_nothing() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());

        // Bad URL.
        h.send(UiCmd::Connect { url: "ftp://nope".into(), username: "a".into(), password: Some("b".into()), api_key: None, remember: true });
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("Invalid server URL"), "{e}"),
            o => panic!("{:?}", o),
        }
        // Wrong password -> 401.
        h.send(UiCmd::Connect { url: m.url(), username: "alice".into(), password: Some("bad".into()), api_key: None, remember: true });
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("Sign-in failed"), "{e}"),
            o => panic!("{:?}", o),
        }
        // Bad API key.
        h.send(UiCmd::Connect { url: m.url(), username: "a".into(), password: None, api_key: Some("wrong".into()), remember: true });
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("Sign-in failed"), "{e}"),
            o => panic!("{:?}", o),
        }
        // Neither password nor key.
        h.send(UiCmd::Connect { url: m.url(), username: "a".into(), password: None, api_key: None, remember: true });
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("password or an API key"), "{e}"),
            o => panic!("{:?}", o),
        }
        // Reachable but not Jellyfin.
        m.cfg.lock().unwrap().not_jellyfin = true;
        h.send(connect_pw(&m.url()));
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("not a Jellyfin server"), "{e}"),
            o => panic!("{:?}", o),
        }
        // Server down.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        h.send(connect_pw(&dead));
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("Network problem"), "{e}"),
            o => panic!("{:?}", o),
        }
        let c = load_config();
        assert!(c.server_url.is_none() && c.user_id.is_none());
        assert!(crate::config::load_token(&m.url()).is_none());
        // Still not connected.
        assert!(matches!(h.drain_via_probe().await.as_slice(), []));
    }

    #[tokio::test]
    async fn remember_me_on_connect_persists_token_and_session() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(UiCmd::Connect {
            url: m.url(),
            username: "alice".into(),
            password: Some("pw".into()),
            api_key: None,
            remember: true,
        });
        assert!(matches!(h.next().await, MainMsg::Connected { .. }));
        let c = load_config();
        assert!(c.remember_me);
        assert_eq!(c.server_url.as_deref(), Some(m.url().as_str()));
        assert_eq!(c.user_id.as_deref(), Some("u1"));
        assert_eq!(crate::config::load_token(&m.url()).as_deref(), Some("tok123"));
    }

    #[tokio::test]
    async fn remember_me_off_connect_persists_nothing() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(UiCmd::Connect {
            url: m.url(),
            username: "alice".into(),
            password: Some("pw".into()),
            api_key: None,
            remember: false,
        });
        assert!(matches!(h.next().await, MainMsg::Connected { .. }));
        let c = load_config();
        assert!(!c.remember_me, "the choice itself is saved");
        assert!(c.server_url.is_none() && c.user_id.is_none() && c.username.is_none());
        assert!(crate::config::load_token(&m.url()).is_none());
        let on_disk = std::fs::read_to_string(crate::config::config_path()).unwrap();
        assert!(!on_disk.contains("tok123") && !on_disk.contains(&m.url()));
    }

    #[tokio::test]
    async fn remember_me_off_connect_forgets_previously_remembered_session() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let mut h = H::start(base_cfg());
        h.send(connect_pw(&m.url()));
        assert!(matches!(h.next().await, MainMsg::Connected { .. }));
        assert!(crate::config::load_token(&m.url()).is_some());
        h.send(UiCmd::Connect {
            url: m.url(),
            username: "alice".into(),
            password: Some("pw".into()),
            api_key: None,
            remember: false,
        });
        assert!(matches!(h.next().await, MainMsg::Connected { .. }));
        assert!(crate::config::load_token(&m.url()).is_none());
        assert!(load_config().server_url.is_none());
    }

    #[tokio::test]
    async fn startup_with_remember_me_off_does_not_auto_connect() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        crate::config::store_token(&m.url(), "tok123");
        let cfg = Config {
            server_url: Some(m.url()),
            user_id: Some("u1".into()),
            username: Some("Alice Display".into()),
            remember_me: false,
            ..base_cfg()
        };
        let mut h = H::start(cfg);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(m.reqs().is_empty(), "no server calls at launch");
        assert!(crate::config::load_token(&m.url()).is_none(), "leftover token removed");
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerDown | MainMsg::ServerUp));
    }

    #[tokio::test]
    async fn startup_auto_connect_uses_stored_token_and_username() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        crate::config::store_token(&m.url(), "tok123");
        let cfg = Config {
            server_url: Some(m.url()),
            user_id: Some("u1".into()),
            username: Some("Alice Display".into()),
            ..base_cfg()
        };
        let mut h = H::start(cfg);
        match h.next().await {
            MainMsg::Connected { user_name, .. } => assert_eq!(user_name, "Alice Display"),
            o => panic!("{:?}", o),
        }
        // Worker is usable right away.
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerUp));
    }

    #[tokio::test]
    async fn startup_without_token_or_with_stale_token_reports_error() {
        let _env = TestEnv::new();
        let m = start_mock().await;
        let cfg = Config { server_url: Some(m.url()), user_id: Some("u1".into()), ..base_cfg() };
        let mut h = H::start(cfg.clone());
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.is_empty()),
            o => panic!("{:?}", o),
        }
        crate::config::store_token(&m.url(), "revoked");
        let mut h = H::start(cfg);
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("token expired"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn fresh_start_without_server_is_silent() {
        let _env = TestEnv::new();
        let mut h = H::start(base_cfg());
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerDown));
    }

    // ----------------------------------------------------------------- views
    #[tokio::test]
    async fn load_view_queries_per_collection_type() {
        let (_e, m, mut h) = connected().await;
        for (ct, ty, name) in [
            (Some("movies"), Some("Movie"), "Movie One"),
            (Some("tvshows"), Some("Series"), "Show One"),
            (Some("music"), Some("MusicAlbum"), "Album One"),
            (Some("boxsets"), None, "Anything"),
            (None, None, "Anything"),
        ] {
            h.send(UiCmd::LoadView { view_id: "vid".into(), name: "Lib".into(), collection_type: ct.map(String::from) });
            let (title, names, back) = items_of(h.next().await);
            assert_eq!(title, "Lib");
            assert!(!back);
            assert_eq!(names[0], name);
            let last = m.find("GET", "/Users/u1/Items").pop().unwrap();
            assert_eq!(last.query.get("IncludeItemTypes").map(String::as_str), ty);
            assert_eq!(last.query["ParentId"], "vid");
            assert_eq!(last.query["SortBy"], "SortName");
            assert_eq!(last.query["SortOrder"], "Ascending");
        }
    }

    #[tokio::test]
    async fn load_view_livetv_lists_channels() {
        let (_e, m, mut h) = connected().await;
        m.cfg.lock().unwrap().live = true;
        h.send(UiCmd::LoadView { view_id: "livetv".into(), name: "Live TV".into(), collection_type: Some("livetv".into()) });
        let (title, names, _) = items_of(h.next().await);
        assert_eq!((title.as_str(), names), ("Live TV", vec!["Channel One".to_string()]));
        // View id alone also selects live TV.
        h.send(UiCmd::LoadView { view_id: "livetv".into(), name: "Live TV".into(), collection_type: None });
        items_of(h.next().await);
    }

    #[tokio::test]
    async fn load_view_errors_become_playback_errors() {
        let (_e, m, mut h) = connected().await;
        m.cfg.lock().unwrap().items_status = Some(500);
        h.send(UiCmd::LoadView { view_id: "v".into(), name: "L".into(), collection_type: Some("movies".into()) });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("HTTP 500"), "{e}"),
            o => panic!("{:?}", o),
        }
        m.cfg.lock().unwrap().items_status = Some(401);
        h.send(UiCmd::Episodes { series_id: "s".into(), series_name: "S".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("token expired"), "{e}"),
            o => panic!("{:?}", o),
        }
        m.stop();
        h.send(UiCmd::Tracks { album_id: "a".into(), album_name: "A".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("Network problem"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn commands_before_connect_are_ignored() {
        let _env = TestEnv::new();
        let mut h = H::start(base_cfg());
        h.send(UiCmd::LoadView { view_id: "v".into(), name: "L".into(), collection_type: None });
        h.send(UiCmd::Play { item_id: "x".into() });
        h.send(UiCmd::Progress { item_id: "x".into(), ticks: 1, paused: false });
        assert!(h.drain_via_probe().await.is_empty());
    }

    #[tokio::test]
    async fn posters_are_fetched_and_cached_on_disk() {
        let (_e, m, mut h) = connected().await;
        m.cfg.lock().unwrap().images = true;
        h.send(UiCmd::LoadView { view_id: "v".into(), name: "L".into(), collection_type: Some("movies".into()) });
        let mut posters = 0;
        let mut loaded = false;
        while posters < 2 || !loaded {
            match h.next().await {
                MainMsg::PosterReady { key, bytes } => {
                    assert!(key == sys_key("m1") || key == sys_key("m2"));
                    assert_eq!(bytes, vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
                    posters += 1;
                }
                MainMsg::ItemsLoaded { .. } => loaded = true,
                o => panic!("{:?}", o),
            }
        }
        assert_eq!(m.find("GET", "/Items/").len(), 2);
        assert_eq!(std::fs::read_dir(crate::config::cache_dir().join("posters")).unwrap().count(), 2);
        // Second load is served from the disk cache: no new image requests.
        h.send(UiCmd::LoadView { view_id: "v".into(), name: "L".into(), collection_type: Some("movies".into()) });
        let (mut posters, mut loaded) = (0, false);
        while posters < 2 || !loaded {
            match h.next().await {
                MainMsg::PosterReady { .. } => posters += 1,
                MainMsg::ItemsLoaded { .. } => loaded = true,
                o => panic!("{:?}", o),
            }
        }
        assert_eq!(m.find("GET", "/Items/").len(), 2);
    }

    // ---------------------------------------------------------------- search
    #[tokio::test]
    async fn search_single_query() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Search { q: "matrix".into() });
        let (title, names, back) = items_of(h.next().await);
        assert_eq!(title, "Results for “matrix”");
        assert_eq!(names, vec!["Found matrix"]);
        assert!(back);
        let r = m.find("GET", "/Users/u1/Items").pop().unwrap();
        assert_eq!(r.query["SearchTerm"], "matrix");
        assert_eq!(r.query["Limit"], "100");
    }

    #[tokio::test]
    async fn blank_search_makes_no_request() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Search { q: "   ".into() });
        assert!(h.drain_via_probe().await.is_empty());
        assert!(m.find("GET", "/Users/u1/Items").is_empty());
    }

    #[tokio::test]
    async fn search_debounce_keeps_newest_and_preserves_order() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Search { q: "a".into() });
        h.send(UiCmd::Search { q: "ab".into() });
        h.send(UiCmd::Episodes { series_id: "sr1".into(), series_name: "Show One".into() });
        h.send(UiCmd::Search { q: "abc".into() });
        h.send(UiCmd::Tracks { album_id: "al1".into(), album_name: "Album One".into() });
        h.send(UiCmd::ProbeServer);
        // Search result first, then the held commands in arrival order.
        let (t, _, back) = items_of(h.next().await);
        assert_eq!(t, "Results for “abc”");
        assert!(back);
        let (t, names, _) = items_of(h.next().await);
        assert_eq!((t.as_str(), names), ("Show One", vec!["Pilot".to_string(), "Second".to_string()]));
        let (t, names, _) = items_of(h.next().await);
        assert_eq!((t.as_str(), names), ("Album One", vec!["Track One".to_string()]));
        assert!(matches!(h.next().await, MainMsg::ServerUp));
        // Exactly one search request, with the newest term, before the episode request.
        let item_reqs = m.find("GET", "/Users/u1/Items");
        let terms: Vec<_> = item_reqs.iter().map(|r| r.query.get("SearchTerm").cloned()).collect();
        assert_eq!(terms, vec![Some("abc".to_string()), None, None]);
        assert_eq!(item_reqs[1].query["IncludeItemTypes"], "Episode");
        assert_eq!(item_reqs[2].query["IncludeItemTypes"], "Audio");
    }

    #[tokio::test]
    async fn search_after_search_each_runs_when_spaced_out() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Search { q: "one".into() });
        assert_eq!(items_of(h.next().await).0, "Results for “one”");
        h.send(UiCmd::Search { q: "two".into() });
        assert_eq!(items_of(h.next().await).0, "Results for “two”");
        assert_eq!(m.find("GET", "/Users/u1/Items").len(), 2);
    }

    #[tokio::test]
    async fn search_error_reports_playback_error() {
        let (_e, m, mut h) = connected().await;
        m.cfg.lock().unwrap().items_status = Some(503);
        h.send(UiCmd::Search { q: "x".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("HTTP 503"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    // ----------------------------------------------------------- episodes etc
    #[tokio::test]
    async fn episodes_and_tracks_queries() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Episodes { series_id: "sr1".into(), series_name: "Show One".into() });
        let (t, names, back) = items_of(h.next().await);
        assert_eq!((t.as_str(), back), ("Show One", true));
        assert_eq!(names, vec!["Pilot", "Second"]);
        let r = m.find("GET", "/Users/u1/Items").pop().unwrap();
        assert_eq!(r.query["ParentId"], "sr1");
        assert_eq!(r.query["IncludeItemTypes"], "Episode");
        assert_eq!(r.query["SortBy"], "ParentIndexNumber,IndexNumber");
        h.send(UiCmd::Tracks { album_id: "al1".into(), album_name: "Album One".into() });
        let (t, names, back) = items_of(h.next().await);
        assert_eq!((t.as_str(), back), ("Album One", true));
        assert_eq!(names, vec!["Track One"]);
        let r = m.find("GET", "/Users/u1/Items").pop().unwrap();
        assert_eq!(r.query["ParentId"], "al1");
        assert_eq!(r.query["IncludeItemTypes"], "Audio");
    }

    // ------------------------------------------------------------------ play
    #[tokio::test]
    async fn play_direct_plays_supported_media() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Play { item_id: "direct".into() });
        let p = playback(h.next().await);
        assert_eq!(p.method, PlaybackMethod::DirectPlay);
        assert_eq!(
            p.url,
            format!("{}/Videos/direct/stream.mp4?static=true&MediaSourceId=ms-1&api_key=tok123", m.url())
        );
        assert_eq!((p.item_id.as_str(), p.title.as_str()), ("direct", "Some Movie"));
        assert_eq!(p.subtitle, None);
        assert_eq!(p.resume_secs, 0.0);
        assert_eq!(p.run_time_secs, 7200.0);
        // Playback start reported, history mirrored.
        let st = m.find("POST", "/Sessions/Playing");
        assert_eq!(st.len(), 1);
        let b: serde_json::Value = serde_json::from_str(&st[0].body).unwrap();
        assert_eq!(b["ItemId"], "direct");
        assert_eq!(b["PositionTicks"], 0);
        assert_eq!(b["PlayMethod"], "DirectPlay");
        let hist = crate::config::load_history();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].item_id, "direct");
    }

    #[tokio::test]
    async fn play_falls_back_to_hls_when_needed() {
        let (_e, m, mut h) = connected().await;
        for id in ["mkv", "dts"] {
            h.send(UiCmd::Play { item_id: id.into() });
            let p = playback(h.next().await);
            assert_eq!(p.method, PlaybackMethod::ServerHls, "{id}");
            assert!(p.url.starts_with(&format!("{}/Videos/{}/master.m3u8?MediaSourceId=ms-1", m.url(), id)), "{}", p.url);
            assert!(p.url.contains("api_key=tok123"));
        }
        // No media sources at all -> HLS keyed on the item id.
        h.send(UiCmd::Play { item_id: "nosrc".into() });
        let p = playback(h.next().await);
        assert_eq!(p.method, PlaybackMethod::ServerHls);
        assert!(p.url.contains("/Videos/nosrc/master.m3u8?MediaSourceId=nosrc"), "{}", p.url);
        // Every HLS start is reported to the server as a transcode.
        for st in m.find("POST", "/Sessions/Playing") {
            let b: serde_json::Value = serde_json::from_str(&st.body).unwrap();
            assert_eq!(b["PlayMethod"], "Transcode");
        }
    }

    #[tokio::test]
    async fn play_honors_bitrate_cap_and_subtitle_settings() {
        let (_e, _m, mut h) = connected().await;
        h.send(UiCmd::Play { item_id: "hibr".into() });
        assert_eq!(playback(h.next().await).method, PlaybackMethod::DirectPlay);
        h.send(UiCmd::SaveSettings {
            settings: Settings { max_bitrate_kbps: 8000, subtitle_lang: "eng".into(), ..Default::default() },
            reconnect: false,
            server_url: None,
        });
        h.send(UiCmd::Play { item_id: "hibr".into() });
        let p = playback(h.next().await);
        assert_eq!(p.method, PlaybackMethod::ServerHls);
        assert!(p.url.contains("VideoBitrate=8000"), "{}", p.url);
        assert!(p.url.contains("SubtitleLanguage=eng"), "{}", p.url);
        // Persisted.
        assert_eq!(load_config().settings.max_bitrate_kbps, 8000);
    }

    #[tokio::test]
    async fn play_resume_and_played_flag() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Play { item_id: "resume".into() });
        let p = playback(h.next().await);
        assert_eq!(p.resume_secs, 600.0);
        let b: serde_json::Value = serde_json::from_str(&m.find("POST", "/Sessions/Playing")[0].body).unwrap();
        assert_eq!(b["PositionTicks"], 6_000_000_000i64);
        // Already-played items restart from the beginning.
        h.send(UiCmd::Play { item_id: "played".into() });
        assert_eq!(playback(h.next().await).resume_secs, 0.0);
        // Episodes carry the series name as subtitle.
        h.send(UiCmd::Play { item_id: "ep1".into() });
        assert_eq!(playback(h.next().await).subtitle.as_deref(), Some("Show One"));
    }

    #[tokio::test]
    async fn play_errors() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Play { item_id: "missing".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("HTTP 404"), "{e}"),
            o => panic!("{:?}", o),
        }
        h.send(UiCmd::Play { item_id: "expired".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("token expired"), "{e}"),
            o => panic!("{:?}", o),
        }
        assert!(m.find("POST", "/Sessions/Playing").is_empty());
        assert!(crate::config::load_history().is_empty());
        m.stop();
        h.send(UiCmd::Play { item_id: "direct".into() });
        match h.next().await {
            MainMsg::PlaybackError(e) => assert!(e.contains("Network problem"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn play_channel_builds_live_url() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::PlayChannel { channel_id: "ch1".into(), name: "Channel One".into() });
        let p = playback(h.next().await);
        assert_eq!(
            p.url,
            format!("{}/videos/ch1/master.m3u8?MediaSourceId=lms-1&ApiKey=tok123&LiveStreamId=ls-1", m.url())
        );
        assert_eq!(m.find("POST", "/Items/ch1/PlaybackInfo").len(), 1);
        assert_eq!(p.subtitle.as_deref(), Some("Live"));
        assert_eq!((p.title.as_str(), p.item_id.as_str()), ("Channel One", "ch1"));
        assert_eq!(p.method, PlaybackMethod::ServerHls);
        assert_eq!(p.resume_secs, 0.0);
        assert_eq!(m.find("POST", "/Sessions/Playing").len(), 1);
        // The live stream is released on the next channel / stop.
        h.send(UiCmd::Stopped { item_id: "ch1".into(), ticks: 0 });
        h.drain_via_probe().await;
        assert_eq!(m.find("POST", "/LiveStreams/Close").len(), 1);
    }

    // ------------------------------------------------------ item detail
    #[tokio::test]
    async fn open_detail_builds_view_and_fetches_art() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::OpenDetail { item_id: "det-movie".into() });
        let v = match h.next().await {
            MainMsg::DetailLoaded(v) => *v,
            o => panic!("{:?}", o),
        };
        assert_eq!(v.name, "Detail Movie");
        assert_eq!((v.year, v.official_rating.as_deref()), (Some(2020), Some("PG-13")));
        assert_eq!(v.community_rating, Some(7.8));
        assert_eq!(v.genres, vec!["Drama".to_string(), "Thriller".to_string()]);
        assert_eq!(v.overview.as_deref(), Some("A detailed overview."));
        assert_eq!(v.run_time_ticks, Some(72_000_000_000));
        assert!(!v.played && !v.resume);
        let poster = sys_key("det-movie");
        let backdrop = sys_key("det-movie#backdrop");
        assert_eq!((v.poster_key, v.backdrop_key), (Some(poster), Some(backdrop)));
        assert_eq!(
            v.people,
            vec![
                DetailPerson { name: "Ada Actor".into(), role: Some("Lead".into()), image_key: Some(sys_key("p-actor")) },
                DetailPerson { name: "Dan Director".into(), role: Some("Director".into()), image_key: None },
                DetailPerson { name: "No Image".into(), role: Some("Extra".into()), image_key: None },
            ]
        );
        // Artwork arrives after the facts: poster, backdrop and the one person photo.
        let mut got = std::collections::HashSet::new();
        while got.len() < 3 {
            match h.next().await {
                MainMsg::PosterReady { key, bytes } => {
                    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
                    got.insert(key);
                }
                o => panic!("{:?}", o),
            }
        }
        assert_eq!(got, [poster, backdrop, sys_key("p-actor")].into_iter().collect());
        let detail = m.find("GET", "/Users/u1/Items/det-movie");
        assert_eq!(detail.len(), 1);
        let fields = detail[0].query.get("Fields").cloned().unwrap_or_default();
        for f in ["People", "Genres", "Overview"] {
            assert!(fields.contains(f), "{fields}");
        }
        assert_eq!(m.find("GET", "/Items/det-movie/Images/Backdrop").len(), 1);
        assert_eq!(m.find("GET", "/Items/p-actor/Images/Primary").len(), 1);
        assert!(m.find("GET", "/Items/p-noimg/Images").is_empty());
    }

    #[tokio::test]
    async fn seasons_select_next_up_season_and_load_its_episodes() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Seasons { series_id: "sr1".into() });
        match h.next().await {
            MainMsg::SeasonsLoaded { series_id, seasons, selected } => {
                assert_eq!(series_id, "sr1");
                let names: Vec<&str> = seasons.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(names, vec!["Season 1", "Season 2"]);
                assert_eq!(selected, 1, "Next Up is in season 2");
            }
            o => panic!("{:?}", o),
        }
        let seasons_req = m.find("GET", "/Shows/sr1/Seasons");
        assert_eq!(seasons_req.len(), 1);
        assert_eq!(seasons_req[0].query.get("UserId").map(String::as_str), Some("u1"));
        let next = m.find("GET", "/Shows/NextUp");
        assert_eq!(next[0].query.get("SeriesId").map(String::as_str), Some("sr1"));

        h.send(UiCmd::SeasonEpisodes { series_id: "sr1".into(), season_id: "sea-b".into() });
        match h.next().await {
            MainMsg::SeasonEpisodesLoaded { series_id, season_id, episodes } => {
                assert_eq!((series_id.as_str(), season_id.as_str()), ("sr1", "sea-b"));
                assert_eq!(episodes.len(), 3);
                assert!(episodes[0].played);
                assert_eq!((episodes[0].number, episodes[0].image_key), (Some(1), None));
                assert_eq!(episodes[1].position_ticks, 6_000_000_000);
                assert_eq!(episodes[1].image_key, Some(sys_key("b2")));
                assert_eq!(episodes[1].overview.as_deref(), Some("About episode 2."));
                assert_eq!(episodes[1].run_time_ticks, Some(27_000_000_000));
                assert_eq!(episodes[2].image_key, None);
            }
            o => panic!("{:?}", o),
        }
        // The one episode with a still gets its thumbnail after the rows.
        match h.next().await {
            MainMsg::PosterReady { key, bytes } => {
                assert_eq!(key, sys_key("b2"));
                assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
            }
            o => panic!("{:?}", o),
        }
        let eps = m.find("GET", "/Shows/sr1/Episodes");
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].query.get("seasonId").map(String::as_str), Some("sea-b"));
        assert_eq!(eps[0].query.get("UserId").map(String::as_str), Some("u1"));
    }

    #[tokio::test]
    async fn seasons_fall_back_and_report_failures() {
        let (_e, _m, mut h) = connected().await;
        // Next Up without a season id: matched by season number.
        h.send(UiCmd::Seasons { series_id: "sr2".into() });
        assert!(matches!(h.next().await, MainMsg::SeasonsLoaded { selected: 1, .. }));
        // Next Up request fails: the first season is shown.
        h.send(UiCmd::Seasons { series_id: "sr3".into() });
        assert!(matches!(h.next().await, MainMsg::SeasonsLoaded { selected: 0, .. }));
        // No seasons at all.
        h.send(UiCmd::Seasons { series_id: "noseasons".into() });
        match h.next().await {
            MainMsg::SeasonsLoaded { seasons, selected, .. } => assert!(seasons.is_empty() && selected == 0),
            o => panic!("{:?}", o),
        }
        h.send(UiCmd::Seasons { series_id: "seasons-500".into() });
        match h.next().await {
            MainMsg::SeasonsError { series_id, message } => {
                assert_eq!(series_id, "seasons-500");
                assert!(message.contains("500"), "{message}");
            }
            o => panic!("{:?}", o),
        }
        h.send(UiCmd::SeasonEpisodes { series_id: "sr1".into(), season_id: "sea-boom".into() });
        assert!(matches!(h.next().await, MainMsg::SeasonsError { .. }));
        h.send(UiCmd::SeasonEpisodes { series_id: "sr1".into(), season_id: "sea-empty".into() });
        match h.next().await {
            MainMsg::SeasonEpisodesLoaded { episodes, .. } => assert!(episodes.is_empty()),
            o => panic!("{:?}", o),
        }
    }

    fn season_of(id: &str, index: u32) -> Season {
        serde_json::from_value(serde_json::json!({"Id": id, "Name": id, "IndexNumber": index})).unwrap()
    }
    fn episode_in(season_id: Option<&str>, parent: Option<u32>) -> Episode {
        serde_json::from_value(serde_json::json!({"Id": "e", "Name": "e", "SeasonId": season_id, "ParentIndexNumber": parent}))
            .unwrap()
    }

    #[test]
    fn default_season_follows_next_up() {
        let seasons = vec![season_of("s1", 1), season_of("s2", 2), season_of("s3", 3)];
        assert_eq!(default_season(&seasons, None), 0, "no Next Up: first season");
        assert_eq!(default_season(&seasons, Some(&episode_in(Some("s3"), Some(3)))), 2, "by season id");
        assert_eq!(default_season(&seasons, Some(&episode_in(None, Some(2)))), 1, "by season number");
        assert_eq!(default_season(&seasons, Some(&episode_in(Some("gone"), Some(9)))), 0, "unknown season");
        assert_eq!(default_season(&[], Some(&episode_in(Some("s1"), Some(1)))), 0, "no seasons");
    }

    #[tokio::test]
    async fn detail_resume_played_and_series_kinds() {
        let (_e, _m, mut h) = connected().await;
        for (id, want_resume, want_played, want_ty) in
            [("det-resume", true, false, "Movie"), ("det-played", false, true, "Movie"), ("det-series", false, false, "Series")]
        {
            h.send(UiCmd::OpenDetail { item_id: id.into() });
            match h.next_nonposter().await {
                MainMsg::DetailLoaded(v) => {
                    assert_eq!((v.resume, v.played, v.item_type.as_str()), (want_resume, want_played, want_ty), "{id}");
                }
                o => panic!("{:?}", o),
            }
        }
    }

    #[tokio::test]
    async fn detail_without_artwork_requests_none() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::OpenDetail { item_id: "det-bare".into() });
        match h.next_nonposter().await {
            MainMsg::DetailLoaded(v) => {
                assert_eq!((v.poster_key, v.backdrop_key), (None, None));
                assert!(v.people.is_empty() && v.genres.is_empty() && v.overview.is_none());
                assert!(v.official_rating.is_none() && v.community_rating.is_none());
            }
            o => panic!("{:?}", o),
        }
        let rest = h.drain_via_probe().await;
        assert!(rest.iter().all(|m| !matches!(m, MainMsg::PosterReady { .. })), "{rest:?}");
        assert!(m.find("GET", "/Items/det-bare/Images").is_empty());
    }

    #[tokio::test]
    async fn detail_error_is_reported() {
        let (_e, _m, mut h) = connected().await;
        h.send(UiCmd::OpenDetail { item_id: "missing".into() });
        match h.next().await {
            MainMsg::DetailError(e) => assert!(e.contains("HTTP 404"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn open_detail_before_connect_is_ignored() {
        let _env = TestEnv::new();
        let mut h = H::start(base_cfg());
        h.send(UiCmd::OpenDetail { item_id: "det-movie".into() });
        h.send(UiCmd::ProbeServer);
        let got = h.drain_via_probe().await;
        assert!(got.is_empty(), "{got:?}");
    }

    // -------------------------------------------------- progress / played
    #[tokio::test]
    async fn progress_stopped_and_mark_played_hit_server() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::Progress { item_id: "i1".into(), ticks: 500, paused: true });
        h.send(UiCmd::Progress { item_id: "i1".into(), ticks: -5, paused: false });
        h.send(UiCmd::Stopped { item_id: "i1".into(), ticks: 900 });
        h.send(UiCmd::MarkPlayed { item_id: "i1".into(), played: true });
        h.send(UiCmd::MarkPlayed { item_id: "i1".into(), played: false });
        h.send(UiCmd::MarkPlayed { item_id: "boom".into(), played: true });
        // No MainMsg for any of these, and the worker survives the 500.
        assert!(h.drain_via_probe().await.is_empty());
        let prog = m.find("POST", "/Sessions/Playing/Progress");
        assert_eq!(prog.len(), 2);
        let b0: serde_json::Value = serde_json::from_str(&prog[0].body).unwrap();
        assert_eq!((b0["PositionTicks"].as_i64(), b0["IsPaused"].as_bool()), (Some(500), Some(true)));
        let b1: serde_json::Value = serde_json::from_str(&prog[1].body).unwrap();
        assert_eq!(b1["PositionTicks"], 0);
        let stop = m.find("POST", "/Sessions/Playing/Stopped");
        assert_eq!(stop.len(), 1);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&stop[0].body).unwrap()["PositionTicks"], 900);
        assert_eq!(m.find("POST", "/Users/u1/PlayedItems/i1").len(), 1);
        assert_eq!(m.find("DELETE", "/Users/u1/PlayedItems/i1").len(), 1);
        assert_eq!(m.find("POST", "/Users/u1/PlayedItems/boom").len(), 1);
    }

    // ------------------------------------------------- settings / sign-out
    #[tokio::test]
    async fn save_settings_with_reconnect_reannounces_connection() {
        let (_e, m, mut h) = connected().await;
        m.cfg.lock().unwrap().live = true;
        h.send(UiCmd::SaveSettings { settings: Settings { theme: "dark".into(), ..Default::default() }, reconnect: true, server_url: None });
        match h.next().await {
            MainMsg::Connected { user_name, has_live_tv, views, .. } => {
                assert_eq!(user_name, "Alice Display");
                assert!(has_live_tv);
                assert_eq!(views.len(), 4);
            }
            o => panic!("{:?}", o),
        }
        assert_eq!(load_config().settings.theme, "dark");
    }

    #[tokio::test]
    async fn save_settings_reconnect_failure_reports_error() {
        let (_e, m, mut h) = connected().await;
        crate::config::store_token(&m.url(), "revoked");
        h.send(UiCmd::SaveSettings { settings: Settings::default(), reconnect: true, server_url: None });
        match h.next().await {
            MainMsg::ConnectError(e) => assert!(e.contains("token expired"), "{e}"),
            o => panic!("{:?}", o),
        }
    }

    #[tokio::test]
    async fn sign_out_clears_state_and_stops_server_calls() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::SignOut);
        assert!(matches!(h.next().await, MainMsg::SignedOut));
        let c = load_config();
        assert!(c.server_url.is_none() && c.username.is_none() && c.user_id.is_none());
        assert!(crate::config::load_token(&m.url()).is_none());
        let before = m.reqs().len();
        h.send(UiCmd::Play { item_id: "direct".into() });
        h.send(UiCmd::LoadView { view_id: "v".into(), name: "L".into(), collection_type: None });
        h.send(UiCmd::Progress { item_id: "direct".into(), ticks: 1, paused: false });
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerDown));
        assert_eq!(m.reqs().len(), before, "no server calls after sign-out");
    }

    #[tokio::test]
    async fn probe_server_up_then_down() {
        let (_e, m, mut h) = connected().await;
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerUp));
        m.stop();
        tokio::task::yield_now().await;
        h.send(UiCmd::ProbeServer);
        assert!(matches!(h.next().await, MainMsg::ServerDown));
    }

    #[tokio::test]
    async fn worker_exits_when_command_channel_closes() {
        let _env = TestEnv::new();
        let h = H::start(base_cfg());
        let H { tx, task, .. } = h;
        drop(tx);
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn quit_saving_is_a_noop() {
        let (_e, _m, mut h) = connected().await;
        h.send(UiCmd::QuitSaving);
        assert!(h.drain_via_probe().await.is_empty());
    }

    // ------------------------------------------------------------------ seerr
    use crate::seerr::models::{Availability, RequestStatus};
    use crate::seerr::testutil::{default_route, start as start_seerr, MockSeerr, Resp as SResp};

    /// Worker with Seerr enabled against `m`, key stored in the (test) keychain.
    fn seerr_cfg(m: &MockSeerr) -> Config {
        crate::config::store_seerr_key(&m.url(), "good-key");
        let mut c = base_cfg();
        c.settings.seerr_enabled = true;
        c.settings.seerr_url = m.url();
        c
    }

    fn seerr_err(m: MainMsg) -> String {
        match m {
            MainMsg::SeerrError(e) => e,
            o => panic!("expected SeerrError, got {:?}", o),
        }
    }

    #[tokio::test]
    async fn seerr_search_returns_movies_and_tv() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrSearch { q: "incep tion".into() });
        match h.next().await {
            MainMsg::SeerrResults(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[0].title, "Inception");
                assert_eq!(v[0].availability, Availability::Available);
            }
            o => panic!("{:?}", o),
        }
        assert_eq!(m.find("GET", "/api/v1/search")[0].query["query"], "incep tion");
    }

    #[tokio::test]
    async fn seerr_posters_come_through_the_server_and_are_cached() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrSearch { q: "incep".into() });
        let mut got = std::collections::HashMap::new();
        let mut results = false;
        while !results || got.len() < 2 {
            match h.next_raw().await {
                MainMsg::SeerrResults(_) => results = true,
                MainMsg::SeerrPoster { path, bytes } => {
                    got.insert(path, bytes);
                }
                o => panic!("{:?}", o),
            }
        }
        assert_eq!(got["/inc.jpg"], b"IMG/inc.jpg");
        assert_eq!(got["/got.jpg"], b"IMG/got.jpg");
        let reqs = m.find("GET", "/imageproxy/tmdb/t/p/w92/inc.jpg");
        assert_eq!(reqs.len(), 1);
        assert!(!reqs[0].headers.contains_key("x-api-key"), "poster fetch must not leak the key");
        // Second search is served from the disk cache.
        h.send(UiCmd::SeerrSearch { q: "incep".into() });
        let (mut results, mut n) = (false, 0);
        while !results || n < 2 {
            match h.next_raw().await {
                MainMsg::SeerrResults(_) => results = true,
                MainMsg::SeerrPoster { .. } => n += 1,
                o => panic!("{:?}", o),
            }
        }
        assert_eq!(m.find("GET", "/imageproxy/tmdb/t/p/w92/inc.jpg").len(), 1);
    }

    #[tokio::test]
    async fn seerr_search_debounce_keeps_newest_and_preserves_other_commands() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrSearch { q: "a".into() });
        h.send(UiCmd::SeerrSearch { q: "ab".into() });
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 1, title: "One".into() });
        h.send(UiCmd::SeerrSearch { q: "abc".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrResults(_)));
        assert!(matches!(h.next().await, MainMsg::SeerrRequested { .. }));
        let s = m.find("GET", "/api/v1/search");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].query["query"], "abc");
        assert_eq!(m.find("POST", "/api/v1/request").len(), 1);
    }

    #[tokio::test]
    async fn seerr_blank_search_clears_without_request() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrSearch { q: "   ".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrResults(v) if v.is_empty()));
        assert!(m.reqs().is_empty());
    }

    #[tokio::test]
    async fn seerr_search_error_reports() {
        let _env = TestEnv::new();
        let m = start_seerr(|_| SResp::json(500, serde_json::json!({"message": "db down"}))).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrSearch { q: "x".into() });
        assert!(seerr_err(h.next().await).contains("db down"));
    }

    #[tokio::test]
    async fn seerr_request_success_movie_and_tv() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 27205, title: "Inception".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrRequested { title } if title == "Inception"));
        h.send(UiCmd::SeerrRequest { media_type: "tv".into(), tmdb_id: 1399, title: "GoT".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrRequested { title } if title == "GoT"));
        let posts = m.find("POST", "/api/v1/request");
        assert!(posts[1].body.contains("\"seasons\":\"all\""));
    }

    #[tokio::test]
    async fn seerr_request_already_requested_and_other_errors() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 999, title: "Dup".into() });
        let e = seerr_err(h.next().await);
        assert!(e.contains("Dup") && e.contains("already been requested"), "{}", e);
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 997, title: "Q".into() });
        assert!(seerr_err(h.next().await).contains("quota"));
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 996, title: "B".into() });
        assert!(seerr_err(h.next().await).contains("boom"));
    }

    #[tokio::test]
    async fn seerr_request_list() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SeerrRequests);
        match h.next().await {
            MainMsg::SeerrRequestList(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[0].title, "Game of Thrones");
                assert_eq!(v[0].status, RequestStatus::Approved);
            }
            o => panic!("{:?}", o),
        }
        assert!(m.find("GET", "/api/v1/request")[0].raw_target.contains("take=20"));
    }

    #[tokio::test]
    async fn seerr_disabled_or_unconfigured_makes_no_network_calls() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        // disabled (url + key present, but switched off)
        let mut c = seerr_cfg(&m);
        c.settings.seerr_enabled = false;
        let mut h = H::start(c);
        h.send(UiCmd::SeerrSearch { q: "x".into() });
        assert!(seerr_err(h.next().await).contains("turned off"));
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 1, title: "T".into() });
        assert!(seerr_err(h.next().await).contains("turned off"));
        h.send(UiCmd::SeerrRequests);
        assert!(seerr_err(h.next().await).contains("turned off"));
        // enabled but no URL
        let mut c = base_cfg();
        c.settings.seerr_enabled = true;
        let mut h2 = H::start(c);
        h2.send(UiCmd::SeerrRequests);
        assert!(seerr_err(h2.next().await).contains("URL"));
        // enabled, URL, but no key stored
        let _ = crate::config::delete_seerr_key(&m.url());
        let mut c = base_cfg();
        c.settings.seerr_enabled = true;
        c.settings.seerr_url = m.url();
        let mut h3 = H::start(c);
        h3.send(UiCmd::SeerrRequests);
        assert!(seerr_err(h3.next().await).contains("API key"));
        assert!(m.reqs().is_empty());
    }

    #[tokio::test]
    async fn save_seerr_validates_then_persists() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(base_cfg());
        h.send(UiCmd::SaveSeerr { enabled: true, url: format!("{}/", m.url()), username: String::new(), password: None, api_key: Some(" good-key ".into()) });
        match h.next().await {
            MainMsg::SeerrStatus { ok, message } => {
                assert!(ok, "{}", message);
                assert!(message.contains("2.7.2") && message.contains("Nathan"), "{}", message);
            }
            o => panic!("{:?}", o),
        }
        let c = load_config();
        assert!(c.settings.seerr_enabled);
        assert_eq!(c.settings.seerr_url, m.url());
        assert_eq!(crate::config::load_seerr_key(&m.url()).as_deref(), Some("good-key"));
        // key never lands in config.json
        let raw = std::fs::read_to_string(crate::config::config_path()).unwrap();
        assert!(!raw.contains("good-key"));
        // and searching now works with the stored key
        h.send(UiCmd::SeerrSearch { q: "x".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrResults(_)));
        // re-save without a key reuses the stored one
        h.send(UiCmd::SaveSeerr { enabled: true, url: m.url(), username: String::new(), password: None, api_key: None });
        assert!(matches!(h.next().await, MainMsg::SeerrStatus { ok: true, .. }));
    }

    #[tokio::test]
    async fn save_seerr_bad_key_persists_nothing() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(base_cfg());
        h.send(UiCmd::SaveSeerr { enabled: true, url: m.url(), username: String::new(), password: None, api_key: Some("bad".into()) });
        match h.next().await {
            MainMsg::SeerrStatus { ok, message } => {
                assert!(!ok);
                assert!(message.contains("API key"), "{}", message);
            }
            o => panic!("{:?}", o),
        }
        assert!(!load_config().settings.seerr_enabled);
        assert_eq!(crate::config::load_seerr_key(&m.url()), None);
        // follow-up commands stay disabled
        h.send(UiCmd::SeerrRequests);
        assert!(seerr_err(h.next().await).contains("turned off"));
    }

    #[tokio::test]
    async fn save_seerr_input_errors_and_unreachable() {
        let _env = TestEnv::new();
        let mut h = H::start(base_cfg());
        h.send(UiCmd::SaveSeerr { enabled: true, url: "nope".into(), username: String::new(), password: None, api_key: Some("k".into()) });
        assert!(matches!(h.next().await, MainMsg::SeerrStatus { ok: false, .. }));
        h.send(UiCmd::SaveSeerr { enabled: true, url: "http://127.0.0.1:1".into(), username: String::new(), password: None, api_key: None });
        match h.next().await {
            MainMsg::SeerrStatus { ok: false, message } => assert!(message.contains("API key")),
            o => panic!("{:?}", o),
        }
        h.send(UiCmd::SaveSeerr { enabled: true, url: "http://127.0.0.1:1".into(), username: String::new(), password: None, api_key: Some("k".into()) });
        match h.next().await {
            MainMsg::SeerrStatus { ok: false, message } => assert!(message.contains("Could not reach"), "{}", message),
            o => panic!("{:?}", o),
        }
        assert!(!load_config().settings.seerr_enabled);
    }

    #[tokio::test]
    async fn save_seerr_disable_keeps_key_and_skips_network() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut h = H::start(seerr_cfg(&m));
        h.send(UiCmd::SaveSeerr { enabled: false, url: m.url(), username: String::new(), password: None, api_key: None });
        assert!(matches!(h.next().await, MainMsg::SeerrStatus { ok: true, .. }));
        assert!(!load_config().settings.seerr_enabled);
        assert_eq!(crate::config::load_seerr_key(&m.url()).as_deref(), Some("good-key"));
        h.send(UiCmd::SeerrSearch { q: "x".into() });
        assert!(seerr_err(h.next().await).contains("turned off"));
        assert!(m.reqs().is_empty());
    }

    #[tokio::test]
    async fn save_seerr_login_mode_end_to_end() {
        use crate::seerr::testutil::{login_route, LoginState};
        let _env = TestEnv::new();
        let st = LoginState::default();
        let m = start_seerr(login_route("nathan", "pw", st.clone())).await;
        let mut h = H::start(base_cfg());
        // wrong password: nothing persisted
        h.send(UiCmd::SaveSeerr {
            enabled: true,
            url: m.url(),
            username: "nathan".into(),
            password: Some("bad".into()),
            api_key: None,
        });
        match h.next().await {
            MainMsg::SeerrStatus { ok: false, message } => assert!(message.contains("sign-in failed"), "{}", message),
            o => panic!("{:?}", o),
        }
        assert!(!load_config().settings.seerr_enabled);
        assert_eq!(crate::config::load_seerr_login(&m.url()), None);
        // missing password
        h.send(UiCmd::SaveSeerr { enabled: true, url: m.url(), username: "nathan".into(), password: None, api_key: None });
        match h.next().await {
            MainMsg::SeerrStatus { ok: false, message } => assert!(message.contains("password"), "{}", message),
            o => panic!("{:?}", o),
        }
        // good password
        h.send(UiCmd::SaveSeerr {
            enabled: true,
            url: format!("{}/", m.url()),
            username: " nathan ".into(),
            password: Some("pw".into()),
            api_key: None,
        });
        match h.next().await {
            MainMsg::SeerrStatus { ok, message } => {
                assert!(ok, "{}", message);
                assert!(message.contains("Nathan"), "{}", message);
            }
            o => panic!("{:?}", o),
        }
        let c = load_config();
        assert!(c.settings.seerr_enabled);
        assert_eq!((c.settings.seerr_url.as_str(), c.settings.seerr_username.as_str()), (m.url().as_str(), "nathan"));
        assert_eq!(crate::config::load_seerr_login(&m.url()).as_deref(), Some("pw"));
        assert_eq!(crate::config::load_seerr_key(&m.url()), None);
        let raw = std::fs::read_to_string(crate::config::config_path()).unwrap();
        assert!(!raw.contains("\"pw\"") && !raw.contains("password"), "{}", raw);
        // commands now use the stored password and survive session expiry
        h.send(UiCmd::SeerrSearch { q: "x".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrResults(_)));
        st.expire_sessions();
        h.send(UiCmd::SeerrRequest { media_type: "movie".into(), tmdb_id: 27205, title: "Inception".into() });
        assert!(matches!(h.next().await, MainMsg::SeerrRequested { .. }));
        // re-save with no password reuses the stored one
        h.send(UiCmd::SaveSeerr { enabled: true, url: m.url(), username: "nathan".into(), password: None, api_key: None });
        assert!(matches!(h.next().await, MainMsg::SeerrStatus { ok: true, .. }));
        // switching to API-key mode clears the username
        let k = start_seerr(default_route).await;
        h.send(UiCmd::SaveSeerr {
            enabled: true,
            url: k.url(),
            username: String::new(),
            password: None,
            api_key: Some("good-key".into()),
        });
        assert!(matches!(h.next().await, MainMsg::SeerrStatus { ok: true, .. }));
        assert_eq!(load_config().settings.seerr_username, "");
    }

    #[tokio::test]
    async fn seerr_login_mode_without_stored_password_makes_no_calls() {
        let _env = TestEnv::new();
        let m = start_seerr(default_route).await;
        let mut c = base_cfg();
        c.settings.seerr_enabled = true;
        c.settings.seerr_url = m.url();
        c.settings.seerr_username = "nathan".into();
        let mut h = H::start(c);
        h.send(UiCmd::SeerrRequests);
        assert!(seerr_err(h.next().await).contains("password"));
        assert!(m.reqs().is_empty());
    }
}
