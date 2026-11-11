#!/bin/bash
# Rex tab picker UI (IntelliJ "Recent Files" style). Started by tab_picker.lua.
# usage: tab-picker.sh SESSION_ID LIST_FILE REX_BIN TITLE RESULT_FILE
# fzf only filters and previews. The highlighted tab's number is continuously written
# to RESULT_FILE; Enter/Esc are handled by Rex key bindings (tab_picker_commit/cancel).
export _ZO_DOCTOR=0
export PATH="/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:$PATH"
session="$1"; list="$2"; REX="$3"; title="$4"; result="$5"; startpos="${6:-2}"
[ -x "$REX" ] || REX=rex
# keep the tab name unchanged while the picker is open
printf "\033]0;%s\007" "$title"

fzf --ansi --layout=reverse --no-sort --tiebreak=index --cycle \
  --delimiter=$'\t' --tabstop=1 --with-nth=3,6 --nth=1 \
  --highlight-line \
  --border=rounded --border-label=' Tabs ' --border-label-pos=4 \
  --padding=1,2 --margin=0 \
  --prompt='❯ ' --pointer='▎' --gutter=' ' --info=inline-right --no-scrollbar \
  --ghost='type to filter…' --separator='─' \
  --color='bg:-1,fg:#a9b1d6,bg+:#292e42,fg+:#c0caf5:bold,hl:#ff9e64,hl+:#ff9e64:bold' \
  --color='pointer:#7aa2f7,prompt:#bb9af7,info:#565f89,spinner:#7aa2f7,gutter:-1,ghost:#565f89' \
  --color='border:#3b4261,label:#7aa2f7:bold,separator:#3b4261' \
  --bind='start:unbind(focus,result)' \
  --bind="load:pos($startpos)+rebind(focus,result)+execute-silent([ -n {4} ] && printf '%s\\n%s\\n%s\\n' {4} {5} {1} > '$result')" \
  --bind="focus:execute-silent([ -n {4} ] && printf '%s\\n%s\\n%s\\n' {4} {5} {1} > '$result')" \
  --bind="result:execute-silent([ -n {4} ] && printf '%s\\n%s\\n%s\\n' {4} {5} {1} > '$result')+execute-silent([ \"\$FZF_MATCH_COUNT\" -eq 0 ] && : > '$result')+transform([ -z {4} ] && echo down)" \
  --bind='down:down+transform:[ -z {4} ] && echo down' --bind='ctrl-j:down+transform:[ -z {4} ] && echo down' --bind='ctrl-n:down+transform:[ -z {4} ] && echo down' \
  --bind='up:up+transform:[ -z {4} ] && echo up' --bind='ctrl-k:up+transform:[ -z {4} ] && echo up' --bind='ctrl-p:up+transform:[ -z {4} ] && echo up' \
  < "$list" > /dev/null
exit 0
