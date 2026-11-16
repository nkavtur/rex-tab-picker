-- tab-watcher.lua: keeps ~/.config/rex/tabs-cache.json up to date with every tab of every
-- session, so the cmd+e picker can open without asking each session (which cost ~85 ms).
-- Started on demand by tab_picker.lua as `rex do --all tab-watcher.lua`; one instance only.
local home     = os.getenv("HOME")
local cache    = home .. "/.config/rex/tabs-cache.json"
local terminal = "com.superlogical.terminal"
-- unique id for this watcher (two started in the same second must not look alike)
local me
do
  local f = io.open("/dev/urandom", "rb")
  local raw = f and f:read(8) or ""
  if f then f:close() end
  me = raw:gsub(".", function(c) return string.format("%02x", c:byte()) end)
  if me == "" then me = tostring(os.time()) .. "-" .. tostring(os.clock()) end
end
local attached = {}

-- What the programs in a tab report over OSC 7501 (Claude Code does): the most urgent state
-- of any block in the window. Returns state ("blocked" | "error" | "working" | "done") and a
-- short label, or nothing when everything is idle.
local urgency = { blocked = 4, error = 3, working = 2, done = 1 }
local blocked_text = { permission = "permission", question = "question", auth = "auth" }
local function window_status(w, sid)
  local best, bestrank, kind = nil, 0, nil
  for _, l in ipairs(w.layers or {}) do
    for _, b in ipairs(l.blocks or {}) do
      local r = rex.block.call(terminal, "program_status", { block_id = b.block_id, session_id = sid })
      for _, rec in ipairs(r and r.records or {}) do
        local rk = urgency[rec.state] or 0
        if rk > bestrank then best, bestrank, kind = rec.state, rk, rec.kind end
      end
    end
  end
  if not best then return nil end
  local text = best == "blocked" and (blocked_text[kind or ""] or "waiting") or best
  return best, text
end

local function attach(sid)
  if not attached[sid] then
    local _, err = rex.session.attach{ session_id = sid }
    if err == nil then attached[sid] = true end
  end
  return attached[sid]
end

local failures = 0
local function snapshot()
  local list = rex.call("session.list")
  if not list then
    -- the server went away (restart?): give up instead of lingering as an orphan
    failures = failures + 1
    if failures >= 3 then rex.stop(1) end
    return
  end
  failures = 0
  local out = { owner = me, ts = os.time(), sessions = {} }
  for _, s in ipairs(list.sessions or {}) do
    local rows = {}
    if attach(s.session_id) then
      local v = rex.session.view{ session_id = s.session_id }
      for i, w in ipairs(v and v.windows or {}) do
        local block = w.focused_block_id
        if not block then
          local b = w.layers and w.layers[1] and w.layers[1].blocks and w.layers[1].blocks[1]
          block = b and b.block_id
        end
        local title, pwd = "", ""
        if block then
          local t = rex.block.call(terminal, "title", { block_id = block, session_id = s.session_id })
          local p = rex.block.call(terminal, "pwd",   { block_id = block, session_id = s.session_id })
          title = t and t.title or ""
          pwd   = p and p.pwd or ""
        end
        local status, status_text = window_status(w, s.session_id)
        rows[#rows + 1] = { id = w.window_id, block = block, n = i, label = w.label,
                            title = title, pwd = pwd, status = status, status_text = status_text }
      end
    end
    out.sessions[#out.sessions + 1] = { id = s.session_id, label = s.label, rows = rows }
  end
  local data = rex.json.encode(out)
  if type(data) ~= "string" then return end
  local tmp = cache .. "." .. me .. ".tmp"        -- per-watcher temp file
  local f = io.open(tmp, "w")
  if f then f:write(data); f:close(); os.rename(tmp, cache) end
end

-- throttle: at most one snapshot per second, however noisy the events are
local pending = false
local function schedule()
  if pending then return end
  pending = true
  rex.after(1.0, function() pending = false; snapshot() end)
end

for _, ev in ipairs({ "session_view_changed", "window_created", "window_closed", "window_label_changed",
                      "session_created", "session_destroyed", "session_label_changed",
                      "terminal.title_changed", "terminal.pwd_changed",
                      "terminal.program_status_changed", "terminal.program_status_removed" }) do
  rex.on(ev, function() schedule() end)
end

snapshot()

-- heartbeat; also quit if another watcher has taken over the cache
rex.every(5, function()
  local f = io.open(cache, "r")
  if f then
    local ok, d = pcall(rex.json.decode, f:read("*a")); f:close()
    if ok and type(d) == "table" and d.owner and d.owner ~= me and (os.time() - (d.ts or 0)) < 8 then
      rex.stop(0)
    end
  end
  snapshot()
end)
