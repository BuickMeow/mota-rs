//! 战斗结算：规则在 Lua（`scripts/rules/battle.lua`），数值在 JSON（data/enemies）。
//!
//! 7630 的战斗不是 RGSS 战斗画面，而是「碰怪 → 确定性伤害」：
//! 本模块只负责把 hero/enemy 喂给 Lua 的 `fight`，再把结果搬回来。

use std::collections::HashMap;

use mlua::{Lua, Result as LuaResult};
use serde::{Deserialize, Serialize};

use crate::db::{Enemy, parse_skill};

/// 内置战斗规则：`scripts/rules/battle.lua` 缺失时的兜底（与文件同源）。
pub const EMBEDDED: &str = include_str!("../../../scripts/rules/battle.lua");

/// 勇士战斗属性（7630 口径：atk=str 攻、def=dex 防、mdef=int 魔防）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hero {
    #[serde(default = "hero_name")]
    pub name: String,
    pub hp: i64,
    #[serde(default = "hero_hp_max")]
    pub hp_max: i64,
    pub atk: i64,
    pub def: i64,
    #[serde(default)]
    pub mdef: i64,
    #[serde(default = "hero_level")]
    pub level: i64,
    #[serde(default)]
    pub exp: i64,
    #[serde(default)]
    pub gold: i64,
}

fn hero_name() -> String {
    "勇士".to_string()
}

fn hero_hp_max() -> i64 {
    1000
}

fn hero_level() -> i64 {
    1
}

impl Default for Hero {
    fn default() -> Self {
        Self {
            name: hero_name(),
            hp: 1000,
            hp_max: 1000,
            atk: 10,
            def: 10,
            mdef: 0,
            level: 1,
            exp: 0,
            gold: 0,
        }
    }
}

/// 一场战斗的结算结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FightReport {
    /// 勇士承受的总伤害（7630 确定性伤害）。
    pub damage: i64,
    /// 0=能打赢；1=攻击不够；2=怪物无敌。
    pub nowin: i64,
    /// 怪物出手次数（先攻已计）。
    pub turns: i64,
    /// 人类可读的战斗摘要（特技/回合/伤害）。
    pub log: Vec<String>,
}

impl FightReport {
    pub fn win(&self) -> bool {
        self.nowin == 0
    }
}

/// 跑一遍 Lua 战斗规则：`script` 为空则用 [`EMBEDDED`]。
pub fn fight(
    script: &str,
    hero: &Hero,
    enemy: &Enemy,
    bag: &HashMap<String, i64>,
) -> LuaResult<FightReport> {
    let lua = Lua::new();

    let hero_t = lua.create_table()?;
    hero_t.set("name", hero.name.as_str())?;
    hero_t.set("hp", hero.hp)?;
    hero_t.set("hp_max", hero.hp_max)?;
    hero_t.set("atk", hero.atk)?;
    hero_t.set("def", hero.def)?;
    hero_t.set("mdef", hero.mdef)?;
    hero_t.set("level", hero.level)?;
    hero_t.set("exp", hero.exp)?;
    hero_t.set("gold", hero.gold)?;
    let items = lua.create_table()?;
    for (id, n) in bag {
        items.set(id.as_str(), *n)?;
    }
    hero_t.set("items", items)?;

    let enemy_t = lua.create_table()?;
    enemy_t.set("id", enemy.id.as_str())?;
    enemy_t.set("name", enemy.name.as_str())?;
    enemy_t.set("hp", enemy.hp)?;
    enemy_t.set("atk", enemy.atk)?;
    enemy_t.set("def", enemy.def)?;
    enemy_t.set("mdef", enemy.mdef)?;
    enemy_t.set("gold", enemy.gold)?;
    enemy_t.set("exp", enemy.exp)?;
    let skills = lua.create_table()?;
    for (i, raw) in enemy.skills.iter().enumerate() {
        // 垃圾特技串跳过：数据校验由 data_load 负责，战斗不因此挂掉。
        let Ok((id, values)) = parse_skill(raw) else {
            continue;
        };
        let st = lua.create_table()?;
        st.set("id", id)?;
        let vals = lua.create_table()?;
        for (j, v) in values.iter().enumerate() {
            vals.set(j + 1, *v)?;
        }
        st.set("values", vals)?;
        skills.set(i + 1, st)?;
    }
    enemy_t.set("skills", skills)?;

    let code = if script.trim().is_empty() {
        EMBEDDED
    } else {
        script
    };
    lua.load(code).exec()?;
    let func: mlua::Function = lua.globals().get("fight")?;
    let out: mlua::Table = func.call((hero_t, enemy_t))?;

    let log = match out.get::<Option<mlua::Table>>("log")? {
        Some(t) => {
            let mut lines = Vec::new();
            for v in t.sequence_values::<String>() {
                lines.push(v?);
            }
            lines
        }
        None => Vec::new(),
    };
    Ok(FightReport {
        damage: out.get("damage")?,
        nowin: out.get("nowin")?,
        turns: out.get("turns")?,
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SpriteRef;

    fn hero() -> Hero {
        Hero {
            hp: 1000,
            hp_max: 1000,
            atk: 100,
            def: 50,
            mdef: 20,
            ..Hero::default()
        }
    }

    fn enemy(hp: i64, atk: i64, def: i64, skills: &[&str]) -> Enemy {
        Enemy {
            id: "e".to_string(),
            name: "测试怪".to_string(),
            hp,
            atk,
            def,
            mdef: 0,
            gold: 1,
            exp: 1,
            battler: String::new(),
            hue: 0,
            sprite: SpriteRef {
                file: String::new(),
                hue: 0,
                col: 0,
                row: 0,
                opacity: 255,
            },
            skills: skills.iter().map(|s| s.to_string()).collect(),
            extra: Default::default(),
        }
    }

    fn run(hero: &Hero, enemy: &Enemy, bag: &HashMap<String, i64>) -> FightReport {
        fight(EMBEDDED, hero, enemy, bag).unwrap()
    }

    #[test]
    fn plain_fight_is_deterministic() {
        // 勇攻100 / 怪防10 / 怪血180 → 每击90，整除2 → 2-1=1 回合
        // (怪攻80 - 勇防50) * 1 * 1 - 魔防20 = 10
        let r = run(&hero(), &enemy(180, 80, 10, &[]), &HashMap::new());
        assert!(r.win());
        assert_eq!(r.damage, 10);
        assert_eq!(r.turns, 1);
    }

    #[test]
    fn combo_and_first_strike_multiply_damage() {
        // combo:3 + first_strike：每击90、180/90=2 整除→1回合，+1先攻=2
        // (80-50)*3*2 - 魔防20 = 160
        let r = run(
            &hero(),
            &enemy(180, 80, 10, &["combo:3", "first_strike"]),
            &HashMap::new(),
        );
        assert!(r.win());
        assert_eq!(r.turns, 2);
        assert_eq!(r.damage, 160);
    }

    #[test]
    fn solid_sets_enemy_def_to_atk_minus_gap() {
        // 坚固（solid，不是夹击 clamp）：怪防 = 100-1 = 99，每击1；血10 → 10-1=9 回合
        // (80-50)*1*9 - 魔防20 = 250
        let r = run(&hero(), &enemy(10, 80, 10, &["solid"]), &HashMap::new());
        assert!(r.win());
        assert_eq!(r.turns, 9);
        assert_eq!(r.damage, 250);
    }

    #[test]
    fn too_weak_hero_cannot_win() {
        let mut h = hero();
        h.atk = 5;
        let r = run(&h, &enemy(180, 80, 10, &[]), &HashMap::new());
        assert!(!r.win());
        assert_eq!(r.nowin, 1);
        assert_eq!(r.damage, 1_000_000);
    }

    #[test]
    fn invincible_needs_holy_cross() {
        let e = enemy(1, 80, 0, &["invincible"]);
        let r = run(&hero(), &e, &HashMap::new());
        assert_eq!(r.nowin, 2);
        assert_eq!(r.damage, 9_999_999);

        let mut bag = HashMap::new();
        bag.insert("holy_cross".to_string(), 1);
        let r = run(&hero(), &e, &bag);
        assert!(r.win());
    }

    #[test]
    fn explode_leaves_one_hp() {
        // 自爆：伤害最多打到 hp-1
        let r = run(&hero(), &enemy(1, 80, 0, &["explode"]), &HashMap::new());
        assert!(r.win());
        assert_eq!(r.damage, 999);
    }

    #[test]
    fn empty_script_falls_back_to_embedded() {
        let r = fight("", &hero(), &enemy(10, 80, 10, &[]), &HashMap::new()).unwrap();
        assert!(r.win());
    }

    #[test]
    fn bad_skill_strings_are_skipped() {
        let r = run(
            &hero(),
            &enemy(180, 80, 0, &["???", "combo"]),
            &HashMap::new(),
        );
        assert!(r.win());
        assert_eq!(r.damage, 10); // combo 缺参数按 1 次；30 - 魔防20
    }
}
