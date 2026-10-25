//! Colour palettes: JSON files in `<config>/themes`, validated and cached.
//!
//! A palette file is `{"base": "dark" | "light", "name": "...", "colors": {...}}`.
//! Colours are `#RRGGBB` or `#RRGGBBAA`; keys left out inherit from the base
//! palette. Files that fail to parse or validate are skipped with a log warning,
//! and the last accepted copy of each palette is kept in `Config::palette_cache`.
//! Pure Rust (no AppKit), so it is unit-tested directly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Colour keys, in slot order. `ok` is an extra key beyond the documented set.
pub const KEYS: [&str; 18] = [
    "window", "panel", "surface", "surface_hover", "surface_active", "outline", "text", "secondary", "dim",
    "accent", "accent_hover", "on_accent", "danger", "warning", "overlay", "shadow", "tint", "ok",
];
pub const SLOTS: usize = 18;
pub const MAX_BYTES: u64 = 64 * 1024;
pub const MAX_FILES: usize = 64;
const INSTALLED_RECORD: &str = ".installed-palettes";

/// (0xRRGGBB, alpha 0-255).
pub type Rgba = (u32, u8);

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub colors: [Rgba; SLOTS],
}

/// Fixed base values that omitted keys inherit from.
const DARK_BASE: [Rgba; SLOTS] = [
    (0x1C1C1E, 255), (0x161618, 255), (0x2C2C2E, 255), (0x3A3A3C, 255), (0x48484A, 255), (0x3A3A3C, 255),
    (0xF2F2F7, 255), (0xAEAEB2, 255), (0x98989F, 255), (0x0A84FF, 255), (0x409CFF, 255), (0xFFFFFF, 255),
    (0xFF453A, 255), (0xFF9F0A, 255), (0x000000, 128), (0x000000, 90), (0x212121, 255), (0x30D158, 255),
];
const LIGHT_BASE: [Rgba; SLOTS] = [
    (0xECECEC, 255), (0xFFFFFF, 255), (0xE5E5EA, 255), (0xDADADF, 255), (0xCFCFD4, 255), (0xD1D1D6, 255),
    (0x1C1C1E, 255), (0x505057, 255), (0x636368, 255), (0x0B63CE, 255), (0x0A56B4, 255), (0xFFFFFF, 255),
    (0xD70015, 255), (0xB25000, 255), (0x000000, 89), (0x000000, 46), (0x212121, 255), (0x1C7C2E, 255),
];

/// Seeded palettes (written once on first launch; see `seed`).
const SEEDS: [(&str, &str); 6] = [
    ("catppuccin-mocha", include_str!("palettes_seed/catppuccin-mocha.json")),
    ("catppuccin-latte", include_str!("palettes_seed/catppuccin-latte.json")),
    ("nord", include_str!("palettes_seed/nord.json")),
    ("rose-pine", include_str!("palettes_seed/rose-pine.json")),
    ("rose-pine-dawn", include_str!("palettes_seed/rose-pine-dawn.json")),
    ("tokyo-night", include_str!("palettes_seed/tokyo-night.json")),
];

static REGISTRY: Mutex<Vec<Theme>> = Mutex::new(Vec::new());

pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => Some((u32::from_str_radix(h, 16).ok()?, 255)),
        8 => Some((u32::from_str_radix(&h[..6], 16).ok()?, u8::from_str_radix(&h[6..], 16).ok()?)),
        _ => None,
    }
}

fn composite(fg: Rgba, bg: Rgba) -> Rgba {
    let a = fg.1 as u32;
    let mut out = 0u32;
    for shift in [16, 8, 0] {
        let f = (fg.0 >> shift) & 0xff;
        let b = (bg.0 >> shift) & 0xff;
        out |= ((f * a + b * (255 - a)) / 255) << shift;
    }
    (out, 255)
}

fn luminance(c: u32) -> f64 {
    let lin = |v: u32| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin((c >> 16) & 0xff) + 0.7152 * lin((c >> 8) & 0xff) + 0.0722 * lin(c & 0xff)
}

/// WCAG contrast ratio between two opaque colours.
pub fn contrast(a: u32, b: u32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn prettify(id: &str) -> String {
    id.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse and validate one palette file. `id` is the file stem.
pub fn parse_theme(id: &str, text: &str) -> Result<Theme, String> {
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("larger than {} KiB", MAX_BYTES / 1024));
    }
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let obj = v.as_object().ok_or("top level must be an object")?;
    let dark = match obj.get("base").and_then(|b| b.as_str()) {
        Some("dark") => true,
        Some("light") => false,
        _ => return Err("\"base\" must be \"dark\" or \"light\"".into()),
    };
    let name = obj.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| prettify(id));
    let mut colors = if dark { DARK_BASE } else { LIGHT_BASE };
    if let Some(c) = obj.get("colors") {
        let map = c.as_object().ok_or("\"colors\" must be an object")?;
        for (k, val) in map {
            let Some(slot) = KEYS.iter().position(|x| x == k) else {
                tracing::warn!("palette {id}: unknown colour key \"{k}\" ignored");
                continue;
            };
            let s = val.as_str().ok_or_else(|| format!("\"{k}\" must be a string"))?;
            colors[slot] = parse_hex(s).ok_or_else(|| format!("\"{k}\" is not #RRGGBB or #RRGGBBAA"))?;
        }
    }
    // Body text must stay readable (WCAG AA 4.5:1) on both the window and the panel.
    for k in ["text", "secondary", "dim"] {
        let slot = KEYS.iter().position(|x| *x == k).unwrap_or(0);
        for bg in [colors[0], colors[1]] {
            let fg = composite(colors[slot], bg);
            if contrast(fg.0, bg.0) < 4.5 {
                return Err(format!("\"{k}\" is below 4.5:1 contrast on the window or panel colour"));
            }
        }
    }
    let accent = colors[KEYS.iter().position(|x| *x == "accent").unwrap_or(9)];
    for bg in [colors[0], colors[1]] {
        if contrast(composite(accent, bg).0, bg.0) < 3.0 {
            return Err("\"accent\" is below 3:1 contrast on the window or panel colour".into());
        }
    }
    Ok(Theme { id: id.to_string(), name, dark, colors })
}

/// Scan `dir` and replace the registry. Valid files refresh `cache`; invalid ones are
/// skipped with a warning and keep whatever the cache already holds.
pub fn scan(dir: &Path, cache: &mut BTreeMap<String, String>) -> usize {
    let mut found: Vec<Theme> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json") && p.is_file())
            .collect();
        paths.sort();
        for p in paths.into_iter().take(MAX_FILES) {
            let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }
            if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(true) {
                tracing::warn!("palette {id}: skipped (file larger than 64 KiB or unreadable)");
                continue;
            }
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("palette {id}: skipped ({e})");
                    continue;
                }
            };
            match parse_theme(&id, &text) {
                Ok(t) => {
                    cache.insert(id, text);
                    found.push(t);
                }
                Err(e) => tracing::warn!("palette {id}: skipped ({e})"),
            }
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let n = found.len();
    *REGISTRY.lock().unwrap_or_else(|e| e.into_inner()) = found;
    n
}

/// Write the seed palettes that were never written before. The record file lists every
/// seed ever considered, so a palette the user deleted (or edited) is never re-added
/// or overwritten.
pub fn seed(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let rec_path = dir.join(INSTALLED_RECORD);
    let mut recorded: BTreeSet<String> = std::fs::read_to_string(&rec_path)
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    for (id, text) in SEEDS {
        if recorded.contains(id) {
            continue;
        }
        let p = dir.join(format!("{id}.json"));
        if !p.exists() {
            std::fs::write(&p, text)?;
        }
        recorded.insert(id.to_string());
    }
    let body: String = recorded.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(rec_path, body)
}

/// Folder holding palette files: `<config>/themes`, or a self-test override.
pub fn themes_dir() -> PathBuf {
    if crate::app::selftest::enabled() {
        if let Ok(d) = std::env::var("JELLYFAST_THEMES_DIR") {
            if !d.is_empty() {
                return PathBuf::from(d);
            }
        }
    }
    crate::config::config_dir().join("themes")
}

/// Seed (first launch), then scan. Returns the number of valid palettes found.
pub fn load_all(cache: &mut BTreeMap<String, String>) -> usize {
    let dir = themes_dir();
    if let Err(e) = seed(&dir) {
        tracing::warn!("could not seed themes folder: {e}");
    }
    scan(&dir, cache)
}

pub fn registry() -> Vec<Theme> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A palette by id: the scanned file if valid, else the last accepted copy in `cache`.
/// The bool is true when it came from the cache.
pub fn find(id: &str, cache: &BTreeMap<String, String>) -> Option<(Theme, bool)> {
    if let Some(t) = registry().into_iter().find(|t| t.id == id) {
        return Some((t, false));
    }
    let text = cache.get(id)?;
    parse_theme(id, text).ok().map(|t| (t, true))
}

pub const BUILTIN_LIGHT: &str = "light";
pub const BUILTIN_DARK: &str = "dark";
pub const FOLLOW_SYSTEM: &str = "system";

/// Top-level Theme picker: Follow system, Light, Dark, a separator (empty id), then
/// every valid palette by name.
pub fn theme_choices() -> Vec<(String, String)> {
    let mut v = vec![
        (FOLLOW_SYSTEM.to_string(), "Follow system".to_string()),
        (BUILTIN_LIGHT.to_string(), "Light".to_string()),
        (BUILTIN_DARK.to_string(), "Dark".to_string()),
        (String::new(), String::new()),
    ];
    v.extend(registry().into_iter().map(|t| (t.id, t.name)));
    v
}

/// Light-mode or dark-mode picker: the Jellyfast built-in first, then matching palettes.
pub fn mode_choices(dark: bool) -> Vec<(String, String)> {
    let (id, name) = if dark { (BUILTIN_DARK, "Jellyfast Dark") } else { (BUILTIN_LIGHT, "Jellyfast Light") };
    let mut v = vec![(id.to_string(), name.to_string())];
    v.extend(registry().into_iter().filter(|t| t.dark == dark).map(|t| (t.id, t.name)));
    v
}

/// What a palette selection resolves to. `theme` is `None` for the Jellyfast built-ins,
/// which use the system colours.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub dark: bool,
    pub theme: Option<Theme>,
    pub note: Option<String>,
}

/// Resolve a concrete choice (`light`, `dark` or a palette id). `want_dark` is the
/// mode the selection must match (None for a single fixed theme); a palette of the
/// wrong base falls back to the built-in for that mode.
pub fn resolve(id: &str, want_dark: Option<bool>, cache: &BTreeMap<String, String>) -> Resolved {
    if id == BUILTIN_LIGHT || id == BUILTIN_DARK {
        return Resolved { dark: id == BUILTIN_DARK, theme: None, note: None };
    }
    let fallback_dark = want_dark.unwrap_or(true);
    let builtin = |note: Option<String>| Resolved { dark: fallback_dark, theme: None, note };
    match find(id, cache) {
        None => builtin(Some(format!("Theme \u{201C}{id}\u{201D} is missing. Using Jellyfast {}.", if fallback_dark { "Dark" } else { "Light" }))),
        Some((t, from_cache)) => {
            if let Some(w) = want_dark {
                if t.dark != w {
                    return builtin(Some(format!("\u{201C}{}\u{201D} is a {} theme for this mode. Using Jellyfast {}.", t.name, if t.dark { "dark" } else { "light" }, if w { "Dark" } else { "Light" })));
                }
            }
            let note = from_cache.then(|| format!("\u{201C}{}\u{201D} is missing or invalid. Using its last saved colours.", t.name));
            Resolved { dark: t.dark, theme: Some(t), note }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-global; tests that touch it run one at a time.
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    const VALID: &str = r##"{"base":"dark","name":"Test","colors":{"accent":"#5AB0FF","text":"#F2F2F7"}}"##;

    #[test]
    fn hex_forms() {
        assert_eq!(parse_hex("#0A84FF"), Some((0x0A84FF, 255)));
        assert_eq!(parse_hex("#0A84FF80"), Some((0x0A84FF, 0x80)));
        assert_eq!(parse_hex("0A84FF"), None);
        assert_eq!(parse_hex("#0A84F"), None);
        assert_eq!(parse_hex("#ZZZZZZ"), None);
    }

    #[test]
    fn valid_palette_inherits_omitted_keys() {
        let t = parse_theme("test", VALID).unwrap();
        assert!(t.dark);
        assert_eq!(t.name, "Test");
        assert_eq!(t.colors[9], (0x5AB0FF, 255));
        assert_eq!(t.colors[0], DARK_BASE[0]);
    }

    #[test]
    fn invalid_palettes_are_rejected() {
        assert!(parse_theme("x", "not json").is_err());
        assert!(parse_theme("x", r#"{"base":"sepia"}"#).is_err());
        assert!(parse_theme("x", r#"{"base":"dark","colors":{"text":"red"}}"#).is_err());
        // Dark grey text on the dark window fails AA.
        assert!(parse_theme("x", r##"{"base":"dark","colors":{"text":"#333333"}}"##).is_err());
        let big = format!(r#"{{"base":"dark","name":"{}"}}"#, "a".repeat(70 * 1024));
        assert!(parse_theme("x", &big).is_err());
    }

    #[test]
    fn seeds_all_validate() {
        for (id, text) in SEEDS {
            assert!(parse_theme(id, text).is_ok(), "seed {id} must validate");
        }
    }

    #[test]
    fn seed_is_written_once_and_respects_deletions() {
        let dir = std::env::temp_dir().join(format!("jf-themes-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        seed(&dir).unwrap();
        assert!(dir.join("nord.json").exists());
        std::fs::remove_file(dir.join("nord.json")).unwrap();
        std::fs::write(dir.join("tokyo-night.json"), VALID).unwrap();
        seed(&dir).unwrap();
        assert!(!dir.join("nord.json").exists(), "deleted seed must not come back");
        assert_eq!(std::fs::read_to_string(dir.join("tokyo-night.json")).unwrap(), VALID);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_skips_invalid_and_cache_keeps_last_good() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("jf-themes-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("good.json"), VALID).unwrap();
        std::fs::write(dir.join("bad.json"), "{").unwrap();
        let mut cache = BTreeMap::new();
        assert_eq!(scan(&dir, &mut cache), 1);
        assert!(cache.contains_key("good") && !cache.contains_key("bad"));
        // The file becomes invalid: the cached copy still resolves, with a note.
        std::fs::write(dir.join("good.json"), "{").unwrap();
        assert_eq!(scan(&dir, &mut cache), 0);
        let r = resolve("good", None, &cache);
        assert!(r.theme.is_some());
        assert!(r.note.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mode_choices_filter_by_base() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *REGISTRY.lock().unwrap() = vec![
            parse_theme("d", VALID).unwrap(),
            parse_theme("l", r#"{"base":"light","name":"L"}"#).unwrap(),
        ];
        let dark: Vec<String> = mode_choices(true).into_iter().map(|c| c.0).collect();
        let light: Vec<String> = mode_choices(false).into_iter().map(|c| c.0).collect();
        assert_eq!(dark, vec!["dark", "d"]);
        assert_eq!(light, vec!["light", "l"]);
        REGISTRY.lock().unwrap().clear();
    }
}
