//! 数值库：怪物 / 物品 / 商店。
//!
//! - enemy：全部数值字段都在 struct 上（schema 也列全部，不只列扩展项）。
//! - item：每个参数都在 struct 上；无 `extra`，无 `price`（价格归商店条目）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 行走图引用（地图默认贴图）：file 无后缀，hue 色相，col=pattern 列，
/// row=方向行，opacity 不透明度（7630 事件页默认值 255）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteRef {
    pub file: String,
    #[serde(default)]
    pub hue: i64,
    #[serde(default)]
    pub col: u32,
    #[serde(default)]
    pub row: u32,
    #[serde(default = "opaque8")]
    pub opacity: i64,
}

fn opaque8() -> i64 {
    255
}

/// 怪物：血攻防魔防金币经验 + 图片 + 特技。
/// 特技对应 7630 的 `element_ranks[1..26]`（值为 1 即拥有），id 见 `data/skills`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Enemy {
    pub id: String,
    pub name: String,
    pub hp: i64,
    pub atk: i64,
    pub def: i64,
    #[serde(default)]
    pub mdef: i64,
    #[serde(default)]
    pub gold: i64,
    #[serde(default)]
    pub exp: i64,
    /// 图片名（每个怪物都有，如 032-03），不是 extra。
    pub battler: String,
    #[serde(default)]
    pub hue: i64,
    /// 地图默认贴图（诚实默认：骷髅就是骷髅；想骗人就去实例上换）。
    pub sprite: SpriteRef,
    /// 特技串，可 0 个、1 个或多个；`id` 或 `id:值`（领域 `field:范围,伤害`）。
    /// 写法贴近 7630 特技名后缀（`中毒:a:3`），冒号后的值随时可改。
    #[serde(default)]
    pub skills: Vec<String>,
    /// 制作人自定义字段（如宝物、宝物概率）。
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// 解析特技串：`id` 或 `id:v1[,v2]`，返回（id，数值）。
pub fn parse_skill(s: &str) -> Result<(String, Vec<i64>), String> {
    let mut parts = s.split(':');
    let id = parts.next().unwrap_or_default().trim().to_string();
    if id.is_empty() {
        return Err("特技串为空".to_string());
    }
    let mut values = Vec::new();
    if let Some(rest) = parts.next() {
        for p in rest.split(',') {
            values.push(
                p.trim()
                    .parse::<i64>()
                    .map_err(|_| format!("特技 {id} 数值不是整数：{p}"))?,
            );
        }
    }
    if parts.next().is_some() {
        return Err(format!("特技 {id} 冒号太多"));
    }
    Ok((id, values))
}

/// 校验特技串：形状对，且已知特技的值个数对（combo/clamp/repel 取 1 个，field 取 2 个）。
/// 未知特技 id 放行（存在性由 data_load 对表校验）。
pub fn check_skill(s: &str) -> Result<(), String> {
    let (id, values) = parse_skill(s)?;
    let want = match id.as_str() {
        "combo" | "clamp" | "repel" => 1,
        "field" => 2,
        _ => 0,
    };
    if values.len() != want {
        return Err(format!("特技 {id} 要 {want} 个值，给了 {}", values.len()));
    }
    Ok(())
}

/// 特技颜色（RGBA）：特技名后缀 `名:a:N` 按 7630 文字色表换算，
/// `名:R:G:B[:A]` 直接取用（A 缺省 255 不透明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRgba {
    pub r: i64,
    pub g: i64,
    pub b: i64,
    pub a: i64,
}

impl SkillRgba {
    /// 7630 `Window_Base#text_color`（`▲Window_Base.rb`）：0 白 … 12 橙，浅黄自带 200。
    pub fn from_text_index(index: i64) -> Self {
        let (r, g, b, a) = match index {
            0 => (255, 255, 255, 255),
            1 => (128, 128, 255, 255),
            2 => (255, 128, 128, 255),
            3 => (128, 255, 128, 255),
            4 => (128, 255, 255, 255),
            5 => (255, 128, 255, 255),
            6 => (255, 255, 128, 255),
            7 => (192, 192, 192, 255),
            8 => (0, 0, 0, 255),
            9 => (255, 50, 50, 255),
            10 => (140, 140, 140, 255),
            11 => (255, 255, 0, 200),
            12 => (255, 192, 128, 255),
            _ => (255, 255, 255, 255),
        };
        Self { r, g, b, a }
    }
}

/// 特技（7630 特技表）：id 与怪物 `skills` 里的引用对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 显示颜色（flatten：与其它字段同层）。
    #[serde(flatten)]
    pub color: SkillRgba,
}

/// 物品种类（存成紧凑字符串）：`key` / `potion:75` / `gem:atk:3` /
/// `cure:poison,weak` / `tool` / `relic` / `equipment:weapon:10,0,5`。
/// 冒号后的值随时可改（数值膨胀直接改数，不用动表结构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// 钥匙类（黄/蓝/红/绿/铁门钥匙）：无字段，开门时按 ID 扣除。
    Key,
    /// 血瓶：拾取/使用时回复 `heal` 点生命。
    Potion { heal: i64 },
    /// 宝石：单属性（红宝石攻 / 蓝宝石防 / 绿宝石魔防 / 黄宝石等级）。
    Gem { stat: GemStat, value: i64 },
    /// 解药：消除列出的状态（poison/weak/curse/slow）。
    Cure { states: Vec<String> },
    /// 一次性宝物（炸弹/飞行器/破墙镐…）：具体效果走指令或脚本。
    Tool,
    /// 持有生效的辅助品（手册/十字架/法杖/靴子…）：效果见 `description`。
    Relic,
    /// 可装备：槽位 `weapon` / `armor` + 增益。
    Equipment {
        slot: String,
        atk: i64,
        def: i64,
        mdef: i64,
    },
}

/// 宝石属性（`gem:` 后第一段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemStat {
    Atk,
    Def,
    Mdef,
    Level,
}

impl GemStat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Atk => "atk",
            Self::Def => "def",
            Self::Mdef => "mdef",
            Self::Level => "level",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "atk" => Ok(Self::Atk),
            "def" => Ok(Self::Def),
            "mdef" => Ok(Self::Mdef),
            "level" => Ok(Self::Level),
            _ => Err(format!("未知宝石属性：{s}")),
        }
    }
}

fn parse_ints(s: &str, what: &str) -> Result<Vec<i64>, String> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<i64>()
                .map_err(|_| format!("{what}不是整数：{p}"))
        })
        .collect()
}

impl ItemKind {
    /// 紧凑串 ↔ 结构互转（`kind` 字段存的就是这个串）。
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut parts = s.split(':');
        let head = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();
        match head {
            "key" | "tool" | "relic" if rest.is_empty() => Ok(match head {
                "key" => Self::Key,
                "tool" => Self::Tool,
                _ => Self::Relic,
            }),
            "potion" if rest.len() == 1 => {
                let nums = parse_ints(rest[0], "heal")?;
                if nums.len() != 1 {
                    return Err("potion 只要 1 个数".to_string());
                }
                Ok(Self::Potion { heal: nums[0] })
            }
            "gem" if rest.len() == 2 => {
                let nums = parse_ints(rest[1], "gem")?;
                if nums.len() != 1 {
                    return Err("gem 只要 1 个数".to_string());
                }
                Ok(Self::Gem {
                    stat: GemStat::parse(rest[0])?,
                    value: nums[0],
                })
            }
            "cure" if rest.len() == 1 => {
                let mut states: Vec<String> = rest[0]
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                states.sort();
                if states.is_empty() {
                    return Err("cure 至少要 1 个状态".to_string());
                }
                for st in &states {
                    if !["poison", "weak", "curse", "slow"].contains(&st.as_str()) {
                        return Err(format!("未知状态：{st}"));
                    }
                }
                Ok(Self::Cure { states })
            }
            "equipment" if rest.len() == 2 => {
                if rest[0] != "weapon" && rest[0] != "armor" {
                    return Err(format!("未知槽位：{}", rest[0]));
                }
                let nums = parse_ints(rest[1], "equipment")?;
                if nums.len() != 3 {
                    return Err("equipment 要 3 个数：atk,def,mdef".to_string());
                }
                Ok(Self::Equipment {
                    slot: rest[0].to_string(),
                    atk: nums[0],
                    def: nums[1],
                    mdef: nums[2],
                })
            }
            _ => Err(format!("未知物品种类：{s}")),
        }
    }

    fn dump(&self) -> String {
        match self {
            Self::Key => "key".to_string(),
            Self::Tool => "tool".to_string(),
            Self::Relic => "relic".to_string(),
            Self::Potion { heal } => format!("potion:{heal}"),
            Self::Gem { stat, value } => format!("gem:{}:{value}", stat.as_str()),
            Self::Cure { states } => format!("cure:{}", states.join(",")),
            Self::Equipment {
                slot,
                atk,
                def,
                mdef,
            } => {
                format!("equipment:{slot}:{atk},{def},{mdef}")
            }
        }
    }
}

impl Serialize for ItemKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.dump())
    }
}

impl<'de> Deserialize<'de> for ItemKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// 物品：钥匙/血瓶/宝石/解药/宝物/辅助品/装备，用 `kind` 区分效果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// 拾取即吃（如血瓶/宝石）还是进背包。
    #[serde(default = "default_true")]
    pub auto_use: bool,
    /// 可多次使用（如楼层传送器）。
    #[serde(default)]
    pub reusable: bool,
    /// 被动持有生效、无需掏出来用（如铁门钥匙、圣十字架）。
    #[serde(default)]
    pub passive: bool,
    #[serde(default)]
    pub description: String,
    /// 地图默认贴图（诚实默认；想骗人就去实例上换）。
    pub sprite: SpriteRef,
    /// 效果种类（紧凑串，如 `"potion:75"`）。
    pub kind: ItemKind,
    /// 工具可破坏的 door_id（7630：破墙镐→暗墙、破冰镐→暗冰、
    /// 地震卷轴→暗墙范围、冰冻徽章→暗熔）。
    #[serde(default)]
    pub breaks: Vec<String>,
    /// 破坏范围（切比雪夫半径；<0=整层，0=只打勇士面前一格，>0=以勇士为中心的方形）。
    #[serde(default)]
    pub break_radius: i64,
}

fn default_true() -> bool {
    true
}

/// 商店条目：买物品或买属性（生命/攻击/防御），价格按条目各写各的。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShopEntry {
    /// `item`（给物品）或 `hp` / `atk` / `def`（加属性）。
    pub kind: String,
    /// kind=item 时的物品 ID。
    #[serde(default)]
    pub item: String,
    /// 给多少（物品个数 / 属性点数）。
    #[serde(default = "one")]
    pub amount: i64,
    /// 金币价（0 = 不用金币）。
    #[serde(default)]
    pub cost_gold: i64,
    /// 经验价（0 = 不用经验）。
    #[serde(default)]
    pub cost_exp: i64,
}

fn one() -> i64 {
    1
}

/// 商店：每个店的货和价格各自写。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shop {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub entries: Vec<ShopEntry>,
}

/// 门：钥匙门写钥匙（开门时扣除）；徒手门（free）一撞就开；
/// 条件墙（暗墙/暗冰/暗熔）`key` 为空且非 free，走剧情开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Door {
    pub id: String,
    pub name: String,
    /// 所需钥匙物品 ID；None = 条件墙/徒手门。
    #[serde(default)]
    pub key: Option<String>,
    /// 开门是否消耗钥匙（如铁门钥匙被动持有，不消耗）。
    #[serde(default)]
    pub consume: bool,
    /// 徒手可开（7630 里没有，原创门类；与暗墙不同，不吃开关）。
    #[serde(default)]
    pub free: bool,
    /// 地图默认贴图。
    pub sprite: SpriteRef,
    #[serde(default)]
    pub description: String,
}

impl Door {
    /// 开门判定：Ok(消耗的钥匙数) 可进；Err(原因） 挡路。
    pub fn try_open(&self, bag: &std::collections::HashMap<String, i64>) -> Result<i64, String> {
        if self.free {
            return Ok(0);
        }
        let Some(key) = &self.key else {
            return Err(format!("{}挡路（需剧情开关）", self.name));
        };
        let has = bag.get(key).copied().unwrap_or(0);
        if has <= 0 {
            return Err(format!("缺少钥匙：{key}"));
        }
        Ok(if self.consume { 1 } else { 0 })
    }
}

/// 从 JSON 文本解析（编辑器/测试共用，IO 由调用方负责）。
pub fn load_enemy(s: &str) -> Result<Enemy, serde_json::Error> {
    serde_json::from_str(s)
}

pub fn load_item(s: &str) -> Result<Item, serde_json::Error> {
    serde_json::from_str(s)
}

pub fn load_shop(s: &str) -> Result<Shop, serde_json::Error> {
    serde_json::from_str(s)
}

pub fn load_skill(s: &str) -> Result<Skill, serde_json::Error> {
    serde_json::from_str(s)
}

pub fn load_door(s: &str) -> Result<Door, serde_json::Error> {
    serde_json::from_str(s)
}

pub fn load_barrier(s: &str) -> Result<Barrier, serde_json::Error> {
    serde_json::from_str(s)
}

/// 路障：7630 Common015 按变量 21 编码（1/2/4/8/32）分支；
/// 事件图皆为 002-npc02 变色相（第一、二层实测）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Barrier {
    pub id: String,
    pub name: String,
    /// 变量 21 编码（1/2/4/8/32）。
    pub code: i64,
    /// 直接伤害（如 var21=4 扣 100 血）。
    #[serde(default)]
    pub damage: i64,
    /// 分支种类：toll（代价）/ weaken（衰弱）/ hurt（扣血）/ flag（开关）。
    #[serde(default)]
    pub effect: String,
    /// 地图默认贴图。
    pub sprite: SpriteRef,
    #[serde(default)]
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_poison_bat() {
        let e = load_enemy(
            r#"{"id":"poison_bat","name":"毒蝙蝠","hp":20,"atk":20,"def":1,"battler":"032-03","sprite":{"file":"032-Monster02"},"skills":["poison"]}"#,
        )
        .unwrap();
        assert_eq!(e.hp, 20);
        assert!(e.extra.is_empty());
        assert_eq!(e.skills, vec!["poison".to_string()]);
    }

    #[test]
    fn decode_iron_key_is_passive() {
        let it = load_item(
            r#"{"id":"iron_key","name":"铁门钥匙","auto_use":false,"passive":true,"reusable":true,"kind":"key","sprite":{"file":"101-item01","col":5}}"#,
        )
        .unwrap();
        assert!(it.passive && !it.auto_use);
        assert!(matches!(it.kind, ItemKind::Key));
    }

    #[test]
    fn decode_shop_item_and_stat_entries() {
        let s = load_shop(
            r#"{"id":"shop1","name":"金币商店","entries":[
                {"kind":"atk","amount":2,"cost_gold":50,"cost_exp":0},
                {"kind":"item","item":"yellow_key","amount":1,"cost_gold":10,"cost_exp":0}
            ]}"#,
        )
        .unwrap();
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0].kind, "atk");
        assert_eq!(s.entries[1].item, "yellow_key");
    }

    #[test]
    fn decode_potion_gem_cure_kinds() {
        let p =
            load_item(r#"{"id":"red_potion","name":"红血瓶","kind":"potion:75","sprite":{"file":"103-item03","row":1}}"#).unwrap();
        assert!(matches!(p.kind, ItemKind::Potion { heal: 75 }));

        let g = load_item(
            r#"{"id":"red_gem","name":"红宝石","kind":"gem:atk:1","sprite":{"file":"103-item03"}}"#,
        )
        .unwrap();
        assert!(matches!(
            g.kind,
            ItemKind::Gem {
                stat: GemStat::Atk,
                value: 1
            }
        ));

        let c =
            load_item(r#"{"id":"antidote","name":"解毒药水","kind":"cure:poison","sprite":{"file":"103-item03","col":1,"row":2}}"#)
                .unwrap();
        assert!(matches!(c.kind, ItemKind::Cure { .. }));

        let e = load_item(r#"{"id":"iron_sword","name":"铁剑","kind":"equipment:weapon:10,0,0","sprite":{"file":"104-item04"}}"#).unwrap();
        assert!(matches!(e.kind, ItemKind::Equipment { atk: 10, .. }));

        // 垃圾进不来
        assert!(
            load_item(r#"{"id":"x","name":"x","kind":"potion","sprite":{"file":"f"}}"#).is_err()
        );
        assert!(
            load_item(r#"{"id":"x","name":"x","kind":"gem:hp:3","sprite":{"file":"f"}}"#).is_err()
        );
        assert!(
            load_item(r#"{"id":"x","name":"x","kind":"boom:1","sprite":{"file":"f"}}"#).is_err()
        );
    }

    #[test]
    fn kind_roundtrip_is_compact() {
        let k = ItemKind::Potion { heal: 200 };
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, r#""potion:200""#);
        assert_eq!(ItemKind::parse(&s[1..s.len() - 1]).unwrap(), k);
    }

    #[test]
    fn decode_enemy_skills_and_params() {
        let e = load_enemy(
            r#"{"id":"skeleton_warrior","name":"骷髅武士","hp":180,"atk":448,"def":400,"battler":"044-04","sprite":{"file":"044-Monster14"},"skills":["magic_atk","combo:2","armor_break"]}"#,
        )
        .unwrap();
        assert_eq!(e.skills.len(), 3);
        assert_eq!(
            parse_skill("combo:2").unwrap(),
            ("combo".to_string(), vec![2])
        );
        assert_eq!(
            parse_skill("field:0,200").unwrap(),
            ("field".to_string(), vec![0, 200])
        );
        assert!(check_skill("clamp:300").is_ok());
        assert!(check_skill("clamp").is_err());
        assert!(check_skill("field:200").is_err());
        assert!(check_skill("poison:1").is_err());
    }

    #[test]
    fn decode_skill_text_and_rgb_colors() {
        // 衰弱 :a:5 → 粉色（7630 文字色 5）
        let t =
            load_skill(r#"{"id":"weak","name":"衰弱","r":255,"g":128,"b":255,"a":255}"#).unwrap();
        assert_eq!(t.color, SkillRgba::from_text_index(5));

        // 领域 255:50:200 → 缺省不透明
        let c =
            load_skill(r#"{"id":"field","name":"领域","r":255,"g":50,"b":200,"a":255}"#).unwrap();
        assert_eq!(c.color.a, 255);
    }

    #[test]
    fn text_color_table_matches_7630() {
        assert_eq!(
            SkillRgba::from_text_index(11),
            SkillRgba {
                r: 255,
                g: 255,
                b: 0,
                a: 200
            }
        );
        // 未知编号回落白色（同 7630 else 分支）
        assert_eq!(
            SkillRgba::from_text_index(99),
            SkillRgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255
            }
        );
    }

    #[test]
    fn door_try_open_consumes_or_blocks() {
        use std::collections::HashMap;
        let yellow =
            load_door(r#"{"id":"yellow","name":"黄门","key":"yellow_key","consume":true,"sprite":{"file":"202-other02"}}"#)
                .unwrap();
        let mut bag = HashMap::new();
        assert!(yellow.try_open(&bag).is_err());
        bag.insert("yellow_key".to_string(), 2);
        assert_eq!(yellow.try_open(&bag), Ok(1));

        let iron =
            load_door(r#"{"id":"iron","name":"铁门","key":"iron_key","consume":false,"sprite":{"file":"203-other03","col":3}}"#).unwrap();
        assert_eq!(iron.try_open(&bag), Err("缺少钥匙：iron_key".to_string()));
        bag.insert("iron_key".to_string(), 1);
        assert_eq!(iron.try_open(&bag), Ok(0));

        let wall = load_door(
            r#"{"id":"dark_wall","name":"暗墙","sprite":{"file":"203-other03","col":1}}"#,
        )
        .unwrap();
        assert!(wall.try_open(&bag).is_err());

        let bare = load_door(
            r#"{"id":"iron_free","name":"徒手铁门","free":true,"sprite":{"file":"203-other03","col":3}}"#,
        )
        .unwrap();
        assert_eq!(bare.try_open(&bag), Ok(0));
    }
}
