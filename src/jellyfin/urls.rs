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
    build_typed_image_url(base, item_id, "Primary", token, max_w)
}

/// Image URL for a given Jellyfin image type (`Primary`, `Thumb`, `Backdrop`).
pub fn build_typed_image_url(base: &str, item_id: &str, kind: &str, token: &str, max_w: u32) -> String {
    // Backdrops are a list, so they take an explicit index; the others are single.
    let index = if kind == "Backdrop" { "/0" } else { "" };
    format!(
        "{}/Items/{}/Images/{}{}?{}",
        base,
        item_id,
        kind,
        index,
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
