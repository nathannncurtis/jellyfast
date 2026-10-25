use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMethod {
    /// Static file, hardware-decoded on-device by AVFoundation/VideoToolbox.
    DirectPlay,
    /// Server-assisted HLS (documented fallback for exotic codecs).
    ServerHls,
}

/// Codecs AVFoundation can decode on macOS 14 (hardware paths on
/// Apple silicon). Kept as data so tests can pin behavior.
pub fn supported_video_codecs() -> &'static [&'static str] {
    &["h264", "hevc", "h265", "mpeg4", "mpeg2video", "prores", "dvh1", "dvhe"]
}
pub fn supported_audio_codecs() -> &'static [&'static str] {
    &["aac", "mp3", "ac3", "eac3", "alac", "flac", "pcm_s16le", "pcm_s24le", "opus"]
}
pub fn supported_containers() -> &'static [&'static str] {
    &["mp4", "m4v", "mov", "m4a", "mp3", "aac", "flac", "wav", "aiff", "m3u8"]
}

fn norm(c: Option<&str>) -> String {
    c.unwrap_or("").trim().to_ascii_lowercase()
}

/// Pure decision function — unit-tested below.
pub fn decide_playback(
    container: Option<&str>,
    video_codec: Option<&str>,
    audio_codec: Option<&str>,
    source_bitrate_kbps: u64,
    max_bitrate_kbps: u64,
) -> PlaybackMethod {
    let container_ok = supported_containers()
        .iter()
        .any(|c| container.map(|k| k.eq_ignore_ascii_case(c)).unwrap_or(false));
    let video_ok = supported_video_codecs().iter().any(|c| {
        let v = norm(video_codec);
        v == *c || (c == &"hevc" && (v == "hevc" || v == "h265" || v == "dvh1" || v == "dvhe"))
    });
    let audio_ok = supported_audio_codecs().iter().any(|c| norm(audio_codec).starts_with(c));
    let bitrate_ok = max_bitrate_kbps == 0 || source_bitrate_kbps == 0 || source_bitrate_kbps <= max_bitrate_kbps;
    if container_ok && video_ok && audio_ok && bitrate_ok {
        PlaybackMethod::DirectPlay
    } else {
        PlaybackMethod::ServerHls
    }
}

pub fn method_label(m: PlaybackMethod) -> &'static str {
    match m {
        PlaybackMethod::DirectPlay => "Direct Play",
        PlaybackMethod::ServerHls => "Transcode (HLS)",
    }
}

/// Resolve which MediaSource/plan to use for a full item. Pure-ish
/// (no IO): takes the already-fetched data.
pub fn build_stream_plan(
    media_source: Option<&crate::jellyfin::models::MediaSource>,
    max_bitrate_kbps: u64,
) -> (PlaybackMethod, u64) {
    match media_source {
        Some(ms) => {
            let method = decide_playback(
                ms.container.as_deref(),
                ms.video_codec(),
                ms.audio_codec(),
                ms.bitrate.unwrap_or(0) / 1000,
                max_bitrate_kbps,
            );
            (method, ms.bitrate.unwrap_or(0) / 1000)
        }
        None => (PlaybackMethod::ServerHls, 0),
    }
}
