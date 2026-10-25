use crate::*;

use super::urls as u;
use super::models::*;
use crate::jellyfin::urls::enc;

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
            .user_agent("JellyfinMac/0.1.0")
            .build()
            .map_err(|e| JFError::Network(e.to_string()))?;
        Ok(Self { http, base, token, user_id, device_id: device_id.to_string() })
    }

    fn auth_header(&self) -> String {
        format!(
            "MediaBrowser Client=\"JellyfinMac\", Device=\"Mac\", DeviceId=\"{}\", Version=\"0.1.0\"",
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
                    "MediaBrowser Client=\"JellyfinMac\", Device=\"Mac\", DeviceId=\"{}\", Version=\"0.1.0\"",
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

    pub async fn image(&self, item_id: &str) -> Result<Vec<u8>, JFError> {
        let url = u::build_image_url(&self.base, item_id, &self.token, 400);
        let r = self.http.get(url).timeout(Duration::from_secs(30)).send().await.map_err(map_err)?;
        let r = check(r).await?;
        r.bytes().await.map(|b| b.to_vec()).map_err(|e| JFError::Network(e.to_string()))
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

    pub async fn report_start(&self, item_id: &str, ticks: i64) {
        let _ = self
            .http
            .post(format!("{}/Sessions/Playing", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .json(&serde_json::json!({
                "ItemId": item_id, "PositionTicks": ticks, "IsPaused": false,
                "PlayMethod": "DirectPlay"
            }))
            .send()
            .await;
    }
    pub async fn report_progress(&self, item_id: &str, ticks: i64, paused: bool) {
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
        let _ = self
            .http
            .post(format!("{}/Sessions/Playing/Stopped", self.base))
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .json(&serde_json::json!({ "ItemId": item_id, "PositionTicks": ticks.max(0) }))
            .send()
            .await;
    }
    pub async fn set_played(&self, item_id: &str, played: bool) -> Result<(), JFError> {
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
        assert!(auth.contains("Client=\"JellyfinMac\""), "{auth}");
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
        client(&m.base).report_start("item-7", 123_000).await;
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
        c.report_start("i", 1).await;
        c.report_progress("i", 1, false).await;
        c.report_stopped("i", 1).await;
    }

    #[tokio::test]
    async fn report_methods_do_not_fail_on_server_error() {
        let m = serve(|_| status_only(500)).await;
        let c = client(&m.base);
        c.report_start("i", 1).await;
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
