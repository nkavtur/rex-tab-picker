# rex-tab-picker

A `cmd+e` tab switcher for [Rex](https://www.superlogical.com/rex). Fuzzy-find any tab in any session and jump to it, like IntelliJ's Recent Files.

> **Fully vibecoded.** Written end to end with Claude Code, with no hand-written code. It works on my machine (Rex Beta, macOS). Read it before you trust it.

- Most recently used first. The previous tab is pre-selected, so `cmd+e` then `Enter` jumps back.
- Tabs from all sessions, grouped under session headers. Type to filter.
- `Enter` opens, `Esc` or `cmd+e` closes.
- Works without Remote Control.

## Install

Needs Rex on macOS and [`fzf`](https://github.com/junegunn/fzf) at `/usr/local/bin/fzf` or on `PATH`.

```sh
git clone https://github.com/nkavtur/rex-tab-picker
cp rex-tab-picker/{tab_picker.lua,tab-picker.sh,tab-dump.lua,tab-watcher.lua} ~/.config/rex/
echo 'require("tab_picker")' >> ~/.config/rex/init.lua
rex config check && rex config reload
```

## How it works

`cmd+e` opens a floating fzf popup. Rex key bindings handle `Enter`/`Esc` while it is open. A small background script (`tab-watcher.lua`, started automatically) caches other sessions' tabs so the popup opens fast.

## Known limits

- "Previous tab" is only learned through the picker. Rex has no event for other tab switches.
- Assumes the default Rex install path under `~/Library/Application Support`.
- Beta software on both sides. Expect rough edges.
