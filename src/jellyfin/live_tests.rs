//! Tests against a REAL Jellyfin server. All `#[ignore]`d and env-gated:
//!   JF_LIVE_URL, JF_LIVE_USER, JF_LIVE_PASS_FILE (path to a file holding the password),
//!   JF_LIVE_OUT (optional dir for raw dumps / result tables),
//!   JF_LIVE_AVCHECK (optional path to the compiled tools/avcheck.swift binary).
//! Run: cargo test -- --ignored jf_live --test-threads=1
//! Strictly read-only: playback reports and played-state changes are disabled
//! (`set_read_only`), and the password/token are never printed.

use super::client::{set_read_only, JellyfinClient};
use super::models::*;
use crate::config::test_env::TestEnv;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(crate) struct Live {
    pub url: String,
    pub user: String,
    pub pass: String,
    pub out: Option<PathBuf>,
}

pub(crate) fn live_env() -> Option<Live> {
    let url = std::env::var("JF_LIVE_URL").ok()?;
    let user = std::env::var("JF_LIVE_USER").ok()?;
    let pf = std::env::var("JF_LIVE_PASS_FILE").ok()?;
    let pass = std::fs::read_to_string(pf).ok()?.trim_end_matches('\n').to_string();
    let out = std::env::var("JF_LIVE_OUT").ok().map(PathBuf::from);
    if let Some(o) = &out {
        let _ = std::fs::create_dir_all(o);
    }
    set_read_only(true);
    Some(Live { url, user, pass, out })
}

/// Remove secrets from anything printed.
pub(crate) fn redact(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        let l = rest.to_ascii_lowercase();
        let pos = ["api_key=", "apikey="].iter().filter_map(|k| l.find(k).map(|p| (p, k.len()))).min();
        match pos {
            None => {
                out.push_str(rest);
                return out;
            }
            Some((p, kl)) => {
                out.push_str(&rest[..p + kl]);
                out.push_str("REDACTED");
                let after = &rest[p + kl..];
                let end = after.find(|c: char| c == '&' || c == '"' || c.is_whitespace()).unwrap_or(after.len());
                rest = &after[end..];
            }
        }
    }
}

pub(crate) async fn login(l: &Live) -> (JellyfinClient, AuthResponse) {
    let dev = "jr-mac-livetest";
    let (auth, _) = JellyfinClient::authenticate(&l.url, &l.user, &l.pass, dev).await.expect("authenticate");
    let c = JellyfinClient::new(&l.url, auth.access_token.clone(), auth.user.id.clone(), dev).unwrap();
    (c, auth)
}

/// Raw GET (authenticated) returning the JSON text; dumps to JF_LIVE_OUT/<name>.json.
async fn raw_dump(l: &Live, c: &JellyfinClient, path_q: &str, name: &str) -> serde_json::Value {
    let r = reqwest::Client::new()
        .get(format!("{}{}", c.base, path_q))
        .header("X-Emby-Token", &c.token)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{} -> {}", name, r.status());
    let text = r.text().await.unwrap();
    if let Some(o) = &l.out {
        std::fs::write(o.join(format!("{}.json", name)), &text).unwrap();
    }
    serde_json::from_str(&text).unwrap()
}

pub(crate) struct FetchReport {
    pub detail: String,
}

/// Fetch a media URL like a player would. Direct: ranged GET must give 200/206 with
/// non-text media bytes. HLS: master -> variant -> first segment must all fetch.
pub(crate) async fn http_check(url: &str) -> Result<FetchReport, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .unwrap();
    let is_hls = url.split('?').next().unwrap_or("").ends_with(".m3u8");
    if !is_hls {
        let r = http.get(url).header("Range", "bytes=0-65535").send().await.map_err(|e| e.to_string())?;
        let st = r.status().as_u16();
        let ct = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if st != 200 && st != 206 {
            return Err(format!("HTTP {}", st));
        }
        if ct.contains("json") || ct.contains("html") || ct.contains("text") {
            return Err(format!("unexpected content-type {}", ct));
        }
        let mut r = r;
        let chunk = r.chunk().await.map_err(|e| e.to_string())?.unwrap_or_default();
        if chunk.is_empty() {
            return Err("empty body".into());
        }
        return Ok(FetchReport { detail: format!("HTTP {} {} first_chunk={}B", st, ct, chunk.len()) });
    }
    let mut cur = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let mut hops = 0;
    loop {
        let r = http.get(cur.clone()).send().await.map_err(|e| e.to_string())?;
        let st = r.status().as_u16();
        if st != 200 {
            return Err(format!("playlist HTTP {} (hop {})", st, hops));
        }
        let body = r.text().await.map_err(|e| e.to_string())?;
        if !body.starts_with("#EXTM3U") {
            return Err(format!("not an m3u8 (hop {})", hops));
        }
        let first = body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'));
        let first = first.ok_or_else(|| format!("playlist has no entries (hop {})", hops))?;
        let next = cur.join(first).map_err(|e| e.to_string())?;
        if body.contains("#EXT-X-STREAM-INF") && hops < 3 {
            cur = next;
            hops += 1;
            continue;
        }
        // media playlist: fetch init segment (if any) + first segment
        if let Some(m) = body.lines().find(|l| l.starts_with("#EXT-X-MAP")) {
            if let Some(uri) = m.split("URI=\"").nth(1).and_then(|s| s.split('"').next()) {
                let u = cur.join(uri).map_err(|e| e.to_string())?;
                let r = http.get(u).send().await.map_err(|e| e.to_string())?;
                if !r.status().is_success() {
                    return Err(format!("init segment HTTP {}", r.status().as_u16()));
                }
            }
        }
        let mut r = http.get(next).send().await.map_err(|e| e.to_string())?;
        let st = r.status().as_u16();
        if st != 200 && st != 206 {
            return Err(format!("segment HTTP {}", st));
        }
        let chunk = r.chunk().await.map_err(|e| e.to_string())?.unwrap_or_default();
        if chunk.is_empty() {
            return Err("empty first segment".into());
        }
        return Ok(FetchReport { detail: format!("HLS ok ({} playlist hops), first segment {}B+", hops + 1, chunk.len()) });
    }
}

/// AVFoundation check via the compiled tools/avcheck.swift binary (URL on stdin).
pub(crate) fn av_check(url: &str) -> Option<String> {
    let bin = std::env::var("JF_LIVE_AVCHECK").ok()?;
    let mut ch = Command::new(bin).arg("30").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
    ch.stdin.take().unwrap().write_all(format!("{}\n", url).as_bytes()).ok()?;
    let o = ch.wait_with_output().ok()?;
    Some(redact(String::from_utf8_lossy(&o.stdout).trim()))
}

#[tokio::test]
#[ignore]
async fn jf_live_client_endpoints() {
    let Some(l) = live_env() else { return eprintln!("JF_LIVE_* not set; skipping") };
    let _e = TestEnv::new();

    let info = JellyfinClient::public_info(&l.url).await.expect("public_info");
    assert_eq!(info.product_name.as_deref(), Some("Jellyfin Server"));
    eprintln!("server version {:?}", info.version);

    let (c, auth) = login(&l).await;
    assert!(!auth.user.name.is_empty());
    assert_eq!(c.first_user_id().await.is_ok(), true);

    let views = c.views().await.expect("views");
    assert!(!views.is_empty());
    for v in &views {
        eprintln!("view {} ({:?})", v.name, v.collection_type);
    }
    raw_dump(&l, &c, &format!("/Users/{}/Views", c.user_id), "views").await;

    // Same queries the worker sends, per library type present.
    let mut series: Option<ListItem> = None;
    let mut movies: Vec<ListItem> = vec![];
    let mut saw = std::collections::BTreeSet::new();
    for v in &views {
        let ct = v.collection_type.as_deref().unwrap_or("");
        if ct == "livetv" {
            continue;
        }
        saw.insert(ct.to_string());
        let mut q = ItemQuery { parent_id: Some(v.id.clone()), ..Default::default() };
        q.limit = 50;
        match ct {
            "movies" => q.types = Some("Movie".into()),
            "tvshows" => q.types = Some("Series".into()),
            "music" => q.types = Some("MusicAlbum".into()),
            _ => {}
        }
        for (sb, so) in [("SortName", "Ascending"), ("DateCreated", "Descending"), ("ProductionYear", "Descending")] {
            q.sort_by = sb.into();
            q.sort_order = so.into();
            let items = c.items(&q).await.unwrap_or_else(|e| panic!("items {} {}: {}", v.name, sb, e));
            eprintln!("{} sort {} -> {} items", v.name, sb, items.len());
            if sb == "SortName" {
                if ct == "movies" {
                    movies.extend(items.iter().cloned());
                }
                if ct == "tvshows" && series.is_none() {
                    series = items.iter().find(|i| i.item_type == "Series").cloned();
                }
                for it in &items {
                    assert!(!it.id.is_empty() && !it.name.is_empty());
                }
            }
        }
    }
    // Fields actually populated from the real JSON.
    assert!(movies.iter().any(|m| m.year.is_some()), "ProductionYear");
    assert!(movies.iter().any(|m| m.has_image()), "ImageTags.Primary");
    assert!(movies.iter().any(|m| m.run_time_ticks.unwrap_or(0) > 0), "RunTimeTicks");
    assert!(movies.iter().all(|m| m.user_data.is_some()), "UserData");
    assert!(movies.iter().any(|m| m.aspect.is_some()), "PrimaryImageAspectRatio");

    // Search
    let mut sq = ItemQuery { search: Some("the".into()), ..Default::default() };
    sq.limit = 100;
    let found = c.items(&sq).await.expect("search");
    eprintln!("search 'the' -> {}", found.len());
    assert!(!found.is_empty());
    let mut kinds = std::collections::BTreeMap::new();
    for f in &found {
        *kinds.entry(f.item_type.clone()).or_insert(0) += 1;
    }
    eprintln!("search result types: {:?}", kinds);

    // Series episodes
    let s = series.expect("a series");
    let mut eq = ItemQuery { series_id: Some(s.id.clone()), types: Some("Episode".into()), ..Default::default() };
    eq.sort_by = "ParentIndexNumber,IndexNumber".into();
    let eps = c.items(&eq).await.expect("episodes");
    assert!(!eps.is_empty());
    assert!(eps.iter().any(|e| e.season.is_some() && e.episode.is_some() && e.series_name.is_some()));
    eprintln!("episodes of series: {}", eps.len());
    let sub = eps[0].sub_label();
    assert!(sub.contains("S0") || sub.contains('S'), "{}", sub);

    // Album tracks (only if a music library exists)
    if saw.contains("music") {
        let mv = views.iter().find(|v| v.collection_type.as_deref() == Some("music")).unwrap();
        let aq = ItemQuery { parent_id: Some(mv.id.clone()), types: Some("MusicAlbum".into()), ..Default::default() };
        if let Some(al) = c.items(&aq).await.unwrap().first() {
            let tq = ItemQuery { parent_id: Some(al.id.clone()), types: Some("Audio".into()), ..Default::default() };
            assert!(!c.items(&tq).await.unwrap().is_empty());
        }
    } else {
        eprintln!("NOTE: no music library on this server; Tracks query not exercised live");
    }

    // full_item for several movies and episodes
    for it in movies.iter().take(4).chain(eps.iter().take(2)) {
        let f = c.full_item(&it.id).await.unwrap_or_else(|e| panic!("full_item {}: {}", it.name, e));
        assert_eq!(f.id, it.id);
        assert!(f.run_time_ticks.unwrap_or(0) > 0, "RunTimeTicks {}", f.name);
        assert!(f.user_data.is_some());
        let ms = f.media_sources.first().expect("MediaSources");
        assert!(!ms.id.is_empty());
        assert!(ms.container.is_some(), "Container");
        assert!(ms.video_codec().is_some(), "video codec");
        assert!(ms.audio_codec().is_some(), "audio codec");
        eprintln!(
            "full_item {} container={:?} v={:?} a={:?} br={:?}",
            f.name, ms.container, ms.video_codec(), ms.audio_codec(), ms.bitrate
        );
    }
    raw_dump(&l, &c, &format!("/Users/{}/Items/{}", c.user_id, movies[0].id), "full_item_movie").await;
    raw_dump(&l, &c, &format!("/Users/{}/Items/{}", c.user_id, eps[0].id), "full_item_episode").await;

    // Image
    let with_img = movies.iter().find(|m| m.has_image()).unwrap();
    let img = c.image(&with_img.id).await.expect("image");
    assert!(img.len() > 500);
    assert!(img.starts_with(&[0xFF, 0xD8]) || img.starts_with(&[0x89, b'P']) || img.starts_with(b"RIFF"), "not an image");

    // Live TV
    let ch = c.livetv_channels().await.expect("channels");
    eprintln!("live tv channels: {}", ch.len());
    assert!(!ch.is_empty());
    raw_dump(&l, &c, &format!("/LiveTv/Channels?UserId={}&Limit=500", c.user_id), "channels").await;
}

/// Sign-in only, against a real server: public info, password login, then one
/// authenticated read to prove the token is accepted. Used by the CI live-auth job.
/// Read-only; the password is read from JF_LIVE_PASS_FILE and never printed.
///   cargo test -- --ignored live_login --test-threads=1
#[tokio::test]
#[ignore]
async fn jf_live_login() {
    let Some(l) = live_env() else { return eprintln!("JF_LIVE_* not set; skipping") };
    let _e = TestEnv::new();

    let info = JellyfinClient::public_info(&l.url).await.expect("public_info");
    assert_eq!(info.product_name.as_deref(), Some("Jellyfin Server"));

    let (c, auth) = login(&l).await;
    assert!(!auth.user.name.is_empty());
    assert!(!auth.access_token.is_empty());
    let views = c.views().await.expect("authenticated views request");
    eprintln!("signed in to Jellyfin {:?}; {} views", info.version, views.len());
}
