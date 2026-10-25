//! Library ordering and sidebar visibility rules, shared by the sidebar and Settings.
//! Pure functions only, so the rules are unit-tested without AppKit.

/// A library as the sidebar sees it: (view id, name, collection type).
pub type View = (String, String, Option<String>);

/// `views` sorted by the saved `order`. Ids in `order` come first, in that sequence;
/// ids no longer on the server are skipped. Views not mentioned in `order` follow in
/// server order, so a library added later appears at the end.
pub fn ordered_views(views: &[View], order: &[String]) -> Vec<View> {
    let mut out: Vec<View> = Vec::with_capacity(views.len());
    for id in order {
        if let Some(v) = views.iter().find(|v| &v.0 == id) {
            if !out.iter().any(|o| o.0 == v.0) {
                out.push(v.clone());
            }
        }
    }
    for v in views {
        if !out.iter().any(|o| o.0 == v.0) {
            out.push(v.clone());
        }
    }
    out
}

/// What the sidebar shows: `ordered_views` minus the hidden ids.
pub fn visible_views(views: &[View], order: &[String], hidden: &[String]) -> Vec<View> {
    ordered_views(views, order).into_iter().filter(|v| !hidden.contains(&v.0)).collect()
}

/// Move the entry at `idx` one step up (`up`) or down. Returns its new index; at either
/// end, or for an out-of-range index, nothing moves and `idx` is returned unchanged.
pub fn move_by(views: &mut [View], idx: usize, up: bool) -> usize {
    if idx >= views.len() {
        return idx;
    }
    if up {
        if idx == 0 {
            return 0;
        }
        views.swap(idx, idx - 1);
        idx - 1
    } else {
        if idx + 1 >= views.len() {
            return idx;
        }
        views.swap(idx, idx + 1);
        idx + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str) -> View {
        (id.into(), format!("Lib {id}"), None)
    }
    fn ids(vs: &[View]) -> Vec<&str> {
        vs.iter().map(|x| x.0.as_str()).collect()
    }
    fn s(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn empty_order_keeps_server_order() {
        let views = vec![v("a"), v("b"), v("c")];
        assert_eq!(ids(&ordered_views(&views, &[])), ["a", "b", "c"]);
    }

    #[test]
    fn saved_order_wins_and_unknown_ids_are_skipped() {
        let views = vec![v("a"), v("b"), v("c")];
        let out = ordered_views(&views, &s(&["c", "gone", "a", "b"]));
        assert_eq!(ids(&out), ["c", "a", "b"]);
    }

    #[test]
    fn new_libraries_follow_saved_ones_in_server_order() {
        let views = vec![v("a"), v("new1"), v("b"), v("new2")];
        let out = ordered_views(&views, &s(&["b", "a"]));
        assert_eq!(ids(&out), ["b", "a", "new1", "new2"]);
    }

    #[test]
    fn duplicate_ids_in_order_do_not_duplicate_views() {
        let views = vec![v("a"), v("b")];
        let out = ordered_views(&views, &s(&["b", "b", "a", "a"]));
        assert_eq!(ids(&out), ["b", "a"]);
    }

    #[test]
    fn hidden_views_are_removed_from_visible_list() {
        let views = vec![v("a"), v("b"), v("c")];
        let out = visible_views(&views, &s(&["c", "b", "a"]), &s(&["b", "unknown"]));
        assert_eq!(ids(&out), ["c", "a"]);
    }

    #[test]
    fn hiding_everything_yields_empty_list() {
        let views = vec![v("a")];
        assert!(visible_views(&views, &[], &s(&["a"])).is_empty());
    }

    #[test]
    fn move_up_and_down_swap_neighbours() {
        let mut views = vec![v("a"), v("b"), v("c")];
        assert_eq!(move_by(&mut views, 2, true), 1);
        assert_eq!(ids(&views), ["a", "c", "b"]);
        assert_eq!(move_by(&mut views, 0, false), 1);
        assert_eq!(ids(&views), ["c", "a", "b"]);
    }

    #[test]
    fn move_at_the_ends_is_a_no_op() {
        let mut views = vec![v("a"), v("b")];
        assert_eq!(move_by(&mut views, 0, true), 0);
        assert_eq!(move_by(&mut views, 1, false), 1);
        assert_eq!(ids(&views), ["a", "b"]);
        assert_eq!(move_by(&mut views, 9, true), 9);
    }
}
