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
    #[serde(rename = "Album", default)]
    pub album: Option<String>,
    #[serde(rename = "AlbumArtist", default)]
    pub album_artist: Option<String>,
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

/// One entry of an item's `People` list (cast and crew).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Person {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    /// Character name for actors; empty for crew.
    #[serde(rename = "Role", default)]
    pub role: Option<String>,
    /// "Actor", "Director", "Writer", "GuestStar", ...
    #[serde(rename = "Type", default)]
    pub kind: Option<String>,
    #[serde(rename = "PrimaryImageTag", default)]
    pub image_tag: Option<String>,
}

/// GET /Users/{uid}/Items/{id}?Fields=... for the item detail page.
#[derive(Clone, Debug, Deserialize)]
pub struct DetailItem {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub item_type: String,
    #[serde(rename = "ProductionYear", default)]
    pub year: Option<u32>,
    #[serde(rename = "OfficialRating", default)]
    pub official_rating: Option<String>,
    #[serde(rename = "CommunityRating", default)]
    pub community_rating: Option<f64>,
    #[serde(rename = "RunTimeTicks", default)]
    pub run_time_ticks: Option<i64>,
    #[serde(rename = "Genres", default)]
    pub genres: Vec<String>,
    #[serde(rename = "Overview", default)]
    pub overview: Option<String>,
    #[serde(rename = "People", default)]
    pub people: Vec<Person>,
    #[serde(rename = "UserData", default)]
    pub user_data: Option<UserData>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: Option<ImageTags>,
    #[serde(rename = "BackdropImageTags", default)]
    pub backdrop_tags: Vec<String>,
}

/// One season of a series (GET /Shows/{id}/Seasons).
#[derive(Clone, Debug, Deserialize)]
pub struct Season {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "IndexNumber", default)]
    pub index: Option<u32>,
}

/// One episode from /Shows/{id}/Episodes or /Shows/NextUp.
#[derive(Clone, Debug, Deserialize)]
pub struct Episode {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "IndexNumber", default)]
    pub number: Option<u32>,
    #[serde(rename = "ParentIndexNumber", default)]
    pub parent_index: Option<u32>,
    #[serde(rename = "SeasonId", default)]
    pub season_id: Option<String>,
    #[serde(rename = "Overview", default)]
    pub overview: Option<String>,
    #[serde(rename = "RunTimeTicks", default)]
    pub run_time_ticks: Option<i64>,
    #[serde(rename = "UserData", default)]
    pub user_data: Option<UserData>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: Option<ImageTags>,
}

impl Episode {
    pub fn has_image(&self) -> bool {
        self.image_tags.as_ref().map(|t| t.primary.is_some()).unwrap_or(false)
    }
}

/// The `{ "Items": [...] }` envelope most Jellyfin list endpoints return.
#[derive(Debug, Deserialize)]
pub struct ItemsPage<T> {
    #[serde(rename = "Items", default = "Vec::new")]
    pub items: Vec<T>,
}

#[cfg(test)]
mod season_model_tests {
    use super::*;

    #[test]
    fn parses_seasons_and_episodes() {
        let seasons: ItemsPage<Season> = serde_json::from_str(
            r#"{"Items":[{"Id":"s1","Name":"Season 1","IndexNumber":1},{"Id":"s0","Name":"Specials","IndexNumber":0}],"TotalRecordCount":2}"#,
        )
        .unwrap();
        assert_eq!(seasons.items.len(), 2);
        assert_eq!((seasons.items[0].id.as_str(), seasons.items[0].index), ("s1", Some(1)));
        assert_eq!(seasons.items[1].name, "Specials");

        let eps: ItemsPage<Episode> = serde_json::from_str(
            r#"{"Items":[{"Id":"e1","Name":"Pilot","IndexNumber":1,"ParentIndexNumber":1,"SeasonId":"s1",
                "Overview":"Hello","RunTimeTicks":12000000000,
                "UserData":{"Played":false,"PlaybackPositionTicks":3000000000},
                "ImageTags":{"Primary":"t"}}]}"#,
        )
        .unwrap();
        let e = &eps.items[0];
        assert_eq!(e.season_id.as_deref(), Some("s1"));
        assert_eq!((e.number, e.parent_index), (Some(1), Some(1)));
        assert_eq!(e.user_data.as_ref().map(|u| u.position_ticks), Some(3_000_000_000));
        assert!(e.has_image());
    }

    #[test]
    fn missing_items_envelope_is_empty() {
        let p: ItemsPage<Episode> = serde_json::from_str("{}").unwrap();
        assert!(p.items.is_empty());
    }
}
