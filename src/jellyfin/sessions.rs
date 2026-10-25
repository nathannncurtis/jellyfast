//! Play On: Jellyfin sessions this user can control (other devices), and the
//! request paths for starting playback and remote playstate commands there.
//! Everything here is pure: parsing and URL building, no I/O.

use super::urls::{enc, ticks_from_secs};
use serde::Deserialize;

/// Raw `Sessions` entry as the server sends it (only the fields we use).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SessionDto {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub supports_remote_control: Option<bool>,
    #[serde(default)]
    pub now_playing_item: Option<NowPlayingDto>,
    #[serde(default)]
    pub play_state: Option<PlayStateDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NowPlayingDto {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub run_time_ticks: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayStateDto {
    #[serde(default)]
    pub position_ticks: Option<i64>,
    #[serde(default)]
    pub is_paused: bool,
}

/// What the UI needs to know about one controllable session.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteSession {
    pub id: String,
    pub device_name: String,
    pub client: String,
    pub now_playing: Option<NowPlaying>,
    pub position_ticks: i64,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub item_id: String,
    /// Episodes: "Series · Name"; movies: the name.
    pub title: String,
    pub run_time_ticks: i64,
}

impl RemoteSession {
    /// "Living Room · Jellyfin Web": the label used in menus and the remote bar.
    pub fn label(&self) -> String {
        match (self.device_name.trim(), self.client.trim()) {
            ("", "") => "Jellyfin device".to_string(),
            (d, "") => d.to_string(),
            ("", c) => c.to_string(),
            (d, c) => format!("{} · {}", d, c),
        }
    }
}

/// Parses the body of `GET /Sessions`. Drops this app's own session (matched by device id),
/// sessions that cannot take remote control, and entries without an id.
pub fn parse_sessions(body: &str, own_device_id: &str) -> Result<Vec<RemoteSession>, String> {
    let dtos: Vec<SessionDto> = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(dtos
        .into_iter()
        .filter(|d| !d.id.is_empty())
        .filter(|d| d.supports_remote_control.unwrap_or(true))
        .filter(|d| own_device_id.is_empty() || d.device_id != own_device_id)
        .map(to_remote)
        .collect())
}

fn to_remote(d: SessionDto) -> RemoteSession {
    let now_playing = d.now_playing_item.and_then(|np| {
        if np.id.is_empty() {
            return None;
        }
        let title = match np.series_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(series) => format!("{} · {}", series, np.name),
            None => np.name,
        };
        Some(NowPlaying { item_id: np.id, title, run_time_ticks: np.run_time_ticks.unwrap_or(0) })
    });
    let ps = d.play_state.unwrap_or_default();
    RemoteSession {
        id: d.id,
        device_name: d.device_name,
        client: d.client,
        now_playing,
        position_ticks: ps.position_ticks.unwrap_or(0).max(0),
        paused: ps.is_paused,
    }
}

/// Playstate commands sent to a session (`POST /Sessions/{id}/Playing/{command}`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RemoteAction {
    Pause,
    Unpause,
    Stop,
    /// Seek to an absolute position, in seconds.
    SeekSecs(f64),
}

/// Path plus query for a list of sessions controllable by `user_id`.
pub fn sessions_path(user_id: &str) -> (String, Vec<(&'static str, String)>) {
    ("/Sessions".to_string(), vec![("ControllableByUserId", user_id.to_string())])
}

/// `POST /Sessions/{id}/Playing?playCommand=PlayNow&itemIds=…&startPositionTicks=…`
pub fn play_now_path(session_id: &str, item_id: &str, start_secs: f64) -> String {
    format!(
        "/Sessions/{}/Playing?playCommand=PlayNow&itemIds={}&startPositionTicks={}",
        enc(session_id),
        enc(item_id),
        ticks_from_secs(start_secs)
    )
}

/// `POST /Sessions/{id}/Playing/{Command}`, with `seekPositionTicks` for seeks.
pub fn playstate_path(session_id: &str, action: RemoteAction) -> String {
    let base = format!("/Sessions/{}/Playing", enc(session_id));
    match action {
        RemoteAction::Pause => format!("{}/Pause", base),
        RemoteAction::Unpause => format!("{}/Unpause", base),
        RemoteAction::Stop => format!("{}/Stop", base),
        RemoteAction::SeekSecs(s) => {
            format!("{}/Seek?seekPositionTicks={}", base, ticks_from_secs(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"[
      {"Id":"s-tv","DeviceId":"tv-1","DeviceName":"Living Room","Client":"Jellyfin Android TV",
       "UserName":"nathan","SupportsRemoteControl":true,
       "NowPlayingItem":{"Id":"ep1","Name":"Pilot","SeriesName":"Show","RunTimeTicks":27000000000},
       "PlayState":{"PositionTicks":1200000000,"IsPaused":true,"CanSeek":true}},
      {"Id":"s-me","DeviceId":"mac-dev","DeviceName":"Nathan's Mac","Client":"Jellyfast","SupportsRemoteControl":true},
      {"Id":"s-web","DeviceId":"web-1","DeviceName":"Office","Client":"Jellyfin Web","SupportsRemoteControl":true},
      {"Id":"s-no","DeviceId":"x","DeviceName":"Locked","Client":"Other","SupportsRemoteControl":false},
      {"DeviceId":"noid","DeviceName":"NoId","Client":"Other"},
      {"Id":"s-min"}
    ]"#;

    #[test]
    fn parse_keeps_controllable_others_and_maps_fields() {
        let v = parse_sessions(BODY, "mac-dev").unwrap();
        let ids: Vec<&str> = v.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s-tv", "s-web", "s-min"]);
        let tv = &v[0];
        assert_eq!(tv.label(), "Living Room · Jellyfin Android TV");
        assert!(tv.paused);
        assert_eq!(tv.position_ticks, 1_200_000_000);
        let np = tv.now_playing.as_ref().unwrap();
        assert_eq!((np.item_id.as_str(), np.title.as_str(), np.run_time_ticks), ("ep1", "Show · Pilot", 27_000_000_000));
        assert!(v[1].now_playing.is_none());
        assert_eq!(v[2].label(), "Jellyfin device");
    }

    #[test]
    fn own_device_is_excluded_even_when_it_is_the_only_session() {
        let only = r#"[{"Id":"a","DeviceId":"mac-dev","DeviceName":"Mac"}]"#;
        assert!(parse_sessions(only, "mac-dev").unwrap().is_empty());
        assert_eq!(parse_sessions(only, "").unwrap().len(), 1);
    }

    #[test]
    fn empty_and_malformed_bodies() {
        assert!(parse_sessions("[]", "x").unwrap().is_empty());
        assert!(parse_sessions("{}", "x").is_err());
        assert!(parse_sessions("not json", "x").is_err());
    }

    #[test]
    fn sessions_query_names_the_user() {
        let (p, q) = sessions_path("u 1");
        assert_eq!(p, "/Sessions");
        assert_eq!(q, vec![("ControllableByUserId", "u 1".to_string())]);
    }

    #[test]
    fn play_now_url_has_command_items_and_start() {
        assert_eq!(
            play_now_path("s/1", "item-9", 90.5),
            "/Sessions/s%2F1/Playing?playCommand=PlayNow&itemIds=item-9&startPositionTicks=905000000"
        );
        assert_eq!(
            play_now_path("s", "i", 0.0),
            "/Sessions/s/Playing?playCommand=PlayNow&itemIds=i&startPositionTicks=0"
        );
    }

    #[test]
    fn playstate_urls() {
        assert_eq!(playstate_path("s-1", RemoteAction::Pause), "/Sessions/s-1/Playing/Pause");
        assert_eq!(playstate_path("s-1", RemoteAction::Unpause), "/Sessions/s-1/Playing/Unpause");
        assert_eq!(playstate_path("s-1", RemoteAction::Stop), "/Sessions/s-1/Playing/Stop");
        assert_eq!(
            playstate_path("s-1", RemoteAction::SeekSecs(2.0)),
            "/Sessions/s-1/Playing/Seek?seekPositionTicks=20000000"
        );
    }
}
