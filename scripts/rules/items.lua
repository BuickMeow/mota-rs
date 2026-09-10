-- 物品使用规则（血瓶/宝石/解药/工具）。
--
-- 输入：
--   hero = {hp, hp_max, atk, def, mdef, level, exp, gold, flags, vars, items}
--   bag  = {[item_id] = 数量}
--   item = {id, name, kind="potion:75"/"gem:atk:1"/"cure:poison"/"tool"/...,
--           reusable, breaks={door_id}, break_tiles={图块号}, break_layer, break_radius}
-- 输出：
--   {message=..., ops={...}, consume=true/false,
--    break={doors=..., tiles=..., layer=..., radius=...}}
--
-- ops 由 Rust 执行：
--   {op="hp", n} {op="gold", n} {op="exp", n}
--   {op="stat", stat="atk"/"def"/"mdef"/"level", n}
--   {op="take", item, n} {op="give", item, n}
--   {op="flag", name, value} {op="var", name, n}
--
-- 工具的“有没有破坏到东西才消耗”由 Rust 按 break 结果决定。

function use_item(hero, bag, item)
  local kind = item.kind or ""

  -- 血瓶
  local heal = tonumber(kind:match("^potion:(-?%d+)$"))
  if heal then
    local after = math.min(hero.hp + heal, hero.hp_max)
    return {
      message = string.format("%s：生命 +%d", item.name, after - hero.hp),
      ops = { { op = "hp", n = after - hero.hp } },
      consume = true,
    }
  end

  -- 宝石
  local stat, val = kind:match("^gem:(%a+):(-?%d+)$")
  if stat then
    return {
      message = string.format("%s：%s +%d", item.name, stat, tonumber(val)),
      ops = { { op = "stat", stat = stat, n = tonumber(val) } },
      consume = true,
    }
  end

  -- 解药：按状态清 flag
  local states = kind:match("^cure:(.+)$")
  if states then
    local ops, cleared = {}, {}
    for s in states:gmatch("[^,]+") do
      s = s:match("^%s*(.-)%s*$")
      if hero.flags and hero.flags[s] then
        ops[#ops + 1] = { op = "flag", name = s, value = false }
        cleared[#cleared + 1] = s
      end
    end
    if #cleared == 0 then
      return { message = item.name .. "：现在没有需要解除的状态" }
    end
    return {
      message = item.name .. "：解除 " .. table.concat(cleared, "、"),
      ops = ops,
      consume = true,
    }
  end

  -- 地形工具：交给 Rust 按 break 计划执行
  if kind == "tool" and ((#(item.breaks or {}) > 0) or (#(item.break_tiles or {}) > 0)) then
    return {
      message = item.name .. "：使用",
      ["break"] = {
        doors = item.breaks or {},
        tiles = item.break_tiles or {},
        layer = item.break_layer or 1,
        radius = item.break_radius or 0,
      },
    }
  end

  return { message = item.name .. "：暂时用不上" }
end
