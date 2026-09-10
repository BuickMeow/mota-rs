-- 战斗胜利后的结算规则（7630 CE1 的战后分支）。
--
-- 输入：
--   hero  = {hp, atk, def, mdef, level, exp, gold, flags, vars, items}
--   bag   = {[item_id] = 数量}
--   enemy = {id, name, gold, exp, skills={{id, values}}}
-- 输出 {message, ops}；ops 由 Rust 执行（见 items.lua 顶部说明）。

local balance = {
  steal_gold_rate = 0.2,     -- 神偷：夺走当前金币的 20%（var112）
  degenerate_atk = 2,        -- 退化：攻 -x（var107）
  degenerate_def = 3,        -- 退化：防 -y（var108）
  forget_rate = 0.8,         -- 遗忘：丢失 80% 经验（var113）
  hate_per_kill = 2,         -- 仇恨：每杀一只 +x（var110）
  hate_half = true,          -- 仇恨：战后释放一半（var111 /= 2）
}

local function has_skill(enemy, id)
  for _, s in ipairs(enemy.skills or {}) do
    if s.id == id then
      return true
    end
  end
  return false
end

function after_win(hero, bag, enemy)
  local ops, msg = {}, {}
  local cursed = hero.flags and hero.flags["curse"]

  -- 金币 / 经验（诅咒状态下不再获得）
  if cursed then
    msg[#msg + 1] = "诅咒：无金币经验"
  else
    if (enemy.gold or 0) ~= 0 then
      ops[#ops + 1] = { op = "gold", n = enemy.gold }
    end
    if (enemy.exp or 0) ~= 0 then
      ops[#ops + 1] = { op = "exp", n = enemy.exp }
    end
  end

  -- 战后状态（对应 7630 CE1 的 got_p 分支）
  if has_skill(enemy, "poison") then
    ops[#ops + 1] = { op = "flag", name = "poison", value = true }
    msg[#msg + 1] = "中毒"
  end
  if has_skill(enemy, "weak") then
    ops[#ops + 1] = { op = "flag", name = "weak", value = true }
    msg[#msg + 1] = "衰弱"
  end
  if has_skill(enemy, "slow") then
    ops[#ops + 1] = { op = "flag", name = "slow", value = true }
    msg[#msg + 1] = "迟缓"
  end
  if has_skill(enemy, "curse") then
    ops[#ops + 1] = { op = "flag", name = "curse", value = true }
    msg[#msg + 1] = "诅咒"
  end

  -- 神偷：四种血瓶减半 + 夺走 20% 金币
  if has_skill(enemy, "thief") then
    for _, id in ipairs({ "red_potion", "blue_potion", "yellow_potion", "green_potion" }) do
      local n = bag[id] or 0
      if n > 0 then
        ops[#ops + 1] = { op = "take", item = id, n = math.floor(n / 2) }
      end
    end
    local g = math.floor((hero.gold or 0) * balance.steal_gold_rate)
    if g > 0 then
      ops[#ops + 1] = { op = "gold", n = -g }
    end
    msg[#msg + 1] = "神偷"
  end

  -- 退化
  if has_skill(enemy, "degenerate") then
    ops[#ops + 1] = { op = "stat", stat = "atk", n = -balance.degenerate_atk }
    ops[#ops + 1] = { op = "stat", stat = "def", n = -balance.degenerate_def }
    msg[#msg + 1] = "退化"
  end

  -- 遗忘
  if has_skill(enemy, "forget") then
    local lose = math.floor((hero.exp or 0) * balance.forget_rate)
    if lose > 0 then
      ops[#ops + 1] = { op = "exp", n = -lose }
      msg[#msg + 1] = "遗忘"
    end
  end

  -- 仇恨：击杀积累；带仇恨的怪战后释放一半
  if has_skill(enemy, "hate") then
    ops[#ops + 1] = { op = "var", name = "hate", n = balance.hate_per_kill }
    if balance.hate_half then
      local half = math.floor(((hero.vars and hero.vars["hate"]) or 0) / 2)
      if half > 0 then
        ops[#ops + 1] = { op = "var", name = "hate", n = -half }
      end
    end
    msg[#msg + 1] = "仇恨"
  end

  return { message = table.concat(msg, "，"), ops = ops }
end
