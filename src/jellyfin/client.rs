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
        Ok(Self { http, base, token, user_id })
    }

    fn auth_header(&self) -> String {
        format!(
            "MediaBrowser Client=\"JellyfinMac\", Device=\"Mac\", DeviceId=\"{}\", Version=\"0.1.0\"",
            enc(&self.device_id_pub())
        )
    }

    fn device_id_pub(&self) -> String {
        // DeviceId is embedded in the auth header string by callers
        // that know it; we derive a stable one from user_id fallback.
        self.user_id.clone()
    }

    fn req(&self) -> reqwest::RequestBuilder {
        self.http
            .request(
                reqwest::Method::GET,
                format!("{}", self.base),
            )
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
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
        if code.as_u16() == 401 {
            return Err(JFError::Auth("wrong username or password".into()));
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
        let mut url = format!(
            "{}/Users/{}/Items",
            self.base,
            enc(&self.user_id)
        );
        if q.parent_id.is_some() || q.series_id.is_some() {
            // series/album scoping uses the same items endpoint
        }
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
            .query(&[("UserId", enc(&self.user_id)), ("Limit", "500".to_string())])
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
        let (m, path) = if played {
            (reqwest::Method::POST, "PlayedItems")
        } else {
            (reqwest::Method::DELETE, "PlayedItems")
        };
        let r = self
            .http
            .request(
                m,
                format!(
                    "{}/Users/{}/Items/{}/{}",
                    self.base,
                    enc(&self.user_id),
                    enc(item_id),
                    path
                ),
            )
            .header("Authorization", self.auth_header())
            .header("X-Emby-Token", &self.token)
            .send()
            .await
            .map_err(map_err)?;
        let r = check(r).await?;
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
