//! Data for the Home screen: fetches every row concurrently and tolerates
//! partial failures (a failed endpoint just drops its rows).

use crate::app::libraries::visible_views;
use crate::app::messages::{HomeRow, HomeRowKind, HomeRows};
use crate::config::HomeSection;
use crate::jellyfin::client::{JFError, JellyfinClient};
use crate::jellyfin::models::{ItemQuery, ListItem, RecommendationDto, View};
use std::future::Future;
use tokio::task::JoinHandle;

/// Items requested per row.
const ROW_LIMIT: u32 = 16;
/// Upper bound for a full library grid (Movies / TV Shows tabs).
const GRID_LIMIT: u32 = 10_000;

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
///
/// Every endpoint is in flight at once. The per-library Latest requests need
/// the library list from `views`, so they are spawned as soon as `views`
/// resolves, from their own task, instead of after the other rows are awaited.
pub(crate) async fn load_home(c: &JellyfinClient) -> HomeRows {
    let (c1, c2, c3, c4, c5) = (c.clone(), c.clone(), c.clone(), c.clone(), c.clone());
    let resume = tokio::spawn(async move { c1.resume_items(ROW_LIMIT).await });
    let next = tokio::spawn(async move { c2.next_up(ROW_LIMIT).await });
    let recs = tokio::spawn(async move { c3.recommendations(ROW_LIMIT).await });
    let views = tokio::spawn(async move { c4.views().await });
    let latest = tokio::spawn(async move {
        let libs: Vec<View> = settle(views, "views")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(is_video_library)
            .collect();
        libs.into_iter()
            .map(|lib| {
                let c = c5.clone();
                let id = lib.id.clone();
                (lib, tokio::spawn(async move { c.latest_items(&id, ROW_LIMIT).await }))
            })
            .collect::<Vec<_>>()
    });

    let resume = settle(resume, "resume").await;
    let next = settle(next, "next up").await;
    let recs = settle(recs, "recommendations").await;
    let latest = match latest.await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("home: latest task failed: {}", e);
            Vec::new()
        }
    };

    let mut rows = Vec::new();
    rows.extend(row("Continue Watching".into(), HomeRowKind::ContinueWatching, resume));
    rows.extend(row("Next Up".into(), HomeRowKind::NextUp, next));
    for (lib, h) in latest {
        let items = settle(h, "latest").await;
        rows.extend(row(format!("Recently Added in {}", lib.name), HomeRowKind::RecentlyAdded, items));
    }
    if let Some(groups) = recs {
        for g in groups {
            let title = recommendation_title(&g);
            rows.extend(row(title, HomeRowKind::Recommended, Some(g.items)));
        }
    }
    HomeRows { section: HomeSection::Home, rows, grid: Vec::new() }
}

/// Movie and show libraries the user has not hidden, as view ids in sidebar order.
#[derive(Default)]
struct VideoLibs {
    movies: Vec<String>,
    shows: Vec<String>,
}

/// Resolve the visible movie and show libraries. A failed views request leaves
/// both lists empty, so the one-big-library tabs show their empty state.
async fn video_libs(c: &JellyfinClient, order: &[String], hidden: &[String]) -> VideoLibs {
    let views = match c.views().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("home: views failed: {}", e);
            Vec::new()
        }
    };
    let all: Vec<(String, String, Option<String>)> =
        views.into_iter().map(|v| (v.id, v.name, v.collection_type)).collect();
    let mut libs = VideoLibs::default();
    for (id, _, kind) in visible_views(&all, order, hidden) {
        match kind.as_deref() {
            Some("movies") => libs.movies.push(id),
            Some("tvshows") => libs.shows.push(id),
            _ => {}
        }
    }
    libs
}

/// Start one request per library id; the answers are gathered by `gather`.
fn fan_out<T, F, Fut>(c: &JellyfinClient, ids: &[String], f: F) -> Vec<JoinHandle<Result<Vec<T>, JFError>>>
where
    T: Send + 'static,
    F: Fn(JellyfinClient, String) -> Fut,
    Fut: Future<Output = Result<Vec<T>, JFError>> + Send + 'static,
{
    ids.iter().map(|id| tokio::spawn(f(c.clone(), id.clone()))).collect()
}

/// Join per-library answers into one list; a failed library contributes nothing.
async fn gather<T>(handles: Vec<JoinHandle<Result<Vec<T>, JFError>>>, what: &str) -> Vec<T> {
    let mut out = Vec::new();
    for h in handles {
        if let Some(v) = settle(h, what).await {
            out.extend(v);
        }
    }
    out
}

/// Await a spawned loader. A panicked task counts as an empty answer.
async fn joined<T: Default>(h: JoinHandle<T>, what: &str) -> T {
    match h.await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("home: {} task failed: {}", what, e);
            T::default()
        }
    }
}

/// Most recently played first. Items never played sort last.
fn played_at(it: &ListItem) -> &str {
    it.user_data.as_ref().and_then(|u| u.last_played_date.as_deref()).unwrap_or("")
}

/// Continue Watching over the given libraries, newest play first.
async fn resume_across(c: JellyfinClient, ids: Vec<String>) -> Vec<ListItem> {
    let mut items = gather(
        fan_out(&c, &ids, |c, id| async move { c.resume_items_in(Some(&id), ROW_LIMIT).await }),
        "resume",
    )
    .await;
    items.sort_by(|a, b| played_at(b).cmp(played_at(a)));
    items.truncate(ROW_LIMIT as usize);
    items
}

/// Next Up over the given show libraries.
async fn next_up_across(c: JellyfinClient, ids: Vec<String>) -> Vec<ListItem> {
    let mut items = gather(
        fan_out(&c, &ids, |c, id| async move { c.next_up_in(Some(&id), ROW_LIMIT).await }),
        "next up",
    )
    .await;
    items.truncate(ROW_LIMIT as usize);
    items
}

/// Recently added over the given libraries, newest first.
async fn latest_across(c: JellyfinClient, ids: Vec<String>) -> Vec<ListItem> {
    let mut items = gather(
        fan_out(&c, &ids, |c, id| async move { c.latest_items(&id, ROW_LIMIT).await }),
        "latest",
    )
    .await;
    items.sort_by(|a, b| b.date_created.cmp(&a.date_created));
    items.truncate(ROW_LIMIT as usize);
    items
}

/// Recommendations over the given movie libraries. The same group (say, "Because
/// you watched X") can come back from several libraries, so groups merge by title.
async fn recommendations_across(c: JellyfinClient, ids: Vec<String>) -> Vec<HomeRow> {
    let groups: Vec<RecommendationDto> = gather(
        fan_out(&c, &ids, |c, id| async move { c.recommendations_in(Some(&id), ROW_LIMIT).await }),
        "recommendations",
    )
    .await;
    let mut merged: Vec<(String, Vec<ListItem>)> = Vec::new();
    for g in groups {
        let title = recommendation_title(&g);
        match merged.iter_mut().find(|(t, _)| *t == title) {
            Some((_, items)) => {
                for it in g.items {
                    if !items.iter().any(|x| x.id == it.id) {
                        items.push(it);
                    }
                }
            }
            None => merged.push((title, g.items)),
        }
    }
    merged
        .into_iter()
        .filter_map(|(title, mut items)| {
            items.truncate(ROW_LIMIT as usize);
            row(title, HomeRowKind::Recommended, Some(items))
        })
        .collect()
}

/// Every item of one type (`Movie` or `Series`) across the given libraries, by name.
async fn grid_across(c: JellyfinClient, ids: Vec<String>, item_type: &'static str) -> Vec<ListItem> {
    let mut items = gather(
        fan_out(&c, &ids, move |c, id| async move {
            let q = ItemQuery {
                parent_id: Some(id),
                types: Some(item_type.to_string()),
                sort_by: "SortName".into(),
                limit: GRID_LIMIT,
                ..Default::default()
            };
            c.items(&q).await
        }),
        "grid",
    )
    .await;
    items.sort_by_key(|it| it.name.to_lowercase());
    items
}

/// Home in the one-big-library layout: Continue Watching, Next Up, Recently Added
/// Movies, Recently Added TV, then recommendations. Only visible libraries count.
pub(crate) async fn load_unified_home(c: &JellyfinClient, order: &[String], hidden: &[String]) -> HomeRows {
    let libs = video_libs(c, order, hidden).await;
    let all: Vec<String> = libs.movies.iter().chain(libs.shows.iter()).cloned().collect();
    // Each loader runs as its own task, so all of them are in flight at once.
    let resume = tokio::spawn(resume_across(c.clone(), all));
    let next = tokio::spawn(next_up_across(c.clone(), libs.shows.clone()));
    let added_movies = tokio::spawn(latest_across(c.clone(), libs.movies.clone()));
    let added_tv = tokio::spawn(latest_across(c.clone(), libs.shows.clone()));
    let recs = tokio::spawn(recommendations_across(c.clone(), libs.movies.clone()));
    let resume = joined(resume, "resume").await;
    let next = joined(next, "next up").await;
    let added_movies = joined(added_movies, "latest movies").await;
    let added_tv = joined(added_tv, "latest tv").await;
    let recs = joined(recs, "recommendations").await;
    let mut rows = Vec::new();
    rows.extend(row("Continue Watching".into(), HomeRowKind::ContinueWatching, Some(resume)));
    rows.extend(row("Next Up".into(), HomeRowKind::NextUp, Some(next)));
    rows.extend(row("Recently Added Movies".into(), HomeRowKind::RecentlyAdded, Some(added_movies)));
    rows.extend(row("Recently Added TV".into(), HomeRowKind::RecentlyAdded, Some(added_tv)));
    rows.extend(recs);
    HomeRows { section: HomeSection::Home, rows, grid: Vec::new() }
}

/// One tab of the one-big-library Home. Movies: Continue Watching, Recently Added,
/// recommendations, then every movie. TV Shows: Next Up, Recently Added, every series.
pub(crate) async fn load_section(
    c: &JellyfinClient,
    section: HomeSection,
    order: &[String],
    hidden: &[String],
) -> HomeRows {
    if section == HomeSection::Home {
        return load_unified_home(c, order, hidden).await;
    }
    let libs = video_libs(c, order, hidden).await;
    match section {
        HomeSection::Home => unreachable!("handled above"),
        HomeSection::Movies => {
            let resume = tokio::spawn(resume_across(c.clone(), libs.movies.clone()));
            let added = tokio::spawn(latest_across(c.clone(), libs.movies.clone()));
            let recs = tokio::spawn(recommendations_across(c.clone(), libs.movies.clone()));
            let grid = tokio::spawn(grid_across(c.clone(), libs.movies.clone(), "Movie"));
            let resume = joined(resume, "resume").await;
            let added = joined(added, "latest movies").await;
            let recs = joined(recs, "recommendations").await;
            let grid = joined(grid, "movie grid").await;
            let mut rows = Vec::new();
            rows.extend(row("Continue Watching".into(), HomeRowKind::ContinueWatching, Some(resume)));
            rows.extend(row("Recently Added".into(), HomeRowKind::RecentlyAdded, Some(added)));
            rows.extend(recs);
            HomeRows { section, rows, grid }
        }
        HomeSection::TvShows => {
            let next = tokio::spawn(next_up_across(c.clone(), libs.shows.clone()));
            let added = tokio::spawn(latest_across(c.clone(), libs.shows.clone()));
            let grid = tokio::spawn(grid_across(c.clone(), libs.shows.clone(), "Series"));
            let next = joined(next, "next up").await;
            let added = joined(added, "latest tv").await;
            let grid = joined(grid, "series grid").await;
            let mut rows = Vec::new();
            rows.extend(row("Next Up".into(), HomeRowKind::NextUp, Some(next)));
            rows.extend(row("Recently Added".into(), HomeRowKind::RecentlyAdded, Some(added)));
            HomeRows { section, rows, grid }
        }
    }
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
