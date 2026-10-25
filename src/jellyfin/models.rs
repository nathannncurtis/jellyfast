use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ServerInfo {
    #[serde(rename = "ProductName", default)]
    pub product_name: Option<String>,
    #[serde(rename = "Version", default)]
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct AuthRequest<'a> {
    #[serde(rename = "Username")]
    pub username: &'a str,
    #[serde(rename = "Pw")]
    pub pw: &'a str,
}

#[derive(Deserialize)]
pub struct AuthResponse {
    #[serde(rename = "AccessToken")]
    pub access_token: String,
    #[serde(rename = "User")]
    pub user: AuthUser,
}
#[derive(Deserialize)]
pub struct AuthUser {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct View {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "CollectionType", default)]
    pub collection_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ItemQuery {
    pub parent_id: Option<String>,
    pub series_id: Option<String>,
    pub types: Option<String>,
    pub search: Option<String>,
    pub sort_by: String,
    pub sort_order: String,
    pub filters: Vec<String>,
    pub limit: u32,
}
impl Default for ItemQuery {
    fn default() -> Self {
        ItemQuery {
            parent_id: None,
            series_id: None,
            types: None,
            search: None,
            sort_by: "SortName".into(),
            sort_order: "Ascending".into(),
            filters: vec![],
            limit: 400,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListItem {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub item_type: String,
    #[serde(rename = "SeriesName", default)]
    pub series_name: Option<String>,
    #[serde(rename = "ProductionYear", default)]
    pub year: Option<u32>,
    #[serde(rename = "ParentIndexNumber", default)]
    pub season: Option<u32>,
    #[serde(rename = "IndexNumber", default)]
    pub episode: Option<u32>,
    #[serde(rename = "UserData", default)]
    pub user_data: Option<UserData>,
    #[serde(rename = "PrimaryImageAspectRatio", default)]
    pub aspect: Option<f64>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: Option<ImageTags>,
    #[serde(rename = "RunTimeTicks", default)]
    pub run_time_ticks: Option<i64>,
}
impl ListItem {
    pub fn has_image(&self) -> bool {
        self.image_tags
            .as_ref()
            .map(|t| t.primary.is_some())
            .unwrap_or(false)
    }
    pub fn sub_label(&self) -> String {
        let mut s = String::new();
        if let Some(y) = self.year {
            s.push_str(&y.to_string());
        }
        if self.item_type == "Episode" {
            if let (Some(sn), Some(ep)) = (self.season, self.episode) {
                s = format!("S{:02}E{:02}", sn, ep);
                if let Some(sr) = &self.series_name {
                    s = format!("{} · {}", sr, s);
                }
            }
        }
        s
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ImageTags {
    #[serde(rename = "Primary", default)]
    pub primary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct UserData {
    #[serde(rename = "Played", default)]
    pub played: bool,
    #[serde(rename = "PlaybackPositionTicks", default)]
    pub position_ticks: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FullItem {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub item_type: String,
    #[serde(rename = "SeriesName", default)]
    pub series_name: Option<String>,
    #[serde(rename = "RunTimeTicks", default)]
    pub run_time_ticks: Option<i64>,
    #[serde(rename = "UserData", default)]
    pub user_data: Option<UserData>,
    #[serde(rename = "MediaSources", default)]
    pub media_sources: Vec<MediaSource>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MediaSource {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Container", default)]
    pub container: Option<String>,
    #[serde(rename = "Bitrate", default)]
    pub bitrate: Option<u64>,
    #[serde(rename = "MediaStreams", default)]
    pub streams: Vec<MediaStream>,
    /// Present on PlaybackInfo responses (relative URL, e.g. "/videos/<id>/master.m3u8?...").
    #[serde(rename = "TranscodingUrl", default)]
    pub transcoding_url: Option<String>,
    #[serde(rename = "LiveStreamId", default)]
    pub live_stream_id: Option<String>,
    #[serde(rename = "SupportsDirectPlay", default)]
    pub supports_direct_play: bool,
    #[serde(rename = "SupportsDirectStream", default)]
    pub supports_direct_stream: bool,
    #[serde(rename = "SupportsTranscoding", default)]
    pub supports_transcoding: bool,
}

/// POST /Items/{id}/PlaybackInfo response.
#[derive(Clone, Debug, Deserialize)]
pub struct PlaybackInfo {
    #[serde(rename = "MediaSources", default)]
    pub media_sources: Vec<MediaSource>,
    #[serde(rename = "PlaySessionId", default)]
    pub play_session_id: Option<String>,
    #[serde(rename = "ErrorCode", default)]
    pub error_code: Option<String>,
}
impl MediaSource {
    pub fn video_codec(&self) -> Option<&str> {
        self.streams
            .iter()
            .find(|s| s.stream_type == "Video")
            .and_then(|s| s.codec.as_deref())
    }
    pub fn audio_codec(&self) -> Option<&str> {
        self.streams
            .iter()
            .find(|s| s.stream_type == "Audio")
            .and_then(|s| s.codec.as_deref())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub stream_type: String,
    #[serde(rename = "Codec", default)]
    pub codec: Option<String>,
    #[serde(rename = "Language", default)]
    pub language: Option<String>,
    #[serde(rename = "DisplayTitle", default)]
    pub display_title: Option<String>,
}
