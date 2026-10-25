//! Media segments (intro, recap, preview, outro, commercial) and the episode that
//! follows the current one. Pure logic, no Objective-C, so it is unit tested directly.
use crate::jellyfin::models::{Episode, MediaSegment};

/// Jellyfin ticks: 10,000,000 per second (100 ns each).
pub const TICKS_PER_SEC: f64 = 10_000_000.0;

/// Up Next starts this long before the end of a long episode, at most (a quarter of a short one).
const UP_NEXT_LEAD_MAX: f64 = 30.0;
/// Countdown after an Outro segment starts, before the next episode plays.
const OUTRO_COUNTDOWN: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Intro,
    Recap,
    Preview,
    Outro,
    Commercial,
}

impl SegmentKind {
    /// Jellyfin's `Type` string. Anything else (including "Unknown") is not a segment we show.
    pub fn parse(s: &str) -> Option<SegmentKind> {
        match s.trim() {
            "Intro" => Some(SegmentKind::Intro),
            "Recap" => Some(SegmentKind::Recap),
            "Preview" => Some(SegmentKind::Preview),
            "Outro" => Some(SegmentKind::Outro),
            "Commercial" => Some(SegmentKind::Commercial),
            _ => None,
        }
    }

    /// Text of the skip button, as Jellyfin Web words it. Outro has none: it drives Up Next.
    pub fn skip_label(self) -> Option<&'static str> {
        match self {
            SegmentKind::Intro => Some("Skip Intro"),
            SegmentKind::Recap => Some("Skip Recap"),
            SegmentKind::Preview => Some("Skip Preview"),
            SegmentKind::Commercial => Some("Skip Commercial"),
            SegmentKind::Outro => None,
        }
    }
}

/// One segment in seconds from the start of the item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: f64,
    pub end: f64,
}

pub fn ticks_to_secs(ticks: i64) -> f64 {
    ticks as f64 / TICKS_PER_SEC
}

/// Usable segments from a `/MediaSegments` reply, sorted by start. Unknown types and
/// empty or inverted ranges are dropped.
pub fn from_models(items: &[MediaSegment]) -> Vec<Segment> {
    let mut out: Vec<Segment> = items
        .iter()
        .filter_map(|m| {
            let kind = SegmentKind::parse(&m.segment_type)?;
            let start = ticks_to_secs(m.start_ticks.max(0));
            let end = ticks_to_secs(m.end_ticks);
            (end > start).then_some(Segment { kind, start, end })
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Index of the segment playback is inside at `pos` seconds. The first one by start wins.
pub fn current(segs: &[Segment], pos: f64) -> Option<usize> {
    segs.iter().position(|s| pos >= s.start && pos < s.end)
}

pub fn outro(segs: &[Segment]) -> Option<Segment> {
    segs.iter().find(|s| s.kind == SegmentKind::Outro).copied()
}

/// When the Up Next card appears (`trigger`) and when the next episode starts (`play_at`), in
/// seconds of the current item. With an Outro the card appears at its start and the countdown
/// is 10 s; without one it appears about 30 s before the end (a quarter of the length for
/// short items) and counts down to the end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpNextWindow {
    pub trigger: f64,
    pub play_at: f64,
}

pub fn up_next_window(duration: f64, outro: Option<Segment>) -> Option<UpNextWindow> {
    if !(duration > 0.0) {
        return None;
    }
    match outro {
        Some(o) if o.start > 0.0 && o.start < duration => Some(UpNextWindow {
            trigger: o.start,
            play_at: (o.start + OUTRO_COUNTDOWN).min(duration),
        }),
        _ => {
            let lead = (duration * 0.25).min(UP_NEXT_LEAD_MAX);
            Some(UpNextWindow { trigger: duration - lead, play_at: duration })
        }
    }
}

/// The episode after `current_id` in series order. Seasons ascend and episodes ascend
/// within a season, so the next one may sit in the next season. Specials (season 0) form
/// their own run: a special leads only to later specials, and a regular episode never
/// leads into specials. Missing numbers sort last, keeping server order for ties.
pub fn next_episode<'a>(eps: &'a [Episode], current_id: &str) -> Option<&'a Episode> {
    let cur = eps.iter().find(|e| e.id == current_id)?;
    let special = cur.parent_index == Some(0);
    let mut run: Vec<&Episode> = eps.iter().filter(|e| (e.parent_index == Some(0)) == special).collect();
    run.sort_by_key(|e| (e.parent_index.unwrap_or(u32::MAX), e.number.unwrap_or(u32::MAX)));
    let i = run.iter().position(|e| e.id == current_id)?;
    run.get(i + 1).copied()
}

/// The episode Up Next offers, as the player needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct NextUp {
    pub id: String,
    pub name: String,
    pub season: Option<u32>,
    pub number: Option<u32>,
    /// Still artwork key (`sys_key` of the episode), when the episode has one.
    pub image_key: Option<u64>,
}

impl NextUp {
    pub fn from_episode(e: &Episode) -> NextUp {
        NextUp { id: e.id.clone(), name: e.name.clone(), season: e.parent_index, number: e.number, image_key: None }
    }

    /// "S1 E3", or "" when neither number is known.
    pub fn code(&self) -> String {
        match (self.season, self.number) {
            (Some(s), Some(e)) => format!("S{} E{}", s, e),
            (Some(s), None) => format!("S{}", s),
            (None, Some(e)) => format!("E{}", e),
            (None, None) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t: &str, start: i64, end: i64) -> MediaSegment {
        serde_json::from_value(serde_json::json!({"Type": t, "StartTicks": start, "EndTicks": end})).unwrap()
    }

    fn s(kind: SegmentKind, start: f64, end: f64) -> Segment {
        Segment { kind, start, end }
    }

    fn ep(id: &str, season: Option<u32>, number: Option<u32>) -> Episode {
        serde_json::from_value(serde_json::json!({
            "Id": id, "Name": id, "IndexNumber": number, "ParentIndexNumber": season
        }))
        .unwrap()
    }

    #[test]
    fn ticks_are_ten_million_per_second() {
        assert_eq!(ticks_to_secs(10_000_000), 1.0);
        assert_eq!(ticks_to_secs(15_000_000_000), 1500.0);
        assert_eq!(ticks_to_secs(0), 0.0);
    }

    #[test]
    fn kinds_parse_and_only_skippable_ones_have_labels() {
        assert_eq!(SegmentKind::parse("Intro"), Some(SegmentKind::Intro));
        assert_eq!(SegmentKind::parse(" Recap "), Some(SegmentKind::Recap));
        assert_eq!(SegmentKind::parse("Preview"), Some(SegmentKind::Preview));
        assert_eq!(SegmentKind::parse("Outro"), Some(SegmentKind::Outro));
        assert_eq!(SegmentKind::parse("Commercial"), Some(SegmentKind::Commercial));
        assert_eq!(SegmentKind::parse("Unknown"), None);
        assert_eq!(SegmentKind::parse("intro"), None, "Jellyfin types are case-exact");
        assert_eq!(SegmentKind::Intro.skip_label(), Some("Skip Intro"));
        assert_eq!(SegmentKind::Recap.skip_label(), Some("Skip Recap"));
        assert_eq!(SegmentKind::Preview.skip_label(), Some("Skip Preview"));
        assert_eq!(SegmentKind::Commercial.skip_label(), Some("Skip Commercial"));
        assert_eq!(SegmentKind::Outro.skip_label(), None);
    }

    #[test]
    fn from_models_converts_sorts_and_drops_bad_ranges() {
        let items = vec![
            seg("Outro", 1_300_000_000, 1_400_000_000),
            seg("Intro", 5_000_000, 600_000_000),
            seg("Unknown", 0, 10_000_000),
            seg("Recap", 100, 100),
            seg("Commercial", 900_000_000, 800_000_000),
        ];
        let out = from_models(&items);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, SegmentKind::Intro);
        assert!((out[0].start - 0.5).abs() < 1e-9 && (out[0].end - 60.0).abs() < 1e-9);
        assert_eq!(out[1].kind, SegmentKind::Outro);
        assert!((out[1].start - 130.0).abs() < 1e-9);
    }

    #[test]
    fn from_models_empty_when_nothing_usable() {
        assert!(from_models(&[]).is_empty());
        assert!(from_models(&[seg("Bogus", 0, 10_000_000)]).is_empty());
    }

    #[test]
    fn current_segment_is_half_open_and_first_match_wins() {
        let segs = vec![s(SegmentKind::Intro, 10.0, 20.0), s(SegmentKind::Recap, 15.0, 30.0)];
        assert_eq!(current(&segs, 9.9), None);
        assert_eq!(current(&segs, 10.0), Some(0));
        assert_eq!(current(&segs, 17.0), Some(0), "overlap: the earlier segment wins");
        assert_eq!(current(&segs, 20.0), Some(1), "intro ends exactly at its end time");
        assert_eq!(current(&segs, 30.0), None);
        assert_eq!(current(&[], 5.0), None);
    }

    #[test]
    fn outro_lookup() {
        let segs = vec![s(SegmentKind::Intro, 0.0, 5.0), s(SegmentKind::Outro, 100.0, 150.0)];
        assert_eq!(outro(&segs).map(|o| o.start), Some(100.0));
        assert_eq!(outro(&segs[..1]), None);
    }

    #[test]
    fn up_next_starts_at_outro_with_short_countdown() {
        let w = up_next_window(1500.0, Some(s(SegmentKind::Outro, 1380.0, 1500.0))).unwrap();
        assert_eq!(w, UpNextWindow { trigger: 1380.0, play_at: 1390.0 });
        // An outro that runs into the end is capped at the duration.
        let w = up_next_window(100.0, Some(s(SegmentKind::Outro, 95.0, 100.0))).unwrap();
        assert_eq!((w.trigger, w.play_at), (95.0, 100.0));
    }

    #[test]
    fn up_next_without_outro_is_about_thirty_seconds_before_the_end() {
        let w = up_next_window(1500.0, None).unwrap();
        assert_eq!(w, UpNextWindow { trigger: 1470.0, play_at: 1500.0 });
        // Short items: a quarter of the length, not 30 s.
        let w = up_next_window(40.0, None).unwrap();
        assert_eq!((w.trigger, w.play_at), (30.0, 40.0));
        // An outro that starts at zero is not a usable trigger.
        let w = up_next_window(60.0, Some(s(SegmentKind::Outro, 0.0, 60.0))).unwrap();
        assert_eq!(w.trigger, 45.0);
    }

    #[test]
    fn up_next_needs_a_known_duration() {
        assert_eq!(up_next_window(0.0, None), None);
        assert_eq!(up_next_window(f64::NAN, None), None);
    }

    #[test]
    fn next_episode_crosses_season_boundaries_in_order() {
        // Deliberately shuffled, as a server may return them.
        let eps = vec![
            ep("s2e1", Some(2), Some(1)),
            ep("s1e10", Some(1), Some(10)),
            ep("s1e2", Some(1), Some(2)),
            ep("s1e1", Some(1), Some(1)),
            ep("s1e9", Some(1), Some(9)),
        ];
        assert_eq!(next_episode(&eps, "s1e1").map(|e| e.id.as_str()), Some("s1e2"));
        assert_eq!(next_episode(&eps, "s1e9").map(|e| e.id.as_str()), Some("s1e10"), "numeric, not lexical");
        assert_eq!(next_episode(&eps, "s1e10").map(|e| e.id.as_str()), Some("s2e1"), "crosses into season 2");
        assert!(next_episode(&eps, "s2e1").is_none(), "last episode of the series");
    }

    #[test]
    fn next_episode_specials_form_their_own_run() {
        let eps = vec![
            ep("sp1", Some(0), Some(1)),
            ep("sp2", Some(0), Some(2)),
            ep("s1e1", Some(1), Some(1)),
            ep("s1e2", Some(1), Some(2)),
        ];
        assert_eq!(next_episode(&eps, "sp1").map(|e| e.id.as_str()), Some("sp2"));
        assert!(next_episode(&eps, "sp2").is_none(), "a special never leads into season 1");
        assert!(next_episode(&eps, "s1e2").is_none(), "regular episodes never lead into specials");
        assert_eq!(next_episode(&eps, "s1e1").map(|e| e.id.as_str()), Some("s1e2"));
    }

    #[test]
    fn next_episode_unknown_or_missing_numbers() {
        let eps = vec![ep("a", Some(1), Some(1)), ep("nonum", Some(1), None), ep("b", Some(1), Some(2))];
        assert_eq!(next_episode(&eps, "a").map(|e| e.id.as_str()), Some("b"));
        assert_eq!(next_episode(&eps, "b").map(|e| e.id.as_str()), Some("nonum"), "missing number sorts last");
        assert!(next_episode(&eps, "absent").is_none());
        assert!(next_episode(&[], "a").is_none());
    }

    #[test]
    fn next_up_label() {
        let n = |s, e| NextUp { id: "x".into(), name: "n".into(), season: s, number: e, image_key: None };
        assert_eq!(n(Some(1), Some(3)).code(), "S1 E3");
        assert_eq!(n(Some(12), None).code(), "S12");
        assert_eq!(n(None, Some(4)).code(), "E4");
        assert_eq!(n(None, None).code(), "");
    }
}
