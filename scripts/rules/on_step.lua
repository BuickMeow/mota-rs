-- 每走一步的规则（7630 Game_Player 的 on_step 分支）。
--
-- 输入：
--   hero = {x, y, hp, hp_max, atk, def, mdef, level, exp, gold, flags, vars, items}
--   bag  = {[item_id] = 数量}
--   near = 本层所有怪物事件：{x, y, skills={{id, values}}, name}
-- 输出 {message, ops}；ops 由 Rust 执行（见 items.lua 顶部说明）。
--
-- 已接：中毒每步掉血、领域伤害（field:半径,伤害；半径 0 = 十字）。
-- 待接：夹击、阻击、潜伏显形（需要怪物配对/推位，见 README 备注）。

local balance = {
  poison_damage = 10, -- var25 中毒每步伤害
}

local function skill_value(skills, id, index)
  for _, s in ipairs(skills or {}) do
    if s.id == id and s.values and s.values[index] then
      return s.values[index]
    end
  end
  return nil
end

function on_step(hero, bag, near)
  local ops, msg = {}, {}

  -- 中毒：7630 每走一步扣 var25 点
  if hero.flags and hero.flags["poison"] then
    ops[#ops + 1] = { op = "hp", n = -balance.poison_damage }
    msg[#msg + 1] = "中毒 -" .. balance.poison_damage
  end

  -- 领域：经过怪物十字/方形范围自动扣血
  for _, e in ipairs(near or {}) do
    local radius = skill_value(e.skills, "field", 1)
    if radius ~= nil then
      local dmg = skill_value(e.skills, "field", 2) or 0
      local dx, dy = math.abs(e.x - hero.x), math.abs(e.y - hero.y)
      local hit
      if radius == 0 then
        hit = (dx + dy) == 1 -- 十字：四邻
      else
        hit = dx <= radius and dy <= radius
      end
      if hit then
        ops[#ops + 1] = { op = "hp", n = -dmg }
        msg[#msg + 1] = string.format("领域 -%d", dmg)
      end
    end
  end

  return { message = table.concat(msg, "，"), ops = ops }
end
