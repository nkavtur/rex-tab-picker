-- tab_picker.lua: cmd+e popup to fuzzy-pick a tab (window) from ALL sessions.
-- Loaded from init.lua with require("tab_picker"). Press cmd+e again to close it.
local home     = os.getenv("HOME")
local dir      = home .. "/.config/rex/"
local script   = dir .. "tab-picker.sh"          -- fzf fallback UI
local uibin    = dir .. "bin/rex-tabs"           -- Rust UI (preferred when present)
local dumper   = dir .. "tab-dump.lua"
local state    = dir .. ".tab-picker.state"          -- layer id + result file of the open picker
local mru      = dir .. "tab-mru.txt"                 -- optional, written by a tracker
local rexbin   = home .. "/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex"
local terminal = "com.superlogical.terminal"
do  -- if Rex lives somewhere else, fall back to `rex` on PATH
  local f = io.open(rexbin, "r")
  if f then f:close() else rexbin = "rex" end
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end
local function clean(s) return (tostring(s or ""):gsub("%c", " ")) end   -- no tabs/newlines/escapes in rows
local function base(p) return p and (p:match("([^/]+)/*$") or p) or "" end

local function layer_exists(view, id)
  for _, w in ipairs(view.windows or {}) do
    for _, l in ipairs(w.layers or {}) do
      if l.layer_id == id then return true end
    end
  end
  return false
end

-- MRU file: one "window_id<TAB>unix_time" per line, most recent first
local mru_ts = {}
local function load_mru()
  local t = {}
  mru_ts = {}
  local f = io.open(mru, "r")
  if f then
    for line in f:lines() do
      local id, ts = line:match("^([^\t]+)\t?(%d*)")
      id = trim(id or "")
      if id ~= "" then t[#t + 1] = id; mru_ts[id] = tonumber(ts) end
    end
    f:close()
  end
  return t
end

local function save_mru(t, stamps)
  local f = io.open(mru, "w")
  if f then
    for _, id in ipairs(t) do f:write(id, "\t", tostring(stamps[id] or os.time()), "\n") end
    f:close()
  end
end

-- move id to the front of the most-recently-used list
local function touch_mru(id)
  local t, out = load_mru(), { id }
  for _, x in ipairs(t) do if x ~= id then out[#out + 1] = x end end
  while #out > 50 do out[#out] = nil end
  mru_ts[id] = os.time()
  save_mru(out, mru_ts)
end

-- Tabs of other sessions come from a cache kept fresh by tab-watcher.lua, so opening the
-- picker doesn't have to ask every session (about 85 ms). If the cache is missing or stale,
-- fall back to asking them directly and (re)start the watcher.
local cachefile = dir .. "tabs-cache.json"
local watcher   = dir .. "tab-watcher.lua"
local lockfile  = dir .. ".tab-watcher.lock"

local function read_cache()
  local f = io.open(cachefile, "r"); if not f then return nil end
  local raw = f:read("*a"); f:close()
  local ok, d = pcall(rex.json.decode, raw)
  if not ok or type(d) ~= "table" or not d.ts or (os.time() - d.ts) > 12 then return nil end
  local byid = {}
  for _, s in ipairs(d.sessions or {}) do byid[s.id] = s end
  return byid
end

local function ensure_watcher()
  local lf = io.open(lockfile, "r")
  if lf then
    local t = tonumber(lf:read("*l")); lf:close()
    if t and (os.time() - t) < 6 then return end      -- one was just started
  end
  local w = io.open(lockfile, "w"); if w then w:write(tostring(os.time()), "\n"); w:close() end
  rex.exec.start({ rexbin, "do", "--all", watcher }, { detach = true })
end

local residents = dir .. ".tab-picker.resident"   -- session id -> resident popup block (kept running)
local opening = dir .. ".tab-picker.opening"   -- re-entry guard while the popup is being built

local function read_registry()
  local t = {}
  local f = io.open(residents, "r")
  if f then
    for line in f:lines() do
      local sid, block, lst, res = line:match("^([^\t]*)\t([^\t]*)\t([^\t]*)\t([^\t]*)$")
      if sid then t[sid] = { block = block, list = lst, result = res } end
    end
    f:close()
  end
  return t
end

local function write_registry(t)
  local f = io.open(residents, "w")
  if f then
    for sid, r in pairs(t) do f:write(sid, "\t", r.block, "\t", r.list, "\t", r.result, "\n") end
    f:close()
  end
end

local function read_state()
  local f = io.open(state, "r"); if not f then return nil end
  local layer = f:read("*l"); local result = f:read("*l"); local list = f:read("*l"); local sid = f:read("*l")
  local uistate = f:read("*l"); local block = f:read("*l")
  f:close()
  if layer and result then
    return { layer = layer, result = result, list = list, sid = sid, uistate = uistate, block = block }
  end
end

local function drop_files(st)
  if st.block and st.block ~= "" then
    os.remove(state)       -- the resident popup keeps its own list/result files
    return
  end
  os.remove(st.result); os.remove(st.result .. ".tmp"); if st.list then os.remove(st.list) end
  if st.uistate and st.uistate ~= "" then os.remove(st.uistate); os.remove(st.uistate .. ".tmp") end
  os.remove(state)
end

-- is the popup of this state still open?
local function popup_alive(st)
  local v = rex.session.view{ session_id = st.sid }
  return v ~= nil and layer_exists(v, st.layer)
end

-- hides the popup and returns the pick { n = tab number, sid = session id, wid = window id }
-- (or nil). A resident popup is only detached (its process keeps running, ready for the next
-- cmd+e); the old fzf popup is closed.
local function finish(pick)
  local st = read_state()
  if not st then return nil end
  local sel
  if pick then
    local rf = io.open(st.result, "r")
    if rf then
      local n, sid, wid = rf:read("*l"), rf:read("*l"), rf:read("*l")
      rf:close()
      n = tonumber(trim(n))
      if n and trim(sid) ~= "" then sel = { n = n, sid = trim(sid), wid = trim(wid) } end
    end
  end
  if st.block and st.block ~= "" then
    local ok = rex.session.detach_block{ session_id = st.sid, block_id = st.block }
    if not ok then rex.session.close_layer{ session_id = st.sid, layer_id = st.layer } end
  else
    rex.session.close_layer{ session_id = st.sid, layer_id = st.layer }
  end
  drop_files(st)
  return sel
end

-- Enter/Esc are bound while the popup is open. If the popup is already gone (fzf quit with
-- ctrl-c, the pane was closed, ...), don't swallow the key: clean up, leave the mode, and
-- send the key to the pane the user is in.
local function popup_gone(ctx, key)
  local st = read_state()
  if st and popup_alive(st) then return false end
  if st then drop_files(st) end
  rex.client.queue("client.mode.exit_all")
  if ctx.block_id then rex.block.call(terminal, "write", { block_id = ctx.block_id, data = key }) end
  return true
end

rex.action{
  name  = "tab_picker",
  title = "Switch Tab…",
  description = "Fuzzy tab switcher with live preview. Run again to close it.",
  keywords = "tab window switch recent picker",
  args = { dry = "boolean?" },
  run = function(ctx, args)
    -- a second cmd+e while the first is still building the popup would open a second popup
    local lf = io.open(opening, "r")
    if lf then
      local t = tonumber(lf:read("*l")); lf:close()
      if t and (os.time() - t) < 4 then return end
    end

    local view = rex.session.view{}
    if not view then return end

    -- toggle: a picker is already open -> close it
    local st = read_state()
    if st then
      if popup_alive(st) then
        finish(false)
        rex.client.queue("client.mode.exit_all")
        return
      end
      drop_files(st)   -- stale
    end

    local lk = io.open(opening, "w"); if lk then lk:write(tostring(os.time()), "\n"); lk:close() end

    -- the tab the user is actually looking at: the window holding the focused block
    local curid = view.active_window_id
    if ctx.block_id then
      for _, w in ipairs(view.windows or {}) do
        for _, l in ipairs(w.layers or {}) do
          for _, b in ipairs(l.blocks or {}) do
            if b.block_id == ctx.block_id then curid = w.window_id end
          end
        end
      end
    end

    -- every tab of every session
    local sessions = (rex.call("session.list") or {}).sessions or {}
    local wins, byid = {}, {}
    local cursid = view.session_id or ctx.session_id

    -- Other sessions are read by separate `rex do` processes (own connection; never
    -- attach/detach from here, that would unhook the app's stream). Start them all at once
    -- so the cost is one process start, not one per session.
    local cached = read_cache()
    if not cached then ensure_watcher() end
    local jobs = {}
    for _, sess in ipairs(sessions) do
      if sess.session_id ~= cursid and not (cached and cached[sess.session_id]) then
        local h = rex.exec.start({ rexbin, "do", "-s", sess.session_id, dumper }, { timeout = 5 })
        if h then jobs[sess.session_id] = h end
      end
    end

    for _, sess in ipairs(sessions) do
      if sess.session_id == cursid then
        for i, w in ipairs(view.windows or {}) do
          local e = { id = w.window_id, block = w.focused_block_id, n = i, label = w.label,
                      sid = sess.session_id, slabel = clean(sess.label) }
          if not e.block then
            local b = w.layers and w.layers[1] and w.layers[1].blocks and w.layers[1].blocks[1]
            e.block = b and b.block_id
          end
          wins[#wins + 1] = e; byid[e.id] = e
        end
      elseif cached and cached[sess.session_id] then
        for _, r in ipairs(cached[sess.session_id].rows or {}) do
          local e = { id = r.id, block = r.block, n = r.n or 0, label = r.label,
                      title = clean(trim(r.title)), pwd = clean(r.pwd),
                      sid = sess.session_id, slabel = clean(sess.label) }
          wins[#wins + 1] = e; byid[e.id] = e
        end
      elseif jobs[sess.session_id] then
        local r = jobs[sess.session_id]:wait(6)
        if r and r.ok then
          local okj, str = pcall(rex.json.decode, trim(r.stdout))
          if okj and type(str) == "string" then
            for row in (str .. "\n"):gmatch("([^\n]*)\n") do
              local f = {}
              for col in (row .. "\t"):gmatch("([^\t]*)\t") do f[#f + 1] = col end
              if f[1] and f[1] ~= "" then
                local e = { id = f[1], block = f[2] ~= "" and f[2] or nil, n = tonumber(f[3]) or 0,
                            label = f[4], title = clean(trim(f[5])), pwd = clean(f[6]),
                            sid = sess.session_id, slabel = clean(sess.label) }
                wins[#wins + 1] = e; byid[e.id] = e
              end
            end
          end
        end
      end
    end
    local multi = #sessions > 1
    
    -- recency: the tab you're on goes to the front, so the previous tab becomes row 2
    if curid then touch_mru(curid) end
    local ordered, seen = {}, {}
    for _, id in ipairs(load_mru()) do
      if byid[id] and not seen[id] then ordered[#ordered + 1] = byid[id]; seen[id] = true end
    end
    for _, e in ipairs(wins) do if not seen[e.id] then ordered[#ordered + 1] = e end end

    -- Style: group by session. Sessions ordered by their most recent tab; tabs inside a
    -- session keep recency order. Only the first tab of a group carries the session header.
    local gorder, groups = {}, {}
    for _, e in ipairs(ordered) do
      if not groups[e.sid] then groups[e.sid] = {}; gorder[#gorder + 1] = e.sid end
      table.insert(groups[e.sid], e)
    end
    local flat = {}
    for _, sid in ipairs(gorder) do
      for i, e in ipairs(groups[sid]) do e.first = (i == 1); flat[#flat + 1] = e end
    end
    local pick = ordered[2] or ordered[1]       -- previous tab (ordered always has the current one)
    local pickidx = 1
    if not pick then os.remove(opening); return end

    local cur = byid[curid]
    local curtitle = ""
    if cur and cur.block then
      local t = rex.block.call(terminal, "title", { block_id = cur.block })
      curtitle = clean(trim(t and t.title))
    end

    local lines = {}
    local nheaders = 0
    for _, e in ipairs(flat) do
      local title, d = "", ""
      if e.title ~= nil then
        title = e.title; d = base(e.pwd)            -- other session: already fetched
      elseif e.block then
        local t = rex.block.call(terminal, "title", { block_id = e.block })
        local p = rex.block.call(terminal, "pwd",   { block_id = e.block })
        title = clean(trim(t and t.title)); d = clean(base(p and p.pwd))
      end
      local custom = e.label and not e.label:match("^Window %x+$")
      local name = clean(custom and e.label or (title ~= "" and title or d ~= "" and d or "shell"))
      local is_cur = (e.id == curid)
      e.dname = name; e.ddir = d
      local mark = is_cur and "\27[38;2;158;206;106m●\27[0m" or " "          -- green dot = current tab
      local num  = string.format("\27[38;2;86;95;137m%d\27[0m", e.n)           -- dim number
      local dirs = (d ~= "" and d ~= name) and ("  \27[38;2;86;95;137m" .. d .. "\27[0m") or ""
      local shown = string.format("  %s  %s  \27[1m%s\27[0m%s", mark, num, name, dirs)
      if multi then
        shown = "  " .. shown
        if e.first then
          -- session header: its own row (not selectable, skipped by the arrow keys). The text is in
          -- field 6, which fzf does not search, so headers vanish while you filter.
          nheaders = nheaders + 1
          lines[#lines + 1] = table.concat({ "", "", "", "", "", "  \27[1;38;2;122;162;247m" .. e.slabel .. "\27[0m" }, "\t")
        end
      end
      lines[#lines + 1] = table.concat({ e.id, e.block or "", shown, tostring(e.n), e.sid, "" }, "\t")
      if e == pick then pickidx = #lines end
    end

    local use_rust = false
    do
      local bf = io.open(uibin, "r")
      if bf then bf:close(); use_rust = true end
    end

    -- the previous tab is preselected, so a very quick Enter already goes to the right place
    local function result_text(p) return table.concat({ tostring(p.n), p.sid, p.id }, "\n") .. "\n" end

    -- size the popup to its content
    local rows, cols = 40, 160
    if cur and cur.block then
      local sz = rex.block.call(terminal, "size", { block_id = cur.block })
      rows = (sz and tonumber(sz.rows)) or rows
      cols = (sz and tonumber(sz.columns)) or cols
    end
    local nsess = 0
    do
      local seen_s = {}
      for _, e in ipairs(ordered) do if not seen_s[e.sid] then seen_s[e.sid] = true; nsess = nsess + 1 end end
    end
    local y = 0.10   -- upper part of the window
    local w, h

    local function atomic_write(path, body)
      local tmp = path .. ".tmp"
      local f = io.open(tmp, "w")
      if f then f:write(body); f:close(); os.rename(tmp, path) end
    end

    local sid_now = view.session_id or ctx.session_id

    ------------------------------------------------------------------------------------------------
    -- Rust UI: a resident popup. Its block keeps running while hidden (detached); showing it is
    -- move_block + float_block, a few milliseconds, with no terminal to start.
    ------------------------------------------------------------------------------------------------
    if use_rust then
      local items = #ordered + nsess
      -- width follows the longest name, not the window: inset 3 + dot 2 + inset 3 + frame
      local longest = 0
      for _, e in ipairs(ordered) do
        longest = math.max(longest, #(e.dname or ""), #(e.slabel or ""))
      end
      w = math.max(0.2, math.min(0.6, math.max(46, math.min(72, longest + 12)) / cols))
      -- rows: 1 pad + filter + separator + 1 blank + items + 1 pad, plus ~3 rows of Rex pane chrome
      -- (one blank row between sessions; the bottom gap is one row shorter than before)
      h = math.max(0.1, math.min(0.85, (math.max(items + nsess - 1, 4) + 7) / rows))
      local rowsj = {}
      for i, e in ipairs(ordered) do
        rowsj[#rowsj + 1] = { wid = e.id, sid = e.sid, n = e.n, session = e.slabel,
                              name = e.dname or "shell", dir = e.ddir or "", current = (e.id == curid) }
        if e == pick then pickidx = i - 1 end     -- 0-based for the Rust UI
      end
      local doc = rex.json.encode({ title = curtitle, start = pickidx, rows = rowsj })

      if args and args.dry then
        os.remove(opening)
        return { ui = "rust", doc = doc, bounds = { (1 - w) / 2, y, w, h } }
      end

      local reg = read_registry()
      local res = reg[sid_now]
      local created = false
      if not res then
        res = { block = "", list = os.tmpname(), result = os.tmpname() }
      end
      atomic_write(res.list, doc)                       -- fresh data before the popup appears
      atomic_write(res.result, result_text(pick))
      if res.block == "" then
        local nb = rex.session.new_block{ session_id = sid_now, flavor = "com.superlogical.terminal.shell",
          options = { command = { uibin, "resident", res.list, res.result }, shell = "none" } }
        if not nb then os.remove(opening); return end
        res.block = nb.block_id; reg[sid_now] = res; write_registry(reg); created = true
      end

      local anchor = ctx.block_id or (cur and cur.block)
      local function place()
        return rex.session.move_block{ session_id = sid_now, block_id = res.block, anchor_block_id = anchor,
                                       direction = "vertical", ratio = 0.995, position = "after" }
      end
      local mv = place()
      if not mv and not created then
        -- the resident process is gone (exited, or Rex restarted): start a new one
        local nb = rex.session.new_block{ session_id = sid_now, flavor = "com.superlogical.terminal.shell",
          options = { command = { uibin, "resident", res.list, res.result }, shell = "none" } }
        if not nb then os.remove(opening); return end
        res.block = nb.block_id; reg[sid_now] = res; write_registry(reg)
        mv = place()
      end
      if not mv then os.remove(opening); return end
      local fl = rex.session.float_block{ session_id = sid_now, block_id = res.block,
                                          bounds = { (1 - w) / 2, y, w, h } }
      if not fl or not fl.layer_id then
        rex.session.detach_block{ session_id = sid_now, block_id = res.block }
        os.remove(opening); return
      end
      rex.session.focus_block{ session_id = sid_now, block_id = res.block }

      local sf = io.open(state, "w")
      sf:write(fl.layer_id, "\n", res.result, "\n", res.list, "\n", sid_now, "\n", "", "\n", res.block, "\n")
      sf:close()
      os.remove(opening)
      rex.client.queue("client.mode.enter", { name = "tabpick" })
      return
    end

    ------------------------------------------------------------------------------------------------
    -- fzf fallback (no Rust binary installed): a fresh popup every time
    ------------------------------------------------------------------------------------------------
    local list = os.tmpname()
    local out = io.open(list, "w"); out:write(table.concat(lines, "\n"), "\n"); out:close()
    local result = os.tmpname()
    atomic_write(result, result_text(pick))
    h = math.max(0.1, math.min(0.85, (math.max(#lines, 14) + 6) / rows))
    w = 0.6
    local command = { "/bin/bash", script, sid_now, list, rexbin, curtitle, result, tostring(pickidx) }
    if args and args.dry then
      os.remove(opening)
      return { ui = "fzf", list = list, result = result, command = command, bounds = { (1 - w) / 2, y, w, h } }
    end
    local r = rex.session.new_layer{
      window_id = curid,
      bounds = { (1 - w) / 2, y, w, h },
      focus = true,
      layout = rex.layout.block{
        flavor = "com.superlogical.terminal.shell",
        options = { command = command, shell = "none" },
      },
    }
    if not r or not r.layer_id then os.remove(list); os.remove(result); os.remove(opening); return end
    local sf = io.open(state, "w")
    sf:write(r.layer_id, "\n", result, "\n", list, "\n", sid_now, "\n", "", "\n", "", "\n")
    sf:close()
    os.remove(opening)
    rex.client.queue("client.mode.enter", { name = "tabpick" })
  end,
}

-- Pre-start the resident popup of the current session without showing it, so even the first
-- cmd+e there is instant.
rex.action{
  name  = "tab_picker_warm",
  title = "Tab Picker: Warm Up",
  run = function(ctx)
    local bf = io.open(uibin, "r")
    if not bf then return end
    bf:close()
    local sid = ctx.session_id
    local reg = read_registry()
    if reg[sid] and reg[sid].block ~= "" then return end
    local res = { block = "", list = os.tmpname(), result = os.tmpname() }
    local f = io.open(res.list, "w"); if f then f:write('{"rows":[]}'); f:close() end
    local nb = rex.session.new_block{ session_id = sid, flavor = "com.superlogical.terminal.shell",
      options = { command = { uibin, "resident", res.list, res.result }, shell = "none" } }
    if nb then res.block = nb.block_id; reg[sid] = res; write_registry(reg) end
  end,
}

-- Recovery: stop every resident popup process (the next cmd+e starts a fresh one).
rex.action{
  name  = "tab_picker_reset",
  title = "Tab Picker: Reset",
  run = function(ctx)
    local reg = read_registry()
    for sid, r in pairs(reg) do
      rex.call("block.close", { session_id = sid, block_id = r.block })
    end
    os.remove(residents)
  end,
}

rex.action{
  name  = "tab_picker_commit",
  title = "Tab Picker: Open Selected Tab",
  run = function(ctx)
    if popup_gone(ctx, "\r") then return end
    local sel = finish(true)
    if sel then touch_mru(sel.wid) end
    rex.client.queue("client.mode.exit_all")
    if not sel then return end
    if sel.sid ~= ctx.session_id then
      -- Another session: session.select takes the window too, so one client action does the
      -- whole switch. (No tab.goto, which would run against the old session first, and no
      -- extra rex process, which cost ~100 ms.)
      rex.client.queue("session.select", { session_id = sel.sid, window_id = sel.wid })
    else
      -- Same session: look the position up again, tabs may have closed since the popup opened.
      local v = rex.session.view{}
      local idx
      for i, w in ipairs(v and v.windows or {}) do if w.window_id == sel.wid then idx = i end end
      if idx then rex.client.queue("client.tab.goto", { index = idx }) end
    end
  end,
}

rex.action{
  name  = "tab_picker_cancel",
  title = "Tab Picker: Cancel",
  run = function(ctx)
    if popup_gone(ctx, "\27") then return end
    finish(false)
    rex.client.queue("client.mode.exit_all")
  end,
}

-- Non-exclusive: typing, arrows etc. still reach the picker; only these keys are intercepted.
rex.mode("tabpick", { exclusive = false })
rex.bind("cmd+e", "tab_picker")
rex.bind("tabpick/enter",  "tab_picker_commit")
rex.bind("tabpick/esc",    "tab_picker_cancel")
rex.bind("tabpick/cmd+e",  "tab_picker_cancel")
