<div align="center">

<img src="docs/icon.png" width="96" alt="">

# Code map

**A treemap of where your agents are editing a repo today.**

[![Haunt plugin](https://img.shields.io/badge/Haunt-plugin-D97757)](https://github.com/highskore/haunt-hackathon)
![Plugin API 0.5](https://img.shields.io/badge/plugin%20API-0.5-111)
![WASM component](https://img.shields.io/badge/WASM-component-654ff0?logo=webassembly&logoColor=white)
![MIT licence](https://img.shields.io/badge/licence-MIT-6b7280)

</div>

Run **Code map** from the ⌘K palette and a pane opens beside the terminal for the repo your agents edited last. Each folder is a tile, two levels deep (`src/auth`, `web/ui`, `docs`). A tile's area is the number of files in it, so the map stays the same shape all day. Its shade is how much your agents edited it today: untouched folders stay dark and the busiest is lightest. Each tile names its folder, with `api, web · 41 edits` under it (beside it on a thin tile): the agents that edited it and how often. When the counts change, tiles ease to their new size. A folder two or more agents edited today gets an orange outline, and a line under the map says so: `▢ src/auth: api and web both editing`.

An agent goes by its folder's name, as on its row. The pane updates as edits land and counts the files again every minute while it is open. Today starts at your local midnight. Only edit counts per folder are kept, for the current day.

Haunt counts the files for the plugin, which never reads or even lists one. Hidden folders and the names in `skip` are left out. Files more than five levels down count toward their folder five levels down, and Haunt stops counting a very large repo after 50,000 entries.

## Settings

```toml
[plugins."dev.haunt.code-map"]
skip = ["node_modules", "target", "dist", "build", "vendor"]  # folder names left out
overlaps = true     # name folders two agents both edited, under the map
unverified = false  # also count edits from events without the tab's nonce
```

## Capabilities

| capability | why |
|---|---|
| `events = ["agent.file-edited"]` | each edit as it lands |
| `event_details` | the edited file's path; Haunt asks in red, since paths and commands can hold secrets |
| `fs_list` | Haunt's file count per folder in the agent's repo; no file names or contents |
| `panel` | the pane |
| `timer` | refreshes while the pane is open, and finds your local midnight |

Events without the tab's nonce are ignored unless `unverified` is on.

## Build and test

```sh
cargo build --release --target wasm32-wasip2   # or ./build.sh
cargo test                                     # unit tests
haunt plugin install .
```

`e2e/code_map.rs` runs the built plugin against a real Haunt core: copy it into a Haunt checkout's `cli/tests/` and run `HAUNT_PLUGINS_DIR=<this repo's parent> cargo test -p haunt-cli --features plugins --test code_map`.

Needs Haunt with plugin API 0.5 (`api = "0.5"`).

## Licence

MIT, see `LICENSE`.
