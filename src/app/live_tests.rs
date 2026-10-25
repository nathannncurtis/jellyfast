//! Worker end-to-end against a REAL Jellyfin server (see jellyfin/live_tests.rs for env + rules).
//! Run: cargo test -- --ignored jf_live --test-threads=1

use super::messages::*;
use super::worker::worker;
use crate::config::test_env::TestEnv;
use crate::config::Config;
use crate::jellyfin::live_tests::{av_check, http_check, live_env, redact, Live};
use crate::transcode::decision::PlaybackMethod;
use crate::*;
use std::io::Write;

struct H {
    tx: UnboundedSender<UiCmd>,
    rx: UnboundedReceiver<MainMsg>,
}
impl H {
    fn start() -> H {
        let (tx, crx) = tokio::sync::mpsc::unbounded_channel();
        let (mtx, rx) = tokio::sync::mpsc::unbounded_channel();
        let cfg = Config { device_id: "jr-mac-livetest".into(), ..Default::default() };
        tokio::spawn(worker(crx, mtx, cfg));
        H { tx, rx }
    }
    async fn next(&mut self, secs: u64) -> MainMsg {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(secs), self.rx.recv())
                .await
                .expect("timed out waiting for MainMsg")
                .expect("channel closed");
            if !matches!(m, MainMsg::PosterReady { .. }) {
                return m;
            }
        }
    }
    async fn connect(&mut self, l: &Live) -> (String, Vec<(String, String, Option<String>)>) {
        self.tx
            .send(UiCmd::Connect {
                url: l.url.clone(),
                username: l.user.clone(),
                password: Some(l.pass.clone()),
                api_key: None,
            })
            .unwrap();
        match self.next(60).await {
            MainMsg::Connected { user_name, views, .. } => (user_name, views),
            o => panic!("expected Connected, got {}", redact(&format!("{:?}", o))),
        }
    }
    async fn items(&mut self, cmd: UiCmd) -> (String, Vec<crate::jellyfin::models::ListItem>) {
        self.tx.send(cmd).unwrap();
        match self.next(120).await {
            MainMsg::ItemsLoaded { title, items, .. } => (title, items),
            o => panic!("expected ItemsLoaded, got {}", redact(&format!("{:?}", o))),
        }
    }
}

fn pm(m: PlaybackMethod) -> &'static str {
    match m {
        PlaybackMethod::DirectPlay => "direct",
        PlaybackMethod::ServerHls => "hls",
    }
}

#[tokio::test]
#[ignore]
async fn jf_live_worker_end_to_end() {
    let Some(l) = live_env() else { return eprintln!("JF_LIVE_* not set; skipping") };
    let _e = TestEnv::new();
    let mut h = H::start();
    let (user, views) = h.connect(&l).await;
    assert_eq!(user, l.user, "display name");
    assert_eq!(views.iter().filter(|v| v.2.as_deref() == Some("livetv")).count(), 1, "exactly one Live TV entry");
    eprintln!("connected as {} with {} views", user, views.len());

    let mut movies = vec![];
    let mut series = vec![];
    for (id, name, ct) in &views {
        let (title, items) = h
            .items(UiCmd::LoadView { view_id: id.clone(), name: name.clone(), collection_type: ct.clone() })
            .await;
        eprintln!("view {} -> {} items", title, items.len());
        assert!(!items.is_empty(), "view {} empty", name);
        if name == "Movies" {
            movies = items.clone();
        }
        if ct.as_deref() == Some("tvshows") && series.is_empty() {
            series = items.clone();
        }
    }
    let (t, found) = h.items(UiCmd::Search { q: "green".into() }).await;
    eprintln!("{} -> {}", t, found.len());
    assert!(!found.is_empty());

    let s = &series[0];
    let (_, eps) = h.items(UiCmd::Episodes { series_id: s.id.clone(), series_name: s.name.clone() }).await;
    assert!(!eps.is_empty());
    // Tracks: no music library on this server -> exercise with an empty-but-valid album id.
    let (_, tr) = h.items(UiCmd::Tracks { album_id: movies[0].id.clone(), album_name: "x".into() }).await;
    eprintln!("tracks of non-album: {}", tr.len());

    let mut table = String::from("item\tmethod\thttp\tavplayer\n");
    let mut failures = vec![];
    let targets: Vec<_> = movies.iter().chain(eps.iter().take(3)).collect();
    for it in targets {
        h.tx.send(UiCmd::Play { item_id: it.id.clone() }).unwrap();
        let p = match h.next(60).await {
            MainMsg::PlaybackReady(p) => p,
            o => panic!("Play {} -> {}", it.name, redact(&format!("{:?}", o))),
        };
        assert_eq!(p.item_id, it.id);
        let http = http_check(&p.url).await;
        let av = av_check(&p.url);
        eprintln!(
            "play {:30} {:6} http={} av={}",
            it.name,
            pm(p.method),
            http.as_ref().map(|r| r.detail.clone()).unwrap_or_else(|e| format!("FAIL {}", redact(&e))),
            av.clone().unwrap_or_else(|| "n/a".into())
        );
        table += &format!(
            "{}\t{}\t{}\t{}\n",
            it.name,
            pm(p.method),
            if http.is_ok() { "ok" } else { "FAIL" },
            av.clone().unwrap_or_default()
        );
        if http.is_err() {
            failures.push(format!("{} http {:?}", it.name, http.err()));
        } else if let Some(a) = &av {
            if !a.starts_with("READY") {
                failures.push(format!("{} av {}", it.name, a));
            }
        }
    }
    if let Some(o) = &l.out {
        std::fs::write(o.join("items_table.tsv"), &table).unwrap();
    }
    assert!(failures.is_empty(), "playback failures: {:#?}", failures);
}

/// Every Live TV channel: PlayChannel -> HTTP fetch of the HLS chain -> AVPlayer.
/// Writes JF_LIVE_OUT/channels.tsv. Dead upstream channels are reported, not asserted
/// beyond "the worker answered with PlaybackReady or a PlaybackError".
#[tokio::test]
#[ignore]
async fn jf_live_all_channels() {
    let Some(l) = live_env() else { return eprintln!("JF_LIVE_* not set; skipping") };
    let _e = TestEnv::new();
    let mut h = H::start();
    let (_, views) = h.connect(&l).await;
    let live = views.iter().find(|v| v.2.as_deref() == Some("livetv")).unwrap().clone();
    let (_, chans) = h
        .items(UiCmd::LoadView { view_id: live.0, name: live.1, collection_type: live.2 })
        .await;
    let limit: usize = std::env::var("JF_LIVE_CH_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let skip: usize = std::env::var("JF_LIVE_CH_SKIP").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = match &l.out {
        Some(o) => Some(std::fs::OpenOptions::new().create(true).append(true).open(o.join("channels.tsv")).unwrap()),
        None => None,
    };
    let (mut ok, mut bad) = (0, 0);
    for ch in chans.iter().skip(skip).take(limit) {
        h.tx.send(UiCmd::PlayChannel { channel_id: ch.id.clone(), name: ch.name.clone() }).unwrap();
        let (http, av) = match h.next(90).await {
            MainMsg::PlaybackReady(p) => {
                let http = match http_check(&p.url).await {
                    Ok(r) => format!("ok ({})", r.detail),
                    Err(e) => format!("FAIL {}", redact(&e)),
                };
                let av = av_check(&p.url).unwrap_or_else(|| "n/a".into());
                (http, av)
            }
            MainMsg::PlaybackError(e) => (format!("worker error: {}", redact(&e)), "-".into()),
            o => panic!("unexpected {}", redact(&format!("{:?}", o))),
        };
        let good = http.starts_with("ok") && av.starts_with("READY");
        if good { ok += 1 } else { bad += 1 }
        let line = format!("{}\t{}\t{}\t{}\n", ch.name, if http.starts_with("ok") { "ok" } else { "FAIL" }, av, http);
        eprint!("{}", line);
        if let Some(f) = out.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }
    eprintln!("channels ok={} bad={}", ok, bad);
}
