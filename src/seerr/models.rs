use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public, UI-agnostic plain data (these travel inside UiCmd/MainMsg).
// ---------------------------------------------------------------------------

/// Library availability of a title, from Seerr `mediaInfo.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Not requested / not in the library (no mediaInfo, or status 1 UNKNOWN).
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

/// One movie/tv search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SeerrResult {
    /// "movie" or "tv"
    pub media_type: String,
    pub tmdb_id: u64,
    pub title: String,
    pub year: Option<u16>,
    /// TMDB-relative path like "/abc.jpg"; see [`poster_url`].
    pub poster_path: Option<String>,
    pub overview: String,
    pub availability: Availability,
    /// Status of the first existing request on this title, if any.
    pub request_status: Option<RequestStatus>,
}

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

/// TMDB image URL for a poster path (w342 is plenty for a grid cell).
pub fn poster_url(path: &str) -> String {
    format!("https://image.tmdb.org/t/p/w342{}", path)
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
pub(crate) struct RawStatus {
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct RawMessage {
    pub message: Option<String>,
}
