//! 规则层：游戏规则全在 `scripts/rules/*.lua`，Rust 只喂状态、执行 Lua 返回的 ops。
//!
//! - `battle.lua`：战斗结算（7630 `Enemy_property#cal_enemy`）
//! - `items.lua`：物品使用（血瓶/宝石/解药/工具破坏计划）
//! - `after_battle.lua`：战后金币经验与状态（诅咒/神偷/退化/遗忘/仇恨）
//! - `on_step.lua`：每步触发（中毒/领域）
//!
//! Lua 不直接改世界，只返回 `{message, ops, consume, break}`；Rust 校验并执行。

use std::collections::HashMap;

use mlua::{Lua, Result as LuaResult, Table};

use crate::battle::{FightReport, Hero};
use crate::cmd::Cmd;
use crate::db::{Enemy, Item, parse_skill};

// build.rs 自动扫 `scripts/rules/**/*.lua` 生成（新增脚本不用改 Rust）。
include!(concat!(env!("OUT_DIR"), "/rules_embedded.rs"));

/// Rust 能执行的规则操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Hp(i64),
    Gold(i64),
    Exp(i64),
    Atk(i64),
    Def(i64),
    Mdef(i64),
    Level(i64),
    Take(String, i64),
    Give(String, i64),
    Flag(String, bool),
    Var(String, i64),
}

/// 工具破坏计划（Rust 用勇士当前朝向/位置执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakPlan {
    pub doors: Vec<String>,
    pub tiles: Vec<i32>,
    pub layer: usize,
    pub radius: i64,
}

/// 事件指令执行结果（`Rules::run_cmds`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventOutcome {
    pub messages: Vec<String>,
    pub ops: Vec<Op>,
    /// 传送（floor, landing）。
    pub goto: Option<(String, String)>,
    /// 触发战斗（enemy id）。
    pub fight: Option<String>,
    /// 打开商店（shop id）。
    pub shop: Option<String>,
    /// 调用公共事件（名字）。
    pub common: Option<String>,
}

/// 一次规则调用的结果。
#[derive(Debug, Clone, Default)]
pub struct RuleOutcome {
    pub message: String,
    pub ops: Vec<Op>,
    /// 消耗品标记（工具不看这个：由破坏结果决定是否消耗）。
    pub consume: bool,
    pub break_plan: Option<BreakPlan>,
}

/// on_step 的输入（超 7 参按规约收拢）。
pub struct StepInput<'a> {
    pub hero: &'a Hero,
    pub bag: &'a HashMap<String, i64>,
    pub flags: &'a HashMap<String, bool>,
    pub vars: &'a HashMap<String, i64>,
    pub x: i32,
    pub y: i32,
    pub near: &'a [(i32, i32, &'a Enemy)],
}

/// 沙箱里的规则引擎（一个 Lua 状态装全部规则脚本）。
pub struct Rules {
    lua: Lua,
}

impl Rules {
    /// 内置脚本：build.rs 自动收集的 `scripts/rules/**/*.lua`。
    pub fn embedded() -> LuaResult<Self> {
        Self::from_named(EMBEDDED_RULES)
    }

    /// 按 (名字, 源码) 顺序载入，名字用于报错定位。
    /// 后载入的同名函数覆盖先载入的（RMXP 脚本列表的语义）。
    pub fn from_named(scripts: &[(&str, &str)]) -> LuaResult<Self> {
        let lua = Lua::new();
        for (name, code) in scripts {
            lua.load(*code).set_name(*name).exec()?;
        }
        Ok(Self { lua })
    }

    /// 匿名片段直载（测试/临时用）。
    pub fn from_scripts(scripts: &[&str]) -> LuaResult<Self> {
        let named: Vec<(&str, &str)> = scripts.iter().map(|s| ("<inline>", *s)).collect();
        Self::from_named(&named)
    }

    /// 战斗结算。
    pub fn fight(
        &self,
        hero: &Hero,
        enemy: &Enemy,
        bag: &HashMap<String, i64>,
        flags: &HashMap<String, bool>,
        vars: &HashMap<String, i64>,
    ) -> LuaResult<FightReport> {
        let hero_t = self.hero_table(hero, bag, flags, vars)?;
        let enemy_t = self.enemy_table(enemy)?;
        let func: mlua::Function = self.lua.globals().get("fight")?;
        let out: Table = func.call((hero_t, enemy_t))?;

        let log = match out.get::<Option<Table>>("log")? {
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

    /// 物品使用规则。
    pub fn use_item(
        &self,
        hero: &Hero,
        bag: &HashMap<String, i64>,
        item: &Item,
        flags: &HashMap<String, bool>,
        vars: &HashMap<String, i64>,
    ) -> LuaResult<RuleOutcome> {
        let hero_t = self.hero_table(hero, bag, flags, vars)?;
        let bag_t = bag_table(&self.lua, bag)?;
        let item_t = self.item_table(item)?;
        let func: mlua::Function = self.lua.globals().get("use_item")?;
        let out: Table = func.call((hero_t, bag_t, item_t))?;
        parse_outcome(&out)
    }

    /// 战斗胜利后的结算（金币经验/状态/神偷/退化/遗忘/仇恨）。
    pub fn after_win(
        &self,
        hero: &Hero,
        enemy: &Enemy,
        bag: &HashMap<String, i64>,
        flags: &HashMap<String, bool>,
        vars: &HashMap<String, i64>,
    ) -> LuaResult<RuleOutcome> {
        let hero_t = self.hero_table(hero, bag, flags, vars)?;
        let bag_t = bag_table(&self.lua, bag)?;
        let enemy_t = self.enemy_table(enemy)?;
        let func: mlua::Function = self.lua.globals().get("after_win")?;
        let out: Table = func.call((hero_t, bag_t, enemy_t))?;
        parse_outcome(&out)
    }

    /// 每走一步的规则（中毒/领域）；near 传本层所有怪物事件。
    pub fn on_step(&self, inp: StepInput<'_>) -> LuaResult<RuleOutcome> {
        let StepInput {
            hero,
            bag,
            flags,
            vars,
            x,
            y,
            near,
        } = inp;
        let hero_t = self.hero_table(hero, bag, flags, vars)?;
        hero_t.set("x", x)?;
        hero_t.set("y", y)?;
        let bag_t = bag_table(&self.lua, bag)?;
        let near_t = self.lua.create_table()?;
        for (i, (ex, ey, enemy)) in near.iter().enumerate() {
            let e = self.enemy_table(enemy)?;
            e.set("x", *ex)?;
            e.set("y", *ey)?;
            near_t.set(i + 1, e)?;
        }
        let func: mlua::Function = self.lua.globals().get("on_step")?;
        let out: Table = func.call((hero_t, bag_t, near_t))?;
        parse_outcome(&out)
    }

    /// 执行事件指令列表（`scripts/rules/commands.lua`）。
    pub fn run_cmds(
        &self,
        hero: &Hero,
        bag: &HashMap<String, i64>,
        flags: &HashMap<String, bool>,
        vars: &HashMap<String, i64>,
        cmds: &[Cmd],
    ) -> LuaResult<EventOutcome> {
        let hero_t = self.hero_table(hero, bag, flags, vars)?;
        let bag_t = bag_table(&self.lua, bag)?;
        let list = self.lua.create_table()?;
        for (i, cmd) in cmds.iter().enumerate() {
            list.set(i + 1, self.cmd_table(cmd)?)?;
        }
        let func: mlua::Function = self.lua.globals().get("run_commands")?;
        let out: Table = func.call((hero_t, bag_t, list))?;

        let messages = match out.get::<Option<Table>>("messages")? {
            Some(t) => {
                let mut v = Vec::new();
                for line in t.sequence_values::<String>() {
                    v.push(line?);
                }
                v
            }
            None => Vec::new(),
        };
        let goto = out
            .get::<Option<Table>>("goto")?
            .map(|g| -> LuaResult<(String, String)> { Ok((g.get("floor")?, g.get("landing")?)) })
            .transpose()?;
        Ok(EventOutcome {
            messages,
            ops: parse_ops(&out)?,
            goto,
            fight: out.get::<Option<String>>("fight")?,
            shop: out.get::<Option<String>>("shop")?,
            common: out.get::<Option<String>>("common")?,
        })
    }

    /// Cmd → Lua 表（字段名与 commands.lua 对齐）。
    fn cmd_table(&self, cmd: &Cmd) -> LuaResult<Table> {
        let t = self.lua.create_table()?;
        match cmd {
            Cmd::Talk { lines } => {
                t.set("op", "talk")?;
                t.set("lines", string_list_table(&self.lua, lines)?)?;
            }
            Cmd::Give { item, n } => {
                t.set("op", "give")?;
                t.set("item", item.as_str())?;
                t.set("n", *n)?;
            }
            Cmd::Take { item, n } => {
                t.set("op", "take")?;
                t.set("item", item.as_str())?;
                t.set("n", *n)?;
            }
            Cmd::SetFlag { name, value } => {
                t.set("op", "set_flag")?;
                t.set("name", name.as_str())?;
                t.set("value", *value)?;
            }
            Cmd::AddVar { name, delta } => {
                t.set("op", "add_var")?;
                t.set("name", name.as_str())?;
                t.set("delta", *delta)?;
            }
            Cmd::Fight { enemy } => {
                t.set("op", "fight")?;
                t.set("enemy", enemy.as_str())?;
            }
            Cmd::Teleport { floor, landing } => {
                t.set("op", "teleport")?;
                t.set("floor", floor.as_str())?;
                t.set("landing", landing.as_str())?;
            }
            Cmd::OpenShop { shop } => {
                t.set("op", "open_shop")?;
                t.set("shop", shop.as_str())?;
            }
            Cmd::CallCommon { name } => {
                t.set("op", "call_common")?;
                t.set("name", name.as_str())?;
            }
            Cmd::Lua { code } => {
                t.set("op", "lua")?;
                t.set("code", code.as_str())?;
            }
        }
        Ok(t)
    }

    fn hero_table(
        &self,
        hero: &Hero,
        bag: &HashMap<String, i64>,
        flags: &HashMap<String, bool>,
        vars: &HashMap<String, i64>,
    ) -> LuaResult<Table> {
        let t = self.lua.create_table()?;
        t.set("name", hero.name.as_str())?;
        t.set("hp", hero.hp)?;
        t.set("hp_max", hero.hp_max)?;
        t.set("atk", hero.atk)?;
        t.set("def", hero.def)?;
        t.set("mdef", hero.mdef)?;
        t.set("level", hero.level)?;
        t.set("exp", hero.exp)?;
        t.set("gold", hero.gold)?;
        t.set("items", bag_table(&self.lua, bag)?)?;
        let f = self.lua.create_table()?;
        for (k, v) in flags {
            f.set(k.as_str(), *v)?;
        }
        t.set("flags", f)?;
        let v = self.lua.create_table()?;
        for (k, n) in vars {
            v.set(k.as_str(), *n)?;
        }
        t.set("vars", v)?;
        Ok(t)
    }

    fn enemy_table(&self, enemy: &Enemy) -> LuaResult<Table> {
        let t = self.lua.create_table()?;
        t.set("id", enemy.id.as_str())?;
        t.set("name", enemy.name.as_str())?;
        t.set("hp", enemy.hp)?;
        t.set("atk", enemy.atk)?;
        t.set("def", enemy.def)?;
        t.set("mdef", enemy.mdef)?;
        t.set("gold", enemy.gold)?;
        t.set("exp", enemy.exp)?;
        let skills = self.lua.create_table()?;
        for (i, raw) in enemy.skills.iter().enumerate() {
            // 垃圾特技串跳过：数据校验另有测试兜底，战斗不因此挂掉。
            let Ok((id, values)) = parse_skill(raw) else {
                continue;
            };
            let st = self.lua.create_table()?;
            st.set("id", id)?;
            let vals = self.lua.create_table()?;
            for (j, v) in values.iter().enumerate() {
                vals.set(j + 1, *v)?;
            }
            st.set("values", vals)?;
            skills.set(i + 1, st)?;
        }
        t.set("skills", skills)?;
        Ok(t)
    }

    fn item_table(&self, item: &Item) -> LuaResult<Table> {
        let t = self.lua.create_table()?;
        t.set("id", item.id.as_str())?;
        t.set("name", item.name.as_str())?;
        // ItemKind 的紧凑串（如 "potion:75"），Lua 侧用模式解析
        let kind = serde_json::to_value(&item.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        t.set("kind", kind)?;
        t.set("reusable", item.reusable)?;
        t.set("breaks", string_list_table(&self.lua, &item.breaks)?)?;
        t.set("break_tiles", int_list_table(&self.lua, &item.break_tiles)?)?;
        t.set("break_layer", item.break_layer)?;
        t.set("break_radius", item.break_radius)?;
        Ok(t)
    }
}

fn bag_table(lua: &Lua, bag: &HashMap<String, i64>) -> LuaResult<Table> {
    let t = lua.create_table()?;
    for (id, n) in bag {
        t.set(id.as_str(), *n)?;
    }
    Ok(t)
}

fn string_list_table(lua: &Lua, list: &[String]) -> LuaResult<Table> {
    let t = lua.create_table()?;
    for (i, s) in list.iter().enumerate() {
        t.set(i + 1, s.as_str())?;
    }
    Ok(t)
}

fn int_list_table(lua: &Lua, list: &[i32]) -> LuaResult<Table> {
    let t = lua.create_table()?;
    for (i, n) in list.iter().enumerate() {
        t.set(i + 1, *n)?;
    }
    Ok(t)
}

fn string_list(t: &Table, key: &str) -> LuaResult<Vec<String>> {
    let mut out = Vec::new();
    if let Some(list) = t.get::<Option<Table>>(key)? {
        for v in list.sequence_values::<String>() {
            out.push(v?);
        }
    }
    Ok(out)
}

fn int_list(t: &Table, key: &str) -> LuaResult<Vec<i32>> {
    let mut out = Vec::new();
    if let Some(list) = t.get::<Option<Table>>(key)? {
        for v in list.sequence_values::<i64>() {
            out.push(v? as i32);
        }
    }
    Ok(out)
}

fn parse_ops(t: &Table) -> LuaResult<Vec<Op>> {
    let mut ops = Vec::new();
    if let Some(list) = t.get::<Option<Table>>("ops")? {
        for item in list.sequence_values::<Table>() {
            let o = item?;
            let op: String = o.get("op")?;
            let n: i64 = o.get::<Option<i64>>("n")?.unwrap_or(0);
            let parsed = match op.as_str() {
                "hp" => Some(Op::Hp(n)),
                "gold" => Some(Op::Gold(n)),
                "exp" => Some(Op::Exp(n)),
                "stat" => match o.get::<String>("stat")?.as_str() {
                    "atk" => Some(Op::Atk(n)),
                    "def" => Some(Op::Def(n)),
                    "mdef" => Some(Op::Mdef(n)),
                    "level" => Some(Op::Level(n)),
                    _ => None,
                },
                "take" => Some(Op::Take(o.get("item")?, n)),
                "give" => Some(Op::Give(o.get("item")?, n)),
                "flag" => Some(Op::Flag(
                    o.get("name")?,
                    o.get::<Option<bool>>("value")?.unwrap_or(true),
                )),
                "var" => Some(Op::Var(o.get("name")?, n)),
                _ => None,
            };
            if let Some(p) = parsed {
                ops.push(p);
            }
        }
    }
    Ok(ops)
}

fn parse_outcome(t: &Table) -> LuaResult<RuleOutcome> {
    let message = t.get::<Option<String>>("message")?.unwrap_or_default();
    let consume = t.get::<Option<bool>>("consume")?.unwrap_or(false);
    let ops = parse_ops(t)?;
    let break_plan = match t.get::<Option<Table>>("break")? {
        Some(b) => Some(BreakPlan {
            doors: string_list(&b, "doors")?,
            tiles: int_list(&b, "tiles")?,
            layer: b.get::<Option<i64>>("layer")?.unwrap_or(1).max(0) as usize,
            radius: b.get::<Option<i64>>("radius")?.unwrap_or(0),
        }),
        None => None,
    };
    Ok(RuleOutcome {
        message,
        ops,
        consume,
        break_plan,
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

    fn rules() -> Rules {
        Rules::embedded().unwrap()
    }

    #[test]
    fn embedded_rules_are_collected_by_build_script() {
        let names: Vec<&str> = EMBEDDED_RULES.iter().map(|(n, _)| *n).collect();
        assert!(names.len() >= 4, "内置规则至少 4 个：{names:?}");
        assert!(names.iter().any(|n| n.ends_with("battle.lua")));
        assert!(names.iter().any(|n| n.ends_with("items.lua")));
    }

    #[test]
    fn from_named_loads_in_order_and_later_overrides() {
        let a =
            "function fight(hero, enemy) return { damage = 1, nowin = 0, turns = 1, log = {} } end";
        let b =
            "function fight(hero, enemy) return { damage = 2, nowin = 0, turns = 1, log = {} } end";
        let rules = Rules::from_named(&[("a.lua", a), ("b.lua", b)]).unwrap();
        let r = rules
            .fight(
                &hero(),
                &enemy(10, 1, 1, &[]),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            )
            .unwrap();
        assert_eq!(r.damage, 2);
    }

    fn run(hero: &Hero, enemy: &Enemy, bag: &HashMap<String, i64>) -> FightReport {
        rules()
            .fight(hero, enemy, bag, &HashMap::new(), &HashMap::new())
            .unwrap()
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
    fn weak_flag_reduces_stats() {
        let mut h = hero();
        h.atk = 100;
        let mut flags = HashMap::new();
        flags.insert("weak".to_string(), true);
        // 衰弱：攻 75 → 对 10 防每击 65；180/65=2（非整除）→ 2 回合
        let r = rules()
            .fight(
                &h,
                &enemy(180, 80, 10, &[]),
                &HashMap::new(),
                &flags,
                &HashMap::new(),
            )
            .unwrap();
        assert!(r.win());
        assert_eq!(r.turns, 2);
    }

    #[test]
    fn after_win_grants_gold_exp_and_status() {
        let e = enemy(10, 1, 1, &["poison"]);
        let out = rules()
            .after_win(
                &hero(),
                &e,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            )
            .unwrap();
        assert!(out.ops.contains(&Op::Gold(1)) && out.ops.contains(&Op::Exp(1)));
        assert!(out.ops.contains(&Op::Flag("poison".into(), true)));
    }

    #[test]
    fn curse_blocks_income_and_thief_steals() {
        let mut h = hero();
        h.gold = 100;
        let mut flags = HashMap::new();
        flags.insert("curse".to_string(), true);
        let out = rules()
            .after_win(
                &h,
                &enemy(10, 1, 1, &[]),
                &HashMap::new(),
                &flags,
                &HashMap::new(),
            )
            .unwrap();
        assert!(!out.ops.iter().any(|o| matches!(o, Op::Gold(_))));

        let flags = HashMap::new();
        let out = rules()
            .after_win(
                &h,
                &enemy(10, 1, 1, &["thief"]),
                &HashMap::new(),
                &flags,
                &HashMap::new(),
            )
            .unwrap();
        assert!(out.ops.contains(&Op::Gold(-20)));
    }

    #[test]
    fn on_step_poison_and_field() {
        let mut flags = HashMap::new();
        flags.insert("poison".to_string(), true);
        let field = enemy(1, 1, 1, &["field:0,200"]);
        let near = [(1, 0, &field)];
        let out = rules()
            .on_step(StepInput {
                hero: &hero(),
                bag: &HashMap::new(),
                flags: &flags,
                vars: &HashMap::new(),
                x: 0,
                y: 0,
                near: &near,
            })
            .unwrap();
        assert!(out.ops.contains(&Op::Hp(-10))); // 中毒
        assert!(out.ops.contains(&Op::Hp(-200))); // 领域（十字邻格）
    }

    #[test]
    fn run_commands_basic() {
        let cmds = vec![
            Cmd::Talk {
                lines: vec!["你好".to_string()],
            },
            Cmd::Give {
                item: "gold".to_string(),
                n: 50,
            },
            Cmd::SetFlag {
                name: "救出仙子".to_string(),
                value: true,
            },
            Cmd::Teleport {
                floor: "1_2".to_string(),
                landing: "下楼梯".to_string(),
            },
        ];
        let out = rules()
            .run_cmds(
                &hero(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &cmds,
            )
            .unwrap();
        assert_eq!(out.messages, vec!["你好".to_string()]);
        assert!(out.ops.contains(&Op::Give("gold".to_string(), 50)));
        assert!(out.ops.contains(&Op::Flag("救出仙子".to_string(), true)));
        assert_eq!(out.goto, Some(("1_2".to_string(), "下楼梯".to_string())));
    }

    #[test]
    fn run_commands_lua_sandbox() {
        let cmds = vec![Cmd::Lua {
            code: r#"talk("欢迎") give("gold", 50) set_flag("救出仙子", true)"#.to_string(),
        }];
        let out = rules()
            .run_cmds(
                &hero(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &cmds,
            )
            .unwrap();
        assert_eq!(out.messages, vec!["欢迎".to_string()]);
        assert!(out.ops.contains(&Op::Give("gold".to_string(), 50)));
        assert!(out.ops.contains(&Op::Flag("救出仙子".to_string(), true)));
    }

    #[test]
    fn use_item_potion_and_tool_plan() {
        let item = Item {
            id: "red_potion".to_string(),
            name: "红血瓶".to_string(),
            auto_use: true,
            reusable: false,
            passive: false,
            description: String::new(),
            sprite: SpriteRef {
                file: String::new(),
                hue: 0,
                col: 0,
                row: 0,
                opacity: 255,
            },
            kind: crate::db::ItemKind::Potion { heal: 75 },
            breaks: Vec::new(),
            break_radius: 0,
            break_tiles: Vec::new(),
            break_layer: 1,
        };
        let mut h = hero();
        h.hp = 950;
        let out = rules()
            .use_item(&h, &HashMap::new(), &item, &HashMap::new(), &HashMap::new())
            .unwrap();
        assert!(out.consume);
        assert_eq!(out.ops, vec![Op::Hp(50)]);

        let mut tool = item.clone();
        tool.id = "pickaxe".to_string();
        tool.kind = crate::db::ItemKind::Tool;
        tool.breaks = vec!["dark_wall".to_string()];
        tool.break_tiles = vec![389];
        let out = rules()
            .use_item(&h, &HashMap::new(), &tool, &HashMap::new(), &HashMap::new())
            .unwrap();
        let plan = out.break_plan.expect("break plan");
        assert_eq!(plan.doors, vec!["dark_wall".to_string()]);
        assert_eq!(plan.tiles, vec![389]);
        assert_eq!(plan.layer, 1);
    }
}
