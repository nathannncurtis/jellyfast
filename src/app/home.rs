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
            format!("Because You Watched {}", b)
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

/// Movie and show libraries the user has not hidden, as view ids in sidebar order.
#[derive(Default)]
struct VideoLibs {
    movies: Vec<String>,
    shows: Vec<String>,
    /// The library list could not be loaded: Home falls back to server-wide rows.
    failed: bool,
}

/// Resolve the visible movie and show libraries. A failed views request leaves
/// both lists empty, so the one-big-library tabs show their empty state.
async fn video_libs(c: &JellyfinClient, order: &[String], hidden: &[String]) -> VideoLibs {
    let views = match c.views().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("home: views failed: {}", e);
            return VideoLibs { failed: true, ..Default::default() };
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

/// Drop repeats of an item (one title can come back from more than one library).
fn dedup_by_id(items: &mut Vec<ListItem>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|it| seen.insert(it.id.clone()));
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
    dedup_by_id(&mut items);
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
    dedup_by_id(&mut items);
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
    dedup_by_id(&mut items);
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
    merge_recommendations(groups)
}

/// Server-wide fallbacks for when the library list failed: the same rows without a
/// library filter.
async fn server_wide_resume(c: JellyfinClient) -> Vec<ListItem> {
    settle(tokio::spawn(async move { c.resume_items(ROW_LIMIT).await }), "resume").await.unwrap_or_default()
}

async fn server_wide_next_up(c: JellyfinClient) -> Vec<ListItem> {
    settle(tokio::spawn(async move { c.next_up(ROW_LIMIT).await }), "next up").await.unwrap_or_default()
}

async fn server_wide_recommendations(c: JellyfinClient) -> Vec<HomeRow> {
    let groups = settle(tokio::spawn(async move { c.recommendations(ROW_LIMIT).await }), "recommendations").await;
    merge_recommendations(groups.unwrap_or_default())
}

/// Recommendation groups as rows, merged by title (see `recommendations_across`).
fn merge_recommendations(groups: Vec<RecommendationDto>) -> Vec<HomeRow> {
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

/// Home rows in display order: Continue Watching, Next Up, Recently Added Movies,
/// Recently Added TV, Because You Watched, Top Rated, two genre rows, then any
/// further recommendations. Only visible libraries count.
pub(crate) async fn load_unified_home(c: &JellyfinClient, order: &[String], hidden: &[String]) -> HomeRows {
    let libs = video_libs(c, order, hidden).await;
    let all: Vec<String> = libs.movies.iter().chain(libs.shows.iter()).cloned().collect();
    // Each loader runs as its own task, so all of them are in flight at once.
    let resume = if libs.failed {
        tokio::spawn(server_wide_resume(c.clone()))
    } else {
        tokio::spawn(resume_across(c.clone(), all.clone()))
    };
    let next = if libs.failed {
        tokio::spawn(server_wide_next_up(c.clone()))
    } else {
        tokio::spawn(next_up_across(c.clone(), libs.shows.clone()))
    };
    let added_movies = tokio::spawn(latest_across(c.clone(), libs.movies.clone()));
    let added_tv = tokio::spawn(latest_across(c.clone(), libs.shows.clone()));
    let recs = if libs.failed {
        tokio::spawn(server_wide_recommendations(c.clone()))
    } else {
        tokio::spawn(recommendations_across(c.clone(), libs.movies.clone()))
    };
    let top = tokio::spawn(rated_across(c.clone(), all.clone(), None));
    let resume = joined(resume, "resume").await;
    let next = joined(next, "next up").await;
    let added_movies = joined(added_movies, "latest movies").await;
    let added_tv = joined(added_tv, "latest tv").await;
    let recs = joined(recs, "recommendations").await;
    let top = joined(top, "top rated").await;

    // Two genre rows, picked from what the other rows already show.
    let picks = top_genres(
        resume.iter().chain(&next).chain(&added_movies).chain(&added_tv).chain(&top),
        GENRE_ROWS,
    );
    let genre_tasks: Vec<_> = picks
        .iter()
        .map(|g| tokio::spawn(rated_across(c.clone(), all.clone(), Some(g.clone()))))
        .collect();
    let mut genre_rows = Vec::new();
    for (g, t) in picks.into_iter().zip(genre_tasks) {
        let items = joined(t, "genre").await;
        genre_rows.extend(row(g, HomeRowKind::Genre, Some(items)));
    }

    let mut rows = Vec::new();
    rows.extend(row("Continue Watching".into(), HomeRowKind::ContinueWatching, Some(resume)));
    rows.extend(row("Next Up".into(), HomeRowKind::NextUp, Some(next)));
    rows.extend(row("Recently Added Movies".into(), HomeRowKind::RecentlyAdded, Some(added_movies)));
    rows.extend(row("Recently Added TV".into(), HomeRowKind::RecentlyAdded, Some(added_tv)));
    let mut recs = recs.into_iter();
    if let Some(first) = recs.next() {
        rows.push(first);
    }
    rows.extend(row("Top Rated".into(), HomeRowKind::TopRated, Some(top)));
    rows.extend(genre_rows);
    rows.extend(recs);
    HomeRows { section: HomeSection::Home, rows, grid: Vec::new() }
}

/// Highest rated movies and shows across the given libraries, optionally in one genre.
async fn rated_across(c: JellyfinClient, ids: Vec<String>, genre: Option<String>) -> Vec<ListItem> {
    let mut items = gather(
        fan_out(&c, &ids, move |c, id| {
            let genre = genre.clone();
            async move {
                let q = ItemQuery {
                    parent_id: Some(id),
                    types: Some("Movie,Series".into()),
                    sort_by: "CommunityRating".into(),
                    sort_order: "Descending".into(),
                    genres: genre,
                    limit: ROW_LIMIT,
                    ..Default::default()
                };
                c.items(&q).await
            }
        }),
        "top rated",
    )
    .await;
    dedup_by_id(&mut items);
    items.sort_by(|a, b| {
        b.community_rating.unwrap_or(0.).partial_cmp(&a.community_rating.unwrap_or(0.)).unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(ROW_LIMIT as usize);
    items
}

/// Genre rows on Home.
const GENRE_ROWS: usize = 2;

/// The `n` genres that appear most often across `items`; ties sort by name.
pub(crate) fn top_genres<'a>(items: impl Iterator<Item = &'a ListItem>, n: usize) -> Vec<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for it in items {
        for g in it.genres.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
            *counts.entry(g).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.into_iter().take(n).map(|(g, _)| g.to_string()).collect()
}

/// Slides on the billboard, at most.
pub const BILLBOARD_MAX: usize = 6;

/// The billboard's slides, in order: resume and next up first, then recently added,
/// then recommended and top rated. Each title appears once and has artwork.
pub fn billboard_items(rows: &[HomeRow]) -> Vec<ListItem> {
    let kind_rows = |kind: HomeRowKind| rows.iter().filter(move |r| r.kind == kind);
    let mut picks: Vec<&ListItem> = Vec::new();
    picks.extend(kind_rows(HomeRowKind::ContinueWatching).find_map(|r| r.items.first()));
    picks.extend(kind_rows(HomeRowKind::NextUp).find_map(|r| r.items.first()));
    for r in kind_rows(HomeRowKind::RecentlyAdded) {
        picks.extend(r.items.iter().take(2));
    }
    picks.extend(kind_rows(HomeRowKind::Recommended).find_map(|r| r.items.first()));
    picks.extend(kind_rows(HomeRowKind::TopRated).find_map(|r| r.items.first()));
    let mut seen = std::collections::HashSet::new();
    picks
        .into_iter()
        .filter(|it| it.wide_art_kind().is_some() && seen.insert(it.id.clone()))
        .take(BILLBOARD_MAX)
        .cloned()
        .collect()
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
        assert_eq!(recommendation_title(&rec(Some("SimilarToRecentlyPlayed"), Some("Dune"))), "Because You Watched Dune");
        assert_eq!(recommendation_title(&rec(Some("SimilarToLikedItem"), Some("Heat"))), "Because You Watched Heat");
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

    fn art_item(ty: &str, id: &str) -> ListItem {
        serde_json::from_value(serde_json::json!({
            "Id": id, "Name": id, "Type": ty, "BackdropImageTags": ["b"]
        }))
        .unwrap()
    }

    fn row_of(kind: HomeRowKind, ids: &[&str]) -> HomeRow {
        HomeRow { title: "t".into(), kind, items: ids.iter().map(|id| art_item("Movie", id)).collect() }
    }

    #[test]
    fn billboard_leads_with_resume_then_recent_then_recommended() {
        let rows = vec![
            row_of(HomeRowKind::ContinueWatching, &["cw1", "cw2"]),
            row_of(HomeRowKind::NextUp, &["nx1"]),
            row_of(HomeRowKind::RecentlyAdded, &["ra1", "ra2", "ra3"]),
            row_of(HomeRowKind::Recommended, &["rec1", "rec2"]),
            row_of(HomeRowKind::TopRated, &["top1"]),
        ];
        let ids: Vec<String> = billboard_items(&rows).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["cw1", "nx1", "ra1", "ra2", "rec1", "top1"]);
    }

    #[test]
    fn billboard_skips_artless_and_repeated_titles() {
        let bare: ListItem = serde_json::from_value(serde_json::json!({"Id": "x", "Name": "x", "Type": "Movie"})).unwrap();
        let mut rows = vec![row_of(HomeRowKind::ContinueWatching, &["same"])];
        rows.push(HomeRow { title: "t".into(), kind: HomeRowKind::RecentlyAdded, items: vec![bare, art_item("Movie", "same")] });
        let ids: Vec<String> = billboard_items(&rows).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["same"]);
    }

    #[test]
    fn billboard_is_capped() {
        let rows: Vec<HomeRow> = (0..5)
            .map(|k| row_of(HomeRowKind::RecentlyAdded, &[&format!("a{k}"), &format!("b{k}")]))
            .collect();
        assert_eq!(billboard_items(&rows).len(), BILLBOARD_MAX);
    }

    #[test]
    fn genres_rank_by_count_then_name() {
        let g = |genres: &[&str]| -> ListItem {
            serde_json::from_value(serde_json::json!({"Id": "x", "Name": "x", "Type": "Movie", "Genres": genres})).unwrap()
        };
        let items = vec![g(&["Drama", "Comedy"]), g(&["Drama"]), g(&["Action", "Comedy"]), g(&["Horror"])];
        assert_eq!(top_genres(items.iter(), 2), ["Comedy", "Drama"]);
        assert_eq!(top_genres(items.iter(), 9).len(), 4);
    }
}
