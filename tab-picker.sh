#!/bin/bash
# Rex tab picker, fzf fallback UI (used when the Rust UI, bin/rex-tabs, is not installed).
# Started by tab_picker.lua.
# usage: tab-picker.sh SESSION_ID LIST_FILE REX_BIN TITLE RESULT_FILE [START_POS]
# fzf only filters and moves the highlight. The highlighted tab is continuously written to
# RESULT_FILE; Enter/Esc are handled by Rex key bindings (tab_picker_commit/cancel).
#
# Works with older fzf versions too: cosmetic options are only used if this fzf knows them,
# and fzf older than 0.45 (no `transform` action) gets a clear message instead of an error.
export _ZO_DOCTOR=0
export PATH="/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:$PATH"
session="$1"; list="$2"; REX="$3"; title="$4"; result="$5"; startpos="${6:-2}"
[ -x "$REX" ] || REX=rex
# keep the tab name unchanged while the picker is open
printf "\033]0;%s\007" "$title"

if ! command -v fzf >/dev/null 2>&1; then
  printf '\n  fzf is not installed.\n  Run: brew install fzf\n'
  sleep 3600; exit 0
fi
if [ -n "$TABPICK_FZF_HELP" ]; then help="$(cat "$TABPICK_FZF_HELP")"; else help="$(fzf --help 2>&1)"; fi
ver="${TABPICK_FZF_VERSION:-$(fzf --version 2>/dev/null | awk '{print $1}')}"
minor="${ver#0.}"; minor="${minor%%.*}"
case "$minor" in ''|*[!0-9]*) minor=0 ;; esac
has() { printf '%s' "$help" | grep -q -e "$1"; }

if [ "$minor" -lt 45 ]; then
  printf '\n  fzf %s is too old for the tab picker (needs 0.45 or newer).\n  Run: brew upgrade fzf\n  (or build the Rust UI: scripts/bootstrap-rex.sh)\n' "$ver"
  sleep 3600; exit 0
fi

write="execute-silent([ -n {4} ] && printf '%s\\n%s\\n%s\\n' {4} {5} {1} > '$result')"
opts=(--ansi --layout=reverse --no-sort --tiebreak=index --cycle
      --delimiter=$'\t' --with-nth=3,6 --nth=1
      --border=rounded --border-label=' Tabs ' --padding=1,2 --margin=0
      --prompt='❯ ' --pointer='▎')
has '--tabstop'           && opts+=(--tabstop=1)
has '--highlight-line'    && opts+=(--highlight-line)
has '--border-label-pos'  && opts+=(--border-label-pos=4)
has '--no-scrollbar'      && opts+=(--no-scrollbar)
has 'inline-right'        && opts+=(--info=inline-right)

colors='bg:-1,fg:#a9b1d6,bg+:#292e42,fg+:#c0caf5:bold,hl:#ff9e64,hl+:#ff9e64:bold,pointer:#7aa2f7,prompt:#bb9af7,info:#565f89,spinner:#7aa2f7,border:#3b4261,label:#7aa2f7:bold'
if has '--gutter';    then opts+=(--gutter=' ');                  colors="$colors,gutter:-1"; fi
if has '--ghost';     then opts+=(--ghost='type to filter…');     colors="$colors,ghost:#565f89"; fi
if has '--separator'; then opts+=(--separator='─');               colors="$colors,separator:#3b4261"; fi
opts+=(--color="$colors")

opts+=(--bind='start:unbind(focus,result)'
       --bind="load:pos($startpos)+rebind(focus,result)+$write"
       --bind="focus:$write"
       --bind="result:$write+execute-silent([ \"\$FZF_MATCH_COUNT\" -eq 0 ] && : > '$result')+transform([ -z {4} ] && echo down)"
       --bind='down:down+transform:[ -z {4} ] && echo down'
       --bind='ctrl-j:down+transform:[ -z {4} ] && echo down'
       --bind='ctrl-n:down+transform:[ -z {4} ] && echo down'
       --bind='up:up+transform:[ -z {4} ] && echo up'
       --bind='ctrl-k:up+transform:[ -z {4} ] && echo up'
       --bind='ctrl-p:up+transform:[ -z {4} ] && echo up')

if [ -n "$TABPICK_DRYRUN" ]; then printf '%s\n' "${opts[@]}"; exit 0; fi

fzf "${opts[@]}" < "$list" > /dev/null
exit 0
