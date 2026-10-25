use crate::*;
use std::collections::VecDeque;
use crate::app::messages::*;
use crate::jellyfin::client::JellyfinClient;
use crate::config::{load_config, save_config, Config};

pub(crate) async fn worker(mut rx: UnboundedReceiver<UiCmd>, main_tx: UnboundedSender<MainMsg>, cfg: Config) {
    let mut client: Option<JellyfinClient> = None;
    let mut cfg = cfg;

    // Poster disk cache: ~/Library/Caches/JellyfinMac/posters
    let poster_dir = crate::config::cache_dir().join("posters");
    let _ = std::fs::create_dir_all(&poster_dir);

    // Silent auto-connect from persisted server + keychain token.
    if let (Some(url), Some(uid)) = (cfg.server_url.clone(), cfg.user_id.clone()) {
        if let Some(token) = crate::config::load_token(&url) {
            match crate::jellyfin::client::JellyfinClient::new(&url, token.clone(), uid.clone(), &cfg.device_id) {
                Ok(c) => match connect_flow(&c, &mut cfg, &main_tx, false).await {
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
            UiCmd::Connect { url, username, password, api_key } => {
                match connect_cmd(&url, &username, password, api_key, &mut cfg, &main_tx).await {
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
            UiCmd::Play { item_id } => {
                if let Some(c) = &client {
                    match c.full_item(&item_id).await {
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
                            c.report_start(&full.id, crate::jellyfin::urls::ticks_from_secs(resume_secs))
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
                                subtitle: full.series_name.clone(),
                                resume_secs,
                                method,
                                run_time_secs: crate::jellyfin::urls::secs_from_ticks(
                                    full.run_time_ticks.unwrap_or(0),
                                ),
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
                    let url = crate::jellyfin::urls::build_livetv_url(&c.base, &channel_id, &c.token);
                    c.report_start(&channel_id, 0).await;
                    let _ = main_tx.send(MainMsg::PlaybackReady(PlaybackStart {
                        url,
                        item_id: channel_id,
                        title: name,
                        subtitle: Some("Live".into()),
                        resume_secs: 0.0,
                        method: crate::transcode::decision::PlaybackMethod::DirectPlay,
                        run_time_secs: 0.0,
                    }));
                }
            }
            UiCmd::Progress { item_id, ticks, paused } => {
                if let Some(c) = &client {
                    c.report_progress(&item_id, ticks, paused).await;
                }
            }
            UiCmd::Stopped { item_id, ticks } => {
                if let Some(c) = &client {
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
            UiCmd::SaveSettings { settings, reconnect } => {
                cfg.settings = settings;
                crate::config::save_config(&cfg);
                if reconnect {
                    if let Some(url) = cfg.server_url.clone() {
                        if let Some(token) = crate::config::load_token(&url) {
                            match crate::jellyfin::client::JellyfinClient::new(
                                &url,
                                token.clone(),
                                cfg.user_id.clone().unwrap_or_default(),
                                &cfg.device_id,
                            ) {
                                Ok(c) => match connect_flow(&c, &mut cfg, &main_tx, true).await {
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
    cfg: &mut Config,
    main_tx: &UnboundedSender<MainMsg>,
) -> Result<JellyfinClient, crate::jellyfin::client::JFError> {
    use crate::jellyfin::client::JFError;
    // Step 1: verify it's Jellyfin.
    let info = JellyfinClient::public_info(url).await?;
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
    connect_flow(&c, cfg, main_tx, true).await?;
    // Persist credentials only after a fully successful connect.
    cfg.server_url = Some(normalize(url));
    cfg.username = Some(user_name.clone());
    cfg.user_id = Some(user_id.clone());
    crate::config::store_token(&normalize(url), &token);
    crate::config::save_config(cfg);
    Ok(c)
}

pub(crate) fn normalize(u: &str) -> String {
    crate::jellyfin::urls::normalize_base_url(u).unwrap_or_else(|_| u.to_string())
}

pub(crate) async fn connect_flow(
    c: &JellyfinClient,
    _cfg: &mut Config,
    main_tx: &UnboundedSender<MainMsg>,
    _announce: bool,
) -> Result<(), crate::jellyfin::client::JFError> {
    let views = c.views().await?;
    let mut v: Vec<(String, String, Option<String>)> = views
        .into_iter()
        .map(|x| (x.id, x.name, x.collection_type))
        .collect();
    // Probe live TV (cheap, 1 item) — sidebar shows the section only if present.
    let has_live = c.livetv_channels().await.map(|i| !i.is_empty()).unwrap_or(false);
    if has_live {
        v.push(("livetv".into(), "Live TV".into(), Some("livetv".into())));
    }
    let _ = main_tx.send(MainMsg::Connected {
        user_name: c.user_id.clone(),
        views: v,
        has_live_tv: false,
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
    let Some(http) = http.cloned() else {
        return;
    };
    let client = c.clone();
    let tx = main_tx.clone();
    let dir = poster_dir.to_path_buf();
    // Spawn a bounded set of concurrent poster fetches (first 200 items).
    for it in items.iter().take(200) {
        if !it.has_image() {
            continue;
        }
        let client = client.clone();
        let tx = tx.clone();
        let dir = dir.clone();
        let id = it.id.clone();
        let http = http.clone();
        tokio::spawn(async move {
            let key = sys_key(&id);
            let path = dir.join(format!("{:016x}.img", key));
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => Some(b),
                Err(_) => {
                    let url = crate::jellyfin::urls::build_image_url(&client.base, &id, &client.token, 400);
                    match http.get(url).send().await {
                        Ok(r) if r.status().is_success() => match r.bytes().await {
                            Ok(b) => {
                                let _ = tokio::fs::write(&path, &b).await;
                                Some(b.to_vec())
                            }
                            Err(_) => None,
                        },
                        _ => None,
                    }
                }
            };
            if let Some(b) = bytes {
                let _ = tx.send(MainMsg::PosterReady { key, bytes: b });
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
