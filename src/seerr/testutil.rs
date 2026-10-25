//! Test-only mock Seerr HTTP server (TcpListener) shared by client and worker tests.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub raw_target: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct Resp {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
}
impl Resp {
    pub fn json(status: u16, v: serde_json::Value) -> Resp {
        Resp { status, body: v.to_string(), headers: vec![] }
    }
    pub fn text(status: u16, s: &str) -> Resp {
        Resp { status, body: s.to_string(), headers: vec![] }
    }
    pub fn with_header(mut self, k: &str, v: &str) -> Resp {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct MockSeerr {
    pub addr: std::net::SocketAddr,
    pub log: Arc<Mutex<Vec<Req>>>,
    task: tokio::task::JoinHandle<()>,
}
impl MockSeerr {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub fn reqs(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
    pub fn find(&self, method: &str, path: &str) -> Vec<Req> {
        self.reqs().into_iter().filter(|r| r.method == method && r.path == path).collect()
    }
}
impl Drop for MockSeerr {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
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

pub async fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> MockSeerr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log: Arc<Mutex<Vec<Req>>> = Default::default();
    let handler = Arc::new(handler);
    let l2 = log.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (log, handler) = (l2.clone(), handler.clone());
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
                let req = Req { method, path: pct_decode(path), raw_target: target.clone(), query, headers, body };
                log.lock().unwrap().push(req.clone());
                let resp = handler(&req);
                let extra: String = resp.headers.iter().map(|(k, v)| format!("{}: {}\r\n", k, v)).collect();
                let head = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    resp.status,
                    extra,
                    resp.body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(resp.body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    MockSeerr { addr, log, task }
}

/// A well-behaved fake Seerr. API key "good-key". Special request ids:
/// 999 -> 409, 998 -> 500 "already requested" message, 997 -> 403, 996 -> 500 boom.
pub fn default_route(r: &Req) -> Resp {
    if r.method == "GET" && r.path == "/api/v1/status" {
        return Resp::json(200, serde_json::json!({"version": "2.7.2", "commitTag": "x", "updateAvailable": false}));
    }
    // Seerr's image proxy needs no credentials.
    if let Some(p) = r.path.strip_prefix("/imageproxy/tmdb/t/p/w92") {
        return match p {
            "/inc.jpg" | "/got.jpg" => Resp::text(200, &format!("IMG{}", p)),
            _ => Resp::text(404, "nope"),
        };
    }
    if r.headers.get("x-api-key").map(String::as_str) != Some("good-key") {
        return Resp::json(403, serde_json::json!({"message": "You do not have permission to access this endpoint"}));
    }
    match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/api/v1/auth/me") => Resp::json(200, serde_json::json!({"id": 1, "displayName": "Nathan", "email": "n@x.io"})),
        ("GET", "/api/v1/search") => {
            let page: u32 = r.query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
            Resp::json(
                200,
                serde_json::json!({
                    "page": page, "totalPages": 3, "totalResults": 4,
                    "results": [
                        {"id": 27205, "mediaType": "movie", "title": "Inception", "releaseDate": "2010-07-16",
                         "posterPath": "/inc.jpg", "overview": "Dreams.", "mediaInfo": {"status": 5, "requests": []}},
                        {"id": 1399, "mediaType": "tv", "name": "Game of Thrones", "firstAirDate": "2011-04-17",
                         "posterPath": "/got.jpg", "overview": "Dragons.",
                         "mediaInfo": {"status": 2, "requests": [{"status": 1}]}},
                        {"id": 31, "mediaType": "person", "name": "Leonardo DiCaprio", "profilePath": "/l.jpg"},
                        {"id": 600, "mediaType": "movie", "title": "No Date", "releaseDate": "", "posterPath": null}
                    ]
                }),
            )
        }
        ("POST", "/api/v1/request") => {
            let b: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
            let id = b["mediaId"].as_u64().unwrap_or(0);
            if b["mediaType"] == "tv" && b["seasons"] != "all" {
                return Resp::json(400, serde_json::json!({"message": "seasons required"}));
            }
            match id {
                999 => Resp::json(409, serde_json::json!({"message": "Request for this media already exists."})),
                998 => Resp::json(500, serde_json::json!({"message": "Media already requested"})),
                997 => Resp::json(403, serde_json::json!({"message": "Request quota exceeded"})),
                996 => Resp::json(500, serde_json::json!({"message": "boom"})),
                _ => Resp::json(201, serde_json::json!({"id": 42, "status": 1, "type": b["mediaType"], "media": {"tmdbId": id}})),
            }
        }
        ("GET", "/api/v1/request") => Resp::json(
            200,
            serde_json::json!({"pageInfo": {"results": 3}, "results": [
                {"id": 7, "status": 2, "type": "tv", "createdAt": "2026-01-02T03:04:05.000Z",
                 "media": {"tmdbId": 1399, "mediaType": "tv", "status": 3}, "requestedBy": {"id": 1, "displayName": "Nathan"}},
                {"id": 6, "status": 1, "type": "movie", "media": {"tmdbId": 27205, "mediaType": "movie", "status": 2},
                 "requestedBy": {"id": 1, "displayName": "Nathan"}},
                {"id": 5, "status": 3, "type": "movie", "media": {"tmdbId": 555, "mediaType": "movie", "status": 1}}
            ]}),
        ),
        ("GET", "/api/v1/movie/27205") => Resp::json(200, serde_json::json!({"id": 27205, "title": "Inception", "releaseDate": "2010-07-16", "posterPath": "/inc.jpg"})),
        ("GET", "/api/v1/tv/1399") => Resp::json(200, serde_json::json!({"id": 1399, "name": "Game of Thrones", "firstAirDate": "2011-04-17", "posterPath": "/got.jpg"})),
        _ => Resp::json(404, serde_json::json!({"message": "not found"})),
    }
}

/// Knobs for [`login_route`].
#[derive(Clone, Default)]
pub struct LoginState {
    /// Bumping this invalidates every previously issued session cookie.
    pub epoch: Arc<std::sync::atomic::AtomicU32>,
    /// While true the login endpoint rejects everyone.
    pub deny_login: Arc<std::sync::atomic::AtomicBool>,
}
impl LoginState {
    pub fn expire_sessions(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Fake Seerr that authenticates by Jellyfin login (`user` / `pass`) and a
/// `connect.sid=s<epoch>` session cookie; API keys are not accepted. Everything
/// behind authentication is delegated to [`default_route`].
pub fn login_route(user: &'static str, pass: &'static str, st: LoginState) -> impl Fn(&Req) -> Resp + Send + Sync + 'static {
    use std::sync::atomic::Ordering::SeqCst;
    move |r: &Req| {
        let epoch = st.epoch.load(SeqCst);
        if r.method == "POST" && r.path == "/api/v1/auth/jellyfin" {
            let b: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
            if st.deny_login.load(SeqCst) || b["username"] != user || b["password"] != pass {
                return Resp::json(401, serde_json::json!({"message": "Invalid credentials"}));
            }
            return Resp::json(200, serde_json::json!({"id": 1, "displayName": "Nathan"}))
                .with_header("Set-Cookie", &format!("connect.sid=s{}; Path=/; HttpOnly", epoch))
                .with_header("Set-Cookie", "locale=en; Path=/");
        }
        if r.method == "GET" && r.path == "/api/v1/status" {
            return default_route(r);
        }
        let ok = r.headers.get("cookie").map(|c| c.contains(&format!("connect.sid=s{}", epoch))).unwrap_or(false);
        if !ok {
            return Resp::json(403, serde_json::json!({"message": "You do not have permission to access this endpoint"}));
        }
        let mut r2 = r.clone();
        r2.headers.insert("x-api-key".into(), "good-key".into());
        default_route(&r2)
    }
}
