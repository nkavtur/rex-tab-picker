# rex-tab-picker

A fast `cmd+e` tab switcher for [Rex](https://www.superlogical.com/rex). Fuzzy-find any tab in any
session, see at a glance which ones have an agent working or waiting for you, and jump to it.

```
╭──────────────────────────────────────────────╮
│  Search tabs                                 │
│  ──────────────────────────────────────────  │
│  work                                        │
│    ● api-server                  ● working   │
│      docs                        ● waiting   │
│      infra                       ● done      │
│                                              │
│  dotfiles                                    │
│      shell                                   │
╰──────────────────────────────────────────────╯
```

## Features

- **All sessions, one list.** Tabs are grouped by session, most recently used first. The previous
  tab is pre-selected, so `cmd+e` then `Enter` jumps straight back.
- **Agent status labels.** Tabs show what the programs in them report over Rex's status protocol
  (OSC 7501, which Claude Code uses): `working`, `waiting` (blocked on a permission, question or
  login), `done` or `error`. Idle tabs show nothing.
- **Instant.** The popup is a resident Rex block that is parked and floated back on demand, so it
  opens in a few milliseconds. Other sessions' tabs come from a background cache.
- **Fuzzy filter.** Matches the tab name, its directory and the session name.
- **No Remote Control needed.**
- **fzf fallback** if the Rust binary isn't installed (needs fzf 0.45+).

## Keys

| Key | Action |
| --- | --- |
| `cmd+e` | Open the picker (press again to close) |
| type | Filter tabs |
| `↑` `↓` | Move the highlight |
| `Enter` | Switch to the highlighted tab |
| `Esc` | Close without switching |

## Install

Needs Rex on macOS (Apple silicon or Intel). Releases ship prebuilt binaries for both, so no Rust
toolchain is needed. The repo is private, so run `gh auth login` first.

```sh
# 1. the UI binary for this Mac
mkdir -p ~/.config/rex/bin
case "$(uname -m)" in arm64) T=aarch64-apple-darwin ;; *) T=x86_64-apple-darwin ;; esac
gh release download --repo nkavtur/rex-tab-picker --pattern "rex-tabs-$T" \
  --output ~/.config/rex/bin/rex-tabs --clobber
chmod +x ~/.config/rex/bin/rex-tabs

# 2. the Lua side
gh repo clone nkavtur/rex-tab-picker /tmp/rex-tab-picker -- --depth 1
cp /tmp/rex-tab-picker/{tab_picker.lua,tab-picker.sh,tab-dump.lua,tab-watcher.lua} ~/.config/rex/
echo 'require("tab_picker")' >> ~/.config/rex/init.lua

# 3. load it
rex config check && rex config reload
```

To build from source instead, run `cargo build --release` and copy `target/release/rex-tabs` to
`~/.config/rex/bin/`.

### Updating

Re-run step 1 (and copy the Lua files again if they changed). `rex-tabs --version` shows what is
installed.

## How it works

`cmd+e` writes the tab list to a file and floats the parked `rex-tabs` block into place
(`session.move_block` + `float_block`). Enter and Esc are Rex key bindings in a `tabpick` mode that
read the highlighted tab from a result file, which is why Remote Control isn't needed.

`tab-watcher.lua` starts automatically and keeps `tabs-cache.json` fresh: every tab of every
session, plus each tab's agent status, updated on Rex events.

| File | Role |
| --- | --- |
| `src/main.rs` | The terminal UI (`rex-tabs`) |
| `tab_picker.lua` | The `cmd+e` action, ordering, popup placement |
| `tab-watcher.lua` | Background cache of all sessions' tabs and statuses |
| `tab-dump.lua` | One-off tab listing, used when the cache is stale |
| `tab-picker.sh` | The fzf fallback UI |

## Troubleshooting

- Popup stuck or stale: `rex do tab_picker_reset` stops the parked popups; the next `cmd+e` starts a
  fresh one. `tab_picker_warm` starts one ahead of time.
- No status labels: they only appear for programs that report status. Check with
  `rex api call -s SESSION com.superlogical.terminal.program_status '{"block_id":"BLOCK"}'`.
- The first `cmd+e` in each session starts the popup (about 0.2 s); later ones are instant.

## Releasing

Bump `version` in `Cargo.toml`, then push a matching tag. GitHub Actions builds both
architectures and publishes the release.

```sh
git tag v0.2.0 && git push origin v0.2.0
```

## Status

Written end to end with Claude Code. It works on my machine (Rex Beta, macOS), and both Rex and this
are beta software, so expect rough edges. Read it before you trust it.
