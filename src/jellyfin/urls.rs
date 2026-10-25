/// Normalize a server base URL: require http/https, strip trailing '/'.
pub fn normalize_base_url(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Server URL is empty.".into());
    }
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }
    let trimmed = s.trim_end_matches('/');
    if trimmed.len() < "http://x".len() {
        return Err("URL is too short to be valid.".into());
    }
    Ok(trimmed.to_string())
}

/// What the user typed as a server address, split into its optional explicit
/// scheme and the bare `host[:port][/path]` part (no trailing slash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInput {
    pub scheme: Option<String>,
    pub rest: String,
}

/// Parse a server address typed by the user. Accepts a bare host such as
/// `media.example.com` or `192.168.1.10:8096/jellyfin/`, or a full URL with an
/// explicit `http://` / `https://`. Query and fragment are dropped, repeated
/// slashes and trailing slashes in the path are collapsed.
pub fn parse_server_input(input: &str) -> Result<ServerInput, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Server URL is empty.".into());
    }
    let (scheme, rest) = match s.find("://") {
        Some(i) => {
            let sc = s[..i].to_ascii_lowercase();
            if sc != "http" && sc != "https" {
                return Err("Use an http:// or https:// address, or just the host name.".into());
            }
            (Some(sc), &s[i + 3..])
        }
        None => (None, s),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let authority = authority.trim();
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return Err("Enter a host name or IP address, like media.example.com or 192.168.1.10:8096.".into());
    }
    let mut out = authority.to_string();
    for seg in path.split('/').filter(|p| !p.is_empty()) {
        out.push('/');
        out.push_str(seg);
    }
    Ok(ServerInput { scheme, rest: out })
}

/// Candidate base URLs to probe, in order. An explicit scheme is used alone;
/// a bare host tries https first and falls back to http.
pub fn candidate_bases(input: &str) -> Result<Vec<String>, String> {
    let p = parse_server_input(input)?;
    Ok(match p.scheme {
        Some(sc) => vec![format!("{}://{}", sc, p.rest)],
        None => vec![format!("https://{}", p.rest), format!("http://{}", p.rest)],
    })
}

/// Display form of a server address: the scheme and trailing slashes removed,
/// e.g. `https://media.example.com/` -> `media.example.com`. Falls back to the
/// trimmed input when it does not parse.
pub fn bare_url(input: &str) -> String {
    match parse_server_input(input) {
        Ok(p) => p.rest,
        Err(_) => input.trim().trim_end_matches('/').to_string(),
    }
}

/// RFC 3986 unreserved-char percent encoding (query values, tokens).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn qs(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Direct-play URL (static file). AVPlayer fetches with Apple's
/// networking; auth rides in the `api_key` query param, which Jellyfin
/// accepts on media/image endpoints.
pub fn build_direct_url(
    base: &str,
    item_id: &str,
    media_source_id: &str,
    container: Option<&str>,
    token: &str,
) -> String {
    let ext = container
        .map(|c| format!(".{}", c.split(',').next().unwrap_or("mp4")))
        .unwrap_or_default();
    format!(
        "{}/Videos/{}/stream{}?{}",
        base,
        item_id,
        ext,
        qs(&[
            ("static", "true".into()),
            ("MediaSourceId", media_source_id.into()),
            ("api_key", token.into()),
        ])
    )
}

/// Server-assisted HLS fallback (documented limitation: used only for
/// codecs/containers Apple cannot decode on this device).
pub fn build_hls_url(
    base: &str,
    item_id: &str,
    media_source_id: &str,
    token: &str,
    max_bitrate_kbps: u64,
    subtitle_lang: &str,
) -> String {
    let mut pairs: Vec<(&str, String)> = vec![
        ("MediaSourceId", media_source_id.into()),
        ("VideoCodec", "h264,hevc".into()),
        ("AudioCodec", "aac,mp3,ac3,eac3".into()),
        ("TranscodingMaxAudioChannels", "2".into()),
        ("SegmentContainer", "mp4".into()),
        ("MinSegments", "1".into()),
        ("BreakOnNonKeyFrames", "True".into()),
        (
            "TranscodeReasons",
            "ContainerNotSupported,VideoCodecNotSupported,AudioCodecNotSupported".into(),
        ),
        ("api_key", token.into()),
    ];
    if max_bitrate_kbps > 0 {
        pairs.push(("VideoBitrate", max_bitrate_kbps.to_string()));
        pairs.push(("MaxWidth", "3840".into()));
        pairs.push(("MaxHeight", "2160".into()));
    }
    if !subtitle_lang.is_empty() {
        // Unverified param name; see report. Previously also sent an empty
        // SubtitleStreamIndex, which carried no meaning and was removed.
        pairs.push(("SubtitleLanguage", subtitle_lang.into()));
    }
    format!("{}/Videos/{}/master.m3u8?{}", base, item_id, qs(&pairs))
}

pub fn build_image_url(base: &str, item_id: &str, token: &str, max_w: u32) -> String {
    format!(
        "{}/Items/{}/Images/Primary?{}",
        base,
        item_id,
        qs(&[
            ("maxWidth", max_w.to_string()),
            ("quality", "85".into()),
            ("api_key", token.into()),
        ])
    )
}

pub fn build_livetv_url(base: &str, channel_id: &str, token: &str) -> String {
    format!(
        "{}/LiveTv/Channels/{}/stream?{}",
        base,
        channel_id,
        qs(&[("api_key", token.into())])
    )
}

/// Jellyfin web deep-link (used by the context menu "Open in Web").
pub fn build_web_item_url(base: &str, item_id: &str) -> String {
    format!("{}/web/index.html#!/itemdetails.html?id={}", base, enc(item_id))
}

/// Ticks conversions (Jellyfin uses 100ns ticks).
pub const TICKS_PER_SEC: i64 = 10_000_000;
pub fn secs_from_ticks(t: i64) -> f64 {
    t as f64 / TICKS_PER_SEC as f64
}
pub fn ticks_from_secs(s: f64) -> i64 {
    (s.max(0.0) * TICKS_PER_SEC as f64) as i64
}
pub fn progress_fraction(pos_ticks: i64, runtime_ticks: Option<i64>) -> f64 {
    match runtime_ticks {
        Some(rt) if rt > 0 => (pos_ticks as f64 / rt as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

pub fn fmt_clock(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, sec)
    } else {
        format!("{}:{:02}", m, sec)
    }
}

pub fn fmt_runtime(ticks: Option<i64>) -> String {
    match ticks {
        Some(t) if t > 0 => {
            let mins = (secs_from_ticks(t) as i64) / 60;
            if mins >= 60 {
                format!("{}h {:02}m", mins / 60, mins % 60)
            } else {
                format!("{}m", mins)
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod server_input_tests {
    use super::*;

    #[test]
    fn bare_hosts_parse_without_a_scheme() {
        let p = parse_server_input("  media.example.com  ").unwrap();
        assert_eq!(p, ServerInput { scheme: None, rest: "media.example.com".into() });
        assert_eq!(parse_server_input("192.168.1.10:8096").unwrap().rest, "192.168.1.10:8096");
    }

    #[test]
    fn explicit_scheme_is_kept_and_lowercased() {
        let p = parse_server_input("HTTP://jf.local:8096").unwrap();
        assert_eq!(p.scheme.as_deref(), Some("http"));
        assert_eq!(p.rest, "jf.local:8096");
        assert!(parse_server_input("ftp://x.io").is_err());
    }

    #[test]
    fn paths_slashes_query_and_fragment_are_normalised() {
        assert_eq!(parse_server_input("x.io/jf//").unwrap().rest, "x.io/jf");
        assert_eq!(parse_server_input("x.io//a///b/").unwrap().rest, "x.io/a/b");
        assert_eq!(parse_server_input("https://x.io/web/?a=1#frag").unwrap().rest, "x.io/web");
        assert_eq!(parse_server_input("x.io/").unwrap().rest, "x.io");
    }

    #[test]
    fn empty_or_unusable_input_is_rejected() {
        assert!(parse_server_input("   ").is_err());
        assert!(parse_server_input("https://").is_err());
        assert!(parse_server_input("/only/a/path").is_err());
        assert!(parse_server_input("bad host:8096").is_err());
    }

    #[test]
    fn bare_host_tries_https_then_http() {
        assert_eq!(
            candidate_bases("media.example.com:8096/").unwrap(),
            vec!["https://media.example.com:8096", "http://media.example.com:8096"]
        );
    }

    #[test]
    fn explicit_scheme_has_a_single_candidate() {
        assert_eq!(candidate_bases("http://h:8096/").unwrap(), vec!["http://h:8096"]);
        assert_eq!(candidate_bases("https://h").unwrap(), vec!["https://h"]);
    }

    #[test]
    fn bare_url_strips_scheme_and_trailing_slash_for_display() {
        assert_eq!(bare_url("https://media.example.com/"), "media.example.com");
        assert_eq!(bare_url("http://h:5055/api"), "h:5055/api");
        assert_eq!(bare_url("h:8096"), "h:8096");
        assert_eq!(bare_url(""), "");
    }
}
