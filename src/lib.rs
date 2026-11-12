//! Code map: a treemap of a repo in a pane beside the terminal. Each folder's area is its file count;
//! its shade is how much your agents edited it today, and a folder two agents both edit is outlined.
// wit-bindgen's flattened `notify` import has 14 params.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({ path: "wit/haunt-plugin.wit", world: "plugin" });

mod map;

use std::sync::Mutex;

use haunt::plugin::log::log;
use haunt::plugin::panel::{self, CanvasNode, Node, PanelNode, Placement, Rect, Shape, TextNode, Tone};
use haunt::plugin::types::LogLevel;
use haunt::plugin::fs_list;
use haunt::plugin::{clock, kv, palette, timer};
use map::{Log, Tile, Today};
use serde::Deserialize;

const PANEL: &str = "map";
const KEY: &str = "today";
const W: f32 = 300.0;
const H: f32 = 180.0;
const TICK_SECS: u32 = 15;
/// File counts are taken again after this long while the pane is open.
const RECOUNT_MS: u64 = 60_000;
/// The host takes at most 4 panel updates a second.
const REDRAW: std::time::Duration = std::time::Duration::from_millis(300);
const GAP: f32 = 1.5;
/// Shade per level, from untouched to busiest; the host shows five steps.
const SHADES: [f32; 4] = [0.0, 0.25, 0.5, 1.0];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder names never counted, besides hidden ones.
    pub skip: Vec<String>,
    /// The "src/auth: api and web both editing" lines under the map.
    pub overlaps: bool,
    pub unverified: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { skip: ["node_modules", "target", "dist", "build", "vendor"].map(String::from).to_vec(), overlaps: true, unverified: false }
    }
}

/// The repo on screen, Haunt's per-folder file counts for it and when they were taken (0: not yet).
type Open = (String, Vec<(String, u32)>, u64);

#[derive(Default)]
struct Live {
    settings: Settings,
    log: Option<Log>,
    open: Option<Open>,
    /// Monotonic: haunt's wall clock only has whole seconds.
    drawn: Option<std::time::Instant>,
    /// The count running for the open repo: (job, root).
    counting: Option<(u64, String)>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Live) -> R) -> R {
    f(LIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Live::default))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn today() -> i64 {
    (now_ms() as i64 / 1000 + i64::from(clock::local_offset_min()) * 60).div_euclid(86_400)
}

fn log_of(l: &mut Live) -> &mut Log {
    l.log.get_or_insert_with(|| kv::get(KEY).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
}

/// File counts, plus a sliver for an edited folder that wasn't counted.
pub fn weights(files: &[(String, u32)], today: &Today) -> Vec<(String, u32)> {
    let mut out = files.to_vec();
    for f in today.edits.keys() {
        if !files.iter().any(|(g, _)| g == f) {
            out.push((f.clone(), 1));
        }
    }
    out
}

/// Haunt draws a rect's second line only this tall or taller.
const SUB_MIN_H: f32 = 30.0;

/// A tile's key, so Haunt eases it when the map shifts: at most 64 characters, one per folder.
fn key(kind: &str, folder: &str) -> Option<String> {
    if folder.chars().count() <= 48 {
        return Some(format!("{kind}:{folder}"));
    }
    let h = folder.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    Some(format!("{kind}:{}~{h:08x}", folder.chars().take(39).collect::<String>()))
}

pub fn shapes(tiles: &[Tile]) -> Vec<Shape> {
    let mut out = Vec::new();
    for t in tiles {
        let (x, y, w, h) = (t.x + GAP, t.y + GAP, (t.w - 2.0 * GAP).max(0.0), (t.h - 2.0 * GAP).max(0.0));
        if t.overlap {
            out.push(Shape::Rect(Rect { x, y, w, h, shade: 0.0, warn: true, label: None, sub: None, key: key("ring", &t.folder) }));
        }
        let inset = if t.overlap { 2.0 } else { 0.0 };
        let h = (h - 2.0 * inset).max(0.0);
        // The caption goes on its own line where it fits, beside the folder where it doesn't.
        let (label, sub) = if h >= SUB_MIN_H { (t.folder.clone(), Some(t.caption.clone())) } else { (format!("{}  {}", t.folder, t.caption), None) };
        out.push(Shape::Rect(Rect {
            x: x + inset,
            y: y + inset,
            w: (w - 2.0 * inset).max(0.0),
            h,
            shade: SHADES[usize::from(t.level.min(3))],
            warn: false,
            label: Some(label),
            sub,
            key: key("tile", &t.folder),
        }));
    }
    out
}

/// `src/auth: api and web both editing`.
pub fn overlap_line(folder: &str, agents: &[&str]) -> String {
    match agents {
        [a, b] => format!("▢ {folder}: {a} and {b} both editing"),
        [rest @ .., last] => format!("▢ {folder}: {} and {last} all editing", rest.join(", ")),
        [] => String::new(),
    }
}

pub fn nodes(files: &[(String, u32)], today: &Today, s: &Settings) -> Vec<PanelNode> {
    let tiles = map::tiles(&weights(files, today), today, W, H);
    let mut out = vec![PanelNode { parent: None, node: Node::Canvas(CanvasNode { width: W as u32, height: H as u32, shapes: shapes(&tiles), ease_ms: None }) }];
    if s.overlaps {
        for (f, agents) in today.overlaps() {
            out.push(PanelNode { parent: None, node: Node::Text(TextNode { text: overlap_line(f, &agents), tone: Tone::Warn, mono: true, bold: false }) });
        }
    }
    out
}

fn draw(l: &mut Live, now: u64) {
    let day = today();
    let Some((root, folders, counted)) = l.open.as_ref() else { return };
    if now.saturating_sub(*counted) >= RECOUNT_MS && l.counting.is_none() {
        // A refused count keeps the last map and tries again on the next redraw.
        match fs_list::tree(root.trim_end_matches('/'), map::DEPTH) {
            Ok(job) => l.counting = Some((job, root.clone())),
            Err(e) => log(LogLevel::Info, &format!("code-map: files not counted: {e}")),
        }
    }
    if *counted == 0 && l.counting.is_some() {
        return; // the first map waits for its count
    }
    let (root, files) = (root.clone(), map::from_tree(&l.settings.skip, folders));
    let t = log_of(l).today(day, &root);
    let title = format!("{} · today", map::agent_name(&root));
    if let Err(e) = panel::show(PANEL, Placement::Right, &title, &nodes(&files, &t, &l.settings)) {
        log(LogLevel::Warn, &format!("code-map: not shown: {e}"));
    }
    l.drawn = Some(std::time::Instant::now());
}

struct Plugin;

impl Guest for Plugin {
    fn init(json: String) {
        with(|l| l.settings = serde_json::from_str(&json).unwrap_or_default());
        palette::command("show", "Code map", None);
    }

    fn on_settings_changed(json: String) {
        palette::command("show", "Code map", None);
        with(|l| {
            l.settings = serde_json::from_str(&json).unwrap_or_default();
            draw(l, now_ms());
        });
    }

    fn on_event(event: AgentEvent) {
        if event.kind != "file_edited" {
            return;
        }
        let (Some(root), Some(path)) = (event.cwd.as_deref(), event.file_path.as_deref()) else { return };
        with(|l| {
            if !event.verified && !l.settings.unverified {
                return;
            }
            let now = now_ms();
            let day = today();
            let log = log_of(l);
            log.record(day, root, path);
            if let Ok(j) = serde_json::to_string(log) {
                let _ = kv::set(KEY, &j);
            }
            let shown = l.open.as_ref().is_some_and(|(r, _, _)| map::folder_of(r, path).is_some());
            if shown && l.drawn.is_none_or(|t| t.elapsed() >= REDRAW) {
                draw(l, now);
            }
        });
    }

    fn palette_items(_mode: String, _query: String) -> Vec<PaletteItem> {
        Vec::new()
    }

    fn on_action(action_id: String, payload: String) {
        with(|l| match (action_id.as_str(), payload.as_str()) {
            ("show", _) => {
                let day = today();
                let Some(root) = log_of(l).latest(day).map(str::to_owned) else {
                    log(LogLevel::Info, "code-map: no edits today yet");
                    return;
                };
                l.open = Some((root, Vec::new(), 0));
                draw(l, now_ms());
                if let Err(e) = timer::every(TICK_SECS) {
                    log(LogLevel::Warn, &format!("code-map: no timer: {e}"));
                }
            }
            ("", p) if p == format!("panel:{PANEL}") => {
                l.open = None;
                timer::stop();
            }
            _ => {}
        });
    }

    fn row_menu(_surface: String) -> Vec<MenuItem> {
        Vec::new()
    }

    fn on_row_moved(_surface: String, _group: String, _before: Option<String>) {}

    fn on_theme_changed() {}

    fn on_tick(now_ms: u64) {
        with(|l| draw(l, now_ms));
    }

    fn on_exec(_job: u64, _outcome: Result<haunt::plugin::exec::Output, String>) {}

    fn on_away_changed(_away: bool, _away_ms: u64) {}

    fn on_hover_hold(_held: bool, _surface: String) {}

    fn on_fs_tree(job: u64, outcome: Result<fs_list::TreeResult, String>) {
        with(|l| {
            let Some((_, root)) = l.counting.take_if(|(j, _)| *j == job) else { return };
            let now = now_ms();
            match (outcome, l.open.as_mut()) {
                (Ok(t), Some(open)) if open.0 == root => (open.1, open.2) = (t.folders.into_iter().map(|f| (f.path, f.files)).collect(), now.max(1)),
                (Ok(_), _) => return,
                (Err(e), _) => {
                    log(LogLevel::Info, &format!("code-map: files not counted: {e}"));
                    return;
                }
            }
            draw(l, now);
        });
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    fn today_like_the_mockup() -> Today {
        let mut t = Today::default();
        for (folder, agent, n) in [("src/auth", "api", 30), ("src/auth", "web", 11), ("web/ui", "web", 22), ("infra/tf", "infra", 9), ("src/db", "api", 6), ("docs", "docs", 3)] {
            for _ in 0..n {
                t.record(folder, agent);
            }
        }
        t
    }

    fn files() -> Vec<(String, u32)> {
        [("src/auth", 50), ("web/ui", 30), ("infra/tf", 20), ("src/db", 15), ("docs", 8), ("tests/e2e", 8), ("scripts", 4)].map(|(f, n)| (f.to_owned(), n)).to_vec()
    }

    fn rects(n: &[PanelNode]) -> Vec<Rect> {
        let Node::Canvas(c) = &n[0].node else { panic!() };
        c.shapes.iter().filter_map(|s| if let Shape::Rect(r) = s { Some(r.clone()) } else { None }).collect()
    }

    fn tile(r: &[Rect], f: &str) -> Rect {
        r.iter().find(|x| x.label.as_deref().is_some_and(|l| l == f || l.starts_with(&format!("{f}  ")))).unwrap().clone()
    }

    /// The folder and its caption, on one line or two.
    fn text(r: &Rect) -> String {
        [r.label.clone(), r.sub.clone()].into_iter().flatten().collect::<Vec<_>>().join("  ")
    }

    #[test]
    fn the_map_reads_like_the_mockup() {
        let n = nodes(&files(), &today_like_the_mockup(), &Settings::default());
        let r = rects(&n);
        assert_eq!(text(&tile(&r, "src/auth")), "src/auth  api, web · 41 edits");
        assert_eq!(tile(&r, "src/auth").sub.as_deref(), Some("api, web · 41 edits"), "a big tile puts the caption under the folder");
        assert_eq!((tile(&r, "src/auth").shade, tile(&r, "web/ui").shade, tile(&r, "tests/e2e").shade), (1.0, 0.5, 0.0), "busiest lightest, untouched darkest");
        assert_eq!(text(&tile(&r, "web/ui")), "web/ui  web · 22");
        assert_eq!(text(&tile(&r, "scripts")), "scripts  0");
        assert!(r.iter().all(|x| x.sub.is_none() || x.h >= SUB_MIN_H), "no second line where Haunt wouldn't draw it");
        let keys: std::collections::BTreeSet<_> = r.iter().map(|x| x.key.clone().unwrap()).collect();
        assert_eq!(keys.len(), r.len(), "one key per shape, so Haunt can ease each");
        assert_eq!(r.iter().filter(|x| x.warn).count(), 1, "one orange outline, round the folder two agents share");
        let Node::Text(t) = &n[1].node else { panic!() };
        assert_eq!((t.text.as_str(), t.mono), ("▢ src/auth: api and web both editing", true));
        assert_eq!(n.len(), 2);
        assert_eq!(nodes(&files(), &today_like_the_mockup(), &Settings { overlaps: false, ..Settings::default() }).len(), 1);
    }

    #[test]
    fn area_follows_file_count_not_edits() {
        let r = rects(&nodes(&files(), &today_like_the_mockup(), &Settings::default()));
        let area = |f: &str| {
            let t = tile(&r, f);
            t.w * t.h
        };
        assert!(area("tests/e2e") > area("scripts") && area("docs") > area("scripts"), "untouched folders keep their size");
        assert!((area("tests/e2e") - area("docs")).abs() < 40.0, "same file count, same size, whatever the edits");
        for x in &r {
            assert!(x.x >= 0.0 && x.y >= 0.0 && x.x + x.w <= W + 0.01 && x.y + x.h <= H + 0.01);
        }
    }

    #[test]
    fn an_edited_folder_that_was_not_counted_still_shows() {
        let mut t = Today::default();
        t.record("new/thing", "api");
        assert!(weights(&files(), &t).contains(&("new/thing".to_owned(), 1)));
    }

    #[test]
    fn overlap_lines_name_everyone() {
        assert_eq!(overlap_line("x", &["a", "b", "c"]), "▢ x: a, b and c all editing");
    }

    #[test]
    fn manifest_asks_for_what_it_uses_and_defaults_match() {
        let m = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/haunt-plugin.toml")).unwrap();
        let t: toml::Table = toml::from_str(&m).unwrap();
        let caps = t["capabilities"].as_table().unwrap();
        assert_eq!(caps.keys().collect::<Vec<_>>(), ["event_details", "events", "fs_list", "panel", "timer"]);
        assert_eq!(caps["events"].as_array().unwrap().len(), 1);
        let defaults: serde_json::Map<String, serde_json::Value> =
            t["settings"].as_table().unwrap().iter().map(|(k, v)| (k.clone(), serde_json::to_value(&v["default"]).unwrap())).collect();
        assert_eq!(serde_json::from_value::<Settings>(serde_json::Value::Object(defaults)).unwrap(), Settings::default());
    }
}
