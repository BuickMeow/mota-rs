//! 左侧调色板：图块页（地图元件/自动元件，画进图层）＋ 事件页（预设物件盖章）。

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use mota_core::db::{Barrier, Door, Enemy, Item, SpriteRef};
use mota_core::map::TemplateKind;

use crate::gfx::{AUTOTILE_QUADS, Textures, autotile_is_small, draw_char, uv};

/// 笔刷种类（对应可快速摆放的事件类型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushKind {
    Npc,
    Monster,
    Item,
    Barrier,
    Door,
    Box,
    Stair,
    Plate,
    Eraser,
}

/// 楼梯笔刷的子选项（同一菜单内四缩略图点选，照 RMXP 那样框选往地图盖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StairPick {
    #[default]
    Up,
    Down,
    LandUp,
    LandDown,
}

impl StairPick {
    /// （选项，图块号，悬停名）。
    const ALL: [(Self, i32, &'static str); 4] = [
        (Self::Up, 633, "上楼"),
        (Self::Down, 632, "下楼"),
        (Self::LandUp, 639, "上楼落点"),
        (Self::LandDown, 631, "下楼落点"),
    ];
}

impl BrushKind {
    pub(crate) const ALL: [Self; 9] = [
        Self::Npc,
        Self::Monster,
        Self::Item,
        Self::Barrier,
        Self::Door,
        Self::Box,
        Self::Stair,
        Self::Plate,
        Self::Eraser,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Npc => "NPC",
            Self::Monster => "怪物",
            Self::Item => "物品",
            Self::Barrier => "路障",
            Self::Door => "门",
            Self::Stair => "楼梯",
            Self::Box => "箱子",
            Self::Plate => "压力板",
            Self::Eraser => "橡皮擦",
        }
    }

    /// 参数输入框的提示语。
    pub fn param_hint(self) -> &'static str {
        match self {
            Self::Npc => "对话（第一句），空则沉默",
            Self::Monster => "monster_id，如 electric_slime",
            Self::Item => "item_id，如 yellow_key",
            Self::Barrier => "barrier_id，如 net_4",
            Self::Door => "door_id，如 yellow",
            Self::Stair => "先点缩略图选上下楼/落点，再填 to_floor",
            Self::Box => "沿面向方向推一格（撞墙不动）",
            Self::Plate => "被箱子压住时置真的开关名",
            Self::Eraser => "点格子清除该格全部实例",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaletteState {
    pub kind: BrushKind,
    /// 各笔刷共用一个文本参数（怪物/物品/门 ID 等）。
    pub param: String,
    /// 楼梯第二参数（落脚点名）。
    pub param2: String,
    /// 楼梯子项（四缩略图点选）。
    pub stair_pick: StairPick,
    pub tab: LeftTab,
    pub tiles: TileState,
    drag_anchor: Option<(u32, u32)>,
}

/// 数据库只读视图（事件页缩略图用，超 7 参按规约收拢）。
#[derive(Debug, Clone, Copy)]
pub struct DbView<'a> {
    pub enemies: &'a HashMap<String, Enemy>,
    pub items: &'a HashMap<String, Item>,
    pub doors: &'a HashMap<String, Door>,
    pub barriers: &'a HashMap<String, Barrier>,
}

/// 左侧页签：图块（地图元件/自动元件）或事件（预设物件盖章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    Tiles,
    #[default]
    Events,
}

/// 图块笔刷：自动元件槽、图块集矩形、擦除（画 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileBrush {
    Autotile { slot: usize },
    Tileset { col: u32, row: u32, w: u32, h: u32 },
    Erase,
}

/// 图块绘制状态（目标层＋笔刷）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileState {
    pub layer: usize,
    pub brush: TileBrush,
}

impl Default for TileState {
    fn default() -> Self {
        Self {
            layer: 0,
            brush: TileBrush::Autotile { slot: 0 },
        }
    }
}

/// 自动元件填充号（与 magic-tower-rs 一致：静态满格，无缝重算后续再接）。
pub fn autotile_tile_id(slot: usize) -> i32 {
    48 + slot as i32 * 48
}

/// 图块集 (col,row) → 图块号（8 列）。
pub fn tileset_tile_id(col: u32, row: u32) -> i32 {
    384 + row as i32 * 8 + col as i32
}

/// 图块笔刷覆盖的相对格 → 图块号（画布落笔用）。
pub fn tile_brush_cells(brush: &TileBrush) -> Vec<(u32, u32, i32)> {
    match *brush {
        TileBrush::Autotile { slot } => vec![(0, 0, autotile_tile_id(slot))],
        TileBrush::Tileset { col, row, w, h } => {
            let mut out = Vec::new();
            for dy in 0..h {
                for dx in 0..w {
                    out.push((dx, dy, tileset_tile_id(col + dx, row + dy)));
                }
            }
            out
        }
        TileBrush::Erase => vec![(0, 0, 0)],
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            kind: BrushKind::Monster,
            param: "electric_slime".to_string(),
            param2: "入口".to_string(),
            stair_pick: StairPick::Up,
            tab: LeftTab::Events,
            tiles: TileState::default(),
            drag_anchor: None,
        }
    }
}

/// 选中框：1px 黑＋2px 白＋1px 黑，均为内描边。
fn selection_frame(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(4.0, egui::Color32::BLACK),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(3.0, egui::Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::BLACK),
        egui::StrokeKind::Inside,
    );
}

impl PaletteState {
    /// 按当前笔刷造一个模板；橡皮擦返回 None（调用方走擦除）。
    pub fn build_template(&self) -> Option<TemplateKind> {
        match self.kind {
            BrushKind::Npc => {
                let dialog = if self.param.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![self.param.trim().to_owned()]
                };
                Some(TemplateKind::Npc {
                    dialog,
                    shop_id: None,
                    route: Vec::new(),
                })
            }
            BrushKind::Monster => Some(TemplateKind::Monster {
                monster_id: self.param.trim().to_owned(),
            }),
            BrushKind::Item => Some(TemplateKind::Item {
                item_id: self.param.trim().to_owned(),
            }),
            BrushKind::Barrier => Some(TemplateKind::Barrier {
                barrier_id: self.param.trim().to_owned(),
            }),
            BrushKind::Door => Some(TemplateKind::Door {
                door_id: self.param.trim().to_owned(),
            }),
            BrushKind::Stair => Some(match self.stair_pick {
                // 上楼梯对下落脚点（目标楼层的下楼落点）。
                StairPick::Up => TemplateKind::StairUp {
                    to_floor: self.param.trim().to_owned(),
                    to_landing: "下楼梯".to_owned(),
                },
                // 下楼梯对上落脚点（目标楼层的上楼落点）。
                StairPick::Down => TemplateKind::StairDown {
                    to_floor: self.param.trim().to_owned(),
                    to_landing: "上楼梯".to_owned(),
                },
                StairPick::LandUp => TemplateKind::Landing {
                    name: "上楼梯".to_owned(),
                },
                StairPick::LandDown => TemplateKind::Landing {
                    name: "下楼梯".to_owned(),
                },
            }),
            BrushKind::Box => Some(TemplateKind::Box),
            BrushKind::Plate => Some(TemplateKind::Plate {
                flag: self.param2.trim().to_owned(),
            }),
            BrushKind::Eraser => None,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tex: &mut Option<Textures>,
        gfx_dir: &Path,
        db: DbView<'_>,
    ) {
        use egui_material_icons::icons::*;
        // 笔刷固定两排（图标按钮，中文名放 hover，不占宽）。
        egui::Grid::new("brushes")
            .num_columns(6)
            .spacing([0.0, 2.0])
            .show(ui, |ui| {
                for (n, kind) in BrushKind::ALL.into_iter().enumerate() {
                    let icon = match kind {
                        BrushKind::Npc => ICON_PERSON,
                        BrushKind::Monster => ICON_SKULL,
                        BrushKind::Item => ICON_KEY,
                        BrushKind::Barrier => ICON_TACTIC,
                        BrushKind::Door => ICON_DOOR_SLIDING,
                        BrushKind::Stair => ICON_STAIRS,
                        BrushKind::Box => ICON_BOX,
                        BrushKind::Plate => ICON_PODIATRY,
                        BrushKind::Eraser => ICON_DELETE,
                    };
                    if ui
                        .add(egui::Button::selectable(
                            self.kind == kind,
                            icon.rich_text().size(20.0),
                        ))
                        .on_hover_text(kind.label())
                        .clicked()
                    {
                        self.kind = kind;
                    }
                    if n % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        ui.label(self.kind.param_hint());
        self.show_db_grid(ui, ctx, tex, gfx_dir, db);
        match self.kind {
            BrushKind::Stair => {
                ui.text_edit_singleline(&mut self.param);
                // 四楼梯缩略图（上楼/下楼/上楼落点/下楼落点），点选定子项。
                let tex = tex.get_or_insert_with(|| Textures::load(ctx, gfx_dir));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    for (pick, tid, name) in StairPick::ALL {
                        let (resp, painter) =
                            ui.allocate_painter(egui::vec2(32.0, 32.0), egui::Sense::click());
                        crate::gfx::draw_tile_id(
                            tex,
                            &painter,
                            tid,
                            resp.rect,
                            egui::Color32::WHITE,
                        );
                        if self.stair_pick == pick {
                            selection_frame(&painter, resp.rect);
                        }
                        if resp.clicked() {
                            self.stair_pick = pick;
                        }
                        resp.on_hover_text(name);
                    }
                });
            }
            BrushKind::Box => {}
            BrushKind::Plate => {
                ui.text_edit_singleline(&mut self.param2);
            }
            BrushKind::Eraser => {}
            _ => {
                ui.text_edit_singleline(&mut self.param);
            }
        }
    }

    /// 数据库缩略图（怪/物/门/路障：点选填 id；NPC 复杂、没数据的以后再做）。
    fn show_db_grid(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tex: &mut Option<Textures>,
        gfx_dir: &Path,
        db: DbView<'_>,
    ) {
        let mut list: Vec<(String, String, SpriteRef)> = match self.kind {
            BrushKind::Monster => db
                .enemies
                .iter()
                .map(|(id, e)| (id.clone(), e.name.clone(), e.sprite.clone()))
                .collect(),
            BrushKind::Item => db
                .items
                .iter()
                .map(|(id, i)| (id.clone(), i.name.clone(), i.sprite.clone()))
                .collect(),
            BrushKind::Door => db
                .doors
                .iter()
                .map(|(id, d)| (id.clone(), d.name.clone(), d.sprite.clone()))
                .collect(),
            BrushKind::Barrier => db
                .barriers
                .iter()
                .map(|(id, b)| (id.clone(), b.name.clone(), b.sprite.clone()))
                .collect(),
            _ => return,
        };
        list.sort_by(|a, b| a.0.cmp(&b.0));
        let tex = tex.get_or_insert_with(|| Textures::load(ctx, gfx_dir));
        // 列数跟面板宽走（自动换行，不固定）。
        ui.horizontal_wrapped(|ui| {
            for (id, name, sp) in list {
                let (resp, painter) =
                    ui.allocate_painter(egui::vec2(32.0, 32.0), egui::Sense::click());
                let tint = egui::Color32::from_rgba_unmultiplied(
                    255,
                    255,
                    255,
                    sp.opacity.clamp(0, 255) as u8,
                );
                draw_char(
                    tex, ctx, gfx_dir, &painter, &sp.file, sp.hue, sp.col, sp.row, resp.rect, tint,
                );
                if self.param == id {
                    selection_frame(&painter, resp.rect);
                }
                if resp.clicked() {
                    self.param = id;
                }
                resp.on_hover_text(name);
            }
        });
    }

    /// 图块页：目标层＋自动元件 8 格（含首格空白）＋图块集点选/拖选。
    pub fn show_tiles(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tex: &mut Option<Textures>,
        gfx_dir: &Path,
    ) {
        ui.heading("图块");
        ui.horizontal(|ui| {
            for (z, s) in ["层1", "层2", "层3"].iter().enumerate() {
                ui.radio_value(&mut self.tiles.layer, z, *s);
            }
        });
        let tex = tex.get_or_insert_with(|| Textures::load(ctx, gfx_dir));
        // 自动元件行与地图元件之间无缝隙。
        ui.spacing_mut().item_spacing.y = 0.0;
        // 与地图元件同倍率 32 格、无缝隙拼一行（8×32＝256 同宽）。
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            // 首格空白＝清除（透明无纹理）。
            let (resp, painter) = ui.allocate_painter(egui::vec2(32.0, 32.0), egui::Sense::click());
            if matches!(self.tiles.brush, TileBrush::Erase) {
                selection_frame(&painter, resp.rect);
            }
            if resp.clicked() {
                self.tiles.brush = TileBrush::Erase;
            }
            for slot in 0..7 {
                let at = tex.autotiles.get(slot).and_then(|o| o.clone());
                let size = tex.autotile_px.get(slot).copied().unwrap_or((96, 128));
                let (resp, painter) =
                    ui.allocate_painter(egui::vec2(32.0, 32.0), egui::Sense::click());
                match at {
                    Some(t) => {
                        if autotile_is_small(size) {
                            painter.image(
                                t.id(),
                                resp.rect,
                                uv((0, 0, 32, 32), size),
                                egui::Color32::WHITE,
                            );
                        } else {
                            // 调色板代表格 pattern 47（与 magic-tower-rs 一致）。
                            for (i, (sx, sy)) in AUTOTILE_QUADS[47].iter().enumerate() {
                                let (ox, oy) = ((i % 2) as f32 * 16.0, (i / 2) as f32 * 16.0);
                                painter.image(
                                    t.id(),
                                    egui::Rect::from_min_size(
                                        resp.rect.min + egui::vec2(ox, oy),
                                        egui::vec2(16.0, 16.0),
                                    ),
                                    uv((*sx, *sy, 16, 16), size),
                                    egui::Color32::WHITE,
                                );
                            }
                        }
                    }
                    None => {
                        painter.rect_filled(resp.rect, 0.0, egui::Color32::DARK_GRAY);
                    }
                }
                if matches!(self.tiles.brush, TileBrush::Autotile { slot: s } if s == slot) {
                    selection_frame(&painter, resp.rect);
                }
                if resp.clicked() {
                    self.tiles.brush = TileBrush::Autotile { slot };
                }
            }
        });
        let th = tex.tileset_tiles_high;
        let (resp, painter) = ui.allocate_painter(
            egui::vec2(256.0, th as f32 * 32.0),
            egui::Sense::click_and_drag(),
        );
        painter.image(
            tex.tileset.id(),
            resp.rect,
            uv((0, 0, 256, th * 32), (256, th * 32)),
            egui::Color32::WHITE,
        );
        if let TileBrush::Tileset { col, row, w, h } = self.tiles.brush {
            selection_frame(
                &painter,
                egui::Rect::from_min_size(
                    resp.rect.min + egui::vec2(col as f32 * 32.0, row as f32 * 32.0),
                    egui::vec2(w as f32 * 32.0, h as f32 * 32.0),
                ),
            );
        }
        let to_cell = |pos: egui::Pos2| {
            let d = pos - resp.rect.min;
            if d.x < 0.0 || d.y < 0.0 {
                return None;
            }
            let (c, r) = ((d.x / 32.0) as u32, (d.y / 32.0) as u32);
            if c < 8 && r < th { Some((c, r)) } else { None }
        };
        // 单击即框一格；按下拖拽扩成多格。
        if resp.clicked()
            && let Some(p) = resp.hover_pos().and_then(to_cell)
        {
            self.drag_anchor = Some(p);
            self.tiles.brush = TileBrush::Tileset {
                col: p.0,
                row: p.1,
                w: 1,
                h: 1,
            };
        }
        if resp.drag_started()
            && let Some(p) = resp.hover_pos().and_then(to_cell)
        {
            self.drag_anchor = Some(p);
            self.tiles.brush = TileBrush::Tileset {
                col: p.0,
                row: p.1,
                w: 1,
                h: 1,
            };
        }
        if resp.dragged()
            && let (Some(a), Some(b)) = (self.drag_anchor, resp.hover_pos().and_then(to_cell))
        {
            self.tiles.brush = TileBrush::Tileset {
                col: a.0.min(b.0),
                row: a.1.min(b.1),
                w: a.0.max(b.0) - a.0.min(b.0) + 1,
                h: a.1.max(b.1) - a.1.min(b.1) + 1,
            };
        }
        if resp.drag_stopped() {
            self.drag_anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eraser_builds_nothing() {
        let p = PaletteState {
            kind: BrushKind::Eraser,
            ..PaletteState::default()
        };
        assert!(p.build_template().is_none());
    }

    #[test]
    fn monster_param_goes_to_template() {
        let p = PaletteState {
            kind: BrushKind::Monster,
            param: "electric_slime".to_string(),
            ..PaletteState::default()
        };
        let t = p.build_template().expect("monster template");
        assert!(
            matches!(t, TemplateKind::Monster { monster_id, .. } if monster_id == "electric_slime")
        );
    }

    #[test]
    fn tile_ids_match_rmxp_math() {
        assert_eq!(autotile_tile_id(0), 48);
        assert_eq!(autotile_tile_id(6), 48 + 6 * 48);
        assert_eq!(tileset_tile_id(0, 0), 384);
        assert_eq!(tileset_tile_id(7, 0), 391);
        assert_eq!(tileset_tile_id(0, 1), 392);
    }

    #[test]
    fn tileset_brush_expands_rect() {
        let cells = tile_brush_cells(&TileBrush::Tileset {
            col: 1,
            row: 2,
            w: 2,
            h: 2,
        });
        assert_eq!(cells.len(), 4);
        assert!(cells.contains(&(0, 0, tileset_tile_id(1, 2))));
        assert!(cells.contains(&(1, 1, tileset_tile_id(2, 3))));
        assert_eq!(tile_brush_cells(&TileBrush::Erase), vec![(0, 0, 0)]);
    }
}
