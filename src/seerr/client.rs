use super::models::*;
use crate::jellyfin::urls::{enc, normalize_base_url};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum SeerrError {
    BadUrl(String),
    Network(String),
    /// Reachable but not a Seerr/Overseerr/Jellyseerr server.
    NotSeerr,
    /// 401/403: missing, wrong, or insufficiently privileged API key.
    Auth(String),
    /// 409 or an "already requested" message.
    AlreadyRequested(String),
    /// Jellyfin-account sign-in was refused (or the session could not be established).
    Login(String),
    Api(u16, String),
    Parse(String),
    /// Caller passed something Seerr can't take (e.g. media type "person").
    Invalid(String),
}

impl fmt::Display for SeerrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeerrError::BadUrl(s) => write!(f, "Invalid Seerr URL: {}", s),
            SeerrError::Network(s) => write!(
                f,
                "Could not reach Seerr: {}. Check the address and that the server is running.",
                s
            ),
            SeerrError::NotSeerr => {
                write!(f, "That address responded, but it does not look like a Seerr server.")
            }
            SeerrError::Auth(s) => write!(f, "Seerr rejected the API key: {}", s),
            SeerrError::Login(s) => write!(f, "Seerr sign-in failed: {}", s),
            SeerrError::AlreadyRequested(s) => write!(f, "{}", s),
            SeerrError::Api(c, s) => write!(f, "Seerr error (HTTP {}): {}", c, s),
            SeerrError::Parse(s) => write!(f, "Unexpected Seerr response: {}", s),
            SeerrError::Invalid(s) => write!(f, "{}", s),
        }
    }
}

fn map_err(e: reqwest::Error) -> SeerrError {
    if e.is_timeout() {
        SeerrError::Network("request timed out".into())
    } else if e.is_connect() {
        SeerrError::Network(format!("could not connect ({})", e))
    } else {
        SeerrError::Network(e.to_string())
    }
}

/// Pull a human message out of a Seerr error body (`{"message": "..."}`).
fn error_message(body: &str) -> String {
    if let Ok(m) = serde_json::from_str::<RawMessage>(body) {
        if let Some(s) = m.message.filter(|s| !s.trim().is_empty()) {
            return s;
        }
    }
    let t = body.trim();
    if t.is_empty() {
        String::new()
    } else {
        t.chars().take(200).collect()
    }
}

/// Map a status code + body to an error (mirrors JellyfinClient's `check`).
/// `Ok(())` for 2xx.
fn check(code: u16, body: &str, what: &str) -> Result<(), SeerrError> {
    if (200..300).contains(&code) {
        return Ok(());
    }
    let msg = error_message(body);
    let shown = if msg.is_empty() { what.to_string() } else { msg.clone() };
    match code {
        401 | 403 => Err(SeerrError::Auth(if msg.is_empty() {
            "invalid API key or insufficient permissions".into()
        } else {
            msg
        })),
        409 => Err(SeerrError::AlreadyRequested(if msg.is_empty() {
            "Already requested.".into()
        } else {
            msg
        })),
        _ if msg.to_ascii_lowercase().contains("already") && what.starts_with("POST") => {
            Err(SeerrError::AlreadyRequested(shown))
        }
        _ => Err(SeerrError::Api(code, shown)),
    }
}

#[derive(Clone)]
enum Auth {
    Key(String),
    /// Jellyfin-account sign-in. `session` is the `Cookie` header value captured
    /// from the login response (reqwest's `cookies` feature is not enabled).
    Login { username: String, password: String, session: Arc<tokio::sync::Mutex<Option<String>>> },
}

#[derive(Clone)]
pub struct SeerrClient {
    http: reqwest::Client,
    pub base: String,
    auth: Auth,
}

impl SeerrClient {
    fn build(base: &str, auth: Auth) -> Result<Self, SeerrError> {
        let base = normalize_base_url(base).map_err(SeerrError::BadUrl)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent("Jellyfast/0.1.0")
            .build()
            .map_err(|e| SeerrError::Network(e.to_string()))?;
        Ok(Self { http, base, auth })
    }

    /// API-key mode.
    pub fn new(base: &str, api_key: &str) -> Result<Self, SeerrError> {
        Self::build(base, Auth::Key(api_key.trim().to_string()))
    }

    /// Jellyfin-account mode: signs in lazily, reuses the session cookie, and
    /// signs in again once if the server answers 401/403.
    pub fn with_login(base: &str, username: &str, password: &str) -> Result<Self, SeerrError> {
        Self::build(
            base,
            Auth::Login {
                username: username.trim().to_string(),
                password: password.to_string(),
                session: Arc::new(tokio::sync::Mutex::new(None)),
            },
        )
    }

    /// POST /api/v1/auth/jellyfin; returns the `Cookie` header value to replay.
    async fn login(&self, username: &str, password: &str) -> Result<String, SeerrError> {
        let r = self
            .http
            .post(format!("{}/api/v1/auth/jellyfin", self.base))
            .header("Accept", "application/json")
            .json(&serde_json::json!({"username": username, "password": password}))
            .send()
            .await
            .map_err(map_err)?;
        let code = r.status().as_u16();
        let cookies: Vec<String> = r
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.split(';').next())
            .map(|v| v.trim().to_string())
            .filter(|v| v.contains('=') && !v.ends_with('='))
            .collect();
        let body = r.text().await.map_err(|e| SeerrError::Network(e.to_string()))?;
        match check(code, &body, "POST /api/v1/auth/jellyfin") {
            Ok(()) => {}
            Err(SeerrError::Auth(m)) => {
                let m = if m.starts_with("invalid API key") { "check your username and password".to_string() } else { m };
                return Err(SeerrError::Login(m));
            }
            Err(SeerrError::Api(404, _)) => return Err(SeerrError::NotSeerr),
            Err(e) => return Err(e),
        }
        if cookies.is_empty() {
            return Err(SeerrError::Login("the server did not start a session".into()));
        }
        Ok(cookies.join("; "))
    }

    /// Current session cookie, signing in if there is none or if `stale` (the
    /// cookie the caller just got a 401/403 with) is still the current one.
    async fn session(&self, stale: Option<&str>) -> Result<String, SeerrError> {
        let Auth::Login { username, password, session } = &self.auth else {
            return Ok(String::new());
        };
        let mut g = session.lock().await;
        if let Some(c) = g.as_ref() {
            if stale != Some(c.as_str()) {
                return Ok(c.clone());
            }
        }
        let c = self.login(username, password).await?;
        *g = Some(c.clone());
        Ok(c)
    }

    /// `authed == false` skips credentials (public endpoints such as /status).
    async fn send_with(&self, rb: reqwest::RequestBuilder, what: &str, authed: bool) -> Result<String, SeerrError> {
        let rb = rb.header("Accept", "application/json");
        let (code, body) = match &self.auth {
            Auth::Key(k) => {
                let r = rb.header("X-Api-Key", k).send().await.map_err(map_err)?;
                let code = r.status().as_u16();
                (code, r.text().await.map_err(|e| SeerrError::Network(e.to_string()))?)
            }
            Auth::Login { .. } if !authed => {
                let r = rb.send().await.map_err(map_err)?;
                let code = r.status().as_u16();
                (code, r.text().await.map_err(|e| SeerrError::Network(e.to_string()))?)
            }
            Auth::Login { .. } => {
                let mut cookie = self.session(None).await?;
                let mut attempt = 0;
                loop {
                    let req = rb.try_clone().ok_or_else(|| SeerrError::Invalid("request not repeatable".into()))?;
                    let r = req.header(reqwest::header::COOKIE, &cookie).send().await.map_err(map_err)?;
                    let code = r.status().as_u16();
                    let body = r.text().await.map_err(|e| SeerrError::Network(e.to_string()))?;
                    if (code == 401 || code == 403) && attempt == 0 {
                        attempt += 1;
                        cookie = self.session(Some(&cookie)).await?; // session expired: sign in again, once
                        continue;
                    }
                    break (code, body);
                }
            }
        };
        match check(code, &body, what) {
            Err(SeerrError::Auth(m)) if matches!(self.auth, Auth::Login { .. }) => Err(SeerrError::Login(format!("not permitted ({})", m))),
            r => r.map(|_| body),
        }
    }

    async fn send(&self, rb: reqwest::RequestBuilder, what: &str) -> Result<String, SeerrError> {
        self.send_with(rb, what, true).await
    }

    async fn get(&self, path: &str) -> Result<String, SeerrError> {
        self.send(self.http.get(format!("{}{}", self.base, path)), &format!("GET {}", path.split('?').next().unwrap_or(path)))
            .await
    }

    /// GET /api/v1/status: unauthenticated health check; proves it's Seerr.
    pub async fn status(&self) -> Result<SeerrStatusInfo, SeerrError> {
        let body = self
            .send_with(self.http.get(format!("{}/api/v1/status", self.base)), "GET /api/v1/status", false)
            .await?;
        let s: RawStatus = serde_json::from_str(&body).map_err(|_| SeerrError::NotSeerr)?;
        match s.version.filter(|v| !v.is_empty()) {
            Some(version) => Ok(SeerrStatusInfo { version }),
            None => Err(SeerrError::NotSeerr),
        }
    }

    /// Resolve a user-typed Seerr address to a working base URL, using the same
    /// https-then-http rule as Jellyfin. Verified via the unauthenticated status
    /// endpoint.
    pub async fn resolve(input: &str) -> Result<(String, SeerrStatusInfo), SeerrError> {
        let candidates = crate::jellyfin::urls::candidate_bases(input).map_err(SeerrError::BadUrl)?;
        let mut last_network = None;
        for base in candidates {
            let client = Self::new(&base, "")?;
            match client.status().await {
                Ok(info) => return Ok((client.base.clone(), info)),
                Err(e @ SeerrError::Network(_)) => last_network = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last_network.unwrap_or_else(|| SeerrError::BadUrl("no address to try".into())))
    }

    /// GET /api/v1/auth/me: authenticated call; verifies the API key.
    pub async fn me(&self) -> Result<SeerrUser, SeerrError> {
        let body = self.get("/api/v1/auth/me").await?;
        let u: RawUser = serde_json::from_str(&body).map_err(|e| SeerrError::Parse(e.to_string()))?;
        let name = [u.display_name, u.username, u.email]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .unwrap_or_else(|| "user".into());
        Ok(SeerrUser { id: u.id, display_name: name })
    }

    /// Health check + key verification.
    pub async fn verify(&self) -> Result<(SeerrStatusInfo, SeerrUser), SeerrError> {
        let st = self.status().await?;
        let me = self.me().await?;
        Ok((st, me))
    }

    /// GET /api/v1/search: keeps movie and tv hits, drops people.
    pub async fn search(&self, query: &str, page: u32) -> Result<SearchPage, SeerrError> {
        let body = self
            .get(&format!("/api/v1/search?query={}&page={}", enc(query.trim()), page.max(1)))
            .await?;
        let raw: RawSearch = serde_json::from_str(&body).map_err(|e| SeerrError::Parse(e.to_string()))?;
        let results = raw
            .results
            .into_iter()
            .filter_map(|r| {
                let mt = r.media_type.clone()?;
                let (title, date) = match mt.as_str() {
                    "movie" => (r.title.or(r.name), r.release_date),
                    "tv" => (r.name.or(r.title), r.first_air_date),
                    _ => return None,
                };
                let (avail, req) = match &r.media_info {
                    Some(mi) => (
                        Availability::from_code(mi.status),
                        mi.requests.first().map(|q| RequestStatus::from_code(q.status)),
                    ),
                    None => (Availability::NotRequested, None),
                };
                Some(SeerrResult {
                    media_type: mt,
                    tmdb_id: r.id,
                    title: title.unwrap_or_default(),
                    year: year_of(date.as_deref()),
                    poster_path: r.poster_path.filter(|p| !p.is_empty()),
                    overview: r.overview.unwrap_or_default(),
                    availability: avail,
                    request_status: req,
                })
            })
            .collect();
        Ok(SearchPage {
            page: raw.page,
            total_pages: raw.total_pages,
            total_results: raw.total_results,
            results,
        })
    }

    /// Poster bytes through this server's image proxy (needs no credentials).
    pub async fn poster(&self, path: &str) -> Result<Vec<u8>, SeerrError> {
        let r = self.http.get(poster_url(&self.base, path)).send().await.map_err(map_err)?;
        let code = r.status().as_u16();
        if !(200..300).contains(&code) {
            return Err(SeerrError::Api(code, "poster".into()));
        }
        let b = r.bytes().await.map_err(|e| SeerrError::Network(e.to_string()))?;
        if b.is_empty() {
            return Err(SeerrError::Parse("empty poster".into()));
        }
        Ok(b.to_vec())
    }

    /// POST /api/v1/request. TV requests ask for every season (`"seasons":"all"`).
    pub async fn request(&self, media_type: &str, tmdb_id: u64) -> Result<CreatedRequest, SeerrError> {
        let body = match media_type {
            "movie" => serde_json::json!({"mediaType": "movie", "mediaId": tmdb_id}),
            "tv" => serde_json::json!({"mediaType": "tv", "mediaId": tmdb_id, "seasons": "all"}),
            other => return Err(SeerrError::Invalid(format!("Cannot request media type \"{}\".", other))),
        };
        let text = self
            .send(
                self.http.post(format!("{}/api/v1/request", self.base)).json(&body),
                "POST /api/v1/request",
            )
            .await?;
        let r: RawRequest = serde_json::from_str(&text).map_err(|e| SeerrError::Parse(e.to_string()))?;
        Ok(CreatedRequest { request_id: r.id, status: RequestStatus::from_code(r.status) })
    }

    /// Title/year/poster for a TMDB id via GET /api/v1/{movie|tv}/{id}.
    async fn details(&self, media_type: &str, tmdb_id: u64) -> Result<(String, Option<u16>, Option<String>), SeerrError> {
        let body = self.get(&format!("/api/v1/{}/{}", media_type, tmdb_id)).await?;
        let r: RawResult = serde_json::from_str(&body).map_err(|e| SeerrError::Parse(e.to_string()))?;
        let (t, d) = if media_type == "tv" {
            (r.name.or(r.title), r.first_air_date)
        } else {
            (r.title.or(r.name), r.release_date)
        };
        Ok((t.unwrap_or_default(), year_of(d.as_deref()), r.poster_path.filter(|p| !p.is_empty())))
    }

    /// GET /api/v1/request: newest first. The list endpoint carries no titles,
    /// so each entry is enriched from the detail endpoint (best effort, concurrent).
    pub async fn requests(&self, take: u32, skip: u32) -> Result<Vec<SeerrRequestItem>, SeerrError> {
        let body = self
            .get(&format!("/api/v1/request?take={}&skip={}&sort=added&filter=all", take, skip))
            .await?;
        let raw: RawRequestList = serde_json::from_str(&body).map_err(|e| SeerrError::Parse(e.to_string()))?;
        let mut tasks = Vec::new();
        for r in raw.results {
            let media_type = r
                .kind
                .clone()
                .or_else(|| r.media.as_ref().and_then(|m| m.media_type.clone()))
                .unwrap_or_default();
            let tmdb_id = r.media.as_ref().and_then(|m| m.tmdb_id).unwrap_or(0);
            let c = self.clone();
            let mt = media_type.clone();
            let detail = tokio::spawn(async move {
                if tmdb_id == 0 || (mt != "movie" && mt != "tv") {
                    return None;
                }
                c.details(&mt, tmdb_id).await.ok()
            });
            tasks.push((r, media_type, tmdb_id, detail));
        }
        let mut out = Vec::new();
        for (r, media_type, tmdb_id, detail) in tasks {
            let d = detail.await.ok().flatten().filter(|d| !d.0.is_empty());
            let (title, year, poster_path) = d.unwrap_or_else(|| (format!("TMDB #{}", tmdb_id), None, None));
            let who = r
                .requested_by
                .and_then(|u| [u.display_name, u.username, u.email].into_iter().flatten().find(|s| !s.is_empty()))
                .unwrap_or_default();
            out.push(SeerrRequestItem {
                request_id: r.id,
                media_type,
                tmdb_id,
                title,
                year,
                poster_path,
                status: RequestStatus::from_code(r.status),
                availability: Availability::from_code(r.media.as_ref().and_then(|m| m.status)),
                requested_by: who,
                created_at: r.created_at.unwrap_or_default(),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seerr::testutil::*;

    const KEY: &str = "good-key";

    async fn client(m: &MockSeerr) -> SeerrClient {
        SeerrClient::new(&m.url(), KEY).unwrap()
    }

    #[test]
    fn poster_url_uses_the_seerr_image_proxy() {
        assert_eq!(poster_url("http://s:5055", "/a.jpg"), "http://s:5055/imageproxy/tmdb/t/p/w92/a.jpg");
        assert_eq!(poster_url("http://s:5055/", "a.jpg"), "http://s:5055/imageproxy/tmdb/t/p/w92/a.jpg");
        assert!(!poster_url("http://s", "/a.jpg").contains("tmdb.org"));
    }

    #[tokio::test]
    async fn poster_fetches_bytes_without_credentials() {
        let m = start(default_route).await;
        let c = client(&m).await;
        assert_eq!(c.poster("/inc.jpg").await.unwrap(), b"IMG/inc.jpg");
        let r = m.find("GET", "/imageproxy/tmdb/t/p/w92/inc.jpg");
        assert_eq!(r.len(), 1);
        assert!(!r[0].headers.contains_key("x-api-key"));
        assert!(matches!(c.poster("/missing.jpg").await, Err(SeerrError::Api(404, _))));
    }

    #[test]
    fn check_maps_codes() {
        assert!(check(200, "", "x").is_ok());
        assert!(check(204, "", "x").is_ok());
        assert!(matches!(check(401, "", "x"), Err(SeerrError::Auth(_))));
        assert!(matches!(check(403, r#"{"message":"nope"}"#, "x"), Err(SeerrError::Auth(m)) if m == "nope"));
        assert!(matches!(check(409, "", "x"), Err(SeerrError::AlreadyRequested(_))));
        assert!(matches!(check(500, "", "GET /a"), Err(SeerrError::Api(500, m)) if m == "GET /a"));
        assert!(matches!(
            check(500, r#"{"message":"Media already requested"}"#, "POST /api/v1/request"),
            Err(SeerrError::AlreadyRequested(_))
        ));
        // "already" on a GET is not an AlreadyRequested.
        assert!(matches!(check(500, r#"{"message":"already"}"#, "GET /a"), Err(SeerrError::Api(500, _))));
    }

    #[tokio::test]
    async fn bad_url_rejected() {
        assert!(matches!(SeerrClient::new("seerr.local", "k"), Err(SeerrError::BadUrl(_))));
        assert!(matches!(SeerrClient::new("", "k"), Err(SeerrError::BadUrl(_))));
    }

    #[tokio::test]
    async fn resolve_bare_host_falls_back_to_http() {
        let m = start(|_| Resp::json(200, serde_json::json!({"version": "2.7.2"}))).await;
        let bare = m.url().trim_start_matches("http://").to_string();
        let (base, info) = SeerrClient::resolve(&bare).await.unwrap();
        assert_eq!(base, m.url());
        assert_eq!(info.version, "2.7.2");
        assert!(matches!(SeerrClient::resolve("").await, Err(SeerrError::BadUrl(_))));
    }

    #[tokio::test]
    async fn status_and_me_and_verify() {
        let m = start(default_route).await;
        let c = SeerrClient::new(&format!("{}/", m.url()), KEY).unwrap();
        assert_eq!(c.status().await.unwrap().version, "2.7.2");
        let me = c.me().await.unwrap();
        assert_eq!((me.id, me.display_name.as_str()), (1, "Nathan"));
        let (st, me) = c.verify().await.unwrap();
        assert_eq!((st.version.as_str(), me.id), ("2.7.2", 1));
        let r = m.find("GET", "/api/v1/auth/me");
        assert_eq!(r[0].headers["x-api-key"], KEY);
    }

    #[tokio::test]
    async fn status_not_seerr() {
        let m = start(|_| Resp::json(200, serde_json::json!({"hello": 1}))).await;
        assert_eq!(client(&m).await.status().await.unwrap_err(), SeerrError::NotSeerr);
        let m2 = start(|_| Resp::text(200, "<html>nginx</html>")).await;
        assert_eq!(client(&m2).await.status().await.unwrap_err(), SeerrError::NotSeerr);
    }

    #[tokio::test]
    async fn bad_key_is_auth_error() {
        let m = start(default_route).await;
        let c = SeerrClient::new(&m.url(), "wrong").unwrap();
        assert!(c.status().await.is_ok()); // status is public
        assert!(matches!(c.me().await, Err(SeerrError::Auth(_))));
        assert!(matches!(c.verify().await, Err(SeerrError::Auth(_))));
        assert!(matches!(c.search("x", 1).await, Err(SeerrError::Auth(_))));
        let m = start(|_| Resp::text(401, "")).await;
        assert!(matches!(client(&m).await.me().await, Err(SeerrError::Auth(_))));
    }

    #[tokio::test]
    async fn network_error() {
        let c = SeerrClient::new("http://127.0.0.1:1", KEY).unwrap();
        assert!(matches!(c.status().await, Err(SeerrError::Network(_))));
    }

    #[tokio::test]
    async fn search_maps_and_filters() {
        let m = start(default_route).await;
        let page = client(&m).await.search("the matrix & co", 2).await.unwrap();
        let r = &m.find("GET", "/api/v1/search")[0];
        assert!(r.raw_target.contains("query=the%20matrix%20%26%20co"), "{}", r.raw_target);
        assert_eq!(r.query["query"], "the matrix & co");
        assert_eq!(r.query["page"], "2");
        assert_eq!((page.page, page.total_pages, page.total_results), (2, 3, 4));
        // person dropped; movie, tv, undated movie kept
        assert_eq!(page.results.len(), 3);
        let mv = &page.results[0];
        assert_eq!(
            (mv.media_type.as_str(), mv.tmdb_id, mv.title.as_str(), mv.year),
            ("movie", 27205, "Inception", Some(2010))
        );
        assert_eq!(mv.poster_path.as_deref(), Some("/inc.jpg"));
        assert_eq!(mv.availability, Availability::Available);
        assert_eq!(mv.request_status, None);
        let tv = &page.results[1];
        assert_eq!((tv.media_type.as_str(), tv.title.as_str(), tv.year), ("tv", "Game of Thrones", Some(2011)));
        assert_eq!(tv.availability, Availability::Pending);
        assert_eq!(tv.request_status, Some(RequestStatus::Pending));
        let bare = &page.results[2];
        assert_eq!((bare.year, bare.availability, bare.poster_path.clone()), (None, Availability::NotRequested, None));
        assert_eq!(bare.overview, "");
    }

    #[tokio::test]
    async fn request_movie_and_tv_bodies() {
        let m = start(default_route).await;
        let c = client(&m).await;
        let r = c.request("movie", 27205).await.unwrap();
        assert_eq!((r.request_id, r.status), (42, RequestStatus::Pending));
        let r = c.request("tv", 1399).await.unwrap();
        assert_eq!(r.request_id, 42);
        let posts = m.find("POST", "/api/v1/request");
        let b0: serde_json::Value = serde_json::from_str(&posts[0].body).unwrap();
        assert_eq!(b0, serde_json::json!({"mediaType": "movie", "mediaId": 27205}));
        let b1: serde_json::Value = serde_json::from_str(&posts[1].body).unwrap();
        assert_eq!(b1, serde_json::json!({"mediaType": "tv", "mediaId": 1399, "seasons": "all"}));
        assert_eq!(posts[1].headers["content-type"], "application/json");
        assert_eq!(posts[1].headers["x-api-key"], KEY);
    }

    #[tokio::test]
    async fn request_error_mapping() {
        let m = start(default_route).await;
        let c = client(&m).await;
        match c.request("movie", 999).await.unwrap_err() {
            SeerrError::AlreadyRequested(msg) => assert!(msg.contains("already exists")),
            e => panic!("{:?}", e),
        }
        assert!(matches!(c.request("movie", 998).await, Err(SeerrError::AlreadyRequested(_))));
        assert!(matches!(c.request("movie", 997).await, Err(SeerrError::Auth(_))));
        assert!(matches!(c.request("movie", 996).await, Err(SeerrError::Api(500, m)) if m == "boom"));
        assert!(matches!(c.request("person", 1).await, Err(SeerrError::Invalid(_))));
        // invalid type never hits the network
        assert_eq!(m.find("POST", "/api/v1/request").len(), 4);
    }

    #[tokio::test]
    async fn requests_list_is_enriched_with_titles() {
        let m = start(default_route).await;
        let items = client(&m).await.requests(20, 5).await.unwrap();
        let r = &m.find("GET", "/api/v1/request")[0];
        assert_eq!(r.raw_target, "/api/v1/request?take=20&skip=5&sort=added&filter=all");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].request_id, 7);
        assert_eq!((items[0].media_type.as_str(), items[0].title.as_str(), items[0].year), ("tv", "Game of Thrones", Some(2011)));
        assert_eq!(items[0].status, RequestStatus::Approved);
        assert_eq!(items[0].availability, Availability::Processing);
        assert_eq!(items[0].requested_by, "Nathan");
        assert_eq!(items[0].created_at, "2026-01-02T03:04:05.000Z");
        assert_eq!((items[1].title.as_str(), items[1].status), ("Inception", RequestStatus::Pending));
        assert_eq!(items[1].poster_path.as_deref(), Some("/inc.jpg"));
        // detail lookup 404 -> placeholder title, still listed
        assert_eq!((items[2].title.as_str(), items[2].status), ("TMDB #555", RequestStatus::Declined));
    }

    #[tokio::test]
    async fn api_error_carries_message() {
        let m = start(|_| Resp::json(500, serde_json::json!({"message": "db down"}))).await;
        match client(&m).await.search("x", 1).await.unwrap_err() {
            SeerrError::Api(500, msg) => assert_eq!(msg, "db down"),
            e => panic!("{:?}", e),
        }
        let m = start(|_| Resp::text(200, "not json")).await;
        assert!(matches!(client(&m).await.search("x", 1).await, Err(SeerrError::Parse(_))));
    }

    // ---- Jellyfin-account login mode ---------------------------------------

    async fn login_mock() -> (MockSeerr, LoginState) {
        let st = LoginState::default();
        let m = start(login_route("nathan", "pw", st.clone())).await;
        (m, st)
    }
    fn logins(m: &MockSeerr) -> Vec<Req> {
        m.find("POST", "/api/v1/auth/jellyfin")
    }

    #[tokio::test]
    async fn login_signs_in_once_and_reuses_cookie() {
        let (m, _st) = login_mock().await;
        let c = SeerrClient::with_login(&m.url(), " nathan ", "pw").unwrap();
        let (st, me) = c.verify().await.unwrap();
        assert_eq!((st.version.as_str(), me.display_name.as_str()), ("2.7.2", "Nathan"));
        c.search("x", 1).await.unwrap();
        c.requests(20, 0).await.unwrap(); // concurrent detail calls must not log in again
        c.request("movie", 27205).await.unwrap();
        let l = logins(&m);
        assert_eq!(l.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&l[0].body).unwrap();
        assert_eq!(body, serde_json::json!({"username": "nathan", "password": "pw"}));
        for r in m.reqs().iter().filter(|r| r.path != "/api/v1/auth/jellyfin" && r.path != "/api/v1/status") {
            assert_eq!(r.headers["cookie"], "connect.sid=s0; locale=en", "{} {}", r.method, r.path);
            assert!(!r.headers.contains_key("x-api-key"));
        }
        // status is public: no credentials, and it did not trigger the login
        assert!(!m.find("GET", "/api/v1/status")[0].headers.contains_key("cookie"));
    }

    #[tokio::test]
    async fn login_wrong_password_is_a_login_error() {
        let (m, _st) = login_mock().await;
        let c = SeerrClient::with_login(&m.url(), "nathan", "nope").unwrap();
        match c.verify().await.unwrap_err() {
            SeerrError::Login(msg) => assert!(msg.contains("Invalid credentials"), "{}", msg),
            e => panic!("{:?}", e),
        }
        assert!(matches!(c.search("x", 1).await, Err(SeerrError::Login(_))));
        assert!(SeerrError::Login("x".into()).to_string().starts_with("Seerr sign-in failed"));
        // 401 with no message gets a friendly one
        let m2 = start(|r| if r.path == "/api/v1/status" { default_route(r) } else { Resp::text(401, "") }).await;
        match SeerrClient::with_login(&m2.url(), "a", "b").unwrap().verify().await.unwrap_err() {
            SeerrError::Login(msg) => assert!(msg.contains("username and password"), "{}", msg),
            e => panic!("{:?}", e),
        }
    }

    #[tokio::test]
    async fn login_not_a_seerr_server() {
        let m = start(|_| Resp::json(404, serde_json::json!({"message": "nope"}))).await;
        let c = SeerrClient::with_login(&m.url(), "a", "b").unwrap();
        assert_eq!(c.me().await.unwrap_err(), SeerrError::NotSeerr);
    }

    #[tokio::test]
    async fn login_session_expiry_relogs_in_once_and_retries() {
        let (m, st) = login_mock().await;
        let c = SeerrClient::with_login(&m.url(), "nathan", "pw").unwrap();
        c.me().await.unwrap();
        assert_eq!(logins(&m).len(), 1);
        st.expire_sessions();
        let page = c.search("x", 1).await.unwrap(); // 403 -> login -> retry
        assert_eq!(page.results.len(), 3);
        assert_eq!(logins(&m).len(), 2);
        let searches = m.find("GET", "/api/v1/search");
        assert_eq!(searches.len(), 2);
        assert_eq!(searches[0].headers["cookie"], "connect.sid=s0; locale=en");
        assert_eq!(searches[1].headers["cookie"], "connect.sid=s1; locale=en");
        // a POST body survives the retry intact
        st.expire_sessions();
        c.request("tv", 1399).await.unwrap();
        let posts = m.find("POST", "/api/v1/request");
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].body, posts[1].body);
        assert!(posts[1].body.contains("\"seasons\":\"all\""));
        assert_eq!(logins(&m).len(), 3);
        // the new session is reused afterwards
        c.me().await.unwrap();
        assert_eq!(logins(&m).len(), 3);
    }

    #[tokio::test]
    async fn login_relogin_failure_and_persistent_403_do_not_loop() {
        let (m, st) = login_mock().await;
        let c = SeerrClient::with_login(&m.url(), "nathan", "pw").unwrap();
        c.me().await.unwrap();
        st.expire_sessions();
        st.deny_login.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(c.me().await, Err(SeerrError::Login(_))));
        assert_eq!(logins(&m).len(), 2); // initial + exactly one retry
        st.deny_login.store(false, std::sync::atomic::Ordering::SeqCst);
        // a genuine permission error (403 even with a fresh session) is reported after one relogin
        match c.request("movie", 997).await.unwrap_err() {
            SeerrError::Login(msg) => assert!(msg.contains("quota"), "{}", msg),
            e => panic!("{:?}", e),
        }
        assert_eq!(m.find("POST", "/api/v1/request").len(), 2);
        assert_eq!(logins(&m).len(), 3);
    }

    #[tokio::test]
    async fn login_without_set_cookie_is_an_error() {
        let m = start(|r| {
            if r.path == "/api/v1/auth/jellyfin" { Resp::json(200, serde_json::json!({"id": 1})) } else { default_route(r) }
        })
        .await;
        let c = SeerrClient::with_login(&m.url(), "a", "b").unwrap();
        assert!(matches!(c.me().await, Err(SeerrError::Login(m)) if m.contains("session")));
    }

    #[tokio::test]
    async fn login_mode_maps_other_errors_like_key_mode() {
        let (m, _st) = login_mock().await;
        let c = SeerrClient::with_login(&m.url(), "nathan", "pw").unwrap();
        assert!(matches!(c.request("movie", 999).await, Err(SeerrError::AlreadyRequested(_))));
        assert!(matches!(c.request("movie", 996).await, Err(SeerrError::Api(500, _))));
    }

    /// Read-only smoke test against a real server. Never creates or deletes
    /// anything (the only POST is the sign-in itself).
    ///   SEERR_LIVE_URL=https://seerr.example SEERR_LIVE_USER=name \
    ///   SEERR_LIVE_PASS_FILE=/path/to/password-file \
    ///   cargo test -- --ignored seerr_live --nocapture
    #[tokio::test]
    #[ignore]
    async fn seerr_live() {
        let (Ok(url), Ok(user), Ok(pf)) = (
            std::env::var("SEERR_LIVE_URL"),
            std::env::var("SEERR_LIVE_USER"),
            std::env::var("SEERR_LIVE_PASS_FILE"),
        ) else {
            panic!("set SEERR_LIVE_URL, SEERR_LIVE_USER and SEERR_LIVE_PASS_FILE");
        };
        let pass = std::fs::read_to_string(&pf).expect("password file");
        let c = SeerrClient::with_login(&url, &user, pass.trim_end_matches(['\r', '\n'])).unwrap();

        let st = c.status().await.expect("status");
        println!("status: version {}", st.version);
        assert!(!st.version.is_empty());

        let me = c.me().await.expect("me (login)");
        println!("me: id={} name={}", me.id, me.display_name);
        assert!(me.id > 0 && !me.display_name.is_empty());

        for (q, expect) in [("dune", "dune"), ("the office", "office")] {
            let page = c.search(q, 1).await.expect("search");
            println!("search {:?}: page {}/{} total {} -> {} kept", q, page.page, page.total_pages, page.total_results, page.results.len());
            assert!(page.page == 1 && page.total_pages >= 1 && page.total_results > 0);
            assert!(!page.results.is_empty());
            assert!(page.results.iter().any(|r| r.title.to_lowercase().contains(expect)), "no title containing {:?}", expect);
            for r in page.results.iter().take(8) {
                println!(
                    "  {:5} {:>7} {:<40} year={:?} avail={:?} req={:?} poster={:?}",
                    r.media_type, r.tmdb_id, r.title, r.year, r.availability, r.request_status, r.poster_path
                );
            }
            for r in &page.results {
                assert!(r.media_type == "movie" || r.media_type == "tv");
                assert!(r.tmdb_id > 0 && !r.title.is_empty(), "{:?}", r);
                if let Some(y) = r.year {
                    assert!((1870..=2100).contains(&y), "{:?}", r);
                }
                if let Some(p) = &r.poster_path {
                    assert!(p.starts_with('/'), "{:?}", r);
                    assert!(poster_url(&c.base, p).contains("/imageproxy/tmdb/t/p/"));
                }
            }
            // availability/request_status must agree with each other
            for r in &page.results {
                if r.request_status.is_some() {
                    assert_ne!(r.availability, Availability::NotRequested, "{:?}", r);
                }
            }
        }

        let reqs = c.requests(20, 0).await.expect("requests");
        println!("requests: {} items", reqs.len());
        for r in reqs.iter().take(20) {
            println!(
                "  #{:<5} {:5} {:>7} {:<40} year={:?} status={:?} avail={:?} by={:?} at={}",
                r.request_id, r.media_type, r.tmdb_id, r.title, r.year, r.status, r.availability, r.requested_by, r.created_at
            );
        }
        for r in &reqs {
            assert!(r.request_id > 0 && r.tmdb_id > 0);
            assert!(r.media_type == "movie" || r.media_type == "tv", "{:?}", r);
            assert!(!r.title.starts_with("TMDB #"), "title enrichment failed: {:?}", r);
            assert!(r.status != RequestStatus::Unknown, "{:?}", r);
            assert!(!r.requested_by.is_empty(), "{:?}", r);
            assert!(r.created_at.starts_with("20"), "{:?}", r);
        }
        // A title that has a request must show up as requested/available in search.
        if let Some(first) = reqs.first() {
            let page = c.search(&first.title, 1).await.expect("search for requested title");
            let hit = page
                .results
                .iter()
                .find(|r| r.tmdb_id == first.tmdb_id && r.media_type == first.media_type)
                .unwrap_or_else(|| panic!("{:?} not found in its own search", first.title));
            println!("cross-check {:?}: avail={:?} req={:?}", hit.title, hit.availability, hit.request_status);
            assert_eq!(hit.availability, first.availability);
            assert_ne!(hit.availability, Availability::NotRequested);
        }
        // newest first
        let times: Vec<&str> = reqs.iter().map(|r| r.created_at.as_str()).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]), "not newest-first: {:?}", times);
    }
}
