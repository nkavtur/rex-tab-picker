-- Prints one line per tab (window) of the session this script runs against:
-- window_id <TAB> block_id <TAB> number <TAB> label <TAB> title <TAB> cwd
-- Run by tab_picker.lua for sessions other than the current one (a separate connection,
-- so it never disturbs the app's own attachment).
local terminal = "com.superlogical.terminal"
local view = rex.session.view{}
local out = {}
for i, w in ipairs(view and view.windows or {}) do
  local block = w.focused_block_id
  if not block then
    local b = w.layers and w.layers[1] and w.layers[1].blocks and w.layers[1].blocks[1]
    block = b and b.block_id
  end
  local title, pwd = "", ""
  if block then
    local t = rex.block.call(terminal, "title", { block_id = block })
    local p = rex.block.call(terminal, "pwd", { block_id = block })
    title = t and t.title or ""
    pwd = p and p.pwd or ""
  end
  local function clean(x) return (tostring(x or ""):gsub("[\t\r\n]", " ")) end
  out[#out + 1] = table.concat({ w.window_id, block or "", tostring(i), clean(w.label), clean(title), clean(pwd) }, "\t")
end
return table.concat(out, "\n")
