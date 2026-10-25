use crate::*;

use super::urls as u;
use super::models::*;
use super::sessions::{parse_sessions, play_now_path, playstate_path, sessions_path, RemoteAction, RemoteSession};
use crate::jellyfin::urls::enc;

/// `Fields` requested for the item detail page.
const DETAIL_FIELDS: &str =
    "People,Genres,Overview,CommunityRating,OfficialRating,ProductionYear,RunTimeTicks,BackdropImageTags";

#[derive(Debug)]
pub enum JFError {
    BadUrl(String),
    Network(String),   // connect/timeout/DNS
    NotJellyfin,       // reachable but not a Jellyfin server
    Auth(String),      // 401/403
    Api(u16, String),  // other HTTP status
    Parse(String),
}
impl fmt::Display for JFError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JFError::BadUrl(s) => write!(f, "Invalid server URL: {}", s),
            JFError::Network(s) => write!(
                f,
                "Network problem: {}. Check the address, your connection, and that the server is running.",
                s
            ),
            JFError::NotJellyfin => write!(
                f,
                "That address responded, but it is not a Jellyfin server (or the path is wrong)."
            ),
            JFError::Auth(s) => write!(f, "Sign-in failed: {}", s),
            JFError::Api(c, s) => write!(f, "Server error (HTTP {}): {}", c, s),
            JFError::Parse(s) => write!(f, "Unexpected server response: {}", s),
        }
    }
}

/// When set, playback reports and played-state changes are skipped. Used by
/// live-server tests so a real server's user data is never modified.
static READ_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_read_only(v: bool) {
    READ_ONLY.store(v, std::sync::atomic::Ordering::SeqCst);
}
fn read_only() -> bool {
    READ_ONLY.load(std::sync::atomic::Ordering::SeqCst)
}

#[derive(Clone)]
pub struct JellyfinClient {
    http: reqwest::Client,
    pub base: String,
    pub token: String,
    pub user_id: String,
    device_id: String,
}

fn map_err(e: reqwest::Error) -> JFError {
    if e.is_timeout() {
        JFError::Network("request timed out".into())
    } else if e.is_connect() {
        JFError::Network(format!("could not connect ({})", e))
    } else {
        JFError::Network(e.to_string())
    }
}

/// Fields requested for every home-screen list.
const HOME_FIELDS: &str = "PrimaryImageAspectRatio,ProductionYear,UserData,SeriesName,DateCreated";

impl JellyfinClient {
    pub fn new(
        base: &str,
        token: String,
        user_id: String,
        device_id: &str,
    ) -> Result<Self, JFError> {
        let base = u::normalize_base_url(base).map_err(JFError::BadUrl)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent("Jellyfast/0.1.0")
            .build()
            .map_err(|e| JFError::Network(e.to_string()))?;
        Ok(Self { http, base, token, user_id, device_id: device_id.to_string() })
    }

    fn auth_header(&self) -> String {
        format!(
            "MediaBrowser Client=\"Jellyfast\", Device=\"Mac\", DeviceId=\"{}\", Version=\"0.1.0\"",
            enc(&self.device_id)
        )
    }

    /// Unauthenticated probe: verifies the URL points at Jellyfin.
    pub async fn public_info(base: &str) -> Result<ServerInfo, JFError> {
        let b = u::normalize_base_url(base).map_err(JFError::BadUrl)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| JFError::Network(e.to_string()))?;
        let r = http
            .get(format!("{}/System/Info/Public", b))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(map_err)?;
        let code = r.status();
        if !code.is_success() {
            return Err(JFError::Api(code.as_u16(), format!("GET /System/Info/Public")));
        }
        let text = r.text().await.map_err(|e| JFError::Network(e.to_string()))?;
        let info: ServerInfo = serde_json::from_str(&text).map_err(|_| JFError::NotJellyfin)?;
        if info.product_name.as_deref().unwrap_or("").is_empty() {
            return Err(JFError::NotJellyfin);
        }
        Ok(info)
    }

    /// Resolve a user-typed server address to a working base URL. Bare hosts
    /// try https first and fall back to http when the connection fails; the
    /// first candidate that answers `/System/Info/Public` wins. Only transport
    /// failures fall through: a reachable server that is not Jellyfin, or
    /// rejects the request, is reported as is.
    pub async fn resolve(input: &str) -> Result<(String, ServerInfo), JFError> {
        let candidates = u::candidate_bases(input).map_err(JFError::BadUrl)?;
        let mut last_network = None;
        for base in candidates {
            match Self::public_info(&base).await {
                Ok(info) => return Ok((base, info)),
                Err(e @ JFError::Network(_)) => last_network = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last_network.unwrap_or_else(|| JFError::BadUrl("no address to try".into())))
    }

    /// Username/password sign-in → token + user id.
    pub async fn authenticate(
        base: &str,
        username: &str,
        password: &str,
        device_id: &str,
    ) -> Result<(AuthResponse, String), JFError> {
        let b = u::normalize_base_url(base).map_err(JFError::BadUrl)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| JFError::Network(e.to_string()))?;
        let r = http
            .post(format!("{}/Users/AuthenticateByName", b))
            .header(
                "Authorization",
                format!(
                    "MediaBrowser Client=\"Jellyfast\", Device=\"Mac\", DeviceId=\"{}\", Version=\"0.1.0\"",
                    enc(device_id)
                ),
            )
            .json(&AuthRequest { username, pw: password })
            .send()
            .await
            .map_err(map_err)?;
        let code = r.status();
        match code.as_u16() {
            401 => return Err(JFError::Auth("wrong username or password".into())),
            403 => return Err(JFError::Auth("access denied by server".into())),
            _ => {}
        }
        if !code.is_success() {
            return Err(JFError::Api(code.as_u16(), "AuthenticateByName".into()));
        }
        let resp: AuthResponse = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        Ok((resp, b.to_string()))
    }

    /// API-key path: token is admin-scoped; pick the first user.
    pub async fn first_user_id(&self) -> Result<String, JFError> {
        #[derive(Deserialize)]
        struct U {
            #[serde(rename = "Id")]
            id: String,
        }
        let r = self
            .http
            .get(format!("{}/Users", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let users: Vec<U> = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        users
            .into_iter()
            .next()
            .map(|u| u.id)
            .ok_or_else(|| JFError::Auth("no user accounts found".into()))
    }

    pub async fn views(&self) -> Result<Vec<View>, JFError> {
        #[derive(Deserialize)]
        struct R {
            #[serde(rename = "Items")]
            items: Vec<View>,
        }
        let r = self
            .http
            .get(format!("{}/Users/{}/Views", self.base, enc(&self.user_id)))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        Ok(v.items)
    }

    pub async fn items(&self, q: &ItemQuery) -> Result<Vec<ListItem>, JFError> {
        #[derive(Deserialize)]
        struct R {
            #[serde(rename = "Items")]
            items: Vec<ListItem>,
        }
        let url = format!(
            "{}/Users/{}/Items",
            self.base,
            enc(&self.user_id)
        );
        let mut pairs: Vec<(&str, String)> = vec![
            ("Recursive", "true".into()),
            ("Fields", "PrimaryImageAspectRatio,ProductionYear,UserData,Overview".into()),
            ("ImageTypeLimit", "1".into()),
            ("Limit", q.limit.to_string()),
            ("SortBy", q.sort_by.clone()),
            ("SortOrder", q.sort_order.clone()),
        ];
        if let Some(p) = &q.parent_id {
            pairs.push(("ParentId", p.clone()));
        }
        if let Some(s) = &q.series_id {
            pairs.push(("ParentId", s.clone()));
        }
        if let Some(t) = &q.types {
            pairs.push(("IncludeItemTypes", t.clone()));
        }
        if let Some(s) = &q.search {
            pairs.push(("SearchTerm", s.clone()));
        }
        if !q.filters.is_empty() {
            pairs.push(("Filters", q.filters.join(",")));
        }
        let r = self
            .http
            .get(url)
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .query(&pairs)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        Ok(v.items)
    }

    pub async fn full_item(&self, id: &str) -> Result<FullItem, JFError> {
        let r = self
            .http
            .get(format!(
                "{}/Users/{}/Items/{}",
                self.base,
                enc(&self.user_id),
                enc(id)
            ))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        r.json().await.map_err(|e| JFError::Parse(e.to_string()))
    }

    /// Item detail with the fields the detail page shows (cast, genres, overview, ratings).
    pub async fn item_detail(&self, id: &str) -> Result<DetailItem, JFError> {
        let r = self
            .http
            .get(format!("{}/Users/{}/Items/{}", self.base, enc(&self.user_id), enc(id)))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .query(&[("Fields", DETAIL_FIELDS)])
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        r.json().await.map_err(|e| JFError::Parse(e.to_string()))
    }

    /// The seasons of a series, in the server's order.
    pub async fn seasons(&self, series_id: &str) -> Result<Vec<Season>, JFError> {
        let path = format!("/Shows/{}/Seasons", enc(series_id));
        let page: ItemsPage<Season> = self.get_json(&path, &[("UserId", self.user_id.clone())]).await?;
        Ok(page.items)
    }

    /// The episodes of one season, in episode order, with their watched state.
    pub async fn season_episodes(&self, series_id: &str, season_id: &str) -> Result<Vec<Episode>, JFError> {
        let path = format!("/Shows/{}/Episodes", enc(series_id));
        let query = [
            ("UserId", self.user_id.clone()),
            ("seasonId", season_id.to_string()),
            ("Fields", "Overview".to_string()),
        ];
        let page: ItemsPage<Episode> = self.get_json(&path, &query).await?;
        Ok(page.items)
    }

    /// The episode the user should watch next in a series, if the server has one.
    pub async fn series_next_up(&self, series_id: &str) -> Result<Option<Episode>, JFError> {
        let path = "/Shows/NextUp";
        let query = [
            ("UserId", self.user_id.clone()),
            ("SeriesId", series_id.to_string()),
            ("Limit", "1".to_string()),
        ];
        let page: ItemsPage<Episode> = self.get_json(path, &query).await?;
        Ok(page.items.into_iter().next())
    }

    /// Skip-button ranges of an item. A 404 means the server has none for it (older
    /// servers, or nothing analysed yet), which is an empty list, not an error.
    pub async fn media_segments(&self, item_id: &str) -> Result<Vec<MediaSegment>, JFError> {
        let path = format!("/MediaSegments/{}", enc(item_id));
        match self.get_json::<ItemsPage<MediaSegment>>(&path, &[]).await {
            Ok(page) => Ok(page.items),
            Err(JFError::Api(404, _)) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Every episode of a series in one list, all seasons and specials, for Up Next.
    pub async fn series_episodes(&self, series_id: &str) -> Result<Vec<Episode>, JFError> {
        let path = format!("/Shows/{}/Episodes", enc(series_id));
        let page: ItemsPage<Episode> = self.get_json(&path, &[("UserId", self.user_id.clone())]).await?;
        Ok(page.items)
    }

    pub async fn image(&self, item_id: &str) -> Result<Vec<u8>, JFError> {
        let url = u::build_image_url(&self.base, item_id, &self.token, 400);
        let r = self.http.get(url).timeout(Duration::from_secs(30)).send().await.map_err(map_err)?;
        let r = check(r).await?;
        r.bytes().await.map(|b| b.to_vec()).map_err(|e| JFError::Network(e.to_string()))
    }

    /// GET a JSON endpoint with the standard auth headers. `pairs` are raw
    /// values; reqwest percent-encodes them.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        pairs: &[(&str, String)],
    ) -> Result<T, JFError> {
        let r = self
            .http
            .get(format!("{}{}", self.base, path))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .query(pairs)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        r.json().await.map_err(|e| JFError::Parse(e.to_string()))
    }

    /// Continue Watching: in-progress movies and episodes, most recent first.
    pub async fn resume_items(&self, limit: u32) -> Result<Vec<ListItem>, JFError> {
        self.resume_items_in(None, limit).await
    }

    /// Continue Watching limited to one library (`parent` is a view id).
    pub async fn resume_items_in(&self, parent: Option<&str>, limit: u32) -> Result<Vec<ListItem>, JFError> {
        #[derive(Deserialize)]
        struct R {
            #[serde(rename = "Items", default)]
            items: Vec<ListItem>,
        }
        let path = format!("/Users/{}/Items/Resume", enc(&self.user_id));
        let mut pairs = vec![
            ("Limit", limit.to_string()),
            ("MediaTypes", "Video".to_string()),
            ("Fields", HOME_FIELDS.to_string()),
            ("ImageTypeLimit", "1".to_string()),
        ];
        if let Some(p) = parent {
            pairs.push(("ParentId", p.to_string()));
        }
        let v: R = self.get_json(&path, &pairs).await?;
        Ok(v.items)
    }

    /// Next Up: the next unwatched episode of each series in progress.
    pub async fn next_up(&self, limit: u32) -> Result<Vec<ListItem>, JFError> {
        self.next_up_in(None, limit).await
    }

    /// Next Up limited to one library (`parent` is a view id).
    pub async fn next_up_in(&self, parent: Option<&str>, limit: u32) -> Result<Vec<ListItem>, JFError> {
        #[derive(Deserialize)]
        struct R {
            #[serde(rename = "Items", default)]
            items: Vec<ListItem>,
        }
        let mut pairs = vec![
            ("UserId", self.user_id.clone()),
            ("Limit", limit.to_string()),
            ("Fields", HOME_FIELDS.to_string()),
            ("ImageTypeLimit", "1".to_string()),
        ];
        if let Some(p) = parent {
            pairs.push(("ParentId", p.to_string()));
        }
        let v: R = self.get_json("/Shows/NextUp", &pairs).await?;
        Ok(v.items)
    }

    /// Recently added items inside one library (`parent_id` is a view id).
    /// The endpoint answers with a bare array, not an `Items` envelope.
    pub async fn latest_items(&self, parent_id: &str, limit: u32) -> Result<Vec<ListItem>, JFError> {
        let path = format!("/Users/{}/Items/Latest", enc(&self.user_id));
        let pairs = [
            ("ParentId", parent_id.to_string()),
            ("Limit", limit.to_string()),
            ("Fields", HOME_FIELDS.to_string()),
            ("ImageTypeLimit", "1".to_string()),
        ];
        self.get_json(&path, &pairs).await
    }

    /// Movie recommendation groups (similar to recent plays, liked people, ...).
    pub async fn recommendations(&self, limit: u32) -> Result<Vec<RecommendationDto>, JFError> {
        self.recommendations_in(None, limit).await
    }

    /// Movie recommendations limited to one library (`parent` is a view id).
    pub async fn recommendations_in(&self, parent: Option<&str>, limit: u32) -> Result<Vec<RecommendationDto>, JFError> {
        let mut pairs = vec![
            ("UserId", self.user_id.clone()),
            ("CategoryLimit", "5".to_string()),
            ("ItemLimit", limit.to_string()),
        ];
        if let Some(p) = parent {
            pairs.push(("ParentId", p.to_string()));
        }
        self.get_json("/Movies/Recommendations", &pairs).await
    }

    pub async fn livetv_channels(&self) -> Result<Vec<ListItem>, JFError> {
        #[derive(Deserialize)]
        struct R {
            #[serde(rename = "Items")]
            items: Vec<ListItem>,
        }
        let r = self
            .http
            .get(format!("{}/LiveTv/Channels", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            // reqwest encodes query values itself; pass the raw id here.
            .query(&[("UserId", self.user_id.as_str()), ("Limit", "500")])
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let v: R = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        Ok(v.items)
    }

    /// POST /Items/{id}/PlaybackInfo with an AVFoundation-oriented device
    /// profile. For live TV channels this opens the live stream (returns a
    /// `LiveStreamId` the caller should close) and yields an HLS
    /// `TranscodingUrl` that AVPlayer can play (raw MPEG-TS cannot be).
    pub async fn playback_info(&self, item_id: &str, max_bitrate_bps: u64) -> Result<PlaybackInfo, JFError> {
        let max = if max_bitrate_bps > 0 { max_bitrate_bps } else { 120_000_000 };
        let body = serde_json::json!({
            "DeviceProfile": {
                "Name": "JellyfinMac",
                "MaxStreamingBitrate": max,
                "DirectPlayProfiles": [
                    {"Container": "mp4,m4v,mov", "Type": "Video",
                     "VideoCodec": "h264,hevc", "AudioCodec": "aac,mp3,ac3,eac3,alac,flac"},
                    {"Container": "mp3,aac,m4a,flac,wav", "Type": "Audio"}
                ],
                "TranscodingProfiles": [
                    {"Container": "ts", "Type": "Video", "Protocol": "hls", "Context": "Streaming",
                     "VideoCodec": "h264,hevc", "AudioCodec": "aac,mp3,ac3,eac3",
                     "MaxAudioChannels": "2", "MinSegments": "1", "BreakOnNonKeyFrames": true,
                     "SegmentLength": 6}
                ],
                "CodecProfiles": [],
                "SubtitleProfiles": []
            },
            "AutoOpenLiveStream": true
        });
        let r = self
            .http
            .post(format!("{}/Items/{}/PlaybackInfo", self.base, enc(item_id)))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .timeout(Duration::from_secs(45))
            .query(&[
                ("UserId", self.user_id.as_str()),
                ("StartTimeTicks", "0"),
                ("IsPlayback", "true"),
                ("AutoOpenLiveStream", "true"),
                ("MaxStreamingBitrate", &max.to_string()),
            ])
            .json(&body)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let info: PlaybackInfo = r.json().await.map_err(|e| JFError::Parse(e.to_string()))?;
        if let Some(code) = &info.error_code {
            return Err(JFError::Api(400, format!("PlaybackInfo: {}", code)));
        }
        Ok(info)
    }

    /// Best-effort release of a live stream opened by `playback_info`.
    pub async fn close_live_stream(&self, live_stream_id: &str) {
        let _ = self
            .http
            .post(format!("{}/LiveStreams/Close", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .query(&[("liveStreamId", live_stream_id)])
            .send()
            .await;
    }

    /// Absolute playable URL for a server-relative TranscodingUrl, with the
    /// token appended if the server did not embed one.
    pub fn absolute_media_url(&self, rel: &str) -> String {
        let mut u = if rel.starts_with("http") { rel.to_string() } else { format!("{}{}", self.base, rel) };
        let l = u.to_ascii_lowercase();
        if !l.contains("api_key=") && !l.contains("apikey=") {
            u.push(if u.contains('?') { '&' } else { '?' });
            u.push_str(&format!("api_key={}", enc(&self.token)));
        }
        u
    }

    /// `play_method` is Jellyfin's PlayMethod: "DirectPlay", "DirectStream" or "Transcode".
    pub async fn report_start(&self, item_id: &str, ticks: i64, play_method: &str) {
        if read_only() {
            return;
        }
        let _ = self
            .http
            .post(format!("{}/Sessions/Playing", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .json(&serde_json::json!({
                "ItemId": item_id, "PositionTicks": ticks, "IsPaused": false,
                "PlayMethod": play_method
            }))
            .send()
            .await;
    }
    pub async fn report_progress(&self, item_id: &str, ticks: i64, paused: bool) {
        if read_only() {
            return;
        }
        let _ = self
            .http
            .post(format!("{}/Sessions/Playing/Progress", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .json(&serde_json::json!({
                "ItemId": item_id, "PositionTicks": ticks.max(0), "IsPaused": paused
            }))
            .send()
            .await;
    }
    pub async fn report_stopped(&self, item_id: &str, ticks: i64) {
        if read_only() {
            return;
        }
        let _ = self
            .http
            .post(format!("{}/Sessions/Playing/Stopped", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .json(&serde_json::json!({ "ItemId": item_id, "PositionTicks": ticks.max(0) }))
            .send()
            .await;
    }
    /// This app's device id. Its own session is never offered for Play On.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Other devices this user can control: `GET /Sessions?ControllableByUserId=`.
    pub async fn controllable_sessions(&self) -> Result<Vec<RemoteSession>, JFError> {
        let (path, pairs) = sessions_path(&self.user_id);
        let r = self
            .http
            .get(format!("{}{}", self.base, path))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .query(&pairs)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
        let body = r.text().await.map_err(map_err)?;
        parse_sessions(&body, &self.device_id).map_err(JFError::Parse)
    }

    /// Play On: start `item_id` on another session from `start_secs` (`PlayNow`).
    /// Read-only mode sends nothing.
    pub async fn play_on_session(&self, session_id: &str, item_id: &str, start_secs: f64) -> Result<(), JFError> {
        if read_only() {
            return Ok(());
        }
        self.post_session(&play_now_path(session_id, item_id, start_secs)).await
    }

    /// Remote playstate command (pause, unpause, stop, seek) for another session.
    /// Read-only mode sends nothing.
    pub async fn remote_control(&self, session_id: &str, action: RemoteAction) -> Result<(), JFError> {
        if read_only() {
            return Ok(());
        }
        self.post_session(&playstate_path(session_id, action)).await
    }

    async fn post_session(&self, path: &str) -> Result<(), JFError> {
        let r = self
            .http
            .post(format!("{}{}", self.base, path))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        check(r).await?;
        Ok(())
    }

    pub async fn set_played(&self, item_id: &str, played: bool) -> Result<(), JFError> {
        if read_only() {
            return Ok(());
        }
        let m = if played { reqwest::Method::POST } else { reqwest::Method::DELETE };
        let r = self
            .http
            .request(
                m,
                format!(
                    "{}/Users/{}/PlayedItems/{}",
                    self.base,
                    enc(&self.user_id),
                    enc(item_id),
                ),
            )
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        check(r).await?;
        Ok(())
    }
}

async fn check(r: reqwest::Response) -> Result<reqwest::Response, JFError> {
    let code = r.status();
    if code.as_u16() == 401 {
        return Err(JFError::Auth("token expired or revoked — sign in again".into()));
    }
    if code.as_u16() == 403 {
        return Err(JFError::Auth("access denied by server".into()));
    }
    if !code.is_success() {
        return Err(JFError::Api(code.as_u16(), String::new()));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    // ------------------------------------------------------------------
    // Mock HTTP server: records every request, replies with canned bytes.
    // ------------------------------------------------------------------

    #[derive(Clone, Debug)]
    struct Recorded {
        method: String,
        /// Raw (still percent-encoded) path, without the query string.
        path: String,
        /// Query pairs, percent-decoded, in wire order.
        query: Vec<(String, String)>,
        /// Header names lowercased.
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Recorded {
        fn header(&self, name: &str) -> Option<&str> {
            let n = name.to_ascii_lowercase();
            self.headers.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str())
        }
        fn qp(&self, key: &str) -> Option<&str> {
            self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
        }
        fn body_json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).expect("request body should be JSON")
        }
    }

    struct Canned {
        status: u16,
        body: Vec<u8>,
    }
    fn reply(status: u16, body: &str) -> Canned {
        Canned { status, body: body.as_bytes().to_vec() }
    }
    fn status_only(status: u16) -> Canned {
        reply(status, "")
    }

    struct Mock {
        base: String,
        log: Arc<Mutex<Vec<Recorded>>>,
    }
    impl Mock {
        fn requests(&self) -> Vec<Recorded> {
            self.log.lock().unwrap().clone()
        }
        fn last(&self) -> Recorded {
            self.requests().pop().expect("mock server saw no request")
        }
        fn count(&self) -> usize {
            self.log.lock().unwrap().len()
        }
    }

    fn pct_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b'%' if i + 2 < b.len() => {
                    let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                    match u8::from_str_radix(hex, 16) {
                        Ok(v) => {
                            out.push(v);
                            i += 3;
                        }
                        Err(_) => {
                            out.push(b[i]);
                            i += 1;
                        }
                    }
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn parse_target(target: &str) -> (String, Vec<(String, String)>) {
        let (path, q) = target.split_once('?').unwrap_or((target, ""));
        let query = q
            .split('&')
            .filter(|s| !s.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (pct_decode(k), pct_decode(v))
            })
            .collect();
        (path.to_string(), query)
    }

    fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    async fn read_request(sock: &mut TcpStream) -> Option<Recorded> {
        let mut buf: Vec<u8> = Vec::new();
        let head_end = loop {
            if let Some(pos) = find_subseq(&buf, b"\r\n\r\n") {
                break pos;
            }
            let mut chunk = [0u8; 1024];
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut parts = lines.next()?.split(' ');
        let method = parts.next()?.to_string();
        let (path, query) = parse_target(parts.next()?);
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let content_len = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = buf[head_end + 4..].to_vec();
        while body.len() < content_len {
            let mut chunk = [0u8; 1024];
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        Some(Recorded {
            method,
            path,
            query,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    async fn serve<F>(handler: F) -> Mock
    where
        F: Fn(&Recorded) -> Canned + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log: Arc<Mutex<Vec<Recorded>>> = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log_task = log.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let handler = handler.clone();
                let log = log_task.clone();
                tokio::spawn(async move {
                    let Some(req) = read_request(&mut sock).await else { return };
                    let canned = (*handler)(&req);
                    log.lock().unwrap().push(req);
                    let head = format!(
                        "HTTP/1.1 {} MOCK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        canned.status,
                        canned.body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&canned.body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Mock { base: format!("http://{}", addr), log }
    }

    /// A base URL with nothing listening (bind, learn the port, close it).
    fn refused_base() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{}", addr)
    }

    const TOKEN: &str = "tok-123";
    const USER: &str = "5f1e2d3c4b5a69788796a5b4c3d2e1f0";
    const DEVICE: &str = "dev-abc-123";

    fn client(base: &str) -> JellyfinClient {
        JellyfinClient::new(base, TOKEN.into(), USER.into(), DEVICE).unwrap()
    }

    fn err<T>(r: Result<T, JFError>) -> JFError {
        match r {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(e) => e,
        }
    }

    fn assert_token_auth(r: &Recorded) {
        assert_eq!(r.header("X-Emby-Token"), Some(TOKEN), "X-Emby-Token");
        let auth = r.header("Authorization").expect("Authorization header missing");
        assert!(auth.starts_with("MediaBrowser "), "bad scheme: {auth}");
        assert!(auth.contains("Client=\"Jellyfast\""), "{auth}");
        assert!(auth.contains(&format!("DeviceId=\"{}\"", DEVICE)), "DeviceId wrong: {auth}");
    }

    // Real-shaped Jellyfin 10.x payloads.
    const PUBLIC_INFO_JSON: &str = r#"{"LocalAddress":"http://192.168.1.10:8096","ServerName":"Home","Version":"10.9.11","ProductName":"Jellyfin Server","OperatingSystem":"Linux","Id":"8a1b2c3d4e5f60718293a4b5c6d7e8f9","StartupWizardCompleted":true}"#;

    const AUTH_JSON: &str = r#"{"User":{"Name":"nathan","ServerId":"srv1","Id":"5f1e2d3c4b5a69788796a5b4c3d2e1f0","HasPassword":true,"HasConfiguredPassword":true,"EnableAutoLogin":false,"Policy":{"IsAdministrator":true}},"SessionInfo":{"Id":"sess1","PlayState":{}},"AccessToken":"tok-xyz","ServerId":"srv1"}"#;

    const VIEWS_JSON: &str = r#"{"Items":[{"Name":"Movies","ServerId":"srv1","Id":"v1","CollectionType":"movies","Type":"CollectionFolder"},{"Name":"Mixed","Id":"v2","Type":"UserView"}],"TotalRecordCount":2,"StartIndex":0}"#;

    const ITEMS_JSON: &str = r#"{"Items":[{"Name":"Pilot","ServerId":"srv1","Id":"ep1","SeriesName":"Show","SeriesId":"s1","SeasonId":"se1","Type":"Episode","ParentIndexNumber":1,"IndexNumber":2,"ProductionYear":2019,"RunTimeTicks":27000000000,"PrimaryImageAspectRatio":1.7777,"ImageTags":{"Primary":"abc"},"UserData":{"PlaybackPositionTicks":1000000,"PlayCount":0,"IsFavorite":false,"Played":false}},{"Name":"Minimal","Id":"m1"}],"TotalRecordCount":2,"StartIndex":0}"#;

    const FULL_ITEM_JSON: &str = r#"{"Name":"Movie","ServerId":"srv1","Id":"mv1","Type":"Movie","RunTimeTicks":7200000000,"UserData":{"PlaybackPositionTicks":0,"Played":true},"MediaSources":[{"Protocol":"File","Id":"ms1","Path":"/media/movie.mkv","Type":"Default","Container":"mkv","Bitrate":8000000,"MediaStreams":[{"Codec":"h264","Type":"Video","Index":0,"DisplayTitle":"1080p H264","Language":null},{"Codec":"dts","Type":"Audio","Index":1,"Language":"eng","DisplayTitle":"English DTS 5.1"},{"Codec":"srt","Type":"Subtitle","Index":2,"Language":"eng"}]}]}"#;

    const LIVETV_JSON: &str = r#"{"Items":[{"Name":"BBC One","ServerId":"srv1","Id":"ch1","Type":"TvChannel","Number":"1"}],"TotalRecordCount":1,"StartIndex":0}"#;

    // ------------------------------------------------------------------
    // Public info (unauthenticated probe)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn public_info_parses_real_shape_and_sends_no_auth() {
        let m = serve(|_| reply(200, PUBLIC_INFO_JSON)).await;
        let info = JellyfinClient::public_info(&m.base).await.unwrap();
        assert_eq!(info.product_name.as_deref(), Some("Jellyfin Server"));
        assert_eq!(info.version.as_deref(), Some("10.9.11"));
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/System/Info/Public");
        assert!(r.header("X-Emby-Token").is_none());
        assert!(r.header("Authorization").is_none());
    }

    #[tokio::test]
    async fn public_info_missing_optional_fields_still_parses() {
        let m = serve(|_| reply(200, r#"{"ProductName":"Jellyfin Server"}"#)).await;
        let info = JellyfinClient::public_info(&m.base).await.unwrap();
        assert_eq!(info.version, None);
    }

    #[tokio::test]
    async fn public_info_empty_product_name_is_not_jellyfin() {
        let m = serve(|_| reply(200, r#"{"ProductName":"","Version":"1"}"#)).await;
        assert!(matches!(err(JellyfinClient::public_info(&m.base).await), JFError::NotJellyfin));
    }

    #[tokio::test]
    async fn public_info_non_json_body_is_not_jellyfin() {
        let m = serve(|_| reply(200, "<html>hello</html>")).await;
        assert!(matches!(err(JellyfinClient::public_info(&m.base).await), JFError::NotJellyfin));
    }

    #[tokio::test]
    async fn public_info_404_is_api_error() {
        let m = serve(|_| status_only(404)).await;
        assert!(matches!(err(JellyfinClient::public_info(&m.base).await), JFError::Api(404, _)));
    }

    #[tokio::test]
    async fn public_info_500_is_api_error() {
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(JellyfinClient::public_info(&m.base).await), JFError::Api(500, _)));
    }

    #[tokio::test]
    async fn public_info_connection_refused_is_network() {
        let e = err(JellyfinClient::public_info(&refused_base()).await);
        assert!(matches!(e, JFError::Network(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn public_info_rejects_bad_scheme_before_network() {
        let e = err(JellyfinClient::public_info("ftp://nope").await);
        assert!(matches!(e, JFError::BadUrl(_)), "got {e:?}");
    }

    // ------------------------------------------------------------------
    // Authenticate (username/password)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn authenticate_posts_credentials_and_parses_response() {
        let m = serve(|_| reply(200, AUTH_JSON)).await;
        let (resp, base) =
            JellyfinClient::authenticate(&format!("{}/", m.base), "nathan", "hunter2", DEVICE)
                .await
                .unwrap();
        assert_eq!(resp.access_token, "tok-xyz");
        assert_eq!(resp.user.id, USER);
        assert_eq!(resp.user.name, "nathan");
        assert_eq!(base, m.base, "trailing slash should be normalized away");

        let r = m.last();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/Users/AuthenticateByName");
        assert_eq!(r.body_json(), serde_json::json!({"Username": "nathan", "Pw": "hunter2"}));
        assert!(r.header("content-type").unwrap_or("").contains("json"));
        let auth = r.header("Authorization").unwrap();
        assert!(auth.contains(&format!("DeviceId=\"{}\"", DEVICE)), "{auth}");
        assert!(r.header("X-Emby-Token").is_none(), "no token before sign-in");
    }

    #[tokio::test]
    async fn authenticate_tolerates_missing_optional_user_fields() {
        let m = serve(|_| {
            reply(200, r#"{"User":{"Name":"u","Id":"id1"},"AccessToken":"t"}"#)
        })
        .await;
        let (resp, _) = JellyfinClient::authenticate(&m.base, "u", "p", DEVICE).await.unwrap();
        assert_eq!(resp.access_token, "t");
        assert_eq!(resp.user.id, "id1");
    }

    #[tokio::test]
    async fn authenticate_401_is_auth() {
        let m = serve(|_| status_only(401)).await;
        let e = err(JellyfinClient::authenticate(&m.base, "u", "bad", DEVICE).await);
        assert!(matches!(e, JFError::Auth(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn authenticate_403_is_auth() {
        let m = serve(|_| status_only(403)).await;
        let e = err(JellyfinClient::authenticate(&m.base, "u", "p", DEVICE).await);
        assert!(matches!(e, JFError::Auth(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn authenticate_500_is_api() {
        let m = serve(|_| status_only(500)).await;
        let e = err(JellyfinClient::authenticate(&m.base, "u", "p", DEVICE).await);
        assert!(matches!(e, JFError::Api(500, _)), "got {e:?}");
    }

    #[tokio::test]
    async fn authenticate_malformed_json_is_parse() {
        let m = serve(|_| reply(200, "{not json")).await;
        let e = err(JellyfinClient::authenticate(&m.base, "u", "p", DEVICE).await);
        assert!(matches!(e, JFError::Parse(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn authenticate_missing_access_token_is_parse() {
        let m = serve(|_| reply(200, r#"{"User":{"Name":"u","Id":"id1"}}"#)).await;
        let e = err(JellyfinClient::authenticate(&m.base, "u", "p", DEVICE).await);
        assert!(matches!(e, JFError::Parse(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn authenticate_connection_refused_is_network() {
        let e = err(JellyfinClient::authenticate(&refused_base(), "u", "p", DEVICE).await);
        assert!(matches!(e, JFError::Network(_)), "got {e:?}");
    }

    // ------------------------------------------------------------------
    // Constructor / auth header plumbing
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn new_normalizes_base_and_rejects_bad_scheme() {
        let c = JellyfinClient::new("http://h:8096/", "t".into(), "u".into(), DEVICE).unwrap();
        assert_eq!(c.base, "http://h:8096");
        assert!(matches!(
            JellyfinClient::new("h:8096", "t".into(), "u".into(), DEVICE),
            Err(JFError::BadUrl(_))
        ));
    }

    #[tokio::test]
    async fn resolve_bare_host_falls_back_to_http_when_tls_fails() {
        // The mock is plain http, so the https attempt fails at the TLS layer
        // and the resolver must fall through to http.
        let m = serve(|_| reply(200, r#"{"ProductName":"Jellyfin Server","Version":"10.9.0"}"#)).await;
        let bare = m.base.trim_start_matches("http://").to_string();
        let (base, info) = JellyfinClient::resolve(&bare).await.unwrap();
        assert_eq!(base, m.base);
        assert_eq!(info.product_name.as_deref(), Some("Jellyfin Server"));
    }

    #[tokio::test]
    async fn resolve_explicit_scheme_does_not_fall_back() {
        let m = serve(|_| reply(200, r#"{"ProductName":"Jellyfin Server"}"#)).await;
        let bare = m.base.trim_start_matches("http://").to_string();
        let err = JellyfinClient::resolve(&format!("https://{}", bare)).await.err().unwrap();
        assert!(matches!(err, JFError::Network(_)), "{err:?}");
        let (base, _) = JellyfinClient::resolve(&format!("http://{}/", bare)).await.unwrap();
        assert_eq!(base, m.base);
    }

    #[tokio::test]
    async fn resolve_reports_a_reachable_non_jellyfin_server_without_retrying() {
        let m = serve(|_| reply(200, r#"{"hello":"world"}"#)).await;
        let bare = m.base.trim_start_matches("http://").to_string();
        assert!(matches!(JellyfinClient::resolve(&bare).await, Err(JFError::NotJellyfin)));
    }

    #[tokio::test]
    async fn base_path_prefix_is_kept_for_reverse_proxies() {
        let m = serve(|_| reply(200, VIEWS_JSON)).await;
        let c = client(&format!("{}/jellyfin/", m.base));
        c.views().await.unwrap();
        assert_eq!(m.last().path, format!("/jellyfin/Users/{}/Views", USER));
    }

    #[tokio::test]
    async fn device_id_header_uses_device_not_user() {
        let m = serve(|_| reply(200, VIEWS_JSON)).await;
        client(&m.base).views().await.unwrap();
        let auth = m.last().header("Authorization").unwrap().to_string();
        assert!(auth.contains(&format!("DeviceId=\"{}\"", DEVICE)), "{auth}");
        assert!(!auth.contains(USER), "user id leaked into DeviceId: {auth}");
    }

    // ------------------------------------------------------------------
    // first_user_id (API-key path)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn first_user_id_returns_first_and_sends_token() {
        let m = serve(|_| {
            reply(200, r#"[{"Name":"a","Id":"first-id"},{"Name":"b","Id":"second-id"}]"#)
        })
        .await;
        let id = client(&m.base).first_user_id().await.unwrap();
        assert_eq!(id, "first-id");
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/Users");
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn first_user_id_empty_list_is_auth() {
        let m = serve(|_| reply(200, "[]")).await;
        let e = err(client(&m.base).first_user_id().await);
        assert!(matches!(e, JFError::Auth(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn first_user_id_object_instead_of_array_is_parse() {
        let m = serve(|_| reply(200, r#"{"Items":[]}"#)).await;
        let e = err(client(&m.base).first_user_id().await);
        assert!(matches!(e, JFError::Parse(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn first_user_id_401_is_auth() {
        let m = serve(|_| status_only(401)).await;
        assert!(matches!(err(client(&m.base).first_user_id().await), JFError::Auth(_)));
    }

    // ------------------------------------------------------------------
    // views
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn views_path_auth_and_parse() {
        let m = serve(|_| reply(200, VIEWS_JSON)).await;
        let views = client(&m.base).views().await.unwrap();
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].id, "v1");
        assert_eq!(views[0].name, "Movies");
        assert_eq!(views[0].collection_type.as_deref(), Some("movies"));
        assert_eq!(views[1].collection_type, None, "CollectionType is optional");
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, format!("/Users/{}/Views", USER));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn views_missing_items_key_is_parse() {
        let m = serve(|_| reply(200, r#"{"TotalRecordCount":0}"#)).await;
        assert!(matches!(err(client(&m.base).views().await), JFError::Parse(_)));
    }

    #[tokio::test]
    async fn views_403_is_auth() {
        let m = serve(|_| status_only(403)).await;
        assert!(matches!(err(client(&m.base).views().await), JFError::Auth(_)));
    }

    #[tokio::test]
    async fn views_500_is_api() {
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(client(&m.base).views().await), JFError::Api(500, _)));
    }

    #[tokio::test]
    async fn views_connection_refused_is_network() {
        let c = client(&refused_base());
        assert!(matches!(err(c.views().await), JFError::Network(_)));
    }

    // ------------------------------------------------------------------
    // items (library query)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn items_default_query_params() {
        let m = serve(|_| reply(200, ITEMS_JSON)).await;
        client(&m.base).items(&ItemQuery::default()).await.unwrap();
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, format!("/Users/{}/Items", USER));
        assert_eq!(r.qp("Recursive"), Some("true"));
        assert_eq!(r.qp("ImageTypeLimit"), Some("1"));
        assert_eq!(r.qp("Limit"), Some("400"));
        assert_eq!(r.qp("SortBy"), Some("SortName"));
        assert_eq!(r.qp("SortOrder"), Some("Ascending"));
        let fields = r.qp("Fields").unwrap();
        for f in ["PrimaryImageAspectRatio", "ProductionYear", "UserData", "Overview"] {
            assert!(fields.split(',').any(|x| x == f), "Fields missing {f}: {fields}");
        }
        for absent in ["ParentId", "IncludeItemTypes", "SearchTerm", "Filters"] {
            assert_eq!(r.qp(absent), None, "{absent} should be omitted by default");
        }
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn items_all_options_map_to_query_params() {
        let m = serve(|_| reply(200, ITEMS_JSON)).await;
        let q = ItemQuery {
            parent_id: Some("parent-1".into()),
            series_id: None,
            types: Some("Episode,Movie".into()),
            search: Some("a&b c".into()),
            sort_by: "DateCreated".into(),
            sort_order: "Descending".into(),
            filters: vec!["IsUnplayed".into(), "IsFavorite".into()],
            limit: 25,
        };
        client(&m.base).items(&q).await.unwrap();
        let r = m.last();
        assert_eq!(r.qp("ParentId"), Some("parent-1"));
        assert_eq!(r.qp("IncludeItemTypes"), Some("Episode,Movie"));
        assert_eq!(r.qp("SearchTerm"), Some("a&b c"), "search must round-trip encoded");
        assert_eq!(r.qp("SortBy"), Some("DateCreated"));
        assert_eq!(r.qp("SortOrder"), Some("Descending"));
        assert_eq!(r.qp("Filters"), Some("IsUnplayed,IsFavorite"));
        assert_eq!(r.qp("Limit"), Some("25"));
    }

    #[tokio::test]
    async fn items_series_id_is_sent_as_parent_id() {
        let m = serve(|_| reply(200, ITEMS_JSON)).await;
        let q = ItemQuery { series_id: Some("series-9".into()), ..Default::default() };
        client(&m.base).items(&q).await.unwrap();
        assert_eq!(m.last().qp("ParentId"), Some("series-9"));
    }

    #[tokio::test]
    async fn items_parses_real_shape_and_defaults_missing_fields() {
        let m = serve(|_| reply(200, ITEMS_JSON)).await;
        let items = client(&m.base).items(&ItemQuery::default()).await.unwrap();
        assert_eq!(items.len(), 2);

        let ep = &items[0];
        assert_eq!(ep.id, "ep1");
        assert_eq!(ep.item_type, "Episode");
        assert_eq!(ep.series_name.as_deref(), Some("Show"));
        assert_eq!(ep.year, Some(2019));
        assert_eq!(ep.season, Some(1));
        assert_eq!(ep.episode, Some(2));
        assert_eq!(ep.run_time_ticks, Some(27_000_000_000));
        assert!(ep.has_image());
        assert_eq!(ep.aspect, Some(1.7777));
        let ud = ep.user_data.as_ref().unwrap();
        assert!(!ud.played);
        assert_eq!(ud.position_ticks, 1_000_000);
        assert_eq!(ep.sub_label(), "Show · S01E02");

        let min = &items[1];
        assert_eq!(min.item_type, "", "Type defaults to empty");
        assert!(min.user_data.is_none());
        assert!(min.image_tags.is_none());
        assert!(!min.has_image());
        assert_eq!(min.run_time_ticks, None);
        assert_eq!(min.sub_label(), "");
    }

    #[tokio::test]
    async fn items_empty_items_array_ok() {
        let m = serve(|_| reply(200, r#"{"Items":[]}"#)).await;
        assert!(client(&m.base).items(&ItemQuery::default()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn items_403_is_auth_and_500_is_api() {
        let m = serve(|_| status_only(403)).await;
        assert!(matches!(err(client(&m.base).items(&ItemQuery::default()).await), JFError::Auth(_)));
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(client(&m.base).items(&ItemQuery::default()).await), JFError::Api(500, _)));
    }

    #[tokio::test]
    async fn items_malformed_json_is_parse() {
        let m = serve(|_| reply(200, "[1,2")).await;
        assert!(matches!(err(client(&m.base).items(&ItemQuery::default()).await), JFError::Parse(_)));
    }

    // ------------------------------------------------------------------
    // full_item
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn full_item_path_auth_and_media_sources() {
        let m = serve(|_| reply(200, FULL_ITEM_JSON)).await;
        let it = client(&m.base).full_item("mv1").await.unwrap();
        assert_eq!(it.id, "mv1");
        assert_eq!(it.run_time_ticks, Some(7_200_000_000));
        assert!(it.user_data.as_ref().unwrap().played);
        assert_eq!(it.media_sources.len(), 1);
        let ms = &it.media_sources[0];
        assert_eq!(ms.id, "ms1");
        assert_eq!(ms.container.as_deref(), Some("mkv"));
        assert_eq!(ms.bitrate, Some(8_000_000));
        assert_eq!(ms.video_codec(), Some("h264"));
        assert_eq!(ms.audio_codec(), Some("dts"));
        assert_eq!(ms.streams[0].language, None, "null Language must deserialize to None");
        assert_eq!(ms.streams[1].display_title.as_deref(), Some("English DTS 5.1"));

        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, format!("/Users/{}/Items/mv1", USER));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn full_item_missing_media_sources_defaults_empty() {
        let m = serve(|_| reply(200, r#"{"Name":"x","Id":"y"}"#)).await;
        let it = client(&m.base).full_item("y").await.unwrap();
        assert!(it.media_sources.is_empty());
        assert_eq!(it.item_type, "");
        assert!(it.user_data.is_none());
    }

    #[tokio::test]
    async fn full_item_percent_encodes_id() {
        let m = serve(|_| reply(200, FULL_ITEM_JSON)).await;
        let _ = client(&m.base).full_item("a/b c").await;
        assert_eq!(m.last().path, format!("/Users/{}/Items/a%2Fb%20c", USER));
    }

    #[tokio::test]
    async fn full_item_404_is_api_and_401_is_auth() {
        let m = serve(|_| status_only(404)).await;
        assert!(matches!(err(client(&m.base).full_item("x").await), JFError::Api(404, _)));
        let m = serve(|_| status_only(401)).await;
        assert!(matches!(err(client(&m.base).full_item("x").await), JFError::Auth(_)));
    }

    // ------------------------------------------------------------------
    // image
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn image_returns_bytes_with_api_key_query() {
        let png = vec![0x89u8, b'P', b'N', b'G', 0, 1, 2, 255];
        let body = png.clone();
        let m = serve(move |_| Canned { status: 200, body: body.clone() }).await;
        let bytes = client(&m.base).image("img-1").await.unwrap();
        assert_eq!(bytes, png);
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/Items/img-1/Images/Primary");
        assert_eq!(r.qp("maxWidth"), Some("400"));
        assert_eq!(r.qp("quality"), Some("85"));
        assert_eq!(r.qp("api_key"), Some(TOKEN));
        assert!(r.header("X-Emby-Token").is_none(), "image uses api_key, not the header");
    }

    #[tokio::test]
    async fn image_404_is_api_error() {
        let m = serve(|_| status_only(404)).await;
        assert!(matches!(err(client(&m.base).image("nope").await), JFError::Api(404, _)));
    }

    #[tokio::test]
    async fn image_401_is_auth_error() {
        let m = serve(|_| status_only(401)).await;
        assert!(matches!(err(client(&m.base).image("x").await), JFError::Auth(_)));
    }

    // ------------------------------------------------------------------
    // Live TV
    // ------------------------------------------------------------------

    const HOME_ITEMS_JSON: &str = r#"{"Items":[{"Id":"a1","Name":"Alpha","Type":"Episode","SeriesName":"Show"}],"TotalRecordCount":1}"#;

    #[tokio::test]
    async fn resume_items_path_query_auth_and_parse() {
        let m = serve(|_| reply(200, HOME_ITEMS_JSON)).await;
        let items = client(&m.base).resume_items(16).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].series_name.as_deref(), Some("Show"));
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, format!("/Users/{}/Items/Resume", USER));
        assert_eq!(r.qp("Limit"), Some("16"));
        assert_eq!(r.qp("MediaTypes"), Some("Video"));
        assert!(r.qp("Fields").unwrap().contains("UserData"));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn next_up_sends_user_id_and_parses_items() {
        let m = serve(|_| reply(200, HOME_ITEMS_JSON)).await;
        let items = client(&m.base).next_up(12).await.unwrap();
        assert_eq!(items[0].id, "a1");
        let r = m.last();
        assert_eq!(r.path, "/Shows/NextUp");
        assert_eq!(r.qp("UserId"), Some(USER));
        assert_eq!(r.qp("Limit"), Some("12"));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn latest_items_sends_parent_and_parses_bare_array() {
        let m = serve(|_| reply(200, r#"[{"Id":"l1","Name":"Newest","Type":"Movie"}]"#)).await;
        let items = client(&m.base).latest_items("lib-7", 8).await.unwrap();
        assert_eq!(items[0].name, "Newest");
        let r = m.last();
        assert_eq!(r.path, format!("/Users/{}/Items/Latest", USER));
        assert_eq!(r.qp("ParentId"), Some("lib-7"));
        assert_eq!(r.qp("Limit"), Some("8"));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn recommendations_parse_groups_with_missing_fields() {
        let body = r#"[
            {"CategoryId":"c","RecommendationType":"SimilarToRecentlyPlayed","BaselineItemName":"Dune",
             "Items":[{"Id":"r1","Name":"Arrival","Type":"Movie"}]},
            {"Items":[]}
        ]"#;
        let m = serve(|_| reply(200, body)).await;
        let groups = client(&m.base).recommendations(8).await.unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].baseline_name.as_deref(), Some("Dune"));
        assert_eq!(groups[0].items[0].name, "Arrival");
        assert!(groups[1].recommendation_type.is_none() && groups[1].items.is_empty());
        let r = m.last();
        assert_eq!(r.path, "/Movies/Recommendations");
        assert_eq!(r.qp("UserId"), Some(USER));
        assert_eq!(r.qp("ItemLimit"), Some("8"));
        assert_eq!(r.qp("CategoryLimit"), Some("5"));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn home_endpoints_map_http_errors() {
        let m = serve(|_| status_only(401)).await;
        let c = client(&m.base);
        assert!(matches!(err(c.resume_items(4).await), JFError::Auth(_)));
        assert!(matches!(err(c.next_up(4).await), JFError::Auth(_)));
        assert!(matches!(err(c.latest_items("p", 4).await), JFError::Auth(_)));
        assert!(matches!(err(c.recommendations(4).await), JFError::Auth(_)));

        let m = serve(|_| status_only(500)).await;
        let c = client(&m.base);
        assert!(matches!(err(c.resume_items(4).await), JFError::Api(500, _)));
        assert!(matches!(err(c.recommendations(4).await), JFError::Api(500, _)));
    }

    #[tokio::test]
    async fn home_endpoints_map_bad_json_and_network_errors() {
        let m = serve(|_| reply(200, "not json")).await;
        assert!(matches!(err(client(&m.base).next_up(4).await), JFError::Parse(_)));
        assert!(matches!(err(client(&m.base).latest_items("p", 4).await), JFError::Parse(_)));
        let dead = refused_base();
        assert!(matches!(err(client(&dead).resume_items(4).await), JFError::Network(_)));
        assert!(matches!(err(client(&dead).recommendations(4).await), JFError::Network(_)));
    }

    #[tokio::test]
    async fn livetv_channels_path_query_and_parse() {
        let m = serve(|_| reply(200, LIVETV_JSON)).await;
        let ch = client(&m.base).livetv_channels().await.unwrap();
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].id, "ch1");
        assert_eq!(ch[0].name, "BBC One");
        assert_eq!(ch[0].item_type, "TvChannel");
        let r = m.last();
        assert_eq!(r.path, "/LiveTv/Channels");
        assert_eq!(r.qp("Limit"), Some("500"));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn livetv_user_id_is_encoded_exactly_once() {
        // A user id with a space must reach the server as "a b", not "a%20b".
        let m = serve(|_| reply(200, LIVETV_JSON)).await;
        let c = JellyfinClient::new(&m.base, TOKEN.into(), "a b".into(), DEVICE).unwrap();
        c.livetv_channels().await.unwrap();
        assert_eq!(m.last().qp("UserId"), Some("a b"));
    }

    #[tokio::test]
    async fn livetv_error_mapping() {
        let m = serve(|_| status_only(403)).await;
        assert!(matches!(err(client(&m.base).livetv_channels().await), JFError::Auth(_)));
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(client(&m.base).livetv_channels().await), JFError::Api(500, _)));
        let m = serve(|_| reply(200, "nope")).await;
        assert!(matches!(err(client(&m.base).livetv_channels().await), JFError::Parse(_)));
    }

    // ------------------------------------------------------------------
    // Playback reporting (fire-and-forget)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn report_start_posts_playing_body() {
        let m = serve(|_| status_only(204)).await;
        client(&m.base).report_start("item-7", 123_000, "DirectPlay").await;
        let r = m.last();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/Sessions/Playing");
        assert_eq!(
            r.body_json(),
            serde_json::json!({"ItemId": "item-7", "PositionTicks": 123000, "IsPaused": false, "PlayMethod": "DirectPlay"})
        );
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn report_progress_posts_and_clamps_negative_ticks() {
        let m = serve(|_| status_only(204)).await;
        client(&m.base).report_progress("item-7", -50, true).await;
        let r = m.last();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/Sessions/Playing/Progress");
        assert_eq!(
            r.body_json(),
            serde_json::json!({"ItemId": "item-7", "PositionTicks": 0, "IsPaused": true})
        );
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn report_stopped_posts_body() {
        let m = serve(|_| status_only(204)).await;
        client(&m.base).report_stopped("item-7", 987_654).await;
        let r = m.last();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/Sessions/Playing/Stopped");
        assert_eq!(r.body_json(), serde_json::json!({"ItemId": "item-7", "PositionTicks": 987654}));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn report_methods_swallow_connection_errors() {
        // Fire-and-forget: must return without panicking or erroring.
        let c = client(&refused_base());
        c.report_start("i", 1, "DirectPlay").await;
        c.report_progress("i", 1, false).await;
        c.report_stopped("i", 1).await;
    }

    #[tokio::test]
    async fn report_methods_do_not_fail_on_server_error() {
        let m = serve(|_| status_only(500)).await;
        let c = client(&m.base);
        c.report_start("i", 1, "DirectPlay").await;
        c.report_progress("i", 1, false).await;
        c.report_stopped("i", 1).await;
        assert_eq!(m.count(), 3);
    }

    // ------------------------------------------------------------------
    // set_played
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn set_played_true_posts_to_played_items() {
        let m = serve(|_| status_only(200)).await;
        client(&m.base).set_played("item-1", true).await.unwrap();
        let r = m.last();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, format!("/Users/{}/PlayedItems/item-1", USER));
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn set_played_false_deletes_played_items() {
        let m = serve(|_| status_only(200)).await;
        client(&m.base).set_played("item-1", false).await.unwrap();
        let r = m.last();
        assert_eq!(r.method, "DELETE");
        assert_eq!(r.path, format!("/Users/{}/PlayedItems/item-1", USER));
    }

    #[tokio::test]
    async fn set_played_error_mapping() {
        let m = serve(|_| status_only(403)).await;
        assert!(matches!(err(client(&m.base).set_played("i", true).await), JFError::Auth(_)));
        let m = serve(|_| status_only(404)).await;
        assert!(matches!(err(client(&m.base).set_played("i", false).await), JFError::Api(404, _)));
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(client(&m.base).set_played("i", true).await), JFError::Api(500, _)));
        assert!(matches!(
            err(client(&refused_base()).set_played("i", true).await),
            JFError::Network(_)
        ));
    }

    #[tokio::test]
    async fn media_segments_path_auth_and_parse() {
        let body = r#"{"Items":[{"Id":"x","ItemId":"ep","Type":"Intro","StartTicks":0,"EndTicks":600000000}],"TotalRecordCount":1}"#;
        let m = serve(|_| reply(200, body)).await;
        let segs = client(&m.base).media_segments("ep 1").await.unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].segment_type, "Intro");
        assert_eq!(segs[0].end_ticks, 600_000_000);
        let r = m.last();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/MediaSegments/ep%201");
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn media_segments_404_and_empty_mean_no_segments() {
        let m = serve(|_| status_only(404)).await;
        assert!(client(&m.base).media_segments("ep").await.unwrap().is_empty());
        let m = serve(|_| reply(200, r#"{"Items":[],"TotalRecordCount":0}"#)).await;
        assert!(client(&m.base).media_segments("ep").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn media_segments_other_errors_still_fail() {
        let m = serve(|_| status_only(500)).await;
        assert!(matches!(err(client(&m.base).media_segments("ep").await), JFError::Api(500, _)));
        let m = serve(|_| status_only(401)).await;
        assert!(matches!(err(client(&m.base).media_segments("ep").await), JFError::Auth(_)));
        assert!(matches!(
            err(client(&refused_base()).media_segments("ep").await),
            JFError::Network(_)
        ));
    }

    #[tokio::test]
    async fn series_episodes_lists_whole_series_without_season_filter() {
        let body = r#"{"Items":[{"Id":"e1","Name":"One","IndexNumber":1,"ParentIndexNumber":1}],"TotalRecordCount":1}"#;
        let m = serve(|_| reply(200, body)).await;
        let eps = client(&m.base).series_episodes("sh 1").await.unwrap();
        assert_eq!(eps[0].id, "e1");
        assert_eq!(eps[0].parent_index, Some(1));
        let r = m.last();
        assert_eq!(r.path, "/Shows/sh%201/Episodes");
        assert_eq!(r.qp("UserId"), Some(USER));
        assert_eq!(r.qp("seasonId"), None);
        assert_token_auth(&r);
    }

    #[tokio::test]
    async fn full_item_carries_series_id() {
        let body = r#"{"Id":"e1","Name":"One","Type":"Episode","SeriesId":"sh1","SeriesName":"Show","ParentIndexNumber":2,"IndexNumber":3}"#;
        let m = serve(|_| reply(200, body)).await;
        let it = client(&m.base).full_item("e1").await.unwrap();
        assert_eq!(it.series_id.as_deref(), Some("sh1"));
    }
}

#[cfg(test)]
mod url_and_model_tests {
    use crate::jellyfin::models::*;
    use crate::jellyfin::urls as u;

    fn query_of(url: &str) -> Vec<(String, String)> {
        let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
        q.split('&')
            .filter(|s| !s.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (k.to_string(), v.replace("%2C", ",").replace("%20", " "))
            })
            .collect()
    }

    #[test]
    fn hls_url_has_no_empty_or_misplaced_params() {
        let url = u::build_hls_url("http://h", "item", "ms", "tok", 20_000, "eng");
        let q = query_of(&url);
        assert!(url.starts_with("http://h/Videos/item/master.m3u8?"));
        assert!(q.iter().all(|(_, v)| !v.is_empty()), "no empty params: {url}");
        assert!(!q.iter().any(|(k, _)| k == "SubtitleStreamIndex"));
        assert!(q.iter().any(|(k, v)| k == "SubtitleLanguage" && v == "eng"));
        assert_eq!(q.last().map(|(k, _)| k.as_str()), Some("SubtitleLanguage"));
        assert!(q.iter().any(|(k, v)| k == "api_key" && v == "tok"));
        assert!(q.iter().any(|(k, v)| k == "VideoBitrate" && v == "20000"));
    }

    #[test]
    fn hls_url_without_bitrate_or_subtitle_is_minimal() {
        let url = u::build_hls_url("http://h", "i", "m", "t", 0, "");
        let q = query_of(&url);
        for absent in ["VideoBitrate", "MaxWidth", "MaxHeight", "SubtitleLanguage"] {
            assert!(!q.iter().any(|(k, _)| k == absent), "{absent} should be absent");
        }
        assert!(q.iter().any(|(k, _)| k == "SegmentContainer"));
    }

    #[test]
    fn direct_url_uses_first_container_and_static_flag() {
        let url = u::build_direct_url("http://h", "i1", "ms1", Some("mkv,webm"), "tok");
        assert!(url.starts_with("http://h/Videos/i1/stream.mkv?"), "{url}");
        let q = query_of(&url);
        assert!(q.iter().any(|(k, v)| k == "static" && v == "true"));
        assert!(q.iter().any(|(k, v)| k == "MediaSourceId" && v == "ms1"));
        assert!(q.iter().any(|(k, v)| k == "api_key" && v == "tok"));
    }

    #[test]
    fn livetv_and_image_urls_carry_api_key() {
        let live = u::build_livetv_url("http://h", "ch9", "tok");
        assert_eq!(live, "http://h/LiveTv/Channels/ch9/stream?api_key=tok");
        let img = u::build_image_url("http://h", "x", "tok", 300);
        let q = query_of(&img);
        assert!(img.starts_with("http://h/Items/x/Images/Primary?"));
        assert!(q.iter().any(|(k, v)| k == "maxWidth" && v == "300"));
        assert!(q.iter().any(|(k, v)| k == "api_key" && v == "tok"));
    }

    #[test]
    fn web_url_escapes_id() {
        assert_eq!(
            u::build_web_item_url("http://h", "a b/c"),
            "http://h/web/index.html#!/itemdetails.html?id=a%20b%2Fc"
        );
    }

    #[test]
    fn enc_is_rfc3986_unreserved_only() {
        assert_eq!(u::enc("AZaz09-_.~"), "AZaz09-_.~");
        assert_eq!(u::enc("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }

    #[test]
    fn normalize_base_url_rules() {
        assert_eq!(u::normalize_base_url("  https://x.io/jf//  ").unwrap(), "https://x.io/jf");
        assert!(u::normalize_base_url("").is_err());
        assert!(u::normalize_base_url("x.io").is_err());
        assert!(u::normalize_base_url("HTTP://x.io").is_ok());
    }

    #[test]
    fn media_source_codec_helpers_handle_missing_streams() {
        let ms: MediaSource = serde_json::from_str(r#"{"Id":"s"}"#).unwrap();
        assert!(ms.streams.is_empty());
        assert_eq!(ms.video_codec(), None);
        assert_eq!(ms.audio_codec(), None);
        let ms: MediaSource = serde_json::from_str(
            r#"{"Id":"s","MediaStreams":[{"Type":"Audio","Codec":"aac"},{"Type":"Audio","Codec":"mp3"}]}"#,
        )
        .unwrap();
        assert_eq!(ms.audio_codec(), Some("aac"), "first audio stream wins");
        assert_eq!(ms.video_codec(), None);
    }

    #[test]
    fn list_item_sub_label_for_movie_and_episode_without_numbers() {
        let movie: ListItem =
            serde_json::from_str(r#"{"Id":"m","Name":"Heat","Type":"Movie","ProductionYear":1995}"#).unwrap();
        assert_eq!(movie.sub_label(), "1995");
        let ep: ListItem = serde_json::from_str(
            r#"{"Id":"e","Name":"X","Type":"Episode","SeriesName":"Show"}"#,
        )
        .unwrap();
        assert_eq!(ep.sub_label(), "");
    }

    #[test]
    fn list_item_unknown_extra_fields_are_ignored() {
        let it: ListItem = serde_json::from_str(
            r#"{"Id":"a","Name":"b","Type":"Series","ServerId":"s","Genres":["Drama"],"People":[],"ImageBlurHashes":{}}"#,
        )
        .unwrap();
        assert_eq!(it.item_type, "Series");
        assert!(!it.has_image());
    }

    #[test]
    fn list_item_requires_id_and_name() {
        assert!(serde_json::from_str::<ListItem>(r#"{"Name":"no id"}"#).is_err());
        assert!(serde_json::from_str::<ListItem>(r#"{"Id":"no name"}"#).is_err());
    }
}
