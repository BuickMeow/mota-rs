//! 中央画布：7630 真贴图渲染（图块三层＋实例精灵＋勇士）。
//!
//! 编辑模式点格盖章；试玩时主画布只看地图（勇士/走路键都在独立游戏窗口里）。

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use mota_core::map::{Floor, Instance, TemplateKind};
use mota_core::rules::Rules;
use mota_core::runtime::{
    Dialog, MoveCtx, Playtest, finish_dialog, step_hero, usable_items, use_item,
};

use crate::gfx::{self, Textures};
use crate::palette::{BrushKind, PaletteState};
use crate::sprites;

pub const CELL: f32 = 32.0;

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
        if floor.instances_at(cx, cy).is_empty() {
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
                floor.paint_tile(cx + *dx as i32, cy + *dy as i32, z, *tid);
            }
            let first = cells.first().map(|c| c.2).unwrap_or(0);
            *status = format!("绘制 L{} ({cx},{cy}) #{} x{}", z + 1, first, cells.len());
        } else if palette.kind == BrushKind::Eraser {
            let n = floor.erase_at(cx, cy);
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
    /// 规则引擎（试玩碰怪/道具/每步触发用）。
    pub rules: Option<&'a Rules>,
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
        rules,
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
                    rules,
                };
                if step_hero(floor, play, &ctx, dx, dy, status) {
                    play.frame = (play.frame + 1) % 4;
                    // 迟缓（7630 var56）：走路变慢
                    play.move_cd = if play.flags.get("slow").copied().unwrap_or(false) {
                        0.30
                    } else {
                        0.16
                    };
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
                use_item(floor, play, rules, item, status);
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

    #[test]
    fn event_rect_shrinks_to_three_quarters_centered() {
        let cell = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(32.0, 32.0));
        let r = event_rect(cell);
        assert_eq!(r.width(), 24.0);
        assert_eq!(r.height(), 24.0);
        assert_eq!(r.center(), cell.center());
    }
}
