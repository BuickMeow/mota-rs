//! 试玩运行时：移动/交互/规则执行，与编辑器 UI 无关，可无窗口测试。
//!
//! 规则公式在 `scripts/rules/*.lua`；这里只做引擎：通行、触发、ops 落账、跨层记忆。

use std::collections::{HashMap, HashSet};

use crate::battle::{FightReport, Hero};
use crate::map::{Floor, Instance, Lifecycle, TemplateKind};
use crate::rules::{BreakPlan, Op, Rules, StepInput};

/// 极简试玩状态（正式规则引擎落地前先能走能捡；战斗走 Lua 规则）。
#[derive(Debug, Clone, Default)]
pub struct Playtest {
    pub on: bool,
    pub hero: (i32, i32),
    /// 勇士战斗属性（血攻防魔防/金币经验），碰怪结算用。
    pub stats: Hero,
    pub bag: HashMap<String, i64>,
    pub flags: HashMap<String, bool>,
    /// 命名变量（仇恨等规则状态）。
    pub vars: HashMap<String, i64>,
    /// 本局已消费的一次性实例 id（Once）：切层重进不再出现。
    pub once_done: HashSet<String>,
    /// 本局已清掉的地形图块 (层id, x, y, z)：切层重进不再长回来。
    pub broken_tiles: HashSet<(String, i32, i32, usize)>,
    /// 踩到楼梯后待切换的（目标楼层, 落脚点），由外层窗口执行。
    pub pending_floor: Option<(String, String)>,
    /// 剧情结束后待切换的（目标楼层, x, y），由外层窗口执行。
    pub pending_goto: Option<(String, i32, i32)>,
    /// 勇士朝向（RMXP 编码：2 下 / 4 左 / 6 右 / 8 上）。
    pub dir: i32,
    /// 行走动画帧（0..3）。
    pub frame: usize,
    /// 连续行走的下一拍冷却（秒）。
    pub move_cd: f64,
    /// 游戏窗口对话框（RMXP「显示文章」）。
    pub dialog: Option<Dialog>,
}

/// 游戏窗口里的对话框：一次显示一句，空格/回车翻页。
#[derive(Debug, Clone)]
pub struct Dialog {
    pub lines: Vec<String>,
    pub page: usize,
    /// 说完要移除的实例 id（消费型 NPC：Once 永久 / 重进恢复档）。
    pub vanish: Option<String>,
    /// 说完切层（开始地图的自动剧情）：目标层 + 坐标。
    pub goto: Option<(String, i32, i32)>,
}

impl Playtest {
    pub fn start(spawn: Option<(i32, i32)>, stats: Hero) -> Self {
        Self {
            on: true,
            hero: spawn.unwrap_or((0, 0)),
            stats,
            bag: HashMap::new(),
            flags: HashMap::new(),
            vars: HashMap::new(),
            once_done: HashSet::new(),
            broken_tiles: HashSet::new(),
            pending_floor: None,
            pending_goto: None,
            dir: 2,
            frame: 0,
            move_cd: 0.0,
            dialog: None,
        }
    }
}

/// 切层前清掉本局已消费的一次性事件（门/物品/怪/NPC）。
pub fn apply_once_done(floor: &mut Floor, once: &HashSet<String>) {
    floor.instances.retain(|inst| !once.contains(&inst.id));
}

/// 切层前重放本局已破坏的地形（工具清掉的墙/冰/熔岩）。
pub fn apply_broken_tiles(floor: &mut Floor, broken: &HashSet<(String, i32, i32, usize)>) {
    let id = floor.id.clone();
    for (fid, x, y, z) in broken {
        if fid == &id {
            floor.set_tile(*x, *y, *z, 0);
        }
    }
}

/// 移动判定上下文（超 7 参按规约收拢；tiles 为空＝不判图块，只判实例）。
pub struct MoveCtx<'a> {
    pub doors: &'a HashMap<String, crate::db::Door>,
    pub barriers: &'a HashMap<String, crate::db::Barrier>,
    pub enemies: &'a HashMap<String, crate::db::Enemy>,
    pub tiles: Option<&'a crate::tiles::Tileset>,
    /// 规则引擎（None＝规则未载入，战斗/触发只提示）。
    pub rules: Option<&'a Rules>,
}

/// 试走一步：图块（按图块表通行）/门（按门表扣钥匙）/怪/NPC/推不动的箱子挡路，
/// Once 物品捡起，箱子沿行走方向推一格（目标被挡则不动），其余通过。返回是否走成了。
pub fn step_hero(
    floor: &mut Floor,
    play: &mut Playtest,
    ctx: &MoveCtx<'_>,
    dx: i32,
    dy: i32,
    status: &mut String,
) -> bool {
    let (nx, ny) = (play.hero.0 + dx, play.hero.1 + dy);
    if nx < 0 || ny < 0 || nx >= floor.width.min(60) as i32 || ny >= floor.height.min(60) as i32 {
        return false;
    }
    // 图块通行（RMXP 规则：离格＋进格双向查；无表则跳过）。
    if let Some(tiles) = ctx.tiles {
        use crate::tiles::reverse_dir;
        let dir = if dx == 1 {
            6
        } else if dx == -1 {
            4
        } else if dy == 1 {
            2
        } else {
            8
        };
        let (hx, hy) = play.hero;
        let tile = |x: i32, y: i32, z: usize| floor.tile(x, y, z);
        if !tiles.exit_at(tile(hx, hy, 2), tile(hx, hy, 1), tile(hx, hy, 0), dir)
            || !tiles.exit_at(
                tile(nx, ny, 2),
                tile(nx, ny, 1),
                tile(nx, ny, 0),
                reverse_dir(dir),
            )
        {
            *status = "图块挡路".to_string();
            return false;
        }
    }
    let doors = ctx.doors;
    let barriers = ctx.barriers;
    let idxs = floor.instances_at(nx, ny);
    let mut pickup: Option<(usize, String)> = None;
    let mut open_door: Option<(usize, String, i64)> = None;
    let mut push_box: Option<(usize, i32, i32)> = None;
    // (实例下标, 战报, 怪物 id)：先算好，走完循环再统一落账。
    let mut kill: Option<(usize, FightReport, String)> = None;
    for i in idxs {
        let inst = &floor.instances[i];
        match &inst.template {
            TemplateKind::Door { door_id } => match doors.get(door_id) {
                None => {
                    *status = format!("门挡路：{door_id}（门表无此门）");
                    return false;
                }
                Some(def) => match def.try_open(&play.bag) {
                    Err(why) => {
                        *status = why;
                        return false;
                    }
                    Ok(cost) => {
                        open_door = Some((i, def.key.clone().unwrap_or_default(), cost));
                    }
                },
            },
            TemplateKind::Monster { monster_id } => {
                let Some(enemy) = ctx.enemies.get(monster_id) else {
                    *status = format!("遭遇：{monster_id}（怪物表无此怪）");
                    return false;
                };
                let Some(rules) = ctx.rules else {
                    *status = format!("遭遇：{monster_id}（战斗规则未载入）");
                    return false;
                };
                let report =
                    match rules.fight(&play.stats, enemy, &play.bag, &play.flags, &play.vars) {
                        Ok(r) => r,
                        Err(e) => {
                            *status = format!("战斗规则出错：{e}");
                            return false;
                        }
                    };
                if !report.win() {
                    let why = if report.nowin == 2 {
                        "无敌"
                    } else {
                        "攻击不够"
                    };
                    *status = format!("打不过 {monster_id}（{why}）：{}", report.log.join("，"));
                    return false;
                }
                if report.damage >= play.stats.hp {
                    *status = format!(
                        "会战败：{monster_id} 要挨{}血，当前{}",
                        report.damage, play.stats.hp
                    );
                    return false;
                }
                kill = Some((i, report, monster_id.clone()));
            }
            TemplateKind::Npc { dialog, .. } => {
                *status = format!("NPC：{}", dialog.first().map_or("……", String::as_str));
                let vanish = match inst.lifecycle() {
                    Lifecycle::Once | Lifecycle::RespawnOnReenter => Some(inst.id.clone()),
                    _ => None,
                };
                play.dialog = Some(Dialog {
                    lines: dialog.clone(),
                    page: 0,
                    vanish,
                    goto: None,
                });
                return false;
            }
            TemplateKind::Box => {
                let (bx, by) = (nx + dx, ny + dy);
                let blocked = bx < 0
                    || by < 0
                    || bx >= floor.width.min(60) as i32
                    || by >= floor.height.min(60) as i32
                    || floor
                        .instances
                        .iter()
                        .any(|b| b.x == bx && b.y == by && b.template.blocks_push());
                if blocked {
                    *status = "箱子推不动".to_string();
                    return false;
                }
                push_box = Some((i, bx, by));
            }
            TemplateKind::Item { item_id } => {
                if inst.lifecycle() == Lifecycle::Once {
                    pickup = Some((i, item_id.clone()));
                }
            }
            TemplateKind::StairUp {
                to_floor,
                to_landing,
            }
            | TemplateKind::StairDown {
                to_floor,
                to_landing,
            } => {
                play.hero = (nx, ny);
                sync_plates(floor, play);
                // 真正的切层由外层窗口处理（要读别的楼层文件）
                play.pending_floor = Some((to_floor.clone(), to_landing.clone()));
                *status = format!("楼梯→{to_floor}:{to_landing}");
                run_on_step(floor, play, ctx, status);
                return true;
            }
            TemplateKind::Barrier { barrier_id } => {
                play.hero = (nx, ny);
                sync_plates(floor, play);
                match barriers.get(barrier_id) {
                    Some(def) => {
                        *status = format!("路障{}：-{}血", def.name, def.damage);
                    }
                    None => {
                        *status = format!("未知路障：{barrier_id}");
                    }
                }
                run_on_step(floor, play, ctx, status);
                return true;
            }
            TemplateKind::Landing { .. } | TemplateKind::Plate { .. } => {}
        }
    }
    play.hero = (nx, ny);
    if let Some((i, key, cost)) = open_door {
        if i < floor.instances.len() {
            let inst = &floor.instances[i];
            let (id, lc) = (inst.id.clone(), inst.lifecycle());
            floor.instances.remove(i);
            if lc == Lifecycle::Once {
                play.once_done.insert(id.clone());
            }
            if cost > 0 {
                *play.bag.entry(key.clone()).or_insert(0) -= cost;
            }
            *status = format!("开门 {id}（-{cost}{key}）");
        }
    } else if let Some((i, bx, by)) = push_box {
        if i < floor.instances.len() {
            floor.instances[i].x = bx;
            floor.instances[i].y = by;
            *status = format!("推动箱子→({bx},{by})");
        }
    } else if let Some((i, item_id)) = pickup {
        if i < floor.instances.len() {
            let inst = &floor.instances[i];
            let (id, lc) = (inst.id.clone(), inst.lifecycle());
            floor.instances.remove(i);
            if lc == Lifecycle::Once {
                play.once_done.insert(id.clone());
            }
            *play.bag.entry(item_id.clone()).or_insert(0) += 1;
            *status = format!("捡起 {item_id}（{id}）");
        }
    } else if let Some((i, report, monster_id)) = kill {
        if i < floor.instances.len() {
            let inst = &floor.instances[i];
            let (id, lc) = (inst.id.clone(), inst.lifecycle());
            floor.instances.remove(i);
            if lc == Lifecycle::Once {
                play.once_done.insert(id.clone());
            }
            play.stats.hp -= report.damage;
            // 战后结算（金币经验/状态/神偷/退化/遗忘/仇恨）全交给 Lua 规则
            let mut extra = String::new();
            if let Some(rules) = ctx.rules
                && let Some(enemy) = ctx.enemies.get(&monster_id)
            {
                match rules.after_win(&play.stats, enemy, &play.bag, &play.flags, &play.vars) {
                    Ok(out) => {
                        apply_rule_ops(play, &out.ops);
                        extra = out.message;
                    }
                    Err(e) => extra = format!("战后规则出错：{e}"),
                }
            }
            let tail = if extra.is_empty() {
                String::new()
            } else {
                format!("，{extra}")
            };
            *status = format!("击败 {id}：-{}血{tail}", report.damage);
        }
    } else {
        *status = format!("({nx},{ny})");
    }
    sync_plates(floor, play);
    run_on_step(floor, play, ctx, status);
    true
}

/// 每走一步的 Lua 规则（中毒/领域）：算完把 ops 落账，摘要附在 status 后面。
fn run_on_step(floor: &Floor, play: &mut Playtest, ctx: &MoveCtx<'_>, status: &mut String) {
    let Some(rules) = ctx.rules else {
        return;
    };
    let near: Vec<(i32, i32, &crate::db::Enemy)> = floor
        .instances
        .iter()
        .filter_map(|inst| match &inst.template {
            TemplateKind::Monster { monster_id } => {
                ctx.enemies.get(monster_id).map(|e| (inst.x, inst.y, e))
            }
            _ => None,
        })
        .collect();
    let outcome = match rules.on_step(StepInput {
        hero: &play.stats,
        bag: &play.bag,
        flags: &play.flags,
        vars: &play.vars,
        x: play.hero.0,
        y: play.hero.1,
        near: &near,
    }) {
        Ok(o) => o,
        Err(e) => {
            status.push_str(&format!("；触发规则出错：{e}"));
            return;
        }
    };
    apply_rule_ops(play, &outcome.ops);
    if !outcome.message.is_empty() {
        status.push('；');
        status.push_str(&outcome.message);
    }
    if play.stats.hp <= 0 {
        status.push_str("（生命耗尽）");
    }
}

/// 执行 Lua 规则返回的 ops（HP/金币/经验/属性/物品/开关/变量）。
fn apply_rule_ops(play: &mut Playtest, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Hp(n) => {
                play.stats.hp = (play.stats.hp + n).clamp(0, play.stats.hp_max);
            }
            Op::Gold(n) => play.stats.gold = (play.stats.gold + n).max(0),
            Op::Exp(n) => play.stats.exp = (play.stats.exp + n).max(0),
            Op::Atk(n) => play.stats.atk = (play.stats.atk + n).max(0),
            Op::Def(n) => play.stats.def = (play.stats.def + n).max(0),
            Op::Mdef(n) => play.stats.mdef = (play.stats.mdef + n).max(0),
            Op::Level(n) => play.stats.level = (play.stats.level + n).max(0),
            Op::Take(id, n) => {
                if let Some(cur) = play.bag.get_mut(id) {
                    *cur -= n;
                    if *cur <= 0 {
                        play.bag.remove(id);
                    }
                }
            }
            Op::Give(id, n) => *play.bag.entry(id.clone()).or_insert(0) += n,
            Op::Flag(name, value) => {
                play.flags.insert(name.clone(), *value);
            }
            Op::Var(name, n) => *play.vars.entry(name.clone()).or_insert(0) += n,
        }
    }
}

/// 压力板同步：有箱子压住的板对应开为真，否则为假。
fn sync_plates(floor: &Floor, play: &mut Playtest) {
    for inst in &floor.instances {
        if let TemplateKind::Plate { flag } = &inst.template {
            let on = floor
                .instances
                .iter()
                .any(|b| matches!(b.template, TemplateKind::Box) && b.x == inst.x && b.y == inst.y);
            play.flags.insert(flag.clone(), on);
        }
    }
}

/// 朝向 → 面前格的偏移。
fn dir_delta(dir: i32) -> (i32, i32) {
    match dir {
        4 => (-1, 0),
        6 => (1, 0),
        8 => (0, -1),
        _ => (0, 1),
    }
}

/// 对话结束：消费型 NPC 移除（Once 记进本局记录）；剧情要求切层就记待切。
pub fn finish_dialog(floor: &mut Floor, play: &mut Playtest) {
    let Some(dialog) = play.dialog.take() else {
        return;
    };
    if let Some(id) = dialog.vanish
        && let Some(pos) = floor.instances.iter().position(|i| i.id == id)
    {
        let lc = floor.instances[pos].lifecycle();
        floor.instances.remove(pos);
        if lc == Lifecycle::Once {
            play.once_done.insert(id);
        }
    }
    if let Some(goto) = dialog.goto {
        play.pending_goto = Some(goto);
    }
}

/// 可快捷使用的物品 id：血瓶/宝石/解药/工具；血瓶按 7630 惯例排 1..4。
pub fn usable_items(
    bag: &HashMap<String, i64>,
    items: &HashMap<String, crate::db::Item>,
) -> Vec<String> {
    use crate::db::ItemKind;
    let mut ids: Vec<String> = bag
        .iter()
        .filter(|(id, n)| {
            **n > 0
                && items.get(*id).is_some_and(|it| {
                    matches!(
                        it.kind,
                        ItemKind::Potion { .. }
                            | ItemKind::Gem { .. }
                            | ItemKind::Cure { .. }
                            | ItemKind::Tool
                    )
                })
        })
        .map(|(id, _)| id.clone())
        .collect();
    let prio = |id: &str| match id {
        "red_potion" => 0,
        "blue_potion" => 1,
        "yellow_potion" => 2,
        "green_potion" => 3,
        _ => 10,
    };
    ids.sort_by_key(|id| (prio(id), id.clone()));
    ids
}

/// 扣物品；数量到 0 就删掉。
fn consume_item(play: &mut Playtest, id: &str) {
    if let Some(n) = play.bag.get_mut(id) {
        *n -= 1;
        if *n <= 0 {
            play.bag.remove(id);
        }
    }
}

/// 破坏满足条件且属于可破坏类型的门（暗墙/暗冰/暗熔），返回被破坏实例的 id。
fn break_doors(
    floor: &mut Floor,
    breaks: &[String],
    hit: impl Fn(&Instance) -> bool,
) -> Vec<String> {
    let mut removed = Vec::new();
    floor.instances.retain(|inst| {
        let target = matches!(&inst.template, TemplateKind::Door { door_id } if breaks.iter().any(|b| b == door_id))
            && hit(inst);
        if target {
            removed.push(inst.id.clone());
        }
        !target
    });
    removed
}

/// 清掉范围内符合工具的地形图块（7630：破墙镐清墙层、破冰镐/冰冻徽章清地形层），
/// 返回清掉的格数并记入本局地形记录（切层重进不再长回来）。
fn clear_break_tiles(
    floor: &mut Floor,
    play: &mut Playtest,
    plan: &BreakPlan,
    in_range: impl Fn(i32, i32) -> bool,
) -> usize {
    if plan.tiles.is_empty() {
        return 0;
    }
    let z = plan.layer;
    let id = floor.id.clone();
    let (w, h) = (floor.width as i32, floor.height as i32);
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            if !in_range(x, y) {
                continue;
            }
            let tid = floor.tile(x, y, z);
            if tid != 0 && plan.tiles.contains(&tid) {
                floor.set_tile(x, y, z, 0);
                play.broken_tiles.insert((id.clone(), x, y, z));
                n += 1;
            }
        }
    }
    n
}

/// 用一件物品：效果规则在 items.lua，Rust 只执行返回的 ops / 破坏计划。
pub fn use_item(
    floor: &mut Floor,
    play: &mut Playtest,
    rules: Option<&Rules>,
    item: &crate::db::Item,
    status: &mut String,
) {
    if play.bag.get(&item.id).copied().unwrap_or(0) <= 0 {
        *status = format!("{}：背包里没有这件物品", item.name);
        return;
    }
    let Some(rules) = rules else {
        *status = "物品规则未载入".to_string();
        return;
    };
    let outcome = match rules.use_item(&play.stats, &play.bag, item, &play.flags, &play.vars) {
        Ok(o) => o,
        Err(e) => {
            *status = format!("物品规则出错：{e}");
            return;
        }
    };
    apply_rule_ops(play, &outcome.ops);
    match &outcome.break_plan {
        None => {
            *status = outcome.message.clone();
            if outcome.consume {
                consume_item(play, &item.id);
            }
        }
        Some(plan) => {
            let (dx, dy) = dir_delta(play.dir);
            let (hx, hy) = play.hero;
            let in_range = |x: i32, y: i32| -> bool {
                if plan.radius < 0 {
                    true
                } else if plan.radius == 0 {
                    (x, y) == (hx + dx, hy + dy)
                } else {
                    let r = plan.radius as i32;
                    (x - hx).abs() <= r && (y - hy).abs() <= r
                }
            };
            let removed = break_doors(floor, &plan.doors, |inst| in_range(inst.x, inst.y));
            let cleared = clear_break_tiles(floor, play, plan, in_range);
            if removed.is_empty() && cleared == 0 {
                *status = format!("{}：面前没有能破坏的东西", item.name);
            } else {
                // 破了要一直破：事件记一次性、地形记图块（切层重进都还在）
                for id in &removed {
                    play.once_done.insert(id.clone());
                }
                *status = format!(
                    "{}：破坏 {} 处，清地形 {} 格",
                    item.name,
                    removed.len(),
                    cleared
                );
                if !item.reusable {
                    consume_item(play, &item.id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::TemplateKind;

    fn empty_floor() -> Floor {
        Floor {
            id: "t".to_string(),
            name: "测试".to_string(),
            width: 5,
            height: 5,
            layers: Vec::new(),
            instances: Vec::new(),
            spawn: None,
            intro: None,
            tower: None,
            level: None,
            parent: None,
            order: None,
        }
    }

    fn door_at(x: i32, y: i32) -> Instance {
        Instance {
            id: "d".to_string(),
            template: TemplateKind::Door {
                door_id: "yellow".to_string(),
            },
            x,
            y,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        }
    }

    #[test]
    fn erase_removes_only_target_cell() {
        let mut f = empty_floor();
        for (x, y) in [(1, 1), (1, 1), (2, 2)] {
            let mut d = door_at(x, y);
            d.id = format!("{x}{y}");
            f.instances.push(d);
        }
        assert_eq!(f.erase_at(1, 1), 2);
        assert_eq!(f.instances.len(), 1);
    }

    fn doors_map() -> HashMap<String, crate::db::Door> {
        [("yellow", "黄门", Some("yellow_key"), true)]
            .into_iter()
            .map(|(id, name, key, consume)| {
                (
                    id.to_string(),
                    crate::db::Door {
                        id: id.to_string(),
                        name: name.to_string(),
                        key: key.map(str::to_string),
                        consume,
                        free: false,
                        sprite: crate::db::SpriteRef {
                            file: "202-other02".to_string(),
                            hue: 0,
                            col: 0,
                            row: 0,
                            opacity: 255,
                        },
                        description: String::new(),
                    },
                )
            })
            .collect()
    }

    fn play_at(x: i32, y: i32) -> Playtest {
        Playtest {
            on: true,
            hero: (x, y),
            stats: Hero::default(),
            bag: HashMap::new(),
            flags: HashMap::new(),
            vars: HashMap::new(),
            once_done: HashSet::new(),
            broken_tiles: HashSet::new(),
            pending_floor: None,
            pending_goto: None,
            dir: 2,
            frame: 0,
            move_cd: 0.0,
            dialog: None,
        }
    }

    fn enemies_map() -> HashMap<String, crate::db::Enemy> {
        [("slime", 90, 70, 20, 12, 12)]
            .into_iter()
            .map(|(id, hp, atk, def, gold, exp)| {
                (
                    id.to_string(),
                    crate::db::Enemy {
                        id: id.to_string(),
                        name: id.to_string(),
                        hp,
                        atk,
                        def,
                        mdef: 0,
                        gold,
                        exp,
                        battler: String::new(),
                        hue: 0,
                        sprite: crate::db::SpriteRef {
                            file: String::new(),
                            hue: 0,
                            col: 0,
                            row: 0,
                            opacity: 255,
                        },
                        skills: Vec::new(),
                        extra: Default::default(),
                    },
                )
            })
            .collect()
    }

    fn monster_at(x: i32, y: i32) -> Instance {
        Instance {
            id: "m1".to_string(),
            template: TemplateKind::Monster {
                monster_id: "slime".to_string(),
            },
            x,
            y,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        }
    }

    #[test]
    fn monster_fight_wins_and_loots() {
        let mut f = empty_floor();
        f.instances.push(monster_at(1, 0));
        let enemies = enemies_map();
        let doors = doors_map();
        let barriers = barriers_map();
        let mut play = play_at(0, 0);
        play.stats.hp = 1000;
        play.stats.atk = 100;
        play.stats.def = 50;
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: Some(&rules),
        };
        // 怪90血/攻70/防20：每击80，90/80 不整除 → 1回合，(70-50)*1=20
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert!(f.instances.is_empty());
        assert_eq!(play.stats.hp, 980);
        assert_eq!(play.stats.gold, 12);
        assert_eq!(play.stats.exp, 12);
    }

    #[test]
    fn monster_fight_blocks_when_damage_is_deadly() {
        let mut f = empty_floor();
        f.instances.push(monster_at(1, 0));
        let enemies = enemies_map();
        let doors = doors_map();
        let barriers = barriers_map();
        let mut play = play_at(0, 0);
        play.stats.hp = 10;
        play.stats.atk = 100;
        play.stats.def = 50;
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: Some(&rules),
        };
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (0, 0));
        assert_eq!(f.instances.len(), 1);
        assert!(status.contains("会战败"));
    }

    fn npc_at(x: i32, y: i32, once: bool) -> Instance {
        Instance {
            id: "n1".to_string(),
            template: TemplateKind::Npc {
                dialog: vec!["你好".to_string()],
                shop_id: None,
                route: Vec::new(),
            },
            x,
            y,
            lifecycle: if once { Some(Lifecycle::Once) } else { None },
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        }
    }

    fn empty_move_ctx<'a>(
        doors: &'a HashMap<String, crate::db::Door>,
        barriers: &'a HashMap<String, crate::db::Barrier>,
        enemies: &'a HashMap<String, crate::db::Enemy>,
    ) -> MoveCtx<'a> {
        MoveCtx {
            doors,
            barriers,
            enemies,
            tiles: None,
            rules: None,
        }
    }

    #[test]
    fn persistent_npc_blocks_and_stays() {
        let mut f = empty_floor();
        f.instances.push(npc_at(1, 0, false));
        let (doors, barriers, enemies) = (doors_map(), barriers_map(), HashMap::new());
        let ctx = empty_move_ctx(&doors, &barriers, &enemies);
        let mut play = play_at(0, 0);
        let mut status = String::new();
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (0, 0));
        assert_eq!(f.instances.len(), 1);
        assert!(status.contains("NPC"));
        // 对话开着、说完不走
        let d = play.dialog.as_ref().expect("dialog");
        assert!(d.vanish.is_none());
        finish_dialog(&mut f, &mut play);
        assert!(play.dialog.is_none());
        assert_eq!(f.instances.len(), 1);
    }

    #[test]
    fn once_npc_vanishes_after_dialog() {
        let mut f = empty_floor();
        f.instances.push(npc_at(1, 0, true));
        let (doors, barriers, enemies) = (doors_map(), barriers_map(), HashMap::new());
        let ctx = empty_move_ctx(&doors, &barriers, &enemies);
        let mut play = play_at(0, 0);
        let mut status = String::new();
        // 对话先占住：NPC 还在，勇士原地不动
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(f.instances.len(), 1);
        assert_eq!(play.dialog.as_ref().unwrap().vanish.as_deref(), Some("n1"));
        // 说完才消失，并记进本局一次性记录
        finish_dialog(&mut f, &mut play);
        assert!(f.instances.is_empty());
        assert!(play.once_done.contains("n1"));
        // 下一秒就能走过该格
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
    }

    #[test]
    fn stairs_set_pending_floor() {
        let mut f = empty_floor();
        f.instances.push(Instance {
            id: "s1".to_string(),
            template: TemplateKind::StairUp {
                to_floor: "1_2".to_string(),
                to_landing: "下楼梯".to_string(),
            },
            x: 1,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        let (doors, barriers, enemies) = (doors_map(), barriers_map(), HashMap::new());
        let ctx = empty_move_ctx(&doors, &barriers, &enemies);
        let mut play = play_at(0, 0);
        let mut status = String::new();
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert_eq!(
            play.pending_floor,
            Some(("1_2".to_string(), "下楼梯".to_string()))
        );
    }

    #[test]
    fn apply_once_done_filters_consumed_instances() {
        let mut f = empty_floor();
        f.instances.push(npc_at(1, 0, true));
        let mut once = HashSet::new();
        once.insert("n1".to_string());
        apply_once_done(&mut f, &once);
        assert!(f.instances.is_empty());
    }

    fn test_item(
        id: &str,
        kind: crate::db::ItemKind,
        breaks: &[&str],
        radius: i64,
    ) -> crate::db::Item {
        crate::db::Item {
            id: id.to_string(),
            name: id.to_string(),
            auto_use: false,
            reusable: false,
            passive: false,
            description: String::new(),
            sprite: crate::db::SpriteRef {
                file: String::new(),
                hue: 0,
                col: 0,
                row: 0,
                opacity: 255,
            },
            kind,
            breaks: breaks.iter().map(|s| s.to_string()).collect(),
            break_radius: radius,
            break_tiles: Vec::new(),
            break_layer: 1,
        }
    }

    fn wall_at(x: i32, y: i32) -> Instance {
        Instance {
            id: format!("w{x}{y}"),
            template: TemplateKind::Door {
                door_id: "dark_wall".to_string(),
            },
            x,
            y,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        }
    }

    #[test]
    fn potion_heals_caps_and_consumes() {
        let mut f = empty_floor();
        let mut play = play_at(0, 0);
        play.stats.hp = 950;
        play.stats.hp_max = 1000;
        play.bag.insert("red_potion".to_string(), 2);
        let item = test_item(
            "red_potion",
            crate::db::ItemKind::Potion { heal: 75 },
            &[],
            0,
        );
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert_eq!(play.stats.hp, 1000);
        assert_eq!(play.bag.get("red_potion"), Some(&1));
        assert!(status.contains("生命 +50"));
    }

    #[test]
    fn pickaxe_breaks_facing_wall_only() {
        let mut f = empty_floor();
        f.instances.push(wall_at(1, 0));
        let mut play = play_at(0, 0);
        play.dir = 6; // 朝右
        let item = test_item("pickaxe", crate::db::ItemKind::Tool, &["dark_wall"], 0);
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        // 没带镐子：不该破坏
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert_eq!(f.instances.len(), 1);
        // 带镐子：破面前的墙并消耗；破掉的墙记进本局记录（切层不再长回来）
        play.bag.insert("pickaxe".to_string(), 1);
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert!(f.instances.is_empty());
        assert!(!play.bag.contains_key("pickaxe"));
        assert!(play.once_done.contains("w10"));
        // 再拿一把对空地用：不消耗，提示没有目标
        play.bag.insert("pickaxe".to_string(), 1);
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert!(status.contains("没有能破坏"));
        assert_eq!(play.bag.get("pickaxe"), Some(&1));
    }

    #[test]
    fn quake_scroll_breaks_whole_floor() {
        let mut f = empty_floor();
        f.instances.push(wall_at(1, 0));
        f.instances.push(wall_at(2, 0));
        let mut play = play_at(0, 0);
        play.bag.insert("quake_scroll".to_string(), 1);
        let item = test_item(
            "quake_scroll",
            crate::db::ItemKind::Tool,
            &["dark_wall"],
            -1,
        );
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert!(f.instances.is_empty());
        assert!(status.contains("破坏 2 处"));
        assert!(play.once_done.contains("w10") && play.once_done.contains("w20"));
        // 切层重进（重新从磁盘载入同层）也不会再出现
        f.instances.push(wall_at(1, 0));
        f.instances.push(wall_at(2, 0));
        apply_once_done(&mut f, &play.once_done);
        assert!(f.instances.is_empty());
    }

    #[test]
    fn pickaxe_clears_wall_tile_and_remembers() {
        let mut f = empty_floor();
        f.layers = vec![vec![vec![395; 5]; 5], vec![vec![0; 5]; 5]];
        f.set_tile(1, 0, 1, 389); // 面前一格是墙图块
        let mut play = play_at(0, 0);
        play.dir = 6;
        play.bag.insert("pickaxe".to_string(), 1);
        let mut item = test_item("pickaxe", crate::db::ItemKind::Tool, &["dark_wall"], 0);
        item.break_tiles = vec![389, 390, 391];
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert_eq!(f.tile(1, 0, 1), 0);
        assert!(status.contains("清地形 1 格"));
        assert!(play.broken_tiles.contains(&("t".to_string(), 1, 0, 1)));
        // 切层重进重放破坏记录
        f.set_tile(1, 0, 1, 389);
        apply_broken_tiles(&mut f, &play.broken_tiles);
        assert_eq!(f.tile(1, 0, 1), 0);
    }

    #[test]
    fn quake_scroll_clears_all_wall_tiles() {
        let mut f = empty_floor();
        f.layers = vec![vec![vec![395; 5]; 5], vec![vec![0; 5]; 5]];
        f.set_tile(1, 0, 1, 389);
        f.set_tile(4, 4, 1, 391);
        let mut play = play_at(0, 0);
        play.bag.insert("quake_scroll".to_string(), 1);
        let mut item = test_item(
            "quake_scroll",
            crate::db::ItemKind::Tool,
            &["dark_wall"],
            -1,
        );
        item.break_tiles = vec![389, 390, 391];
        let rules = Rules::embedded().unwrap();
        let mut status = String::new();
        use_item(&mut f, &mut play, Some(&rules), &item, &mut status);
        assert_eq!(f.tile(1, 0, 1), 0);
        assert_eq!(f.tile(4, 4, 1), 0);
        assert_eq!(play.broken_tiles.len(), 2);
    }

    #[test]
    fn intro_goto_sets_pending_after_dialog() {
        let mut f = empty_floor();
        let mut play = play_at(0, 0);
        play.dialog = Some(Dialog {
            lines: vec!["开场白".to_string()],
            page: 0,
            vanish: None,
            goto: Some(("0_0".to_string(), 12, 1)),
        });
        finish_dialog(&mut f, &mut play);
        assert_eq!(play.pending_goto, Some(("0_0".to_string(), 12, 1)));
        assert!(play.dialog.is_none());
    }

    fn barriers_map() -> HashMap<String, crate::db::Barrier> {
        [("net_4", "网", 4, 100)]
            .into_iter()
            .map(|(id, name, code, damage)| {
                (
                    id.to_string(),
                    crate::db::Barrier {
                        id: id.to_string(),
                        name: name.to_string(),
                        code,
                        damage,
                        effect: "hurt".to_string(),
                        sprite: crate::db::SpriteRef {
                            file: "002-npc02".to_string(),
                            hue: 0,
                            col: 0,
                            row: 0,
                            opacity: 255,
                        },
                        description: String::new(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn door_blocks_and_item_pickup() {
        let mut f = empty_floor();
        f.instances.push(door_at(1, 0));
        f.instances.push(Instance {
            id: "k".to_string(),
            template: TemplateKind::Item {
                item_id: "yellow_key".to_string(),
            },
            x: 0,
            y: 1,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        let mut play = play_at(0, 0);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: None,
        };
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (0, 0));
        assert!(step_hero(&mut f, &mut play, &ctx, 0, 1, &mut status));
        assert_eq!(play.hero, (0, 1));
        assert_eq!(play.bag.get("yellow_key"), Some(&1));
        assert_eq!(f.instances.len(), 1);
    }

    #[test]
    fn door_opens_with_key_and_consumes() {
        let mut f = empty_floor();
        f.instances.push(door_at(1, 0));
        let mut play = play_at(0, 0);
        play.bag.insert("yellow_key".to_string(), 2);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: None,
        };
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert_eq!(play.bag.get("yellow_key"), Some(&1));
        assert!(f.instances.is_empty());
    }

    #[test]
    fn box_pushes_and_plate_flag_syncs() {
        let mut f = empty_floor();
        f.instances.push(Instance {
            id: "b".to_string(),
            template: TemplateKind::Box,
            x: 1,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        f.instances.push(Instance {
            id: "p".to_string(),
            template: TemplateKind::Plate {
                flag: "箱子到位".to_string(),
            },
            x: 2,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        let mut play = play_at(0, 0);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: None,
        };
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert_eq!(play.flags.get("箱子到位"), Some(&true));
        // 再推一格，箱子离开压力板，开关回假
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (2, 0));
        assert_eq!(play.flags.get("箱子到位"), Some(&false));
    }

    #[test]
    fn box_blocked_at_wall_or_by_door() {
        let mut f = empty_floor();
        f.instances.push(Instance {
            id: "b".to_string(),
            template: TemplateKind::Box,
            x: 1,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        f.instances.push(door_at(2, 0));
        let mut play = play_at(0, 0);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: None,
        };
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (0, 0));
    }

    #[test]
    fn barrier_passes_with_damage_message() {
        let mut f = empty_floor();
        f.instances.push(Instance {
            id: "b".to_string(),
            template: TemplateKind::Barrier {
                barrier_id: "net_4".to_string(),
            },
            x: 1,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        });
        let mut play = play_at(0, 0);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            rules: None,
        };
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert!(status.contains("100"));
    }

    #[test]
    fn tile_passage_blocks_and_allows() {
        // 地面 395 可走，墙 424 禁行（7630 实测值）。
        let mut tiles = crate::tiles::Tileset {
            id: "t".to_string(),
            name: "t".to_string(),
            passages: vec![0; 688],
            priorities: vec![0; 688],
            terrains: vec![0; 688],
        };
        tiles.passages[424] = 15;
        let mut f = empty_floor();
        f.layers = vec![vec![vec![395; 5]; 5]];
        f.layers[0][0][1] = 424;
        let mut play = play_at(0, 0);
        let mut status = String::new();
        let doors = doors_map();
        let barriers = barriers_map();
        let enemies = HashMap::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: Some(&tiles),
            rules: None,
        };
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(status, "图块挡路");
        assert!(step_hero(&mut f, &mut play, &ctx, 0, 1, &mut status));
        assert_eq!(play.hero, (0, 1));
    }

    #[test]
    fn paint_tile_grows_layers_and_clamps_bounds() {
        let mut f = empty_floor();
        f.paint_tile(1, 1, 2, 500);
        assert_eq!(f.layers.len(), 3);
        assert_eq!(f.tile(1, 1, 2), 500);
        assert_eq!(f.tile(0, 0, 2), 0);
        // 越界落笔静默跳过
        f.paint_tile(-1, 0, 0, 500);
        f.paint_tile(9, 9, 0, 500);
        assert_eq!(f.tile(-1, 0, 0), 0);
    }
}
