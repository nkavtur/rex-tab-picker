# rex-tab-picker

A `cmd+e` tab switcher for [Rex](https://www.superlogical.com/rex), in the spirit of IntelliJ's
Recent Files and LazyVim's Telescope. Fuzzy-filter your tabs, move with the arrow keys, press Enter.

- Most recently used first; the current tab is row 1 and the **previous tab is pre-selected**, so
  `cmd+e`, `Enter` jumps back.
- `cmd+e`, `Esc` or `Enter` close the popup (it toggles).
- One rounded frame, Tokyo Night colors.

## Requirements

- Rex (macOS) with `rex.action` / `rex.session` Lua API
- [`fzf`](https://github.com/junegunn/fzf) 0.60+ at `/usr/local/bin/fzf` or on `PATH`

## Install

```sh
cp tab_picker.lua tab-picker.sh ~/.config/rex/
echo 'require("tab_picker")' >> ~/.config/rex/init.lua
rex config check && rex config reload
```

## Known limits

- Only tabs in the current session are listed.
- "Previous tab" is learned when you open or pick from the picker. Rex gives plugins no event for
  switching tabs by other means (`cmd+1`…`9`, `ctrl+tab`, mouse).
- An Enter pressed before the `cmd+e` action finishes (about 100 ms) goes to the terminal.
- The Rex binary path assumes the default `RexServer` install location under `~/Library`.
