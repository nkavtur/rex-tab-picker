-- tab_picker.lua: cmd+e popup to fuzzy-pick a tab (window) in the current session.
-- Loaded from init.lua with require("tab_picker"). Press cmd+e again to close it.
local home     = os.getenv("HOME")
local dir      = home .. "/.config/rex/"
local script   = dir .. "tab-picker.sh"
local state    = dir .. ".tab-picker.state"          -- layer id + result file of the open picker
local mru      = dir .. "tab-mru.txt"                 -- optional, written by a tracker
local rexbin   = home .. "/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex"
local terminal = "com.superlogical.terminal"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end
local function base(p) return p and (p:match("([^/]+)/*$") or p) or "" end

local function layer_exists(view, id)
  for _, w in ipairs(view.windows) do
    for _, l in ipairs(w.layers or {}) do
      if l.layer_id == id then return true end
    end
  end
  return false
end

local function load_mru()
  local t = {}
  local f = io.open(mru, "r")
  if f then for line in f:lines() do line = trim(line); if line ~= "" then t[#t + 1] = line end end; f:close() end
  return t
end

local function save_mru(t)
  local f = io.open(mru, "w")
  if f then f:write(table.concat(t, "\n"), "\n"); f:close() end
end

-- move id to the front of the most-recently-used list
local function touch_mru(id)
  local t, out = load_mru(), { id }
  for _, x in ipairs(t) do if x ~= id then out[#out + 1] = x end end
  while #out > 50 do out[#out] = nil end
  save_mru(out)
end

local function read_state()
  local f = io.open(state, "r"); if not f then return nil end
  local layer = f:read("*l"); local result = f:read("*l"); local list = f:read("*l"); f:close()
  if layer and result then return { layer = layer, result = result, list = list } end
end

-- closes the popup and cleans up; returns the picked tab number (or nil)
local function finish(pick)
  local st = read_state()
  if not st then return nil end
  local n
  if pick then
    local rf = io.open(st.result, "r")
    if rf then n = tonumber(trim(rf:read("*a"))); rf:close() end
  end
  rex.session.close_layer{ layer_id = st.layer }
  os.remove(st.result); os.remove(st.result .. ".tmp"); if st.list then os.remove(st.list) end
  os.remove(state)
  return n
end

rex.action{
  name  = "tab_picker",
  title = "Switch Tab…",
  description = "Fuzzy tab switcher with live preview. Run again to close it.",
  keywords = "tab window switch recent picker",
  run = function(ctx)
    local view = rex.session.view{}
    if not view then return end

    -- toggle: a picker is already open -> close it
    local st = read_state()
    if st then
      if layer_exists(view, st.layer) then
        finish(false)
        rex.client.queue("client.mode.exit_all")
        return
      end
      os.remove(state)   -- stale
    end

    -- the tab the user is actually looking at: the window holding the focused block
    local curid = view.active_window_id
    if ctx.block_id then
      for _, w in ipairs(view.windows) do
        for _, l in ipairs(w.layers or {}) do
          for _, b in ipairs(l.blocks or {}) do
            if b.block_id == ctx.block_id then curid = w.window_id end
          end
        end
      end
    end

    local wins, byid = {}, {}
    for i, w in ipairs(view.windows) do
      local e = { id = w.window_id, block = w.focused_block_id, n = i, label = w.label }
      if not e.block then
        local b = w.layers and w.layers[1] and w.layers[1].blocks and w.layers[1].blocks[1]
        e.block = b and b.block_id
      end
      wins[#wins + 1] = e; byid[e.id] = e
    end

    -- recency: the tab you're on goes to the front, so the previous tab becomes row 2
    touch_mru(curid)
    local ordered, seen = {}, {}
    for _, id in ipairs(load_mru()) do
      if byid[id] and not seen[id] then ordered[#ordered + 1] = byid[id]; seen[id] = true end
    end
    for _, e in ipairs(wins) do if not seen[e.id] then ordered[#ordered + 1] = e end end

    local cur = byid[curid]
    if cur and cur.block then
      local t = rex.block.call(terminal, "title", { block_id = cur.block })
      curtitle = trim(t and t.title)
    end

    local lines = {}
    for _, e in ipairs(ordered) do
      local title, d = "", ""
      if e.block then
        local t = rex.block.call(terminal, "title", { block_id = e.block })
        local p = rex.block.call(terminal, "pwd",   { block_id = e.block })
        title = trim(t and t.title); d = base(p and p.pwd)
      end
      local custom = e.label and not e.label:match("^Window %x+$")
      local name = custom and e.label or (title ~= "" and title or d ~= "" and d or "shell")
      local is_cur = (e.id == curid)
      local mark = is_cur and "\27[38;2;158;206;106m●\27[0m" or " "          -- green dot = current tab
      local num  = string.format("\27[38;2;86;95;137m%d\27[0m", e.n)           -- dim number
      local dirs = (d ~= "" and d ~= name) and ("  \27[38;2;86;95;137m" .. d .. "\27[0m") or ""
      local shown = string.format(" %s  %s  \27[1m%s\27[0m%s", mark, num, name, dirs)
      lines[#lines + 1] = table.concat({ e.id, e.block or "", shown, tostring(e.n) }, "\t")
    end

    local list = os.tmpname()
    local out = io.open(list, "w"); out:write(table.concat(lines, "\n"), "\n"); out:close()
    -- Pre-select the previous tab right away, so a very quick Enter (before the popup has
    -- drawn) already goes to the right place. fzf only overwrites this after the user moves.
    local result = os.tmpname()
    local pick = ordered[2] or ordered[1]
    local rfile = io.open(result, "w"); if rfile then rfile:write(tostring(pick.n)); rfile:close() end

    -- size the popup to its content: borders, search box, footer + one row per tab
    local rows = 40
    if cur and cur.block then
      local sz = rex.block.call(terminal, "size", { block_id = cur.block })
      rows = (sz and tonumber(sz.rows)) or rows
    end
    local h = math.max(0.2, math.min(0.85, (math.max(#lines, 8) + 8) / rows))
    local w = 0.4
    local y = 0.10   -- upper part of the window, like Telescope

    local r = rex.session.new_layer{
      window_id = curid,
      bounds = { (1 - w) / 2, y, w, h },
      focus = true,
      layout = rex.layout.block{
        flavor = "com.superlogical.terminal.shell",
        options = {
          command = { "/bin/bash", script, view.session_id or ctx.session_id, list, rexbin, curtitle, result },
          shell = "none",
        },
      },
    }
    if not r or not r.layer_id then os.remove(list); return end

    local sf = io.open(state, "w"); sf:write(r.layer_id, "\n", result, "\n", list, "\n"); sf:close()
    -- while the popup is open, Enter / Esc / cmd+e are handled by the tabpick key mode
    rex.client.queue("client.mode.enter", { name = "tabpick" })
  end,
}

rex.action{
  name  = "tab_picker_commit",
  title = "Tab Picker: Open Selected Tab",
  run = function(ctx)
    local n = finish(true)
    if n then
      local v = rex.session.view{}
      local w = v and v.windows[n]
      if w then touch_mru(w.window_id) end
    end
    rex.client.queue("client.mode.exit_all")
    if n then rex.client.queue("client.tab.goto", { index = n }) end
  end,
}

rex.action{
  name  = "tab_picker_cancel",
  title = "Tab Picker: Cancel",
  run = function(ctx)
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
