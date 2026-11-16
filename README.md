# rex-tab-picker

A `cmd+e` tab switcher for [Rex](https://www.superlogical.com/rex). Fuzzy-find any tab in any session and jump to it.

> **Fully vibecoded.** Written end to end with Claude Code, with no hand-written code. It works on my machine (Rex Beta, macOS). Read it before you trust it.

- Session → tabs outline, most recent first. The previous tab is pre-selected, so `cmd+e` then `Enter` jumps back.
- Type to filter, arrows to move (the search bar shows the highlighted tab). `Enter` opens, `Esc` or `cmd+e` closes.
- Opens in a few milliseconds: the popup is a resident Rex block that is parked (detached) and floated back each time.
- Works without Remote Control.

## Install

Needs Rex on macOS (Apple silicon or Intel). Releases ship prebuilt binaries, so no Rust is needed
(an `fzf` popup is used as a fallback if the binary is missing). The repo is private: run `gh auth login` first.

```sh
mkdir -p ~/.config/rex/bin
case "$(uname -m)" in arm64) T=aarch64-apple-darwin ;; *) T=x86_64-apple-darwin ;; esac
gh release download --repo nkavtur/rex-tab-picker --pattern "rex-tabs-$T" --output ~/.config/rex/bin/rex-tabs --clobber
chmod +x ~/.config/rex/bin/rex-tabs
gh repo clone nkavtur/rex-tab-picker /tmp/rex-tab-picker -- --depth 1
cp /tmp/rex-tab-picker/{tab_picker.lua,tab-picker.sh,tab-dump.lua,tab-watcher.lua} ~/.config/rex/
echo 'require("tab_picker")' >> ~/.config/rex/init.lua
rex config check && rex config reload
```

Building from source instead: `cargo build --release`, then copy `target/release/rex-tabs` to `~/.config/rex/bin/`.

`rex-tabs --version` prints the installed version. To publish a release, bump `version` in `Cargo.toml`,
then push a tag: `git tag v0.2.0 && git push origin v0.2.0`. GitHub Actions builds both architectures.

## How it works

`cmd+e` writes the tab list to a file and floats the parked `rex-tabs` block into place (`session.move_block` + `float_block`). Enter/Esc are Rex key bindings in a `tabpick` mode that read the highlighted tab from a result file, which is why Remote Control isn't needed. `tab-watcher.lua` (started automatically) caches other sessions' tabs.

Recovery: `rex do tab_picker_reset` stops the parked popups; `tab_picker_warm` starts one ahead of time.

## Known limits

- "Previous tab" is only learned through the picker.
- The first `cmd+e` in each session starts the resident popup (about 0.2 s), later ones are instant.
- Beta software on both sides. Expect rough edges.
