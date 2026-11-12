//! End to end against a real haunt core. Copy into haunt's `cli/tests/` and run
//! `cargo test -p haunt-cli --features plugins --test code_map` (see README).
#![cfg(feature = "plugins")]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use haunt_core::plugin_panel::{Panel, PanelNodeKind, PanelPlacement, PanelShape};
use haunt_core::plugin_settings::PluginConfig;
use haunt_core::plugin_store::{PluginSet, Store, attach};
use haunt_core::plugins::{HostRequest, PluginHost};
use haunt_core::{Core, SurfaceEnv};

const ID: &str = "dev.haunt.code-map";

fn repo() -> PathBuf {
    std::env::var_os("HAUNT_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../haunt-plugins"))
        .join("haunt-code-map")
}

fn until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(sock: &mut UnixStream, env: &SurfaceEnv, nonce: bool, cwd: &str, rest: &str) {
    let nonce = if nonce { format!(r#","nonce":"{}""#, env.nonce) } else { String::new() };
    let line = format!(r#"{{"v":1,"agent":"claude","surface_id":"{}"{nonce},"session_id":"s1","cwd":"{cwd}",{rest}}}"#, env.surface_id);
    sock.write_all(format!("{line}\n").as_bytes()).unwrap();
}

fn edit(path: &Path) -> String {
    format!(r#""event":"file_edited","tool_name":"Edit","tool_input":{{"file_path":"{}"}}"#, path.display())
}

fn last_map(set: &Mutex<PluginSet>) -> Option<Panel> {
    set.lock().unwrap().take_requests().into_iter().rev().find_map(|r| match r {
        HostRequest::ShowPanel { plugin_id, panel, .. } if plugin_id == ID => Some(panel),
        _ => None,
    })
}

/// The next map shown, waiting for Haunt's count to come back.
fn next_map(set: &Mutex<PluginSet>) -> Panel {
    let t = Instant::now();
    loop {
        if let Some(p) = last_map(set) {
            return p;
        }
        assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for the map");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Each tile's label and second line, shade and whether it is outlined in orange.
fn tiles(p: &Panel) -> Vec<(String, f32, bool)> {
    let PanelNodeKind::Canvas { shapes, .. } = &p.nodes[0].kind else { panic!("{:?}", p.nodes[0]) };
    let outlined: Vec<(f32, f32)> = shapes.iter().filter_map(|s| if let PanelShape::Rect { x, y, warn: true, .. } = s { Some((*x, *y)) } else { None }).collect();
    shapes
        .iter()
        .filter_map(|s| match s {
            PanelShape::Rect { x, y, shade, label: Some(l), sub, .. } => {
                let text = sub.as_ref().map_or_else(|| l.clone(), |s| format!("{l}  {s}"));
                Some((text, *shade, outlined.iter().any(|(ox, oy)| (x - ox - 2.0).abs() < 0.1 && (y - oy - 2.0).abs() < 0.1)))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_pane_maps_the_repo_and_todays_edits() {
    let dir = repo();
    if !dir.join("haunt-plugin.toml").is_file() {
        eprintln!("skipped: haunt-code-map not found; set HAUNT_PLUGINS_DIR");
        return;
    }
    let home = PathBuf::from(format!("/tmp/haunt-fp-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // Outside HAUNT_HOME: haunt never lets a plugin's ${cwd} point into its own folder.
    let work = PathBuf::from(format!("/tmp/haunt-fp-map-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let shop = work.join("shop");
    for (folder, n) in [("src/auth", 6), ("web/ui", 4), ("docs", 2), (".git", 9), ("node_modules/x", 9)] {
        std::fs::create_dir_all(shop.join(folder)).unwrap();
        for i in 0..n {
            std::fs::write(shop.join(folder).join(format!("f{i}")), "secret").unwrap();
        }
    }
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&dir)
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .unwrap();
    assert!(status.success());
    let o = Command::new(env!("CARGO_BIN_EXE_haunt")).args(["plugin", "install", dir.to_str().unwrap(), "--yes"]).env("HAUNT_HOME", &home).output().unwrap();
    assert!(o.status.success(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));

    // ${cwd} is only ever a tab's folder strictly inside $HOME.
    unsafe { std::env::set_var("HOME", &work) };
    let core = Core::new(&home);
    core.start().unwrap();
    let (a, b, forged) = (
        core.register_surface(shop.to_str().unwrap()).unwrap(),
        core.register_surface(shop.join("web").to_str().unwrap()).unwrap(),
        core.register_surface(shop.to_str().unwrap()).unwrap(),
    );
    let set = Store::open(&home).unwrap().load_all(&PluginHost::new().unwrap(), Some(&core.handle())).unwrap();
    assert!(set.errors().is_empty(), "{:?}", set.errors());
    let set = Arc::new(Mutex::new(set));
    attach(&core, &set);

    let (shop_s, web_s) = (shop.to_str().unwrap().to_owned(), shop.join("web").to_str().unwrap().to_owned());
    let mut sock = UnixStream::connect(&a.socket_path).unwrap();
    send(&mut sock, &b, true, &web_s, &edit(&shop.join("web/ui/f0")));
    std::thread::sleep(Duration::from_millis(50));
    send(&mut sock, &forged, false, &shop_s, &edit(&shop.join("docs/f0")));
    for i in 0..3 {
        send(&mut sock, &a, true, &shop_s, &edit(&shop.join(format!("src/auth/f{i}"))));
    }
    send(&mut sock, &a, true, &shop_s, &edit(&shop.join("web/ui/f1")));
    std::thread::sleep(Duration::from_millis(300));

    set.lock().unwrap().invoke(ID, "show", true).unwrap();
    let p = next_map(&set);
    assert_eq!((p.id.as_str(), p.placement, p.title.as_str()), ("map", PanelPlacement::Right, "shop · today"));
    let mut t = tiles(&p);
    t.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        t,
        [("docs  0".to_owned(), 0.0, false), ("src/auth  shop · 3 edits".to_owned(), 1.0, false), ("web/ui  shop, web · 2".to_owned(), 0.5, true)],
        "file counts from the repo, hidden and skipped folders left out, the forged edit not counted"
    );
    let PanelNodeKind::Text { text, mono: true, .. } = &p.nodes[1].kind else { panic!("{:?}", p.nodes) };
    assert_eq!(text, "▢ web/ui: shop and web both editing");

    std::thread::sleep(Duration::from_millis(350));
    send(&mut sock, &a, true, &shop_s, &edit(&shop.join("docs/f1")));
    until("the pane to follow a new edit", || {
        std::thread::sleep(Duration::from_millis(50));
        last_map(&set).is_some_and(|p| tiles(&p).iter().any(|t| t.0 == "docs  shop · 1"))
    });

    set.lock().unwrap().set_config(&PluginConfig::parse(&format!("[plugins.\"{ID}\"]\noverlaps = false\nskip = []\n")));
    let p = last_map(&set).expect("an open pane follows its settings");
    assert_eq!(p.nodes.len(), 1);
    let generation = p.generation;
    assert!(tiles(&p).iter().any(|t| t.0 == "node_modules/x  0"));

    set.lock().unwrap().panel_action(ID, "map", generation, "").unwrap();
    send(&mut sock, &a, true, &shop_s, &edit(&shop.join("docs/f0")));
    std::thread::sleep(Duration::from_millis(400));
    assert!(last_map(&set).is_none(), "a closed pane stays closed");

    drop(core);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&work);
}
