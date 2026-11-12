//! Today's edits per folder of a repo, laid out as a treemap. Free of host calls.

use std::collections::BTreeMap;

/// The folder a file counts toward: its first two directories (`src/auth`), or one for a file
/// directly inside a top folder (`docs`), or `.` for a file at the root. `None` outside the repo.
pub fn folder_of(root: &str, path: &str) -> Option<String> {
    let rel = path.strip_prefix(root.trim_end_matches('/'))?.strip_prefix('/')?;
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    let dirs = &parts[..parts.len() - 1];
    Some(match dirs {
        [] => ".".into(),
        [one] => (*one).into(),
        [a, b, ..] => format!("{a}/{b}"),
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Today {
    /// Folder → agent name → edits.
    pub edits: BTreeMap<String, BTreeMap<String, u32>>,
}

impl Today {
    pub fn record(&mut self, folder: &str, agent: &str) {
        let n = self.edits.entry(folder.into()).or_default().entry(agent.into()).or_default();
        *n = n.saturating_add(1);
    }

    pub fn total(&self, folder: &str) -> u32 {
        self.edits.get(folder).map_or(0, |m| m.values().sum())
    }

    pub fn agents(&self, folder: &str) -> Vec<&str> {
        self.edits.get(folder).map_or_else(Vec::new, |m| m.keys().map(String::as_str).collect())
    }

    /// Folders two or more agents edited today.
    pub fn overlaps(&self) -> Vec<(&str, Vec<&str>)> {
        self.edits.iter().filter(|(_, m)| m.len() > 1).map(|(f, m)| (f.as_str(), m.keys().map(String::as_str).collect())).collect()
    }
}

/// Today's edits by absolute directory, so any repo's map can be drawn from them.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Log {
    pub day: i64,
    /// Directory → agent name → edits.
    pub dirs: BTreeMap<String, BTreeMap<String, u32>>,
    /// Each repo's folder, with its last edit's place in the day's order; haunt's clock is per second.
    pub roots: BTreeMap<String, u64>,
    pub edits: u64,
}

/// Distinct directories kept per day.
pub const DIRS_MAX: usize = 4096;

impl Log {
    /// Records one edit of `path` by the agent working in `root`. A new local day starts afresh.
    pub fn record(&mut self, day: i64, root: &str, path: &str) {
        if day != self.day {
            *self = Log { day, ..Log::default() };
        }
        let Some((dir, _)) = path.rsplit_once('/') else { return };
        if folder_of(root, path).is_none() || (!self.dirs.contains_key(dir) && self.dirs.len() >= DIRS_MAX) {
            return;
        }
        let n = self.dirs.entry(dir.into()).or_default().entry(agent_name(root)).or_default();
        *n = n.saturating_add(1);
        self.edits += 1;
        self.roots.insert(root.trim_end_matches('/').into(), self.edits);
    }

    /// The repo edited last today.
    pub fn latest(&self, day: i64) -> Option<&str> {
        (day == self.day).then(|| self.roots.iter().max_by_key(|(_, t)| **t).map(|(r, _)| r.as_str())).flatten()
    }

    /// Today's edits under `root`, by folder.
    pub fn today(&self, day: i64, root: &str) -> Today {
        let mut t = Today::default();
        if day != self.day {
            return t;
        }
        for (dir, agents) in &self.dirs {
            if let Some(f) = folder_of(root, &format!("{dir}/f")).filter(|f| f != ".") {
                for (a, n) in agents {
                    let e = t.edits.entry(f.clone()).or_default().entry(a.clone()).or_default();
                    *e = e.saturating_add(*n);
                }
            }
        }
        t
    }
}

/// An agent goes by its folder's name, as on its row: `/w/api` is `api`.
pub fn agent_name(root: &str) -> String {
    root.trim_end_matches('/').rsplit('/').next().unwrap_or(root).to_owned()
}

/// Levels below the repo that Haunt counts; deeper files count toward their folder at this depth.
pub const DEPTH: u8 = 5;

/// Files per folder (as `folder_of` names them), from Haunt's per-folder counts with paths relative to
/// the repo. Folders named in `skip`, and anything under them, are left out; so are files at the root.
pub fn from_tree(skip: &[String], folders: &[(String, u32)]) -> Vec<(String, u32)> {
    let mut files: BTreeMap<String, u32> = BTreeMap::new();
    for (path, n) in folders {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || *n == 0 || parts.iter().any(|p| skip.iter().any(|s| s == p)) {
            continue;
        }
        *files.entry(parts[..parts.len().min(2)].join("/")).or_default() += n;
    }
    files.into_iter().collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub folder: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub edits: u32,
    /// 0 (untouched) to 3 (busiest), relative to the busiest folder.
    pub level: u8,
    pub overlap: bool,
    /// `api, web · 41 edits`, `web · 22`, `0`.
    pub caption: String,
}

pub fn level(n: u32, max: u32) -> u8 {
    if n == 0 || max == 0 { 0 } else { (1 + (u64::from(n) * 3 - 1) / u64::from(max)).min(3) as u8 }
}

fn caption(agents: &[&str], n: u32, first: bool) -> String {
    match (agents, n) {
        (_, 0) => "0".into(),
        (a, n) if first => format!("{} · {n} edits", a.join(", ")),
        (a, n) => format!("{} · {n}", a.join(", ")),
    }
}

/// Squarified treemap of `(folder, weight)` in a `w`×`h` box; folders with weight 0 are dropped.
pub fn squarify(items: &[(String, f32)], w: f32, h: f32) -> Vec<(String, f32, f32, f32, f32)> {
    let mut items: Vec<_> = items.iter().filter(|(_, v)| *v > 0.0).cloned().collect();
    items.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let total: f32 = items.iter().map(|i| i.1).sum();
    let mut out = Vec::new();
    if total <= 0.0 {
        return out;
    }
    let scale = w * h / total;
    let (mut x, mut y, mut w, mut h) = (0.0f32, 0.0f32, w, h);
    let mut rest: &[(String, f32)] = &items;
    while !rest.is_empty() {
        let side = w.min(h);
        let worst = |row: &[(String, f32)]| {
            let s: f32 = row.iter().map(|r| r.1 * scale).sum();
            let (mx, mn) = row.iter().fold((0f32, f32::MAX), |(a, b), r| (a.max(r.1 * scale), b.min(r.1 * scale)));
            (side * side * mx / (s * s)).max(s * s / (side * side * mn))
        };
        let mut n = 1;
        while n < rest.len() && worst(&rest[..n + 1]) <= worst(&rest[..n]) {
            n += 1;
        }
        let (row, tail) = rest.split_at(n);
        let s: f32 = row.iter().map(|r| r.1 * scale).sum();
        let thick = s / side;
        let mut off = 0.0;
        for (f, v) in row {
            let len = v * scale / thick;
            if w >= h {
                out.push((f.clone(), x, y + off, thick, len));
            } else {
                out.push((f.clone(), x + off, y, len, thick));
            }
            off += len;
        }
        if w >= h {
            x += thick;
            w -= thick;
        } else {
            y += thick;
            h -= thick;
        }
        rest = tail;
    }
    out
}

/// Tiles sized by each folder's file count, shaded by today's edits.
pub fn tiles(files: &[(String, u32)], today: &Today, w: f32, h: f32) -> Vec<Tile> {
    let weights: Vec<(String, f32)> = files.iter().map(|(f, n)| (f.clone(), *n as f32)).collect();
    let max = files.iter().map(|(f, _)| today.total(f)).max().unwrap_or(0);
    let busiest = files.iter().filter(|(f, _)| today.total(f) == max && max > 0).map(|(f, _)| f.as_str()).next();
    squarify(&weights, w, h)
        .into_iter()
        .map(|(folder, x, y, w, h)| {
            let edits = today.total(&folder);
            let agents = today.agents(&folder);
            Tile { caption: caption(&agents, edits, busiest == Some(folder.as_str())), level: level(edits, max), overlap: agents.len() > 1, edits, folder, x, y, w, h }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_count_toward_two_levels_of_folder() {
        assert_eq!(folder_of("/w/shop", "/w/shop/src/auth/session.rs").as_deref(), Some("src/auth"));
        assert_eq!(folder_of("/w/shop/", "/w/shop/src/auth/deep/x.rs").as_deref(), Some("src/auth"));
        assert_eq!(folder_of("/w/shop", "/w/shop/docs/a.md").as_deref(), Some("docs"));
        assert_eq!(folder_of("/w/shop", "/w/shop/README.md").as_deref(), Some("."));
        assert_eq!(folder_of("/w/shop", "/w/shopping/x"), None);
        assert_eq!(folder_of("/w/shop", "/w/shop/../etc/x"), None);
    }

    #[test]
    fn overlaps_and_captions() {
        let mut t = Today::default();
        for _ in 0..30 {
            t.record("src/auth", "api");
        }
        for _ in 0..11 {
            t.record("src/auth", "web");
        }
        for _ in 0..22 {
            t.record("web/ui", "web");
        }
        assert_eq!(t.overlaps(), [("src/auth", vec!["api", "web"])]);
        let files = vec![("src/auth".to_owned(), 40), ("web/ui".to_owned(), 20), ("scripts".to_owned(), 5), ("empty".to_owned(), 0)];
        let tiles = tiles(&files, &t, 100.0, 50.0);
        assert_eq!(tiles.len(), 3, "a folder with no files has no area");
        let auth = tiles.iter().find(|t| t.folder == "src/auth").unwrap();
        assert_eq!((auth.caption.as_str(), auth.level, auth.overlap), ("api, web · 41 edits", 3, true));
        let ui = tiles.iter().find(|t| t.folder == "web/ui").unwrap();
        assert_eq!((ui.caption.as_str(), ui.level), ("web · 22", 2));
        assert_eq!(tiles.iter().find(|t| t.folder == "scripts").unwrap().caption, "0");
    }

    #[test]
    fn the_log_keeps_one_day_and_any_repo_reads_it() {
        let mut l = Log::default();
        l.record(10, "/w/api", "/w/api/src/auth/a.rs");
        l.record(10, "/w/api", "/w/api/src/auth/b.rs");
        l.record(10, "/w/shop", "/w/shop/web/ui/x.tsx");
        l.record(10, "/w/api", "/etc/passwd");
        assert_eq!(l.latest(10), Some("/w/shop"));
        let t = l.today(10, "/w/api");
        assert_eq!((t.total("src/auth"), t.agents("src/auth")), (2, vec!["api"]));
        assert_eq!(l.today(10, "/w/shop").total("web/ui"), 1);
        assert_eq!(l.today(10, "/w").total("api/src"), 2, "a parent folder sees its repos' edits");
        assert!(l.today(11, "/w/api").edits.is_empty() && l.latest(11).is_none(), "yesterday's edits don't show");
        l.record(11, "/w/api", "/w/api/docs/x.md");
        assert_eq!((l.dirs.len(), l.roots.len()), (1, 1), "a new day starts afresh");
    }

    #[test]
    fn counts_roll_up_to_two_levels_and_leave_out_skipped_folders() {
        let tree: Vec<(String, u32)> = [("", 2), ("docs", 1), ("node_modules/x", 40), ("src", 1), ("src/auth", 2), ("src/auth/deep", 1), ("src/db", 0)]
            .iter()
            .map(|(p, n)| (p.to_string(), *n))
            .collect();
        let counts = from_tree(&["node_modules".to_owned()], &tree);
        assert_eq!(counts, [("docs".to_owned(), 1), ("src".to_owned(), 1), ("src/auth".to_owned(), 3)]);
    }

    #[test]
    fn the_treemap_fills_the_box_in_proportion() {
        let items: Vec<(String, f32)> = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0].iter().enumerate().map(|(i, v)| (format!("f{i}"), *v)).collect();
        let r = squarify(&items, 60.0, 40.0);
        let area: f32 = r.iter().map(|t| t.3 * t.4).sum();
        assert!((area - 2400.0).abs() < 0.5);
        for (f, _, _, w, h) in &r {
            let v = items.iter().find(|i| &i.0 == f).unwrap().1;
            assert!((w * h - v * 100.0).abs() < 0.5, "{f}");
        }
        for (_, x, y, w, h) in &r {
            assert!(*x >= -0.01 && *y >= -0.01 && x + w <= 60.01 && y + h <= 40.01);
        }
        assert!(squarify(&[], 10.0, 10.0).is_empty());
    }
}
