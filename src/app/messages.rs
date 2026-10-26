use crate::config::{Config, HomeSection, LayoutMode};
use crate::jellyfin::models::ListItem;
use crate::jellyfin::segments::{NextUp, Segment};
use crate::jellyfin::sessions::{RemoteAction, RemoteSession};
use crate::seerr::models::{DiscoverRow, SearchPage, SeerrRequestItem, SeerrResult, TitleDetail};
use crate::transcode::decision::PlaybackMethod;
use serde::{Deserialize, Serialize};

// ============================================================================
// SECTION: Message types between UI (main thread) and worker (tokio threads)
// ============================================================================
// SPLIT: move into src/app/messages.rs. These enums are the seam between the
// UI process and the async process; both sides own only senders/handles.

#[derive(Debug, Clone)]
pub enum UiCmd {
    Connect {
        url: String,
        username: String,
        password: Option<String>,
        api_key: Option<String>,
        /// "Remember me": keep the session token in the Keychain and sign in on launch.
        remember: bool,
    },
    LoadView {
        view_id: String,
        name: String,
        collection_type: Option<String>,
    },
    /// Filter the grid on screen by `q`. `parent_id` and `types` scope it to that grid
    /// (see `library_query::filter_scope`); the sort comes from the worker's config.
    Search {
        q: String,
        parent_id: Option<String>,
        types: Option<String>,
    },
    /// The header sort popup changed. The UI saved it; this keeps the worker's copy in step.
    SetSort {
        sort_by: String,
    },
    Episodes {
        series_id: String,
        series_name: String,
    },
    Tracks {
        album_id: String,
        album_name: String,
    },
    /// Open the detail page for a movie or series.
    OpenDetail {
        item_id: String,
    },
    /// Load the seasons of a series shown on its detail page, and the one holding Next Up.
    Seasons {
        series_id: String,
    },
    /// Load the episodes of one season for the series detail page.
    SeasonEpisodes {
        series_id: String,
        season_id: String,
    },
    Play {
        item_id: String,
    },
    PlayChannel {
        channel_id: String,
        name: String,
    },
    Progress {
        item_id: String,
        ticks: i64,
        paused: bool,
    },
    Stopped {
        item_id: String,
        ticks: i64,
    },
    MarkPlayed {
        item_id: String,
        played: bool,
    },
    /// Add or remove an item from the user's favourites (no button yet).
    SetFavorite {
        item_id: String,
        favorite: bool,
    },
    /// Unified Browse: every movie and show in one grid (`ItemsLoaded`, title "Browse").
    LoadBrowse,
    /// Layout chosen in onboarding or Settings. The UI already saved the config;
    /// this only keeps the worker's copy in step.
    SetLayoutMode {
        mode: LayoutMode,
    },
    /// Fetch the Home screen rows (continue watching, next up, recently added,
    /// recommendations). Answered with `MainMsg::HomeLoaded`.
    LoadHome,
    /// One Home tab in the one-big-library layout: Movies or TV Shows. Its rows and
    /// poster grid come back as `MainMsg::HomeLoaded` tagged with the same section.
    LoadHomeSection {
        section: HomeSection,
    },
    SaveSettings {
        settings: Settings,
        reconnect: bool,
        /// Raw server address typed in Settings; resolved by the worker before
        /// reconnecting. `None` when the address is unchanged.
        server_url: Option<String>,
    },
    SignOut,
    ProbeServer,
    /// Sidebar library order and hidden ids, written from the Settings sheet.
    SaveLibraries {
        order: Vec<String>,
        hidden: Vec<String>,
    },
    /// Debounced like `Search`. Blank query yields empty `SeerrResults`.
    SeerrSearch {
        q: String,
    },
    /// `media_type` is "movie" or "tv" (tv requests every regular season).
    SeerrRequest {
        media_type: String,
        tmdb_id: u64,
        title: String,
    },
    /// The signed-in Seerr user's recent requests, newest first.
    SeerrRequests,
    /// One page (1-based) of a home-page discover row.
    SeerrDiscover {
        row: DiscoverRow,
        page: u32,
    },
    /// Full detail for one title: cast, trailers, seasons.
    SeerrTitle {
        media_type: String,
        tmdb_id: u64,
    },
    /// First page of recommendations for one title.
    SeerrRecommendations {
        media_type: String,
        tmdb_id: u64,
    },
    /// Validate the connection, then persist. A non-blank `username` selects
    /// Jellyfin-account sign-in (`password: None`/blank reuses the Keychain
    /// password for this URL); a blank one selects API-key mode (`api_key:
    /// None`/blank reuses the stored key).
    SaveSeerr {
        enabled: bool,
        url: String,
        username: String,
        password: Option<String>,
        api_key: Option<String>,
    },
    /// Play On: fetch the other devices this user can control. Answered with
    /// `MainMsg::SessionsLoaded`. `silent` (the remote bar's poll) reports no errors.
    LoadSessions {
        silent: bool,
    },
    /// Play On: start `item_id` on another session, from `start_secs` (`PlayNow`).
    CastPlay {
        session_id: String,
        item_id: String,
        start_secs: f64,
    },
    /// Play On a series: the server picks its Next Up episode (else the first one) and starts it.
    CastSeries {
        session_id: String,
        series_id: String,
    },
    /// Remote control (pause, unpause, stop, seek) for another session.
    CastControl {
        session_id: String,
        action: RemoteAction,
    },
    QuitSaving,
}

#[derive(Debug)]
pub enum MainMsg {
    Connected {
        user_name: String,
        views: Vec<(String, String, Option<String>)>, // id, name, collection_type
        has_live_tv: bool,
    },
    ConnectError(String),
    /// The worker is resuming a remembered session (Remember me + Keychain token)
    /// at launch. Only then does the login card show "Connecting…"; with no saved
    /// session nothing is sent and the card waits for the user.
    Resuming { url: String },
    /// The resolved (scheme-complete) server base URL, sent after a connect or
    /// Settings URL change so the UI uses the same address the worker stored.
    ServerUrl(String),
    /// Home screen rows, in display order. Empty rows are never included.
    HomeLoaded(HomeRows),
    ItemsLoaded {
        title: String,
        items: Vec<ListItem>,
        can_go_back: bool,
    },
    PosterReady {
        key: u64,
        bytes: Vec<u8>,
    },
    PlaybackReady(PlaybackStart),
    /// Facts for the detail page; its artwork follows as `PosterReady` keys.
    DetailLoaded(Box<DetailView>),
    /// The detail page could not be loaded (shown as a toast).
    DetailError(String),
    /// A favourite was added or removed on the server.
    FavoriteChanged {
        item_id: String,
        favorite: bool,
    },
    /// A favourite could not be changed (shown as a toast).
    FavoriteError(String),
    /// The series' seasons, and which one to show first (its Next Up season, else the first).
    SeasonsLoaded {
        series_id: String,
        seasons: Vec<SeasonEntry>,
        selected: usize,
    },
    /// Episodes of one season; their thumbnails follow as `PosterReady` keys.
    SeasonEpisodesLoaded {
        series_id: String,
        season_id: String,
        episodes: Vec<EpisodeRow>,
    },
    /// Seasons or episodes of a series page could not be loaded.
    SeasonsError {
        series_id: String,
        message: String,
    },
    PlaybackError(String),
    ServerUp,
    ServerDown,
    SignedOut,
    SettingsSnapshot(Box<Config>),
    SeerrResults(Vec<SeerrResult>),
    SeerrRequested {
        title: String,
    },
    SeerrRequestList(Vec<SeerrRequestItem>),
    /// Poster bytes for a TMDB-relative `poster_path`, fetched through the Seerr server.
    SeerrPoster {
        path: String,
        bytes: Vec<u8>,
    },
    /// Search/request/list failure, "already requested", or Seerr disabled/unconfigured.
    SeerrError(String),
    SeerrDiscover {
        row: DiscoverRow,
        page: SearchPage,
    },
    SeerrTitle(Box<TitleDetail>),
    SeerrRecommendations {
        media_type: String,
        tmdb_id: u64,
        results: Vec<SeerrResult>,
    },
    /// Discover, title or recommendations failure (kept apart from `SeerrError`, which the search UI shows).
    SeerrBrowseError(String),
    /// Controllable sessions on the server, without this app's own.
    SessionsLoaded(Vec<RemoteSession>),
    /// A Play On request or remote command failed (shown as a toast).
    CastError(String),
    /// Outcome of `SaveSeerr` (ok=false: nothing was persisted).
    SeerrStatus {
        ok: bool,
        message: String,
    },
}

/// What a Home row shows; the UI picks its layout and "see all" target from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeRowKind {
    ContinueWatching,
    NextUp,
    /// One library's newest items; the title names the library.
    RecentlyAdded,
    Recommended,
}

impl HomeRowKind {
    /// Wide (16:9) cards with a progress bar; every other row uses posters.
    pub fn is_wide(self) -> bool {
        matches!(self, HomeRowKind::ContinueWatching | HomeRowKind::NextUp)
    }
}

#[derive(Debug, Clone)]
pub struct HomeRow {
    pub title: String,
    pub kind: HomeRowKind,
    pub items: Vec<ListItem>,
}

/// Home content for one tab. `grid` is the full poster grid under the rows (Movies
/// and TV Shows tabs only; empty on Home). The UI drops a reply whose `section` is
/// no longer the one on screen.
#[derive(Debug, Clone, Default)]
pub struct HomeRows {
    pub section: HomeSection,
    pub rows: Vec<HomeRow>,
    pub grid: Vec<ListItem>,
}

#[derive(Debug, Clone)]
pub struct PlaybackStart {
    pub url: String,
    pub item_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub resume_secs: f64,
    pub method: PlaybackMethod,
    pub run_time_secs: f64,
    /// `poster_key` of the item's artwork (for the buffering backdrop), when it has some.
    pub image_key: Option<u64>,
    pub media_kind: MediaKind,
    /// Episodes: the show's name. `None` for movies, audio and live TV.
    pub series_name: Option<String>,
    /// Episodes: season number (`ParentIndexNumber`).
    pub season_number: Option<u32>,
    /// Episodes: episode number (`IndexNumber`).
    pub episode_number: Option<u32>,
    /// Movies: production year (`ProductionYear`).
    pub production_year: Option<u32>,
    /// Skip-button ranges (intro, recap, preview, outro, commercial). Empty when the
    /// server has none or the lookup failed.
    pub segments: Vec<Segment>,
    /// Episodes: the show's id, which keys the per-show "skip intros" setting.
    pub series_id: Option<String>,
    /// Episodes: the next episode in series order, when there is one.
    pub next_up: Option<NextUp>,
}

impl PlaybackStart {
    /// The line under the title in the player's upper-left corner.
    /// Episodes: "Series Name · S1 E2". Movies: the year. Anything else keeps `subtitle`.
    pub fn header_subtitle(&self) -> Option<String> {
        if let Some(series) = self.series_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(episode_line(series, self.season_number, self.episode_number));
        }
        if self.media_kind == MediaKind::Video {
            if let Some(year) = self.production_year {
                return Some(year.to_string());
            }
        }
        self.subtitle.clone()
    }
}

/// "Series Name · S1 E2"; when a season or episode number is missing, only the known parts appear.
pub fn episode_line(series: &str, season: Option<u32>, episode: Option<u32>) -> String {
    let code = match (season, episode) {
        (Some(s), Some(e)) => format!("S{} E{}", s, e),
        (Some(s), None) => format!("S{}", s),
        (None, Some(e)) => format!("E{}", e),
        (None, None) => String::new(),
    };
    if code.is_empty() { series.to_string() } else { format!("{} · {}", series, code) }
}

/// Whether playback shows the full-window video view or the now-playing bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

/// Everything the detail page shows for one item. Artwork is keyed by
/// `poster_key`-style hashes and arrives separately as `PosterReady`.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailView {
    pub id: String,
    pub name: String,
    pub item_type: String,
    pub year: Option<u32>,
    pub official_rating: Option<String>,
    pub community_rating: Option<f64>,
    pub run_time_ticks: Option<i64>,
    pub genres: Vec<String>,
    pub overview: Option<String>,
    pub played: bool,
    /// Has a saved position and is not finished: the button says "Resume".
    pub resume: bool,
    /// Saved position (ticks), where Play On starts the item.
    pub resume_ticks: i64,
    pub poster_key: Option<u64>,
    pub backdrop_key: Option<u64>,
    pub people: Vec<DetailPerson>,
    /// Remote trailer links (YouTube and the like), in the server's order.
    pub trailer_urls: Vec<String>,
    pub tagline: Option<String>,
    /// Critic score, 0 to 100.
    pub critic_rating: Option<f64>,
    /// Quality badges such as "4K", "Dolby Vision", "Atmos", "CC/SDH".
    pub badges: Vec<&'static str>,
    pub is_favorite: bool,
    /// "More like this": the server's similar items, best match first.
    pub similar: Vec<SimilarItem>,
    /// Series only: the episode the user should watch next.
    pub next_up: Option<NextUpEpisode>,
}

/// One title in a detail page's similar list.
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarItem {
    pub id: String,
    pub name: String,
    pub year: Option<u32>,
}

/// The Next Up episode of a series, with the saved position in it.
#[derive(Debug, Clone, PartialEq)]
pub struct NextUpEpisode {
    pub id: String,
    pub name: String,
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub position_ticks: i64,
    pub run_time_ticks: Option<i64>,
}

/// One season in the series page's season picker.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonEntry {
    pub id: String,
    pub name: String,
}

/// One episode row on the series page.
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeRow {
    pub id: String,
    /// Episode number within its season.
    pub number: Option<u32>,
    pub name: String,
    pub overview: Option<String>,
    pub run_time_ticks: Option<i64>,
    pub played: bool,
    pub position_ticks: i64,
    /// Thumbnail artwork key, when the episode has a still.
    pub image_key: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailPerson {
    pub name: String,
    /// Character name, or the crew job ("Director") when there is no role.
    pub role: Option<String>,
    pub image_key: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: String, // "system" (Follow system) | "light" | "dark" | a palette id
    /// Light-mode theme used under "Follow system": "light" (Jellyfast) or a palette id.
    #[serde(default)]
    pub system_light: String,
    /// Dark-mode theme used under "Follow system": "dark" (Jellyfast) or a palette id.
    #[serde(default)]
    pub system_dark: String,
    #[serde(default)]
    pub max_bitrate_kbps: u64, // 0 = original
    #[serde(default = "default_sub_lang")]
    pub subtitle_lang: String, // ISO 639-2/1, "" = off
    #[serde(default)]
    pub audio_lang: String,
    #[serde(default)]
    pub seerr_enabled: bool,
    /// Base URL of the Seerr instance. The API key lives in the Keychain
    /// (account "seerr@<url>"), never here.
    #[serde(default)]
    pub seerr_url: String,
    /// Non-empty = sign in with a Jellyfin account (password in the Keychain,
    /// account `seerr-login@<url>`); empty = API-key mode.
    #[serde(default)]
    pub seerr_username: String,
    /// Show the playback-info panel (time, resolution, bitrate, method) over the video.
    #[serde(default)]
    pub show_stats: bool,
    /// Play the next episode of a show when this one ends (Up Next countdown).
    #[serde(default = "default_true")]
    pub autoplay_next: bool,
    /// Skip intros automatically for shows with no setting of their own.
    #[serde(default)]
    pub skip_intros_default: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: String::new(),
            system_light: String::new(),
            system_dark: String::new(),
            max_bitrate_kbps: 0,
            subtitle_lang: default_sub_lang(),
            audio_lang: String::new(),
            seerr_enabled: false,
            seerr_url: String::new(),
            seerr_username: String::new(),
            show_stats: false,
            autoplay_next: true,
            skip_intros_default: false,
        }
    }
}
fn default_true() -> bool {
    true
}
fn default_sub_lang() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_deserialize_with_defaults_and_ignore_unknown() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!((s.theme.as_str(), s.max_bitrate_kbps, s.subtitle_lang.as_str()), ("", 0, ""));
        let s: Settings = serde_json::from_str(r#"{"theme":"dark","max_bitrate_kbps":5000,"extra":1}"#).unwrap();
        assert_eq!((s.theme.as_str(), s.max_bitrate_kbps), ("dark", 5000));
    }

    #[test]
    fn autoplay_defaults_on_and_skip_default_off() {
        assert!(Settings::default().autoplay_next);
        assert!(!Settings::default().skip_intros_default);
        // Old config files have neither key: autoplay stays on, skip stays off.
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(s.autoplay_next && !s.skip_intros_default);
        let s: Settings = serde_json::from_str(r#"{"autoplay_next":false,"skip_intros_default":true}"#).unwrap();
        assert!(!s.autoplay_next && s.skip_intros_default);
    }

    #[test]
    fn seerr_settings_default_and_unknown_keys_ignored() {
        let s: Settings = serde_json::from_str(r#"{"legacy_flag":true,"legacy_url":"https://s.x"}"#).unwrap();
        assert!(!s.seerr_enabled && s.seerr_url.is_empty());
        let s: Settings = serde_json::from_str(r#"{"seerr_enabled":true,"seerr_url":"http://h:5055"}"#).unwrap();
        assert!(s.seerr_enabled);
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("seerr_url") );
        assert!(!j.contains("api_key"));
    }

    fn start(series: Option<&str>, season: Option<u32>, episode: Option<u32>, year: Option<u32>, kind: MediaKind) -> PlaybackStart {
        PlaybackStart {
            url: String::new(),
            item_id: "x".into(),
            title: "Pilot".into(),
            subtitle: Some("fallback".into()),
            resume_secs: 0.0,
            method: crate::transcode::decision::PlaybackMethod::DirectPlay,
            run_time_secs: 0.0,
            image_key: None,
            media_kind: kind,
            series_name: series.map(str::to_string),
            season_number: season,
            episode_number: episode,
            production_year: year,
            segments: Vec::new(),
            series_id: None,
            next_up: None,
        }
    }

    #[test]
    fn episode_line_formats_season_and_episode() {
        assert_eq!(episode_line("Show", Some(1), Some(2)), "Show · S1 E2");
        assert_eq!(episode_line("Show", Some(12), Some(105)), "Show · S12 E105");
        assert_eq!(episode_line("Show", Some(3), None), "Show · S3");
        assert_eq!(episode_line("Show", None, Some(4)), "Show · E4");
        assert_eq!(episode_line("Show", None, None), "Show");
    }

    #[test]
    fn header_subtitle_for_episode_movie_and_other() {
        let ep = start(Some("Show"), Some(1), Some(2), Some(2019), MediaKind::Video);
        assert_eq!(ep.header_subtitle().as_deref(), Some("Show · S1 E2"));
        let blank = start(Some("  "), Some(1), Some(2), None, MediaKind::Video);
        assert_eq!(blank.header_subtitle().as_deref(), Some("fallback"));
        let movie = start(None, None, None, Some(1999), MediaKind::Video);
        assert_eq!(movie.header_subtitle().as_deref(), Some("1999"));
        let undated = start(None, None, None, None, MediaKind::Video);
        assert_eq!(undated.header_subtitle().as_deref(), Some("fallback"));
        let audio = start(None, None, None, Some(2001), MediaKind::Audio);
        assert_eq!(audio.header_subtitle().as_deref(), Some("fallback"));
    }
}
