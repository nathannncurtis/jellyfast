//! Query building for the library grid, shared by the header sort popup and the
//! filter field. Pure: no UI, no I/O, so the choices are unit-tested here.

use crate::jellyfin::models::ItemQuery;

/// Item types the unified Browse grid shows.
pub const BROWSE_TYPES: &str = "Movie,Series";

/// Sort choices in popup order: (menu title, Jellyfin `SortBy`, Jellyfin `SortOrder`).
pub const SORT_CHOICES: [(&str, &str, &str); 4] = [
    ("Name", "SortName", "Ascending"),
    ("Date Added", "DateCreated", "Descending"),
    ("Rating", "CommunityRating", "Descending"),
    ("Last Played", "DatePlayed", "Descending"),
];

/// Popup index for a saved `SortBy` key. Unset or unknown keys mean Name.
pub fn sort_index(sort_by: Option<&str>) -> usize {
    sort_by.and_then(|key| SORT_CHOICES.iter().position(|c| c.1 == key)).unwrap_or(0)
}

/// `(SortBy, SortOrder)` for a popup index. An out-of-range index means Name.
pub fn sort_for_index(index: usize) -> (&'static str, &'static str) {
    let (_, by, order) = *SORT_CHOICES.get(index).unwrap_or(&SORT_CHOICES[0]);
    (by, order)
}

/// Jellyfin `IncludeItemTypes` for a library's collection type, when the grid filters by type.
pub fn library_types(collection_type: Option<&str>) -> Option<&'static str> {
    match collection_type {
        Some("movies") => Some("Movie"),
        Some("tvshows") => Some("Series"),
        Some("music") => Some("MusicAlbum"),
        _ => None,
    }
}

fn base(sort_by: Option<&str>) -> ItemQuery {
    let (by, order) = sort_for_index(sort_index(sort_by));
    ItemQuery { sort_by: by.into(), sort_order: order.into(), ..Default::default() }
}

/// One library's grid, in the chosen order.
pub fn library_query(view_id: &str, collection_type: Option<&str>, sort_by: Option<&str>) -> ItemQuery {
    ItemQuery {
        parent_id: Some(view_id.into()),
        types: library_types(collection_type).map(Into::into),
        ..base(sort_by)
    }
}

/// The unified Browse grid: every movie and show, in the chosen order.
pub fn browse_query(sort_by: Option<&str>) -> ItemQuery {
    ItemQuery { types: Some(BROWSE_TYPES.into()), ..base(sort_by) }
}

/// Where the filter searches for the grid on screen: `(parent_id, IncludeItemTypes)`.
/// Browse searches its movies and shows; a library searches inside itself; Live TV
/// searches its channels.
pub fn filter_scope(browse: bool, view: Option<(&str, Option<&str>)>) -> (Option<String>, Option<&'static str>) {
    if browse {
        return (None, Some(BROWSE_TYPES));
    }
    match view {
        Some((id, ct)) if ct == Some("livetv") || id == "livetv" => (None, Some("LiveTvChannel")),
        Some((id, ct)) => (Some(id.to_string()), library_types(ct)),
        None => (None, None),
    }
}

/// The filter's query in the chosen order, or None when the text is blank.
pub fn filter_query(text: &str, parent_id: Option<&str>, types: Option<&str>, sort_by: Option<&str>) -> Option<ItemQuery> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(ItemQuery {
        search: Some(text.into()),
        parent_id: parent_id.map(Into::into),
        types: types.map(Into::into),
        limit: 100,
        ..base(sort_by)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_index_round_trips_every_choice() {
        for (i, (_, by, _)) in SORT_CHOICES.iter().enumerate() {
            assert_eq!(sort_index(Some(by)), i);
            assert_eq!(sort_for_index(i).0, *by);
        }
    }

    #[test]
    fn unset_or_unknown_sort_is_name_ascending() {
        assert_eq!(sort_index(None), 0);
        assert_eq!(sort_index(Some("NoSuchKey")), 0);
        assert_eq!(sort_for_index(0), ("SortName", "Ascending"));
        assert_eq!(sort_for_index(99), ("SortName", "Ascending"));
    }

    #[test]
    fn date_added_sorts_newest_first() {
        assert_eq!(sort_for_index(1), ("DateCreated", "Descending"));
    }

    #[test]
    fn library_query_carries_parent_types_and_sort() {
        let q = library_query("lib-1", Some("movies"), Some("DatePlayed"));
        assert_eq!(q.parent_id.as_deref(), Some("lib-1"));
        assert_eq!(q.types.as_deref(), Some("Movie"));
        assert_eq!((q.sort_by.as_str(), q.sort_order.as_str()), ("DatePlayed", "Descending"));
        assert!(q.search.is_none());
        let plain = library_query("lib-2", None, None);
        assert_eq!(plain.types, None);
        assert_eq!((plain.sort_by.as_str(), plain.sort_order.as_str()), ("SortName", "Ascending"));
    }

    #[test]
    fn browse_query_has_movies_and_shows_and_no_parent() {
        let q = browse_query(Some("CommunityRating"));
        assert_eq!(q.parent_id, None);
        assert_eq!(q.types.as_deref(), Some("Movie,Series"));
        assert_eq!(q.sort_by, "CommunityRating");
    }

    #[test]
    fn filter_scope_follows_the_grid_on_screen() {
        assert_eq!(filter_scope(true, Some(("x", Some("movies")))), (None, Some("Movie,Series")));
        assert_eq!(
            filter_scope(false, Some(("lib-1", Some("tvshows")))),
            (Some("lib-1".to_string()), Some("Series"))
        );
        assert_eq!(filter_scope(false, Some(("livetv", Some("livetv")))), (None, Some("LiveTvChannel")));
        assert_eq!(filter_scope(false, None), (None, None));
    }

    #[test]
    fn blank_filter_text_sends_no_query() {
        assert!(filter_query("", None, None, None).is_none());
        assert!(filter_query("   \t", Some("lib"), Some("Movie"), None).is_none());
    }

    #[test]
    fn filter_query_trims_text_and_keeps_scope_and_sort() {
        let q = filter_query("  dune ", Some("lib-1"), Some("Movie"), Some("DateCreated")).unwrap();
        assert_eq!(q.search.as_deref(), Some("dune"));
        assert_eq!(q.parent_id.as_deref(), Some("lib-1"));
        assert_eq!(q.types.as_deref(), Some("Movie"));
        assert_eq!((q.sort_by.as_str(), q.sort_order.as_str()), ("DateCreated", "Descending"));
        assert_eq!(q.limit, 100);
    }
}
