//! 战斗数据类型：规则公式在 `scripts/rules/battle.lua`，这里只留状态与结果。

use serde::{Deserialize, Serialize};

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
