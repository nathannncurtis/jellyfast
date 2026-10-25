//! Data for the Home screen: fetches every row concurrently and tolerates
//! partial failures (a failed endpoint just drops its rows).

use crate::app::messages::{HomeRow, HomeRowKind, HomeRows};
use crate::jellyfin::client::{JFError, JellyfinClient};
use crate::jellyfin::models::{ListItem, RecommendationDto, View};

/// Items requested per row.
const ROW_LIMIT: u32 = 16;

/// Only movie and show libraries get a "Recently Added" row.
fn is_video_library(v: &View) -> bool {
    matches!(v.collection_type.as_deref(), Some("movies") | Some("tvshows"))
}

/// Log a failed request and keep going with nothing for that row.
async fn settle<T>(h: tokio::task::JoinHandle<Result<T, JFError>>, what: &str) -> Option<T> {
    match h.await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            tracing::warn!("home: {} failed: {}", what, e);
            None
        }
        Err(e) => {
            tracing::warn!("home: {} task failed: {}", what, e);
            None
        }
    }
}

/// A row only exists when it has something to show.
fn row(title: String, kind: HomeRowKind, items: Option<Vec<ListItem>>) -> Option<HomeRow> {
    items
        .filter(|v| !v.is_empty())
        .map(|items| HomeRow { title, kind, items })
}

/// Title for one recommendation group. Only the first two relations have a
/// baseline name the user can recognise, so the rest fall back to a generic title.
pub(crate) fn recommendation_title(r: &RecommendationDto) -> String {
    let baseline = r.baseline_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match (r.recommendation_type.as_deref(), baseline) {
        (Some("SimilarToRecentlyPlayed") | Some("SimilarToLikedItem"), Some(b)) => {
            format!("Because you watched {}", b)
        }
        (Some("HasDirectorFromRecentlyPlayed") | Some("HasLikedDirector"), Some(b)) => {
            format!("More by {}", b)
        }
        (Some("HasActorFromRecentlyPlayed") | Some("HasLikedActor"), Some(b)) => {
            format!("More with {}", b)
        }
        _ => "Recommended for you".to_string(),
    }
}

/// Fetch all Home rows in display order: Continue Watching, Next Up, one
/// Recently Added row per movie/show library, then recommendations.
pub(crate) async fn load_home(c: &JellyfinClient) -> HomeRows {
    let (c1, c2, c3, c4) = (c.clone(), c.clone(), c.clone(), c.clone());
    let resume = tokio::spawn(async move { c1.resume_items(ROW_LIMIT).await });
    let next = tokio::spawn(async move { c2.next_up(ROW_LIMIT).await });
    let recs = tokio::spawn(async move { c3.recommendations(ROW_LIMIT).await });
    let views = tokio::spawn(async move { c4.views().await });

    let mut rows = Vec::new();
    let resume = settle(resume, "resume").await;
    rows.extend(row("Continue Watching".into(), HomeRowKind::ContinueWatching, resume));
    let next = settle(next, "next up").await;
    rows.extend(row("Next Up".into(), HomeRowKind::NextUp, next));

    // Latest per library: start every request before awaiting any of them.
    let libs: Vec<View> = settle(views, "views")
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(is_video_library)
        .collect();
    let latest: Vec<(View, tokio::task::JoinHandle<Result<Vec<ListItem>, JFError>>)> = libs
        .into_iter()
        .map(|lib| {
            let c = c.clone();
            let id = lib.id.clone();
            (lib, tokio::spawn(async move { c.latest_items(&id, ROW_LIMIT).await }))
        })
        .collect();
    for (lib, h) in latest {
        let items = settle(h, "latest").await;
        rows.extend(row(format!("Recently Added in {}", lib.name), HomeRowKind::RecentlyAdded, items));
    }

    if let Some(groups) = settle(recs, "recommendations").await {
        for g in groups {
            let title = recommendation_title(&g);
            rows.extend(row(title, HomeRowKind::Recommended, Some(g.items)));
        }
    }
    HomeRows { rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: Option<&str>, baseline: Option<&str>) -> RecommendationDto {
        RecommendationDto {
            items: vec![],
            recommendation_type: kind.map(String::from),
            baseline_name: baseline.map(String::from),
        }
    }

    #[test]
    fn recommendation_titles_per_relation() {
        assert_eq!(recommendation_title(&rec(Some("SimilarToRecentlyPlayed"), Some("Dune"))), "Because you watched Dune");
        assert_eq!(recommendation_title(&rec(Some("SimilarToLikedItem"), Some("Heat"))), "Because you watched Heat");
        assert_eq!(recommendation_title(&rec(Some("HasLikedDirector"), Some("Denis Villeneuve"))), "More by Denis Villeneuve");
        assert_eq!(recommendation_title(&rec(Some("HasActorFromRecentlyPlayed"), Some("Zendaya"))), "More with Zendaya");
    }

    #[test]
    fn recommendation_title_falls_back_without_baseline() {
        assert_eq!(recommendation_title(&rec(Some("SimilarToRecentlyPlayed"), None)), "Recommended for you");
        assert_eq!(recommendation_title(&rec(Some("SimilarToRecentlyPlayed"), Some("   "))), "Recommended for you");
        assert_eq!(recommendation_title(&rec(Some("SomethingNew"), Some("X"))), "Recommended for you");
        assert_eq!(recommendation_title(&rec(None, None)), "Recommended for you");
    }

    #[test]
    fn empty_rows_are_dropped() {
        assert!(row("T".into(), HomeRowKind::NextUp, None).is_none());
        assert!(row("T".into(), HomeRowKind::NextUp, Some(vec![])).is_none());
    }

    #[test]
    fn only_movie_and_show_libraries_get_latest_rows() {
        let v = |ct: Option<&str>| View { id: "x".into(), name: "x".into(), collection_type: ct.map(String::from) };
        assert!(is_video_library(&v(Some("movies"))));
        assert!(is_video_library(&v(Some("tvshows"))));
        assert!(!is_video_library(&v(Some("music"))));
        assert!(!is_video_library(&v(Some("livetv"))));
        assert!(!is_video_library(&v(None)));
    }
}
