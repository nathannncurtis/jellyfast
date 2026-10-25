use super::models::*;
use crate::jellyfin::urls::{enc, normalize_base_url};
use std::fmt;
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
pub struct SeerrClient {
    http: reqwest::Client,
    pub base: String,
    api_key: String,
}

impl SeerrClient {
    pub fn new(base: &str, api_key: &str) -> Result<Self, SeerrError> {
        let base = normalize_base_url(base).map_err(SeerrError::BadUrl)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .user_agent("JellyfinMac/0.1.0")
            .build()
            .map_err(|e| SeerrError::Network(e.to_string()))?;
        Ok(Self { http, base, api_key: api_key.trim().to_string() })
    }

    async fn send(&self, rb: reqwest::RequestBuilder, what: &str) -> Result<String, SeerrError> {
        let r = rb
            .header("X-Api-Key", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(map_err)?;
        let code = r.status().as_u16();
        let body = r.text().await.map_err(|e| SeerrError::Network(e.to_string()))?;
        check(code, &body, what)?;
        Ok(body)
    }

    async fn get(&self, path: &str) -> Result<String, SeerrError> {
        self.send(self.http.get(format!("{}{}", self.base, path)), &format!("GET {}", path.split('?').next().unwrap_or(path)))
            .await
    }

    /// GET /api/v1/status: unauthenticated health check; proves it's Seerr.
    pub async fn status(&self) -> Result<SeerrStatusInfo, SeerrError> {
        let body = self.get("/api/v1/status").await?;
        let s: RawStatus = serde_json::from_str(&body).map_err(|_| SeerrError::NotSeerr)?;
        match s.version.filter(|v| !v.is_empty()) {
            Some(version) => Ok(SeerrStatusInfo { version }),
            None => Err(SeerrError::NotSeerr),
        }
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
}
