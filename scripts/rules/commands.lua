-- 事件指令执行：把可视化指令列表（Cmd）跑成结果。
--
-- 输入：
--   hero, bag       当前状态（条件/变量指令以后会用到）
--   cmds            指令表（Rust 由 cmd.rs 的 Cmd 转过来）：
--     {op="talk", lines={...}}      {op="give", item, n}     {op="take", item, n}
--     {op="set_flag", name, value}  {op="add_var", name, delta}
--     {op="fight", enemy}           {op="teleport", floor, landing}
--     {op="open_shop", shop}        {op="call_common", name} {op="lua", code}
-- 输出：
--   {messages={...}, ops={...}, goto={floor,landing}, fight=enemy,
--    shop=shop, common=name}
--
-- Lua 指令块是作者逃生舱：在沙箱里跑，只暴露 talk/give/take/set_flag/add_var，
-- 没有文件/网络 IO（对应旧 eval_snippet，但结果统一走 ops）。

function run_commands(hero, bag, cmds)
  local out = { messages = {}, ops = {} }

  local function add_op(t)
    out.ops[#out.ops + 1] = t
  end
  local function talk(s)
    out.messages[#out.messages + 1] = tostring(s)
  end
  local function give(id, n)
    add_op({ op = "give", item = id, n = n or 1 })
  end
  local function take(id, n)
    add_op({ op = "take", item = id, n = n or 1 })
  end
  local function set_flag(name, value)
    add_op({ op = "flag", name = name, value = value and true or false })
  end
  local function add_var(name, delta)
    add_op({ op = "var", name = name, n = delta or 0 })
  end

  for _, c in ipairs(cmds or {}) do
    local op = c.op
    if op == "talk" then
      for _, line in ipairs(c.lines or {}) do
        talk(line)
      end
    elseif op == "give" then
      give(c.item, c.n)
    elseif op == "take" then
      take(c.item, c.n)
    elseif op == "set_flag" then
      set_flag(c.name, c.value)
    elseif op == "add_var" then
      add_var(c.name, c.delta)
    elseif op == "fight" then
      out.fight = c.enemy
    elseif op == "teleport" then
      out["goto"] = { floor = c.floor, landing = c.landing }
    elseif op == "open_shop" then
      out.shop = c.shop
    elseif op == "call_common" then
      out.common = c.name
    elseif op == "lua" then
      local chunk, err = load(c.code, "cmd.lua", "t", {
        talk = talk,
        give = give,
        take = take,
        set_flag = set_flag,
        add_var = add_var,
      })
      if not chunk then
        error(err)
      end
      chunk()
    end
  end

  return out
end
