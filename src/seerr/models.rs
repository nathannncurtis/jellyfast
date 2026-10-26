use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public, UI-agnostic plain data (these travel inside UiCmd/MainMsg).
// ---------------------------------------------------------------------------

/// Library availability of a title, from Seerr `mediaInfo.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Availability {
    /// Not requested / not in the library (no mediaInfo, or status 1 UNKNOWN).
    #[default]
    NotRequested,
    Pending,
    Processing,
    PartiallyAvailable,
    Available,
    Blocklisted,
}

impl Availability {
    pub fn from_code(code: Option<u8>) -> Self {
        match code {
            Some(2) => Availability::Pending,
            Some(3) => Availability::Processing,
            Some(4) => Availability::PartiallyAvailable,
            Some(5) => Availability::Available,
            Some(6) => Availability::Blocklisted,
            _ => Availability::NotRequested,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Availability::NotRequested => "Not requested",
            Availability::Pending => "Pending",
            Availability::Processing => "Processing",
            Availability::PartiallyAvailable => "Partially available",
            Availability::Available => "Available",
            Availability::Blocklisted => "Blocklisted",
        }
    }
}

/// Status of a request, from Seerr `MediaRequest.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Approved,
    Declined,
    Failed,
    Completed,
    Unknown,
}

impl RequestStatus {
    pub fn from_code(code: Option<u8>) -> Self {
        match code {
            Some(1) => RequestStatus::Pending,
            Some(2) => RequestStatus::Approved,
            Some(3) => RequestStatus::Declined,
            Some(4) => RequestStatus::Failed,
            Some(5) => RequestStatus::Completed,
            _ => RequestStatus::Unknown,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            RequestStatus::Pending => "Pending approval",
            RequestStatus::Approved => "Approved",
            RequestStatus::Declined => "Declined",
            RequestStatus::Failed => "Failed",
            RequestStatus::Completed => "Completed",
            RequestStatus::Unknown => "Unknown",
        }
    }
}

/// One movie/tv hit: a search result, a discover row item or a recommendation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeerrResult {
    /// "movie" or "tv"
    pub media_type: String,
    pub tmdb_id: u64,
    pub title: String,
    pub year: Option<u16>,
    /// Full release (movie) or first-air (tv) date, "YYYY-MM-DD", when Seerr sent one.
    pub release_date: Option<String>,
    /// TMDB-relative path like "/abc.jpg"; see [`image_url`].
    pub poster_path: Option<String>,
    /// TMDB-relative path of the wide image; see [`image_url`].
    pub backdrop_path: Option<String>,
    pub overview: String,
    /// TMDB average vote, 0-10. None when unrated.
    pub vote_average: Option<f32>,
    /// TMDB genre ids; names come from [`Genre`] lists.
    pub genre_ids: Vec<u32>,
    pub availability: Availability,
    /// Status of the first existing request on this title, if any.
    pub request_status: Option<RequestStatus>,
}

/// One page of results: search, discover rows, genre lists, recommendations.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    pub page: u32,
    pub total_pages: u32,
    pub total_results: u32,
    pub results: Vec<SeerrResult>,
}

/// One entry in the user's request list.
#[derive(Debug, Clone, PartialEq)]
pub struct SeerrRequestItem {
    pub request_id: u64,
    pub media_type: String,
    pub tmdb_id: u64,
    /// Looked up from the movie/tv detail endpoint; "TMDB #<id>" if that failed.
    pub title: String,
    pub year: Option<u16>,
    pub poster_path: Option<String>,
    pub status: RequestStatus,
    pub availability: Availability,
    pub requested_by: String,
    /// ISO-8601 timestamp as sent by Seerr (may be empty).
    pub created_at: String,
}

/// Result of a successful POST /request.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedRequest {
    pub request_id: u64,
    pub status: RequestStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeerrStatusInfo {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeerrUser {
    pub id: u64,
    pub display_name: String,
}

/// A TMDB genre, as listed by /genres/movie or /genres/tv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genre {
    pub id: u32,
    pub name: String,
}

/// The home-page discover rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverRow {
    Trending,
    Movies,
    Tv,
    UpcomingMovies,
    UpcomingTv,
}

/// One cast member from the detail page's credits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CastMember {
    pub name: String,
    pub character: String,
    /// TMDB-relative path; see [`image_url`].
    pub profile_path: Option<String>,
}

/// A trailer from the detail page's relatedVideos (type "Trailer").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trailer {
    pub name: String,
    /// Site key, e.g. the YouTube video id.
    pub key: String,
    /// e.g. "YouTube".
    pub site: String,
}

/// One TV season, with its own library availability (season 0 is specials).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonInfo {
    pub number: u32,
    pub name: String,
    pub episode_count: u32,
    /// NotRequested when Seerr has no per-season status for it.
    pub availability: Availability,
}

/// Full detail for one movie or series, from GET /api/v1/{movie|tv}/{id}.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TitleDetail {
    /// "movie" or "tv"
    pub media_type: String,
    pub tmdb_id: u64,
    pub title: String,
    pub year: Option<u16>,
    pub release_date: Option<String>,
    pub overview: String,
    pub tagline: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f32>,
    /// Minutes: the movie runtime, or the first episode runtime for a series.
    pub runtime: Option<u32>,
    pub genres: Vec<String>,
    /// Top-billed cast, in Seerr's order.
    pub cast: Vec<CastMember>,
    pub trailers: Vec<Trailer>,
    /// Empty for movies.
    pub seasons: Vec<SeasonInfo>,
    pub availability: Availability,
    pub request_status: Option<RequestStatus>,
}

/// TMDB image widths. Seerr's proxy serves exactly these (plus others we do not use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSize {
    /// Search-list thumbnail (40x60 at 2x).
    Thumb,
    PosterSmall,
    PosterMedium,
    PosterLarge,
    BackdropMedium,
    BackdropLarge,
}

impl ImageSize {
    /// The TMDB size segment, e.g. "w342".
    pub fn tmdb_name(self) -> &'static str {
        match self {
            ImageSize::Thumb => "w92",
            ImageSize::PosterSmall => "w185",
            ImageSize::PosterMedium => "w342",
            ImageSize::PosterLarge => "w500",
            ImageSize::BackdropMedium => "w780",
            ImageSize::BackdropLarge => "w1280",
        }
    }
}

/// Image URL on the user's own Seerr server (its TMDB image proxy), never TMDB directly.
pub fn image_url(base: &str, size: ImageSize, path: &str) -> String {
    let slash = if path.starts_with('/') { "" } else { "/" };
    format!("{}/imageproxy/tmdb/t/p/{}{}{}", base.trim_end_matches('/'), size.tmdb_name(), slash, path)
}

pub(crate) fn year_of(date: Option<&str>) -> Option<u16> {
    let d = date?.trim();
    if d.len() < 4 {
        return None;
    }
    d[..4].parse().ok().filter(|y| *y > 0)
}

// ---------------------------------------------------------------------------
// Raw wire models (camelCase, everything optional: Seerr omits freely).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawSearch {
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub total_pages: u32,
    #[serde(default)]
    pub total_results: u32,
    #[serde(default)]
    pub results: Vec<RawResult>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawResult {
    #[serde(default)]
    pub id: u64,
    pub media_type: Option<String>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub overview: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f32>,
    #[serde(default)]
    pub genre_ids: Vec<u32>,
    pub media_info: Option<RawMediaInfo>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawMediaInfo {
    pub status: Option<u8>,
    pub tmdb_id: Option<u64>,
    pub media_type: Option<String>,
    #[serde(default)]
    pub requests: Vec<RawRequestStub>,
    #[serde(default)]
    pub seasons: Vec<RawMediaSeason>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawMediaSeason {
    pub season_number: Option<u32>,
    pub status: Option<u8>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawRequestStub {
    pub status: Option<u8>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawRequestList {
    #[serde(default)]
    pub results: Vec<RawRequest>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawRequest {
    #[serde(default)]
    pub id: u64,
    pub status: Option<u8>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub media: Option<RawMediaInfo>,
    pub requested_by: Option<RawUser>,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawUser {
    #[serde(default)]
    pub id: u64,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawDetail {
    #[serde(default)]
    pub id: u64,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub vote_average: Option<f32>,
    pub runtime: Option<u32>,
    #[serde(default)]
    pub episode_run_time: Vec<u32>,
    #[serde(default)]
    pub genres: Vec<RawGenre>,
    pub credits: Option<RawCredits>,
    #[serde(default)]
    pub related_videos: Vec<RawVideo>,
    #[serde(default)]
    pub seasons: Vec<RawSeason>,
    pub media_info: Option<RawMediaInfo>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawGenre {
    #[serde(default)]
    pub id: u32,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawCredits {
    #[serde(default)]
    pub cast: Vec<RawCast>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawCast {
    pub name: Option<String>,
    pub character: Option<String>,
    pub profile_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawVideo {
    pub key: Option<String>,
    pub name: Option<String>,
    pub site: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawSeason {
    #[serde(default)]
    pub season_number: u32,
    pub name: Option<String>,
    #[serde(default)]
    pub episode_count: u32,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawStatus {
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawMessage {
    pub message: Option<String>,
}

/// Maps one Seerr media item to a [`SeerrResult`]. `kind` is the endpoint's media type,
/// used when the item omits `mediaType` (the movie and tv discover lists). People and
/// anything else return None.
pub(crate) fn result_of(r: RawResult, kind: Option<&str>) -> Option<SeerrResult> {
    let mt = r.media_type.clone().or_else(|| kind.map(str::to_string))?;
    let (title, date) = match mt.as_str() {
        "movie" => (r.title.clone().or(r.name.clone()), r.release_date.clone()),
        "tv" => (r.name.clone().or(r.title.clone()), r.first_air_date.clone()),
        _ => return None,
    };
    let (availability, request_status) = match &r.media_info {
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
        release_date: date.filter(|d| !d.is_empty()),
        poster_path: r.poster_path.filter(|p| !p.is_empty()),
        backdrop_path: r.backdrop_path.filter(|p| !p.is_empty()),
        overview: r.overview.unwrap_or_default(),
        vote_average: r.vote_average.filter(|v| *v > 0.0),
        genre_ids: r.genre_ids,
        availability,
        request_status,
    })
}

/// A paged list of media items; see [`result_of`] for `kind`.
pub(crate) fn page_of(raw: RawSearch, kind: Option<&str>) -> SearchPage {
    SearchPage {
        page: raw.page,
        total_pages: raw.total_pages,
        total_results: raw.total_results,
        results: raw.results.into_iter().filter_map(|r| result_of(r, kind)).collect(),
    }
}

/// Maps GET /api/v1/{movie|tv}/{id} to a [`TitleDetail`].
pub(crate) fn detail_of(r: RawDetail, media_type: &str) -> TitleDetail {
    let is_tv = media_type == "tv";
    let (availability, request_status, season_status) = match &r.media_info {
        Some(mi) => (
            Availability::from_code(mi.status),
            mi.requests.first().map(|q| RequestStatus::from_code(q.status)),
            mi.seasons.iter().map(|s| (s.season_number, Availability::from_code(s.status))).collect::<Vec<_>>(),
        ),
        None => (Availability::NotRequested, None, Vec::new()),
    };
    let runtime = if is_tv {
        r.episode_run_time.iter().copied().find(|m| *m > 0)
    } else {
        r.runtime.filter(|m| *m > 0)
    };
    let (title, date) = if is_tv {
        (r.name.clone().or(r.title.clone()), r.first_air_date.clone())
    } else {
        (r.title.clone().or(r.name.clone()), r.release_date.clone())
    };
    let cast = r
        .credits
        .map(|c| c.cast)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            Some(CastMember {
                name: c.name.filter(|n| !n.is_empty())?,
                character: c.character.unwrap_or_default(),
                profile_path: c.profile_path.filter(|p| !p.is_empty()),
            })
        })
        .collect();
    let trailers = r
        .related_videos
        .into_iter()
        .filter(|v| v.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("trailer")))
        .filter_map(|v| {
            Some(Trailer { name: v.name.unwrap_or_default(), key: v.key.filter(|k| !k.is_empty())?, site: v.site.unwrap_or_default() })
        })
        .collect();
    let seasons = r
        .seasons
        .into_iter()
        .map(|s| SeasonInfo {
            number: s.season_number,
            availability: season_status
                .iter()
                .find(|(n, _)| *n == Some(s.season_number))
                .map(|(_, a)| *a)
                .unwrap_or_default(),
            name: s.name.unwrap_or_default(),
            episode_count: s.episode_count,
        })
        .collect();
    TitleDetail {
        media_type: media_type.to_string(),
        tmdb_id: r.id,
        title: title.unwrap_or_default(),
        year: year_of(date.as_deref()),
        release_date: date.filter(|d| !d.is_empty()),
        overview: r.overview.unwrap_or_default(),
        tagline: r.tagline.unwrap_or_default(),
        poster_path: r.poster_path.filter(|p| !p.is_empty()),
        backdrop_path: r.backdrop_path.filter(|p| !p.is_empty()),
        vote_average: r.vote_average.filter(|v| *v > 0.0),
        runtime,
        genres: r.genres.into_iter().filter_map(|g| g.name.filter(|n| !n.is_empty())).collect(),
        cast,
        trailers,
        seasons,
        availability,
        request_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_result(v: serde_json::Value) -> RawResult {
        serde_json::from_value(v).unwrap()
    }

    fn raw_detail(v: serde_json::Value) -> RawDetail {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn image_urls_always_go_through_the_seerr_proxy() {
        let sizes = [
            (ImageSize::Thumb, "w92"),
            (ImageSize::PosterSmall, "w185"),
            (ImageSize::PosterMedium, "w342"),
            (ImageSize::PosterLarge, "w500"),
            (ImageSize::BackdropMedium, "w780"),
            (ImageSize::BackdropLarge, "w1280"),
        ];
        for (size, name) in sizes {
            assert_eq!(image_url("http://s:5055", size, "/a.jpg"), format!("http://s:5055/imageproxy/tmdb/t/p/{}/a.jpg", name));
            assert_eq!(image_url("http://s:5055/", size, "a.jpg"), format!("http://s:5055/imageproxy/tmdb/t/p/{}/a.jpg", name));
            let url = image_url("https://seerr.example", size, "/x.jpg");
            assert!(url.starts_with("https://seerr.example/imageproxy/tmdb/t/p/"));
            assert!(!url.contains("tmdb.org"), "{}", url);
        }
    }

    #[test]
    fn result_parses_discover_fields() {
        let r = result_of(
            raw_result(serde_json::json!({
                "id": 27205, "mediaType": "movie", "title": "Inception", "releaseDate": "2010-07-16",
                "posterPath": "/p.jpg", "backdropPath": "/b.jpg", "voteAverage": 8.4, "genreIds": [28, 878],
                "overview": "Dreams.", "mediaInfo": {"status": 5, "requests": [{"status": 2}]}
            })),
            None,
        )
        .unwrap();
        assert_eq!((r.media_type.as_str(), r.tmdb_id, r.year), ("movie", 27205, Some(2010)));
        assert_eq!(r.release_date.as_deref(), Some("2010-07-16"));
        assert_eq!((r.poster_path.as_deref(), r.backdrop_path.as_deref()), (Some("/p.jpg"), Some("/b.jpg")));
        assert_eq!(r.vote_average, Some(8.4));
        assert_eq!(r.genre_ids, vec![28, 878]);
        assert_eq!((r.availability, r.request_status), (Availability::Available, Some(RequestStatus::Approved)));
    }

    #[test]
    fn result_uses_endpoint_kind_when_media_type_is_missing_and_drops_people() {
        let tv = result_of(raw_result(serde_json::json!({"id": 1, "name": "Show", "firstAirDate": "2020-01-01", "voteAverage": 0})), Some("tv")).unwrap();
        assert_eq!((tv.media_type.as_str(), tv.title.as_str(), tv.year), ("tv", "Show", Some(2020)));
        assert_eq!(tv.vote_average, None, "an unrated title has no average");
        assert!(result_of(raw_result(serde_json::json!({"id": 31, "mediaType": "person", "name": "X"})), None).is_none());
        assert!(result_of(raw_result(serde_json::json!({"id": 31, "name": "X"})), None).is_none());
    }

    #[test]
    fn movie_detail_parses_runtime_genres_cast_and_trailers() {
        let d = detail_of(
            raw_detail(serde_json::json!({
                "id": 27205, "title": "Inception", "releaseDate": "2010-07-16", "runtime": 148, "tagline": "Your mind is the scene of the crime.",
                "voteAverage": 8.4, "genres": [{"id": 28, "name": "Action"}, {"id": 878, "name": "Science Fiction"}],
                "credits": {"cast": [
                    {"name": "Leonardo DiCaprio", "character": "Cobb", "profilePath": "/l.jpg"},
                    {"name": "Ken Watanabe", "character": "Saito", "profilePath": null},
                    {"character": "nameless"}
                ]},
                "relatedVideos": [
                    {"key": "abc", "name": "Official Trailer", "site": "YouTube", "type": "Trailer"},
                    {"key": "def", "name": "Teaser", "site": "YouTube", "type": "Teaser"}
                ],
                "mediaInfo": {"status": 5, "requests": []}
            })),
            "movie",
        );
        assert_eq!((d.title.as_str(), d.year, d.runtime), ("Inception", Some(2010), Some(148)));
        assert_eq!(d.tagline, "Your mind is the scene of the crime.");
        assert_eq!(d.genres, vec!["Action", "Science Fiction"]);
        assert_eq!(d.vote_average, Some(8.4));
        assert_eq!(d.cast.len(), 2, "nameless cast entries are dropped");
        assert_eq!(d.cast[0], CastMember { name: "Leonardo DiCaprio".into(), character: "Cobb".into(), profile_path: Some("/l.jpg".into()) });
        assert_eq!(d.cast[1].profile_path, None);
        assert_eq!(d.trailers, vec![Trailer { name: "Official Trailer".into(), key: "abc".into(), site: "YouTube".into() }]);
        assert!(d.seasons.is_empty());
        assert_eq!(d.availability, Availability::Available);
    }

    #[test]
    fn tv_detail_parses_seasons_with_per_season_availability() {
        let d = detail_of(
            raw_detail(serde_json::json!({
                "id": 1399, "name": "Game of Thrones", "firstAirDate": "2011-04-17", "episodeRunTime": [0, 57],
                "seasons": [
                    {"seasonNumber": 0, "name": "Specials", "episodeCount": 2},
                    {"seasonNumber": 1, "name": "Season 1", "episodeCount": 10},
                    {"seasonNumber": 2, "name": "Season 2", "episodeCount": 10}
                ],
                "mediaInfo": {"status": 4, "requests": [{"status": 2}], "seasons": [
                    {"seasonNumber": 1, "status": 5}, {"seasonNumber": 2, "status": 2}
                ]}
            })),
            "tv",
        );
        assert_eq!((d.title.as_str(), d.runtime), ("Game of Thrones", Some(57)));
        assert_eq!(d.release_date.as_deref(), Some("2011-04-17"));
        let got: Vec<(u32, &str, u32, Availability)> =
            d.seasons.iter().map(|s| (s.number, s.name.as_str(), s.episode_count, s.availability)).collect();
        assert_eq!(
            got,
            vec![
                (0, "Specials", 2, Availability::NotRequested),
                (1, "Season 1", 10, Availability::Available),
                (2, "Season 2", 10, Availability::Pending),
            ]
        );
        assert_eq!((d.availability, d.request_status), (Availability::PartiallyAvailable, Some(RequestStatus::Approved)));
    }
}
