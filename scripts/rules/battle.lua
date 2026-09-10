-- 战斗结算规则：7630「■enemy property.rb / Enemy_property#cal_enemy」的移植。
--
-- 输入两个表，返回一个表：
--   hero  = {hp, hp_max, atk, def, mdef, level, exp, gold, items={[id]=数量}}
--   enemy = {id, name, hp, atk, def, mdef, gold, exp,
--            skills={{id="combo", values={2}}, ...}}
--   fight(hero, enemy) = {damage=..., nowin=..., turns=..., log={...}}
--
-- damage：勇士这场战斗要承受的总伤害（7630 是确定性公式，不看随机）
-- nowin ：0=能打赢 / 1=攻击不够 / 2=怪物无敌
-- turns ：怪物出手次数（先攻已计）
--
-- 7630 用 var42..var113 一堆裸变量存平衡参数；这里全部改成命名值，
-- 调平衡只改 balance，公式与 cal_enemy 逐行对应（行号见注释）。

local balance = {
  -- 魔杖倍率：CE21 初值（CE16 按等级刷新）。var44/45/47/42
  orb_atk = 1.0,             -- var44 攻击魔杖
  orb_def = 1.0,             -- var45 防御魔杖
  orb_combo = 1.0,           -- var47 连击魔杖
  orb_mdef = 1.0,            -- var42 魔防魔杖
  use_mdef = true,           -- 开关19：采用勇士魔防；关掉则 mdef=0、净化也归零
  vampire_to_enemy = false,  -- 开关52：吸血是否加到怪物血量

  -- 特技参数（7630 是裸变量；未初始化，这里给设计文档默认）
  vampire_rate = 0.34,       -- var53 吸血：勇生命 * 34%
  sturdy_gap = 1,            -- var54 坚固：怪防 = 勇攻 - 1
  first_hits = 1,            -- var55 先攻次数
  magic_atk_rate = 1.0,      -- var57 魔攻：勇防 * 1
  armor_break_rate = 0.9,    -- var58 破甲：勇防 * 90%
  mimic_rate = 1.0,          -- var60 模仿：勇攻 / 勇防
  counter_rate = 0.1,        -- var70 反击：勇攻 * 10%
  purify_rate = 3.0,         -- var104 净化：勇魔防 * 3
  hate = 0,                  -- var111 已积累的仇恨值（无视防御）

  invincible_item = "holy_cross", -- var71 破无敌物品
  damage_cap = 999999,       -- 7630 勇者生命上限
  no_win_damage = 1000000,   -- 打不过时的显示伤害
  invincible_damage = 9999999,
}

-- 怪物是否拥有某特技（技能表只收录 element_ranks==1 的属性）。
local function has_skill(enemy, id)
  for _, s in ipairs(enemy.skills or {}) do
    if s.id == id then
      return true
    end
  end
  return false
end

-- 取特技第 index 个参数（如 combo:3 → value(1)=3）。
local function skill_value(enemy, id, index, default)
  for _, s in ipairs(enemy.skills or {}) do
    if s.id == id and s.values and s.values[index] then
      return s.values[index]
    end
  end
  return default
end

-- cal_enemy 106-107 行：整除时最后一击不吃反击，攻击次数 -1。
local function attack_times(total, per)
  if per <= 0 then
    return 0
  end
  local turns = math.floor(total / per)
  if turns == total / per then
    turns = turns - 1
  end
  return turns
end

function fight(hero, enemy)
  local log = {}
  local mhp, ma, md, mmd = hero.hp, hero.atk, hero.def, hero.mdef

  -- 特技基础量（cal_enemy 52-102 行）
  local vampire = 0
  local sturdy = 0
  local first = 0
  local magicatk = 0
  local combo = 1
  local armor_break = 0
  local enemy_atk, enemy_def = enemy.atk, enemy.def
  local counter = 0
  local purify = 0
  local hate = 0
  local blow = false

  if has_skill(enemy, "vampire") then
    vampire = math.floor(mhp * balance.vampire_rate)
    log[#log + 1] = "吸血"
  end
  if has_skill(enemy, "solid") and enemy_def < ma then
    sturdy = math.max(0, ma - balance.sturdy_gap)
    log[#log + 1] = "坚固"
  end
  if has_skill(enemy, "first_strike") then
    first = balance.first_hits
    log[#log + 1] = "先攻"
  end
  if has_skill(enemy, "magic_atk") then
    magicatk = math.floor(md * balance.magic_atk_rate)
    log[#log + 1] = "魔攻"
  end
  if has_skill(enemy, "combo") then
    combo = skill_value(enemy, "combo", 1, 1)
    log[#log + 1] = "连击x" .. combo
  end
  if has_skill(enemy, "armor_break") then
    armor_break = math.floor(md * balance.armor_break_rate)
    log[#log + 1] = "破甲"
  end
  if has_skill(enemy, "mimic") then
    enemy_atk = math.floor(ma * balance.mimic_rate)
    enemy_def = math.floor(md * balance.mimic_rate)
    log[#log + 1] = "模仿"
  end
  if has_skill(enemy, "counter") then
    counter = math.floor(ma * balance.counter_rate)
    log[#log + 1] = "反击"
  end
  if has_skill(enemy, "purify") then
    purify = math.floor(mmd * balance.purify_rate)
    log[#log + 1] = "净化"
  end
  if has_skill(enemy, "explode") then
    blow = true
    log[#log + 1] = "自爆"
  end
  if has_skill(enemy, "hate") then
    hate = balance.hate
    log[#log + 1] = "仇恨"
  end

  -- 双方最终数值（cal_enemy 85-97 行）
  local ehp = enemy.hp
  if balance.vampire_to_enemy then
    ehp = ehp + vampire
  end
  local hero_atk = math.floor(ma * balance.orb_atk)
  local hero_def = math.floor(md * balance.orb_def)
  local hero_mdef = 0
  if balance.use_mdef then
    hero_mdef = math.floor(mmd * balance.orb_mdef)
  else
    purify = 0
  end

  local enemy_ed = math.max(enemy_def, sturdy)
  local damage, nowin, turns

  if hero_atk > enemy_ed then
    -- cal_enemy 104-127 行
    local per = (hero_atk - enemy_ed) * balance.orb_combo
    turns = attack_times(ehp, per) + first
    if turns < 0 then
      turns = 0
    end
    local reatk = counter * (turns + 1)
    if hero_def - magicatk >= enemy_atk then
      damage = reatk + vampire + armor_break + purify - hero_mdef
    else
      damage = (enemy_atk - hero_def + magicatk) * combo * turns
        + reatk + vampire + armor_break + purify - hero_mdef
    end
    if damage <= 0 then
      damage = 0
    end
    damage = damage + hate
    if blow and damage <= mhp - 1 then
      damage = mhp - 1
    end
    nowin = 0
    if damage >= balance.damage_cap then
      damage = balance.damage_cap
      nowin = 1
      log[#log + 1] = "伤害封顶，打不赢"
    end
  else
    -- cal_enemy 128-131 行
    damage = balance.no_win_damage
    nowin = 1
    turns = 0
    log[#log + 1] = string.format("勇攻%d ≤ 怪防%d", hero_atk, enemy_ed)
  end

  -- cal_enemy 133-138 行：无敌
  if has_skill(enemy, "invincible") then
    local n = (hero.items and hero.items[balance.invincible_item]) or 0
    if n < 1 then
      damage = balance.invincible_damage
      nowin = 2
      log[#log + 1] = "无敌（需" .. balance.invincible_item .. "）"
    end
  end

  log[#log + 1] = string.format("承伤%d", damage)
  return { damage = damage, nowin = nowin, turns = turns, log = log }
end
