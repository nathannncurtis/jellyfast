use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Pipe-separated genre names (`Genres` query param), e.g. "Comedy|Drama".
    pub genres: Option<String>,
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
            genres: None,
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
    #[serde(rename = "BackdropImageTags", default)]
    pub backdrop_tags: Option<Vec<String>>,
    #[serde(rename = "RunTimeTicks", default)]
    pub run_time_ticks: Option<i64>,
    /// ISO 8601 date the item was added; orders Recently Added across libraries.
    #[serde(rename = "DateCreated", default)]
    pub date_created: Option<String>,
    #[serde(rename = "Overview", default)]
    pub overview: Option<String>,
    #[serde(rename = "Genres", default)]
    pub genres: Vec<String>,
    /// Critic/community score out of 10 (e.g. 7.8).
    #[serde(rename = "CommunityRating", default)]
    pub community_rating: Option<f64>,
    /// Content rating such as "PG-13" or "TV-MA".
    #[serde(rename = "OfficialRating", default)]
    pub official_rating: Option<String>,
    /// External ids keyed by provider name: "Tmdb", "Imdb", "Tvdb".
    #[serde(rename = "ProviderIds", default)]
    pub provider_ids: HashMap<String, String>,
}
impl ListItem {
    /// Tag of the first backdrop image (wide, unlabelled artwork), if any.
    pub fn backdrop_tag(&self) -> Option<&str> {
        self.backdrop_tags.as_ref().and_then(|b| b.first()).map(String::as_str)
    }
    /// Tag of the transparent title logo, if the item has one.
    pub fn logo_tag(&self) -> Option<&str> {
        self.image_tags.as_ref().and_then(|t| t.logo.as_deref())
    }
    /// TMDB id, used to address Seerr image proxy and detail lookups.
    pub fn tmdb_id(&self) -> Option<&str> {
        self.provider_ids.get("Tmdb").map(String::as_str)
    }
    pub fn has_image(&self) -> bool {
        self.image_tags
            .as_ref()
            .map(|t| t.primary.is_some())
            .unwrap_or(false)
    }
    /// Image type for a wide (16:9) card: Thumb, else Backdrop, else Primary
    /// (cropped). `None` when the item has no artwork at all.
    pub fn wide_art_kind(&self) -> Option<&'static str> {
        let tags = self.image_tags.as_ref();
        if tags.and_then(|t| t.thumb.as_ref()).is_some() {
            Some("Thumb")
        } else if self.backdrop_tags.as_ref().is_some_and(|b| !b.is_empty()) {
            Some("Backdrop")
        } else if tags.and_then(|t| t.primary.as_ref()).is_some() {
            Some("Primary")
        } else {
            None
        }
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
    #[serde(rename = "Thumb", default)]
    pub thumb: Option<String>,
    #[serde(rename = "Logo", default)]
    pub logo: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct UserData {
    #[serde(rename = "Played", default)]
    pub played: bool,
    #[serde(rename = "PlaybackPositionTicks", default)]
    pub position_ticks: i64,
    /// ISO 8601 time of the last play; used to order Continue Watching across libraries.
    #[serde(rename = "LastPlayedDate", default)]
    pub last_played_date: Option<String>,
    #[serde(rename = "IsFavorite", default)]
    pub is_favorite: bool,
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
    #[serde(rename = "SeriesId", default)]
    pub series_id: Option<String>,
    #[serde(rename = "ParentIndexNumber", default)]
    pub season: Option<u32>,
    #[serde(rename = "IndexNumber", default)]
    pub episode: Option<u32>,
    #[serde(rename = "ProductionYear", default)]
    pub production_year: Option<u32>,
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
    #[serde(rename = "Width", default)]
    pub width: Option<u32>,
    /// "SDR", "HDR" or "Unknown" on video streams.
    #[serde(rename = "VideoRange", default)]
    pub video_range: Option<String>,
    /// Finer than VideoRange: "DOVI", "DOVIWithHDR10", "HDR10", "HDR10Plus", "HLG", ...
    #[serde(rename = "VideoRangeType", default)]
    pub video_range_type: Option<String>,
    #[serde(rename = "Channels", default)]
    pub channels: Option<u32>,
    /// Audio profile, e.g. "Dolby Atmos" on TrueHD/E-AC-3 tracks.
    #[serde(rename = "Profile", default)]
    pub profile: Option<String>,
    #[serde(rename = "IsHearingImpaired", default)]
    pub is_hearing_impaired: bool,
}

/// Quality badges for the detail page, from the item's first media source:
/// resolution, HDR format, audio layout, then captions. Empty when nothing is known.
pub fn quality_badges(source: Option<&MediaSource>) -> Vec<&'static str> {
    let Some(src) = source else { return Vec::new() };
    let mut out = Vec::new();
    let video = src.streams.iter().find(|s| s.stream_type == "Video");
    if let Some(w) = video.and_then(|v| v.width) {
        if w >= 3800 {
            out.push("4K");
        } else if w >= 1200 {
            out.push("HD");
        }
    }
    if let Some(v) = video {
        let range_type = v.video_range_type.as_deref().unwrap_or("");
        if range_type.starts_with("DOVI") {
            out.push("Dolby Vision");
        } else if range_type.starts_with("HDR10") || v.video_range.as_deref() == Some("HDR") {
            out.push("HDR10");
        }
    }
    if let Some(a) = src.streams.iter().find(|s| s.stream_type == "Audio") {
        let atmos = [a.display_title.as_deref(), a.profile.as_deref()]
            .into_iter()
            .flatten()
            .any(|t| t.to_ascii_lowercase().contains("atmos"));
        if atmos {
            out.push("Atmos");
        } else if a.channels.unwrap_or(0) >= 8 {
            out.push("7.1");
        } else if a.channels == Some(6) {
            out.push("5.1");
        }
    }
    let captions = src.streams.iter().any(|s| {
        s.stream_type == "Subtitle"
            && (s.is_hearing_impaired
                || s.display_title.as_deref().is_some_and(|t| t.contains("SDH") || t.contains("CC")))
    });
    if captions {
        out.push("CC/SDH");
    }
    out
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
    #[serde(rename = "MediaSources", default)]
    pub media_sources: Vec<MediaSource>,
    #[serde(rename = "RemoteTrailers", default)]
    pub remote_trailers: Vec<RemoteTrailer>,
    /// Critic score (0-100, e.g. Rotten Tomatoes), when the server has one.
    #[serde(rename = "CriticRating", default)]
    pub critic_rating: Option<f64>,
    #[serde(rename = "Taglines", default)]
    pub taglines: Vec<String>,
}

/// One entry of an item's `RemoteTrailers` list (a trailer on an outside site).
#[derive(Clone, Debug, Deserialize)]
pub struct RemoteTrailer {
    #[serde(rename = "Url")]
    pub url: String,
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
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

/// One range from GET /MediaSegments/{itemId} (Jellyfin 10.10+). Ticks, not seconds.
#[derive(Clone, Debug, Deserialize)]
pub struct MediaSegment {
    #[serde(rename = "Type", default)]
    pub segment_type: String,
    #[serde(rename = "StartTicks", default)]
    pub start_ticks: i64,
    #[serde(rename = "EndTicks", default)]
    pub end_ticks: i64,
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

/// One entry of GET /Movies/Recommendations: a titled group of suggestions.
#[derive(Clone, Debug, Deserialize)]
pub struct RecommendationDto {
    #[serde(rename = "Items", default)]
    pub items: Vec<ListItem>,
    /// e.g. "SimilarToRecentlyPlayed", "HasActorFromRecentlyPlayed".
    #[serde(rename = "RecommendationType", default)]
    pub recommendation_type: Option<String>,
    /// The item or person the group was derived from.
    #[serde(rename = "BaselineItemName", default)]
    pub baseline_name: Option<String>,
}

#[cfg(test)]
mod wide_art_tests {
    use super::*;

    fn item(tags: serde_json::Value, backdrops: serde_json::Value) -> ListItem {
        serde_json::from_value(serde_json::json!({
            "Id": "x", "Name": "n", "Type": "Episode",
            "ImageTags": tags, "BackdropImageTags": backdrops
        }))
        .unwrap()
    }

    #[test]
    fn wide_art_prefers_thumb_then_backdrop_then_primary() {
        assert_eq!(item(serde_json::json!({"Thumb": "t", "Primary": "p"}), serde_json::json!(["b"])).wide_art_kind(), Some("Thumb"));
        assert_eq!(item(serde_json::json!({"Primary": "p"}), serde_json::json!(["b"])).wide_art_kind(), Some("Backdrop"));
        assert_eq!(item(serde_json::json!({"Primary": "p"}), serde_json::json!([])).wide_art_kind(), Some("Primary"));
        assert_eq!(item(serde_json::json!({}), serde_json::json!([])).wide_art_kind(), None);
    }
}

#[cfg(test)]
mod quality_badge_tests {
    use super::*;

    fn source(v: serde_json::Value) -> MediaSource {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn resolution_and_hdr_badges() {
        let uhd = source(serde_json::json!({"Id": "a", "MediaStreams": [
            {"Type": "Video", "Width": 3840, "VideoRange": "HDR", "VideoRangeType": "HDR10"}]}));
        assert_eq!(quality_badges(Some(&uhd)), vec!["4K", "HDR10"]);
        let dv = source(serde_json::json!({"Id": "b", "MediaStreams": [
            {"Type": "Video", "Width": 1920, "VideoRange": "HDR", "VideoRangeType": "DOVIWithHDR10"}]}));
        assert_eq!(quality_badges(Some(&dv)), vec!["HD", "Dolby Vision"]);
        let sdr = source(serde_json::json!({"Id": "c", "MediaStreams": [
            {"Type": "Video", "Width": 720, "VideoRange": "SDR", "VideoRangeType": "SDR"}]}));
        assert!(quality_badges(Some(&sdr)).is_empty());
    }

    #[test]
    fn audio_and_caption_badges() {
        let atmos = source(serde_json::json!({"Id": "a", "MediaStreams": [
            {"Type": "Audio", "Channels": 8, "DisplayTitle": "English - TRUEHD Atmos 7.1"},
            {"Type": "Subtitle", "IsHearingImpaired": true}]}));
        assert_eq!(quality_badges(Some(&atmos)), vec!["Atmos", "CC/SDH"]);
        let surround = source(serde_json::json!({"Id": "b", "MediaStreams": [
            {"Type": "Audio", "Channels": 8, "DisplayTitle": "English 7.1"},
            {"Type": "Subtitle", "DisplayTitle": "English (SDH)"}]}));
        assert_eq!(quality_badges(Some(&surround)), vec!["7.1", "CC/SDH"]);
        let five = source(serde_json::json!({"Id": "c", "MediaStreams": [
            {"Type": "Audio", "Channels": 6, "Profile": "Dolby Digital"},
            {"Type": "Subtitle", "DisplayTitle": "English"}]}));
        assert_eq!(quality_badges(Some(&five)), vec!["5.1"]);
    }

    #[test]
    fn no_source_means_no_badges() {
        assert!(quality_badges(None).is_empty());
        let bare = source(serde_json::json!({"Id": "d", "MediaStreams": []}));
        assert!(quality_badges(Some(&bare)).is_empty());
    }
}

#[cfg(test)]
mod list_data_tests {
    use super::*;

    #[test]
    fn parses_overview_genres_ratings_and_provider_ids() {
        let page: ItemsPage<ListItem> = serde_json::from_str(
            r#"{"Items":[{"Id":"m1","Name":"Heat","Type":"Movie",
                "Overview":"A cop and a thief.","Genres":["Crime","Thriller"],
                "CommunityRating":8.3,"OfficialRating":"R",
                "ProviderIds":{"Tmdb":"949","Imdb":"tt0113277"},
                "BackdropImageTags":["bd1","bd2"],
                "ImageTags":{"Primary":"p","Logo":"lg"}}]}"#,
        )
        .unwrap();
        let it = &page.items[0];
        assert_eq!(it.overview.as_deref(), Some("A cop and a thief."));
        assert_eq!(it.genres, vec!["Crime".to_string(), "Thriller".to_string()]);
        assert_eq!(it.community_rating, Some(8.3));
        assert_eq!(it.official_rating.as_deref(), Some("R"));
        assert_eq!(it.tmdb_id(), Some("949"));
        assert_eq!(it.provider_ids.get("Imdb").map(String::as_str), Some("tt0113277"));
        assert_eq!(it.backdrop_tag(), Some("bd1"));
        assert_eq!(it.logo_tag(), Some("lg"));
    }

    #[test]
    fn missing_list_data_defaults_cleanly() {
        let it: ListItem = serde_json::from_str(r#"{"Id":"x","Name":"n"}"#).unwrap();
        assert!(it.overview.is_none() && it.genres.is_empty());
        assert!(it.community_rating.is_none() && it.official_rating.is_none());
        assert!(it.provider_ids.is_empty() && it.tmdb_id().is_none());
        assert!(it.backdrop_tag().is_none() && it.logo_tag().is_none());
    }

    #[test]
    fn empty_backdrop_list_has_no_backdrop_tag() {
        let it: ListItem = serde_json::from_str(r#"{"Id":"x","Name":"n","BackdropImageTags":[]}"#).unwrap();
        assert!(it.backdrop_tag().is_none());
    }
}
