//! 中央画布：7630 真贴图渲染（图块三层＋实例精灵＋勇士）。
//!
//! 编辑模式点格盖章；试玩时主画布只看地图（勇士/走路键都在独立游戏窗口里）。

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use mota_core::map::{Floor, Instance, Lifecycle, TemplateKind};

use crate::gfx::{self, Textures};
use crate::palette::{BrushKind, PaletteState};
use crate::sprites;

pub const CELL: f32 = 32.0;

/// 极简试玩状态（正式规则引擎落地前先能走能捡；战斗走 Lua 规则）。
#[derive(Debug, Clone, Default)]
pub struct Playtest {
    pub on: bool,
    pub hero: (i32, i32),
    /// 勇士战斗属性（血攻防魔防/金币经验），碰怪结算用。
    pub stats: mota_core::battle::Hero,
    pub bag: HashMap<String, i64>,
    pub flags: HashMap<String, bool>,
    /// 本局已消费的一次性实例 id（Once）：切层重进不再出现。
    pub once_done: std::collections::HashSet<String>,
    /// 本局已清掉的地形图块 (层id, x, y, z)：切层重进不再长回来。
    pub broken_tiles: std::collections::HashSet<(String, i32, i32, usize)>,
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
    pub fn start(spawn: Option<(i32, i32)>, stats: mota_core::battle::Hero) -> Self {
        Self {
            on: true,
            hero: spawn.unwrap_or((0, 0)),
            stats,
            bag: HashMap::new(),
            flags: HashMap::new(),
            once_done: std::collections::HashSet::new(),
            broken_tiles: std::collections::HashSet::new(),
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
pub fn apply_once_done(floor: &mut Floor, once: &std::collections::HashSet<String>) {
    floor.instances.retain(|inst| !once.contains(&inst.id));
}

/// 切层前重放本局已破坏的地形（工具清掉的墙/冰/熔岩）。
pub fn apply_broken_tiles(
    floor: &mut Floor,
    broken: &std::collections::HashSet<(String, i32, i32, usize)>,
) {
    let id = floor.id.clone();
    for (fid, x, y, z) in broken {
        if fid == &id {
            floor.set_tile(*x, *y, *z, 0);
        }
    }
}

/// 双击打开的详情窗（格坐标）。
pub type EventCell = (i32, i32);

/// 双击回滚快照：记下单击前的楼层，双击事件时撤销这一拍单击的落笔
/// （事件精灵小一圈后，双击查看详情不应顺带盖章/画画）。
#[derive(Debug, Clone)]
pub struct ClickSnap {
    pub cell: EventCell,
    pub floor: Floor,
}

/// 打包画布参数（超 7 参按规约收拢）。
pub struct CanvasIn<'a> {
    pub floor: &'a mut Floor,
    pub tex: &'a mut Option<Textures>,
    pub gfx_dir: &'a Path,
    pub palette: &'a PaletteState,
    pub selected: &'a mut Option<(i32, i32)>,
    pub next_seq: &'a mut usize,
    pub status: &'a mut String,
    pub play: &'a mut Playtest,
    pub doors: &'a HashMap<String, mota_core::db::Door>,
    pub enemies: &'a HashMap<String, mota_core::db::Enemy>,
    pub items: &'a HashMap<String, mota_core::db::Item>,
    pub barriers: &'a HashMap<String, mota_core::db::Barrier>,
    /// 双击事件打开的详情窗（None=关）。
    pub event_detail: &'a mut Option<EventCell>,
    /// 双击判定用的单击快照。
    pub click_snap: &'a mut Option<ClickSnap>,
    pub zoom: f32,
    pub show_grid: bool,
}

/// 找出某格的全部实例下标。
pub fn instances_at(floor: &Floor, x: i32, y: i32) -> Vec<usize> {
    floor
        .instances
        .iter()
        .enumerate()
        .filter(|(_, inst)| inst.x == x && inst.y == y)
        .map(|(i, _)| i)
        .collect()
}

/// 清除某格全部实例，返回清掉的数量。
pub fn erase_at(floor: &mut Floor, x: i32, y: i32) -> usize {
    let before = floor.instances.len();
    floor.instances.retain(|inst| !(inst.x == x && inst.y == y));
    before - floor.instances.len()
}

/// 图块落笔：自动扩层，越界格跳过。
pub fn paint_tile(floor: &mut Floor, x: i32, y: i32, z: usize, tid: i32) {
    if x < 0 || y < 0 || x >= floor.width as i32 || y >= floor.height as i32 {
        return;
    }
    while floor.layers.len() <= z {
        floor
            .layers
            .push(vec![vec![0; floor.width as usize]; floor.height as usize]);
    }
    floor.layers[z][y as usize][x as usize] = tid;
}

/// 实例在状态栏里的单字标记（贴图缺失时的文字兜底也用它）。
pub fn cell_glyph(inst: &Instance) -> &'static str {
    match &inst.template {
        TemplateKind::Npc { .. } => "N",
        TemplateKind::Monster { .. } => "怪",
        TemplateKind::Item { .. } => "物",
        TemplateKind::Barrier { .. } => "障",
        TemplateKind::Door { .. } => "门",
        TemplateKind::StairUp { .. } => "上",
        TemplateKind::StairDown { .. } => "下",
        TemplateKind::Landing { .. } => "落",
        TemplateKind::Box => "箱",
        TemplateKind::Plate { .. } => "板",
    }
}

fn cell_rect(origin: egui::Pos2, x: i32, y: i32, cell: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        origin + egui::vec2(x as f32 * cell, y as f32 * cell),
        egui::vec2(cell, cell),
    )
}

/// 事件标记框：居中内缩一圈（32→24，每边缩 1/8），图形保持原大小，
/// 只在外面套一个矩形，跟普通图块区分（RMXP 编辑器同款标记）。
pub fn event_rect(cell: egui::Rect) -> egui::Rect {
    cell.shrink(cell.width() / 8.0)
}

/// 落脚点是否被同格楼梯盖住（盖住就不画落脚点，避免重叠渲染）。
fn landing_covered(floor: &Floor, x: i32, y: i32) -> bool {
    floor.instances.iter().any(|o| {
        o.x == x
            && o.y == y
            && matches!(
                o.template,
                TemplateKind::StairUp { .. } | TemplateKind::StairDown { .. }
            )
    })
}

/// 实例绘制参数（超 7 参按规约收拢）。
struct DrawIn<'a> {
    tex: &'a mut Textures,
    ctx: &'a egui::Context,
    gfx_dir: &'a Path,
    painter: &'a egui::Painter,
    inst: &'a Instance,
    enemies: &'a HashMap<String, mota_core::db::Enemy>,
    items: &'a HashMap<String, mota_core::db::Item>,
    doors: &'a HashMap<String, mota_core::db::Door>,
    barriers: &'a HashMap<String, mota_core::db::Barrier>,
    dst: egui::Rect,
    /// 编辑模式：图形照常画，外套一个内缩的事件标记框；试玩模式不画。
    marker: bool,
    /// 停止时动画：给行走图的列号追加的帧相位（0..3）。
    anim_phase: Option<usize>,
}

/// 事件标记框颜色（RMXP 编辑器的事件蓝框）。
const EVENT_MARK: egui::Color32 = egui::Color32::from_rgb(80, 170, 255);

fn draw_instance(d: DrawIn<'_>) {
    match sprites::sprite_of(
        &d.inst.template,
        d.inst.visual.as_ref(),
        d.enemies,
        d.items,
        d.doors,
        d.barriers,
    ) {
        Some(sprites::Sprite::Char(s)) => {
            let tint =
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, s.opacity.clamp(0, 255) as u8);
            let col = d
                .anim_phase
                .map_or(s.col, |p| ((s.col as usize + p) % 4) as u32);
            gfx::draw_char(
                d.tex, d.ctx, d.gfx_dir, d.painter, &s.file, s.hue, col, s.row, d.dst, tint,
            );
        }
        Some(sprites::Sprite::Tile(tid)) => {
            if tid == 0 {
                // 压力板等无贴图事件：画个小琥珀块，不然编辑时看不见。
                let r = event_rect(d.dst).shrink(d.dst.width() / 8.0);
                d.painter
                    .rect_filled(r, 2.0, egui::Color32::from_rgb(255, 200, 80));
            } else {
                gfx::draw_tile_id(d.tex, d.painter, tid, d.dst, egui::Color32::WHITE);
            }
        }
        // DB 无此 id：大红块警示，不兜底成别的图。
        None => {
            d.painter.rect_filled(d.dst, 0.0, egui::Color32::RED);
        }
    }
    if d.marker {
        d.painter.rect_stroke(
            event_rect(d.dst),
            0.0,
            egui::Stroke::new(1.5, EVENT_MARK),
            egui::StrokeKind::Inside,
        );
    }
}

/// 移动判定上下文（超 7 参按规约收拢；tiles 为空＝不判图块，只判实例）。
pub struct MoveCtx<'a> {
    pub doors: &'a HashMap<String, mota_core::db::Door>,
    pub barriers: &'a HashMap<String, mota_core::db::Barrier>,
    pub enemies: &'a HashMap<String, mota_core::db::Enemy>,
    pub tiles: Option<&'a mota_core::tiles::Tileset>,
    /// 战斗规则 Lua 源码（None＝战斗未载入，碰怪只提示）。
    pub battle_lua: Option<&'a str>,
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
        use mota_core::tiles::reverse_dir;
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
    let idxs = instances_at(floor, nx, ny);
    let mut pickup: Option<(usize, String)> = None;
    let mut open_door: Option<(usize, String, i64)> = None;
    let mut push_box: Option<(usize, i32, i32)> = None;
    // (实例下标, 战报, 金币, 经验)：先算好，走完循环再统一落账。
    let mut kill: Option<(usize, mota_core::battle::FightReport, i64, i64)> = None;
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
                let Some(lua) = ctx.battle_lua else {
                    *status = format!("遭遇：{monster_id}（战斗规则未载入）");
                    return false;
                };
                let report = match mota_core::battle::fight(lua, &play.stats, enemy, &play.bag) {
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
                kill = Some((i, report, enemy.gold, enemy.exp));
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
                if inst.lifecycle() == mota_core::map::Lifecycle::Once {
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
    } else if let Some((i, report, gold, exp)) = kill {
        if i < floor.instances.len() {
            let inst = &floor.instances[i];
            let (id, lc) = (inst.id.clone(), inst.lifecycle());
            floor.instances.remove(i);
            if lc == Lifecycle::Once {
                play.once_done.insert(id.clone());
            }
            play.stats.hp -= report.damage;
            play.stats.gold += gold;
            play.stats.exp += exp;
            *status = format!("击败 {id}：-{}血 +{}金 +{}经验", report.damage, gold, exp);
        }
    } else {
        *status = format!("({nx},{ny})");
    }
    sync_plates(floor, play);
    true
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

pub fn show_canvas(ui: &mut egui::Ui, inp: CanvasIn<'_>) {
    let CanvasIn {
        floor,
        tex,
        gfx_dir,
        palette,
        selected,
        next_seq,
        status,
        play,
        doors,
        enemies,
        items,
        barriers,
        event_detail,
        click_snap,
        zoom,
        show_grid,
    } = inp;
    let ctx = ui.ctx().clone();
    let tex = tex.get_or_insert_with(|| Textures::load(&ctx, gfx_dir));
    // 点对点再缩放：逻辑尺寸先除系统 PPP（1 纹理像素＝1 物理像素），再乘 zoom。
    let cell = CELL * zoom / ctx.pixels_per_point().max(f32::EPSILON);

    let w = floor.width.min(60) as i32;
    let h = floor.height.min(60) as i32;
    ui.label(format!(
        "{}（{}x{}）{}",
        floor.name,
        floor.width,
        floor.height,
        if play.on {
            " · 试玩中，请看游戏窗口"
        } else {
            ""
        }
    ));
    // 试玩中的主窗口提示（走路/瞬移请去独立游戏窗口）。
    if play.on {
        ui.label("试玩中，请看游戏窗口");
    }

    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(w as f32 * cell, h as f32 * cell),
        egui::Sense::click(),
    );
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));

    // 三层图块
    for z in 0..3 {
        for y in 0..h {
            for x in 0..w {
                let tid = floor.tile(x, y, z);
                if tid != 0 {
                    gfx::draw_tile_id(
                        tex,
                        &painter,
                        tid,
                        cell_rect(rect.min, x, y, cell),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
    }
    // 网格
    if show_grid {
        let c = egui::Color32::from_gray(90);
        for x in 0..=w {
            let px = rect.min.x + x as f32 * cell;
            painter.line_segment(
                [
                    egui::pos2(px, rect.min.y),
                    egui::pos2(px, rect.min.y + h as f32 * cell),
                ],
                egui::Stroke::new(1.0, c),
            );
        }
        for y in 0..=h {
            let py = rect.min.y + y as f32 * cell;
            painter.line_segment(
                [
                    egui::pos2(rect.min.x, py),
                    egui::pos2(rect.min.x + w as f32 * cell, py),
                ],
                egui::Stroke::new(1.0, c),
            );
        }
    }
    // 实例精灵（落脚点被同格楼梯盖住时跳过，不重叠渲染）
    for inst in floor.instances.iter() {
        let (ix, iy) = (inst.x, inst.y);
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            continue;
        }
        if matches!(inst.template, TemplateKind::Landing { .. }) && landing_covered(floor, ix, iy) {
            continue;
        }
        draw_instance(DrawIn {
            tex,
            ctx: &ctx,
            gfx_dir,
            painter: &painter,
            inst,
            enemies,
            items,
            doors,
            barriers,
            dst: cell_rect(rect.min, ix, iy, cell),
            marker: true,
            anim_phase: None,
        });
    }
    // 出生点：地上画 S 标志（编辑模式；试玩走位在游戏窗口里画）
    if !play.on
        && let Some((sx, sy)) = floor.spawn
    {
        let dst = cell_rect(rect.min, sx, sy, cell);
        painter.rect_filled(
            dst,
            2.0,
            egui::Color32::from_rgba_unmultiplied(120, 255, 120, 40),
        );
        painter.rect_stroke(
            dst,
            2.0,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 220, 120)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            dst.center(),
            egui::Align2::CENTER_CENTER,
            "S",
            egui::FontId::proportional(cell * 0.7),
            egui::Color32::from_rgb(160, 255, 160),
        );
    }
    // 选中框
    if let Some((sx, sy)) = *selected {
        painter.rect_stroke(
            cell_rect(rect.min, sx, sy, cell),
            0.0,
            egui::Stroke::new(2.0, egui::Color32::YELLOW),
            egui::StrokeKind::Outside,
        );
    }

    // 试玩走路键已搬进独立游戏窗口，主窗口不再响应（只看地图）

    let click_cell = resp.hover_pos().and_then(|p| {
        let cx = ((p.x - rect.min.x) / cell) as i32;
        let cy = ((p.y - rect.min.y) / cell) as i32;
        (cx >= 0 && cy >= 0 && cx < w && cy < h).then_some((cx, cy))
    });

    // 双击事件：撤销这一拍单击的落笔，打开详情页（RMXP 编辑事件式弹窗）。
    if resp.double_clicked() {
        if play.on {
            *status = "试玩中，请看游戏窗口".to_string();
            return;
        }
        let Some((cx, cy)) = click_cell else { return };
        // 首击若落过笔（画图块/盖章/擦除），双击先回滚，避免查看详情时改动地图
        if let Some(snap) = click_snap.take()
            && snap.cell == (cx, cy)
        {
            *floor = snap.floor;
        }
        if instances_at(floor, cx, cy).is_empty() {
            return;
        }
        *selected = Some((cx, cy));
        *event_detail = Some((cx, cy));
        *status = format!("事件详情 ({cx},{cy})");
        return;
    }

    // 单击：选中＋按当前笔刷落笔
    if resp.clicked() {
        let Some((cx, cy)) = click_cell else { return };
        // 试玩中主画布不吃点击（瞬移请去游戏窗口），只给一句提示
        if play.on {
            *status = "试玩中，请看游戏窗口".to_string();
            return;
        }
        // 落笔前留快照，双击事件时回滚
        *click_snap = Some(ClickSnap {
            cell: (cx, cy),
            floor: floor.clone(),
        });
        *selected = Some((cx, cy));
        if palette.tab == crate::palette::LeftTab::Tiles {
            let z = palette.tiles.layer;
            let cells = crate::palette::tile_brush_cells(&palette.tiles.brush);
            for (dx, dy, tid) in &cells {
                paint_tile(floor, cx + *dx as i32, cy + *dy as i32, z, *tid);
            }
            let first = cells.first().map(|c| c.2).unwrap_or(0);
            *status = format!("绘制 L{} ({cx},{cy}) #{} x{}", z + 1, first, cells.len());
        } else if palette.kind == BrushKind::Eraser {
            let n = erase_at(floor, cx, cy);
            *status = format!("擦除 ({cx},{cy}) x{n}");
        } else if let Some(template) = palette.build_template() {
            *next_seq += 1;
            let id = format!("u{next_seq:04}");
            let probe = Instance {
                id: String::new(),
                template: template.clone(),
                x: cx,
                y: cy,
                lifecycle: None,
                trigger: None,
                visible_flag: None,
                visual: None,
                walk_anime: false,
                step_anime: false,
            };
            let desc = cell_glyph(&probe);
            floor.instances.push(Instance {
                id: id.clone(),
                ..probe
            });
            *status = format!("摆放 {desc} {id} @ ({cx},{cy})");
        }
    }
}

/// 打包游戏窗口参数（副 viewport 内调用，超 7 参按规约收拢）。
pub struct PlayViewIn<'a> {
    pub floor: &'a mut Floor,
    pub play: &'a mut Playtest,
    pub tex: &'a mut Textures,
    pub gfx_dir: &'a Path,
    pub doors: &'a HashMap<String, mota_core::db::Door>,
    pub enemies: &'a HashMap<String, mota_core::db::Enemy>,
    pub items: &'a HashMap<String, mota_core::db::Item>,
    pub barriers: &'a HashMap<String, mota_core::db::Barrier>,
    pub tiles: Option<&'a mota_core::tiles::Tileset>,
    /// 战斗规则 Lua 源码（试玩碰怪用）。
    pub battle_lua: Option<&'a str>,
    pub status: &'a mut String,
    pub zoom: f32,
}

/// 游戏窗口方向键（按住连续走），返回 (dx, dy, 朝向)。
fn play_dir_from_input(vctx: &egui::Context) -> Option<(i32, i32, i32)> {
    vctx.input(|i| {
        use egui::Key as K;
        if i.key_down(K::ArrowUp) || i.key_down(K::W) {
            Some((0, -1, 8))
        } else if i.key_down(K::ArrowDown) || i.key_down(K::S) {
            Some((0, 1, 2))
        } else if i.key_down(K::ArrowLeft) || i.key_down(K::A) {
            Some((-1, 0, 4))
        } else if i.key_down(K::ArrowRight) || i.key_down(K::D) {
            Some((1, 0, 6))
        } else {
            None
        }
    })
}

/// 朝向 → RMXP 行走图行（0 下 / 1 左 / 2 右 / 3 上）。
fn dir_row(dir: i32) -> u32 {
    match dir {
        4 => 1,
        6 => 2,
        8 => 3,
        _ => 0,
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
fn finish_dialog(floor: &mut Floor, play: &mut Playtest) {
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
fn usable_items(
    bag: &HashMap<String, i64>,
    items: &HashMap<String, mota_core::db::Item>,
) -> Vec<String> {
    use mota_core::db::ItemKind;
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
    item: &mota_core::db::Item,
    in_range: impl Fn(i32, i32) -> bool,
) -> usize {
    if item.break_tiles.is_empty() {
        return 0;
    }
    let z = item.break_layer.max(0) as usize;
    let id = floor.id.clone();
    let (w, h) = (floor.width as i32, floor.height as i32);
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            if !in_range(x, y) {
                continue;
            }
            let tid = floor.tile(x, y, z);
            if tid != 0 && item.break_tiles.contains(&tid) {
                floor.set_tile(x, y, z, 0);
                play.broken_tiles.insert((id.clone(), x, y, z));
                n += 1;
            }
        }
    }
    n
}

/// 用一件物品：血瓶加血、宝石加点、工具破坏面向/范围的门与地形。
fn use_item(
    floor: &mut Floor,
    play: &mut Playtest,
    item: &mota_core::db::Item,
    status: &mut String,
) {
    use mota_core::db::{GemStat, ItemKind};
    if play.bag.get(&item.id).copied().unwrap_or(0) <= 0 {
        *status = format!("{}：背包里没有这件物品", item.name);
        return;
    }
    match &item.kind {
        ItemKind::Potion { heal } => {
            let before = play.stats.hp;
            play.stats.hp = (play.stats.hp + heal).min(play.stats.hp_max);
            *status = format!("{}：生命 +{}", item.name, play.stats.hp - before);
            consume_item(play, &item.id);
        }
        ItemKind::Gem { stat, value } => {
            let label = match stat {
                GemStat::Atk => {
                    play.stats.atk += value;
                    "攻击"
                }
                GemStat::Def => {
                    play.stats.def += value;
                    "防御"
                }
                GemStat::Mdef => {
                    play.stats.mdef += value;
                    "魔防"
                }
                GemStat::Level => {
                    play.stats.level += value;
                    "等级"
                }
            };
            *status = format!("{}：{label} +{value}", item.name);
            consume_item(play, &item.id);
        }
        ItemKind::Cure { .. } => {
            // 状态系统还没上：先提示，不消耗
            *status = format!("{}：现在没有需要解除的状态", item.name);
        }
        ItemKind::Tool if !item.breaks.is_empty() || !item.break_tiles.is_empty() => {
            let (dx, dy) = dir_delta(play.dir);
            let (hx, hy) = play.hero;
            let in_range = |x: i32, y: i32| -> bool {
                if item.break_radius < 0 {
                    true
                } else if item.break_radius == 0 {
                    (x, y) == (hx + dx, hy + dy)
                } else {
                    let r = item.break_radius as i32;
                    (x - hx).abs() <= r && (y - hy).abs() <= r
                }
            };
            let removed = break_doors(floor, &item.breaks, |inst| in_range(inst.x, inst.y));
            let cleared = clear_break_tiles(floor, play, item, in_range);
            if !removed.is_empty() || cleared > 0 {
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
            } else {
                *status = format!("{}：面前没有能破坏的东西", item.name);
            }
        }
        ItemKind::Tool => {
            *status = format!("{}：暂时用不上", item.name);
        }
        _ => {
            *status = format!("{}：不能直接使用", item.name);
        }
    }
}

/// 游戏窗口 HUD：生命/攻防魔/金币经验 + 钥匙 + 物品快捷键。
fn draw_hud(
    painter: &egui::Painter,
    rect: egui::Rect,
    play: &Playtest,
    items: &HashMap<String, mota_core::db::Item>,
) {
    let font = egui::FontId::proportional(13.0);
    let hud = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(180.0, 120.0));
    painter.rect_filled(hud, 4.0, egui::Color32::from_black_alpha(180));
    painter.rect_stroke(
        hud,
        4.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(110)),
        egui::StrokeKind::Inside,
    );
    let x = hud.min.x + 8.0;
    let mut y = hud.min.y + 6.0;
    let mut line = |text: String, color: egui::Color32| {
        painter.text(
            egui::pos2(x, y),
            egui::Align2::LEFT_TOP,
            text,
            font.clone(),
            color,
        );
        y += 17.0;
    };
    let white = egui::Color32::WHITE;
    let hp_color = if play.stats.hp * 3 <= play.stats.hp_max {
        egui::Color32::from_rgb(255, 120, 120)
    } else {
        white
    };
    line(
        format!("生命 {}/{}", play.stats.hp, play.stats.hp_max),
        hp_color,
    );
    line(
        format!("攻击 {}  防御 {}", play.stats.atk, play.stats.def),
        white,
    );
    line(
        format!("魔防 {}  等级 {}", play.stats.mdef, play.stats.level),
        white,
    );
    line(
        format!("金币 {}  经验 {}", play.stats.gold, play.stats.exp),
        egui::Color32::from_gray(200),
    );
    let key = |id: &str| play.bag.get(id).copied().unwrap_or(0);
    line(
        format!(
            "钥匙 黄{} 蓝{} 红{} 绿{} 铁{}",
            key("yellow_key"),
            key("blue_key"),
            key("red_key"),
            key("green_key"),
            key("iron_key")
        ),
        egui::Color32::from_rgb(255, 230, 140),
    );
    let usable = usable_items(&play.bag, items);
    if !usable.is_empty() {
        let text: Vec<String> = usable
            .iter()
            .take(9)
            .enumerate()
            .map(|(n, id)| {
                let name = items.get(id).map_or(id.as_str(), |i| i.name.as_str());
                format!(
                    "{}{}x{}",
                    n + 1,
                    name,
                    play.bag.get(id).copied().unwrap_or(0)
                )
            })
            .collect();
        painter.text(
            egui::pos2(rect.min.x + 8.0, rect.max.y - 22.0),
            egui::Align2::LEFT_TOP,
            text.join("  "),
            font,
            egui::Color32::from_rgb(180, 230, 255),
        );
    }
}

/// 对话窗：底部黑框显示当前句，空格/回车翻页。
fn draw_dialog_box(painter: &egui::Painter, rect: egui::Rect, dialog: &Dialog) {
    let b = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 8.0, rect.max.y - 68.0),
        egui::pos2(rect.max.x - 8.0, rect.max.y - 8.0),
    );
    painter.rect_filled(b, 6.0, egui::Color32::from_black_alpha(225));
    painter.rect_stroke(
        b,
        6.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(140)),
        egui::StrokeKind::Inside,
    );
    let text = dialog.lines.get(dialog.page).map_or("", String::as_str);
    painter.text(
        egui::pos2(b.min.x + 10.0, b.min.y + 10.0),
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(15.0),
        egui::Color32::WHITE,
    );
    let hint = if dialog.page + 1 < dialog.lines.len() {
        "空格 ▼"
    } else {
        "空格 关闭"
    };
    painter.text(
        egui::pos2(b.max.x - 10.0, b.max.y - 8.0),
        egui::Align2::RIGHT_BOTTOM,
        hint,
        egui::FontId::proportional(12.0),
        egui::Color32::from_gray(180),
    );
}

/// 独立游戏窗口视图：渲染同一张地图＋勇士，方向键/WASD 走路、点击格瞬移。
/// 必须在子 viewport 的 Ui 里调用（`ui.ctx()` 就是游戏窗口独立的 Context，
/// 行走图懒加载的纹理也落在游戏窗口侧，不跟主窗口混用）。
/// 移动规则复用 [`step_hero`]（签名不变）。
pub fn show_play_view(ui: &mut egui::Ui, inp: PlayViewIn<'_>) {
    let PlayViewIn {
        floor,
        play,
        tex,
        gfx_dir,
        doors,
        enemies,
        items,
        barriers,
        tiles,
        battle_lua,
        status,
        zoom,
    } = inp;
    // 游戏窗口独立 Context（纹理/输入都归它）。
    let vctx = ui.ctx().clone();
    let dt = vctx.input(|i| i.stable_dt).min(0.1) as f64;
    let now = vctx.input(|i| i.time);

    // 输入：对话翻页 > 走路 > 物品快捷键 1..9
    if play.dialog.is_some() {
        let advance = vctx.input(|i| {
            use egui::Key as K;
            i.key_pressed(K::Space)
                || i.key_pressed(K::Enter)
                || i.key_pressed(K::Z)
                || i.key_pressed(K::X)
        });
        if advance {
            let done = {
                let d = play.dialog.as_mut().expect("dialog");
                d.page += 1;
                d.page >= d.lines.len()
            };
            if done {
                finish_dialog(floor, play);
            }
        }
    } else {
        // 走路：挡路/捡物/推箱/碰怪结算走 step_hero；按住方向键连续走
        if let Some((dx, dy, dir)) = play_dir_from_input(&vctx) {
            play.dir = dir;
            play.move_cd -= dt;
            if play.move_cd <= 0.0 {
                let ctx = MoveCtx {
                    doors,
                    barriers,
                    enemies,
                    tiles,
                    battle_lua,
                };
                if step_hero(floor, play, &ctx, dx, dy, status) {
                    play.frame = (play.frame + 1) % 4;
                    play.move_cd = 0.16;
                } else {
                    play.move_cd = 0.22;
                }
            }
        } else {
            play.move_cd = 0.0;
            play.frame = 0;
        }
        // 物品快捷键
        use egui::Key as K;
        let digits = [
            K::Num1,
            K::Num2,
            K::Num3,
            K::Num4,
            K::Num5,
            K::Num6,
            K::Num7,
            K::Num8,
            K::Num9,
        ];
        if let Some(n) = vctx.input(|i| digits.iter().position(|k| i.key_pressed(*k))) {
            let usable = usable_items(&play.bag, items);
            if let Some(id) = usable.get(n)
                && let Some(item) = items.get(id)
            {
                use_item(floor, play, item, status);
            }
        }
    }
    // 点对点再缩放（跟主画布同公式，纹理 NEAREST 保持像素 crisp）。
    let cell = CELL * zoom / vctx.pixels_per_point().max(f32::EPSILON);
    let w = floor.width.min(60) as i32;
    let h = floor.height.min(60) as i32;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(w as f32 * cell, h as f32 * cell),
        egui::Sense::click(),
    );
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
    // 三层图块（同一张地图）
    for z in 0..3 {
        for y in 0..h {
            for x in 0..w {
                let tid = floor.tile(x, y, z);
                if tid != 0 {
                    gfx::draw_tile_id(
                        tex,
                        &painter,
                        tid,
                        cell_rect(rect.min, x, y, cell),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
    }
    // 实例精灵（落脚点被同格楼梯盖住时跳过；停止时动画按 RMXP 慢速循环）
    let phase = ((now * 5.0) as usize) % 4;
    for inst in floor.instances.iter() {
        let (ix, iy) = (inst.x, inst.y);
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            continue;
        }
        if matches!(inst.template, TemplateKind::Landing { .. }) && landing_covered(floor, ix, iy) {
            continue;
        }
        draw_instance(DrawIn {
            tex,
            ctx: &vctx,
            gfx_dir,
            painter: &painter,
            inst,
            enemies,
            items,
            doors,
            barriers,
            dst: cell_rect(rect.min, ix, iy, cell),
            marker: false,
            anim_phase: inst.step_anime.then_some(phase),
        });
    }
    // 勇士（朝向行 + 行走帧列；停下回到站立帧）
    let hero = sprites::hero_sprite();
    gfx::draw_char(
        tex,
        &vctx,
        gfx_dir,
        &painter,
        &hero.file,
        hero.hue,
        play.frame as u32,
        dir_row(play.dir),
        cell_rect(rect.min, play.hero.0, play.hero.1, cell),
        egui::Color32::WHITE,
    );
    // HUD 与对话窗
    draw_hud(&painter, rect, play, items);
    if let Some(d) = &play.dialog {
        draw_dialog_box(&painter, rect, d);
    }
    // 点击格瞬移勇士（对话中不响应）
    if play.dialog.is_none()
        && resp.clicked()
        && let Some((cx, cy)) = resp.hover_pos().and_then(|p| {
            let cx = ((p.x - rect.min.x) / cell) as i32;
            let cy = ((p.y - rect.min.y) / cell) as i32;
            (cx >= 0 && cy >= 0 && cx < w && cy < h).then_some((cx, cy))
        })
    {
        play.hero = (cx, cy);
        *status = format!("瞬移 ({cx},{cy})");
    }
    // 无动画也要重绘，保证按键/瞬移即时刷新
    vctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use mota_core::map::TemplateKind;

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
        assert_eq!(erase_at(&mut f, 1, 1), 2);
        assert_eq!(f.instances.len(), 1);
    }

    fn doors_map() -> HashMap<String, mota_core::db::Door> {
        [("yellow", "黄门", Some("yellow_key"), true)]
            .into_iter()
            .map(|(id, name, key, consume)| {
                (
                    id.to_string(),
                    mota_core::db::Door {
                        id: id.to_string(),
                        name: name.to_string(),
                        key: key.map(str::to_string),
                        consume,
                        free: false,
                        sprite: mota_core::db::SpriteRef {
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
            stats: mota_core::battle::Hero::default(),
            bag: HashMap::new(),
            flags: HashMap::new(),
            once_done: std::collections::HashSet::new(),
            broken_tiles: std::collections::HashSet::new(),
            pending_floor: None,
            pending_goto: None,
            dir: 2,
            frame: 0,
            move_cd: 0.0,
            dialog: None,
        }
    }

    fn enemies_map() -> HashMap<String, mota_core::db::Enemy> {
        [("slime", 90, 70, 20, 12, 12)]
            .into_iter()
            .map(|(id, hp, atk, def, gold, exp)| {
                (
                    id.to_string(),
                    mota_core::db::Enemy {
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
                        sprite: mota_core::db::SpriteRef {
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
        let mut status = String::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            battle_lua: Some(mota_core::battle::EMBEDDED),
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
        let mut status = String::new();
        let ctx = MoveCtx {
            doors: &doors,
            barriers: &barriers,
            enemies: &enemies,
            tiles: None,
            battle_lua: Some(mota_core::battle::EMBEDDED),
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
        doors: &'a HashMap<String, mota_core::db::Door>,
        barriers: &'a HashMap<String, mota_core::db::Barrier>,
        enemies: &'a HashMap<String, mota_core::db::Enemy>,
    ) -> MoveCtx<'a> {
        MoveCtx {
            doors,
            barriers,
            enemies,
            tiles: None,
            battle_lua: None,
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
                to_floor: "m05".to_string(),
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
            Some(("m05".to_string(), "下楼梯".to_string()))
        );
    }

    #[test]
    fn apply_once_done_filters_consumed_instances() {
        let mut f = empty_floor();
        f.instances.push(npc_at(1, 0, true));
        let mut once = std::collections::HashSet::new();
        once.insert("n1".to_string());
        apply_once_done(&mut f, &once);
        assert!(f.instances.is_empty());
    }

    fn test_item(
        id: &str,
        kind: mota_core::db::ItemKind,
        breaks: &[&str],
        radius: i64,
    ) -> mota_core::db::Item {
        mota_core::db::Item {
            id: id.to_string(),
            name: id.to_string(),
            auto_use: false,
            reusable: false,
            passive: false,
            description: String::new(),
            sprite: mota_core::db::SpriteRef {
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
            mota_core::db::ItemKind::Potion { heal: 75 },
            &[],
            0,
        );
        let mut status = String::new();
        use_item(&mut f, &mut play, &item, &mut status);
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
        let item = test_item("pickaxe", mota_core::db::ItemKind::Tool, &["dark_wall"], 0);
        let mut status = String::new();
        // 没带镐子：不该破坏
        use_item(&mut f, &mut play, &item, &mut status);
        assert_eq!(f.instances.len(), 1);
        // 带镐子：破面前的墙并消耗；破掉的墙记进本局记录（切层不再长回来）
        play.bag.insert("pickaxe".to_string(), 1);
        use_item(&mut f, &mut play, &item, &mut status);
        assert!(f.instances.is_empty());
        assert!(!play.bag.contains_key("pickaxe"));
        assert!(play.once_done.contains("w10"));
        // 再拿一把对空地用：不消耗，提示没有目标
        play.bag.insert("pickaxe".to_string(), 1);
        use_item(&mut f, &mut play, &item, &mut status);
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
            mota_core::db::ItemKind::Tool,
            &["dark_wall"],
            -1,
        );
        let mut status = String::new();
        use_item(&mut f, &mut play, &item, &mut status);
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
        let mut item = test_item("pickaxe", mota_core::db::ItemKind::Tool, &["dark_wall"], 0);
        item.break_tiles = vec![389, 390, 391];
        let mut status = String::new();
        use_item(&mut f, &mut play, &item, &mut status);
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
            mota_core::db::ItemKind::Tool,
            &["dark_wall"],
            -1,
        );
        item.break_tiles = vec![389, 390, 391];
        let mut status = String::new();
        use_item(&mut f, &mut play, &item, &mut status);
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
            goto: Some(("m01".to_string(), 12, 1)),
        });
        finish_dialog(&mut f, &mut play);
        assert_eq!(play.pending_goto, Some(("m01".to_string(), 12, 1)));
        assert!(play.dialog.is_none());
    }

    fn barriers_map() -> HashMap<String, mota_core::db::Barrier> {
        [("net_4", "网", 4, 100)]
            .into_iter()
            .map(|(id, name, code, damage)| {
                (
                    id.to_string(),
                    mota_core::db::Barrier {
                        id: id.to_string(),
                        name: name.to_string(),
                        code,
                        damage,
                        effect: "hurt".to_string(),
                        sprite: mota_core::db::SpriteRef {
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
            battle_lua: None,
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
            battle_lua: None,
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
            battle_lua: None,
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
            battle_lua: None,
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
            battle_lua: None,
        };
        assert!(step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(play.hero, (1, 0));
        assert!(status.contains("100"));
    }

    #[test]
    fn tile_passage_blocks_and_allows() {
        // 地面 395 可走，墙 424 禁行（7630 实测值）。
        let mut tiles = mota_core::tiles::Tileset {
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
            battle_lua: None,
        };
        assert!(!step_hero(&mut f, &mut play, &ctx, 1, 0, &mut status));
        assert_eq!(status, "图块挡路");
        assert!(step_hero(&mut f, &mut play, &ctx, 0, 1, &mut status));
        assert_eq!(play.hero, (0, 1));
    }

    #[test]
    fn event_rect_shrinks_to_three_quarters_centered() {
        let cell = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(32.0, 32.0));
        let r = event_rect(cell);
        assert_eq!(r.width(), 24.0);
        assert_eq!(r.height(), 24.0);
        assert_eq!(r.center(), cell.center());
    }

    #[test]
    fn paint_tile_grows_layers_and_clamps_bounds() {
        let mut f = empty_floor();
        paint_tile(&mut f, 1, 1, 2, 500);
        assert_eq!(f.layers.len(), 3);
        assert_eq!(f.tile(1, 1, 2), 500);
        assert_eq!(f.tile(0, 0, 2), 0);
        // 越界落笔静默跳过
        paint_tile(&mut f, -1, 0, 0, 500);
        paint_tile(&mut f, 9, 9, 0, 500);
        assert_eq!(f.tile(-1, 0, 0), 0);
    }
}
