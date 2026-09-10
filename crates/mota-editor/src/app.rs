//! 编辑器主窗口：RMXP 式菜单＋工具栏＋左笔刷/地图树＋中画布＋右检查器＋底状态。

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use mota_core::db::{Barrier, Door, Enemy, Item};
use mota_core::map::Floor;
use mota_core::tiles::Tileset;

use crate::canvas::{CanvasIn, PlayViewIn, Playtest};
use crate::fonts;
use crate::gfx::Textures;
use crate::palette::{BrushKind, LeftTab, PaletteState};
use crate::{canvas, inspector};

/// 启动落在 7630 的「开始地图」（m02，必须保留），出生点在 S 标志处。
const START_FLOOR: &str = include_str!("../../../data/floors/m02.json");

/// 真地面（Map004 底色，石板路）。
const GROUND_TILE: i32 = 395;

fn blank_floor() -> Floor {
    Floor {
        id: "f01".to_string(),
        name: "1F".to_string(),
        width: 20,
        height: 15,
        layers: vec![vec![vec![GROUND_TILE; 20]; 15]],
        instances: Vec::new(),
        spawn: Some((10, 13)),
        intro: None,
    }
}

/// 载入规则脚本：优先 `scripts/rules/*.lua`（可热改），缺文件回落到内置版本。
fn load_rules() -> mota_core::rules::Rules {
    let files = ["battle", "items", "after_battle", "on_step"];
    let mut scripts = Vec::new();
    for name in files {
        match std::fs::read_to_string(format!("scripts/rules/{name}.lua")) {
            Ok(text) => scripts.push(text),
            Err(_) => {
                // 任一文件缺失就整体用内置，避免磁盘/内置混版
                scripts.clear();
                break;
            }
        }
    }
    let refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
    if refs.len() == files.len() {
        mota_core::rules::Rules::from_scripts(&refs)
    } else {
        mota_core::rules::Rules::embedded()
    }
    .unwrap_or_else(|e| {
        eprintln!("规则脚本载入失败：{e}，改用内置版本");
        mota_core::rules::Rules::embedded().expect("内置规则必须能载入")
    })
}

/// 素材目录：优先工作区 `assets/Graphics`，其次可执行文件旁（发行版布局）。
fn detect_gfx_dir() -> PathBuf {
    let cwd = PathBuf::from("assets/Graphics");
    if cwd.is_dir() {
        return cwd;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let next = dir.join("assets/Graphics");
        if next.is_dir() {
            return next;
        }
    }
    cwd
}

pub struct EditorApp {
    floor: Floor,
    palette: PaletteState,
    selected: Option<(i32, i32)>,
    /// 双击事件打开的详情窗（格坐标）。
    event_detail: Option<(i32, i32)>,
    /// 双击判定用：上一拍单击的快照（双击时回滚这一拍落笔）。
    click_snap: Option<canvas::ClickSnap>,
    next_seq: usize,
    status: String,
    textures: Option<Textures>,
    gfx_dir: PathBuf,
    play: Playtest,
    /// 游戏窗口独立纹理（副 viewport 有自己独立的 Context，主窗口的不能跨窗口用）。
    play_tex: Option<Textures>,
    /// 游戏窗口是否已定位／上次应用的倍率（首帧或倍率变化时带尺寸，不跟用户手调打架）。
    play_placed: bool,
    play_zoom: f32,
    doors: HashMap<String, Door>,
    enemies: HashMap<String, Enemy>,
    items: HashMap<String, Item>,
    barriers: HashMap<String, Barrier>,
    /// 图块集（单图块工程：所有楼共用 magictower；缺文件就不判图块）。
    tiles: Option<Tileset>,
    /// 勇士初始属性（`data/hero.json`，试玩开档用）。
    hero: mota_core::battle::Hero,
    /// 规则引擎：`scripts/rules/*.lua`（缺文件用内置 include_str! 版本）。
    rules: mota_core::rules::Rules,
    /// 工作区 `data/floors` 楼层 (id, name)（地图树用）。
    floors: Vec<(String, String)>,
    /// 魔塔样板组折叠状态（默认展开）。
    tower_open: bool,
    zoom: f32,
    show_grid: bool,
    show_about: bool,
    zoom_init: bool,
}

/// 工作区 `data/floors` 下的楼层 (id, name)（地图树用）。
/// name 读对应 JSON 的 name 字段，解析失败回落用 stem 当 name。
fn list_floors() -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir("data/floors") else {
        return Vec::new();
    };
    let mut stems: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|p| p.file_stem().and_then(|x| x.to_str()).map(str::to_string))
        .collect();
    stems.sort();
    stems
        .into_iter()
        .map(|stem| {
            let path = PathBuf::from("data/floors").join(format!("{stem}.json"));
            let name = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .unwrap_or_else(|| stem.clone());
            (stem, name)
        })
        .collect()
}

/// 工作区 `data/<dir>` 下的表（钥匙/消耗、默认贴图都从这里来）；
/// 缺目录就是空表，引用到的实例画红块警示。
fn load_table<T>(dir: &str) -> HashMap<String, T>
where
    T: serde::de::DeserializeOwned,
{
    let mut map = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return map;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().map(|x| x == "json").unwrap_or(false) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            if let Ok(v) = serde_json::from_str::<T>(&text)
                && let Some(stem) = p.file_stem().and_then(|x| x.to_str())
            {
                map.insert(stem.to_string(), v);
            }
        }
    }
    map
}

/// 工作区 `data/doors` 下的门表（钥匙/消耗）；缺目录就是空表，门全挡路。
fn load_doors() -> HashMap<String, Door> {
    load_table("data/doors")
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        fonts::setup_fonts(&cc.egui_ctx);
        let floor = serde_json::from_str(START_FLOOR).unwrap_or_else(|_| blank_floor());
        let gfx_dir = detect_gfx_dir();
        let doors = load_doors();
        let enemies: HashMap<String, Enemy> = load_table("data/enemies");
        let items: HashMap<String, Item> = load_table("data/items");
        let barriers: HashMap<String, Barrier> = load_table("data/barriers");
        let tiles = std::fs::read_to_string("data/tilesets/magictower.json")
            .ok()
            .and_then(|text| mota_core::tiles::load_tileset(&text).ok());
        let hero = std::fs::read_to_string("data/hero.json")
            .ok()
            .and_then(|text| serde_json::from_str::<mota_core::battle::Hero>(&text).ok())
            .unwrap_or_default();
        let rules = load_rules();
        let status = if gfx_dir.is_dir() {
            format!(
                "已载入开始地图 m02 · 素材 {} · 门{}种/怪{}种/物{}种/障{}种/块表{}",
                gfx_dir.display(),
                doors.len(),
                enemies.len(),
                items.len(),
                barriers.len(),
                if tiles.is_some() { "有" } else { "无" },
            )
        } else {
            "已载入开始地图 m02 · 缺素材（把 7630 Graphics 放到 assets/）".to_string()
        };
        Self {
            floor,
            palette: PaletteState::default(),
            selected: None,
            event_detail: None,
            click_snap: None,
            next_seq: 100,
            status,
            textures: None,
            gfx_dir,
            play: Playtest::default(),
            play_tex: None,
            play_placed: false,
            play_zoom: 1.0,
            doors,
            enemies,
            items,
            barriers,
            tiles,
            hero,
            rules,
            floors: list_floors(),
            tower_open: true,
            zoom: 1.0,
            show_grid: true,
            show_about: false,
            zoom_init: false,
        }
    }

    /// HiDPI 默认倍率：Retina 起步 1.5x，普通屏 1x（首帧按实际 PPP 定一次）。
    fn default_zoom(ppp: f32) -> f32 {
        if ppp >= 2.0 { 1.5 } else { 1.0 }
    }

    fn reset_floor(&mut self, floor: Floor, msg: String) {
        self.floor = floor;
        self.selected = None;
        self.event_detail = None;
        self.click_snap = None;
        self.play = Playtest::default();
        self.play_tex = None;
        self.play_placed = false;
        self.status = msg;
    }

    fn load_example(&mut self) {
        match serde_json::from_str(START_FLOOR) {
            Ok(floor) => self.reset_floor(floor, "已载入开始地图 m02".to_string()),
            Err(e) => self.status = format!("示例解析失败：{e}"),
        }
    }

    /// 系统级打开对话框（不用 egui 内建窗口）。
    fn open_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("楼层JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Floor>(&text) {
                Ok(floor) => self.reset_floor(floor, format!("已打开 {}", path.display())),
                Err(e) => self.status = format!("解析失败：{e}"),
            },
            Err(e) => self.status = format!("读取失败：{e}"),
        }
    }

    /// 从工作区 `data/floors/<stem>.json` 打开（地图树用）。
    fn open_floor(&mut self, stem: &str) {
        let path = PathBuf::from("data/floors").join(format!("{stem}.json"));
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Floor>(&text) {
                Ok(floor) => self.reset_floor(floor, format!("已打开 {}", path.display())),
                Err(e) => self.status = format!("解析失败：{e}"),
            },
            Err(e) => self.status = format!("读取失败：{e}"),
        }
    }

    /// 系统级保存对话框。
    fn save_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("楼层JSON", &["json"])
            .set_file_name(format!("{}.json", self.floor.id))
            .save_file()
        else {
            return;
        };
        match serde_json::to_string_pretty(&self.floor) {
            Ok(text) => match std::fs::write(&path, text) {
                Ok(()) => self.status = format!("已保存 {}", path.display()),
                Err(e) => self.status = format!("写入失败：{e}"),
            },
            Err(e) => self.status = format!("序列化失败：{e}"),
        }
    }

    fn toggle_play(&mut self) {
        if self.play.on {
            self.play = Playtest::default();
            self.play_tex = None;
            self.play_placed = false;
            self.status = "回编辑模式".to_string();
        } else {
            // 开试玩前重读规则/初始属性：改 scripts/rules/*.lua 和 hero.json 不用重启编辑器
            self.rules = load_rules();
            if let Ok(text) = std::fs::read_to_string("data/hero.json")
                && let Ok(h) = serde_json::from_str::<mota_core::battle::Hero>(&text)
            {
                self.hero = h;
            }
            let spawn = self.floor.spawn;
            self.play = Playtest::start(spawn, self.hero.clone());
            // 开始地图的自动剧情：进游戏直接弹对话，播完由 pending_goto 切层
            if let Some(intro) = &self.floor.intro {
                self.play.dialog = Some(canvas::Dialog {
                    lines: intro.lines.clone(),
                    page: 0,
                    vanish: None,
                    goto: intro
                        .to_floor
                        .as_ref()
                        .map(|f| (f.clone(), intro.to_x, intro.to_y)),
                });
            }
            // 游戏窗口纹理随开随建（副 Context 下持有，不复用主窗口的）
            self.play_tex = None;
            self.play_placed = false;
            self.event_detail = None;
            self.click_snap = None;
            self.status = "试玩：请看游戏窗口（方向键/WASD 走路，点击格瞬移）".to_string();
        }
    }

    /// 独立游戏窗口（副 viewport，有自己独立的 egui Context）。
    /// 输入（走路/瞬移）/绘制全在子 viewport 内，主窗口只看地图。
    /// 主窗口的 Textures 不能跨 viewport 用，这里用 play_tex 单独持有一份
    /// （参考原型 per-viewport 处理；随开随建，回编辑/关窗即丢）。
    /// 主窗口关闭时游戏窗口默认一起关，不用特殊处理。
    fn draw_play_window(&mut self, ctx: &egui::Context) {
        if !self.play.on {
            return;
        }
        let w = self.floor.width.min(60) as f32;
        let h = self.floor.height.min(60) as f32;
        let zoom = self.zoom;
        let mut builder = egui::ViewportBuilder::default().with_title("试玩");
        // 倍率变化才带尺寸（避免跟用户手动缩放打架）；首帧带一次定大小
        if !self.play_placed || (self.play_zoom - zoom).abs() > f32::EPSILON {
            let ppp = ctx.pixels_per_point().max(f32::EPSILON);
            builder = builder.with_inner_size(egui::vec2(
                w * canvas::CELL * zoom / ppp,
                h * canvas::CELL * zoom / ppp,
            ));
            self.play_zoom = zoom;
            self.play_placed = true;
        }
        let mut close = false;
        // 闭包非 'static 即可（egui 0.36 立即执行），直接借 &mut self 全家
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("playtest"),
            builder,
            |vui, _| {
                if vui.input(|i| i.viewport().close_requested()) {
                    close = true;
                    return;
                }
                // 游戏窗口独立 Context：纹理单独加载一份
                let vctx = vui.ctx().clone();
                let gfx = self.gfx_dir.clone();
                let tex = self.play_tex.get_or_insert_with(|| {
                    // 纹理不能跨 viewport 用：游戏窗口单独加载一份
                    Textures::load(&vctx, &gfx)
                });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(vui, |ui| {
                        canvas::show_play_view(
                            ui,
                            PlayViewIn {
                                floor: &mut self.floor,
                                play: &mut self.play,
                                tex,
                                gfx_dir: &gfx,
                                doors: &self.doors,
                                enemies: &self.enemies,
                                items: &self.items,
                                barriers: &self.barriers,
                                tiles: self.tiles.as_ref(),
                                rules: Some(&self.rules),
                                status: &mut self.status,
                                zoom,
                            },
                        );
                    });
            },
        );
        if close {
            self.play = Playtest::default();
            self.play_tex = None;
            self.play_placed = false;
            self.status = "试玩已关闭".to_string();
        }
        // 踩楼梯：在子 viewport 外切层（要读别的楼层 JSON，不借 floor/play）
        if let Some((to_floor, to_landing)) = self.play.pending_floor.take() {
            self.play_switch_floor(&to_floor, &to_landing);
        }
        // 自动剧情结束：切到目标层的指定坐标
        if let Some((to_floor, x, y)) = self.play.pending_goto.take() {
            self.play_goto_floor(&to_floor, x, y);
        }
    }

    /// 试玩中切层：读目标楼层 JSON，落到指定落脚点；一次性事件沿用本局记录。
    fn play_switch_floor(&mut self, stem: &str, landing: &str) {
        let Some(mut floor) = self.load_play_floor(stem) else {
            return;
        };
        let pos = floor.landing_pos(landing).or(floor.spawn).unwrap_or((0, 0));
        self.apply_play_state(&mut floor);
        self.floor = floor;
        self.play.hero = pos;
        self.play_placed = false; // 新楼层尺寸可能不同，允许重算窗口
        self.status = format!("切层→{stem}:{landing} ({},{})", pos.0, pos.1);
    }

    /// 试玩中按坐标切层（开始地图自动剧情的落点）。
    fn play_goto_floor(&mut self, stem: &str, x: i32, y: i32) {
        let Some(mut floor) = self.load_play_floor(stem) else {
            return;
        };
        self.apply_play_state(&mut floor);
        self.floor = floor;
        self.play.hero = (x, y);
        self.play_placed = false;
        self.status = format!("剧情→{stem} ({x},{y})");
    }

    /// 读目标楼层 JSON（试玩切层共用）。
    fn load_play_floor(&mut self, stem: &str) -> Option<Floor> {
        let path = PathBuf::from("data/floors").join(format!("{stem}.json"));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("切层失败 {stem}：{e}");
                return None;
            }
        };
        match serde_json::from_str(&text) {
            Ok(f) => Some(f),
            Err(e) => {
                self.status = format!("切层解析失败 {stem}：{e}");
                None
            }
        }
    }

    /// 重放本局进程：消费掉的一次性事件 + 破坏过的地形。
    fn apply_play_state(&self, floor: &mut Floor) {
        canvas::apply_once_done(floor, &self.play.once_done);
        canvas::apply_broken_tiles(floor, &self.play.broken_tiles);
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.zoom_init {
            self.zoom = Self::default_zoom(ui.ctx().pixels_per_point());
            self.zoom_init = true;
        }
        // 菜单栏（RMXP 式；只有已落地的功能才出现）。
        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("文件(F)", |ui| {
                    if ui.button("载入开始地图").clicked() {
                        self.load_example();
                    }
                    if ui.button("打开…").clicked() {
                        self.open_file();
                    }
                    if ui.button("保存…").clicked() {
                        self.save_file();
                    }
                });
                ui.menu_button("模式(M)", |ui| {
                    for kind in BrushKind::ALL {
                        ui.radio_value(&mut self.palette.kind, kind, kind.label());
                    }
                });
                ui.menu_button("视图(V)", |ui| {
                    ui.checkbox(&mut self.show_grid, "网格");
                });
                ui.menu_button("比例(S)", |ui| {
                    for z in [1.0, 1.5, 2.0, 3.0] {
                        if ui
                            .selectable_label(self.zoom == z, format!("{z}x"))
                            .clicked()
                        {
                            self.zoom = z;
                        }
                    }
                });
                ui.menu_button("游戏(G)", |ui| {
                    let label = if self.play.on { "回编辑" } else { "试玩" };
                    if ui.button(label).clicked() {
                        self.toggle_play();
                    }
                });
                ui.menu_button("帮助(H)", |ui| {
                    if ui.button("关于").clicked() {
                        self.show_about = true;
                    }
                });
            });
        });

        // 工具栏：文件＋试玩＋比例＋网格。
        egui::Panel::top("toolbar").show(ui, |ui| {
            use egui_material_icons::icons::*;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(ICON_FOLDER_OPEN.rich_text().size(18.0))
                    .on_hover_text("打开…")
                    .clicked()
                {
                    self.open_file();
                }
                if ui
                    .button(ICON_SAVE.rich_text().size(18.0))
                    .on_hover_text("保存…")
                    .clicked()
                {
                    self.save_file();
                }
                ui.separator();
                let (play_icon, play_tip) = if self.play.on {
                    (ICON_STOP, "回编辑")
                } else {
                    (ICON_PLAY_ARROW, "试玩")
                };
                if ui
                    .button(play_icon.rich_text().size(18.0))
                    .on_hover_text(play_tip)
                    .clicked()
                {
                    self.toggle_play();
                }
                ui.separator();
                for z in [1.0, 1.5, 2.0, 3.0] {
                    if ui
                        .selectable_label(self.zoom == z, format!("{z}x"))
                        .on_hover_text("缩放")
                        .clicked()
                    {
                        self.zoom = z;
                    }
                }
                ui.toggle_value(&mut self.show_grid, ICON_GRID_ON.rich_text().size(18.0))
                    .on_hover_text("网格");
                ui.separator();
                ui.label(format!(
                    "门{}·怪{}·物{}·障{}",
                    self.doors.len(),
                    self.enemies.len(),
                    self.items.len(),
                    self.barriers.len()
                ));
            });
        });

        // 底状态：左图号名尺寸，中反馈，右光标/试玩。
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{}: {} ({} × {})",
                    self.floor.id, self.floor.name, self.floor.width, self.floor.height
                ));
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.play.on {
                        ui.label(format!(
                            "勇({},{}) 血{}/{} 攻{} 防{} 魔{} 金{} 包{:?}",
                            self.play.hero.0,
                            self.play.hero.1,
                            self.play.stats.hp,
                            self.play.stats.hp_max,
                            self.play.stats.atk,
                            self.play.stats.def,
                            self.play.stats.mdef,
                            self.play.stats.gold,
                            self.play.bag
                        ));
                    } else if let Some((x, y)) = self.selected {
                        ui.label(format!(
                            "{x:03},{y:03} 实例{}个",
                            self.floor.instances.len()
                        ));
                    } else {
                        ui.label(format!("实例{}个", self.floor.instances.len()));
                    }
                });
            });
        });

        egui::Panel::left("left")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.palette.tab, LeftTab::Tiles, "图块");
                    ui.selectable_value(&mut self.palette.tab, LeftTab::Events, "事件");
                });
                ui.separator();
                // 上半笔刷区固定拿六成、下半地图树拿剩余（照 RMXP 上下分栏）。
                let pal_h = (ui.available_height() * 0.60).clamp(160.0, 900.0);
                egui::ScrollArea::vertical()
                    .id_salt("lefttab")
                    .max_height(pal_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.palette.tab {
                        LeftTab::Tiles => {
                            let ctx = ui.ctx().clone();
                            self.palette
                                .show_tiles(ui, &ctx, &mut self.textures, &self.gfx_dir);
                        }
                        LeftTab::Events => {
                            let ctx = ui.ctx().clone();
                            self.palette.show(
                                ui,
                                &ctx,
                                &mut self.textures,
                                &self.gfx_dir,
                                crate::palette::DbView {
                                    enemies: &self.enemies,
                                    items: &self.items,
                                    doors: &self.doors,
                                    barriers: &self.barriers,
                                },
                            );
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("地图");
                    if ui
                        .small_button(
                            egui_material_icons::icons::ICON_REFRESH
                                .rich_text()
                                .size(14.0),
                        )
                        .on_hover_text("更新")
                        .clicked()
                    {
                        self.floors = list_floors();
                    }
                });
                if self.floors.is_empty() {
                    ui.label("（data/floors 无楼层）");
                } else {
                    use egui_material_icons::icons::*;
                    // 7630 工程结构写死：顶层依次为开始地图(m02)、00(m01)、
                    // 魔塔样板组(m03 本体，可折叠，子项为名字含冒号的塔楼地图)、
                    // 空白地图(m07)；不在上述集合的楼层追加在最后顶层。
                    let floors = self.floors.clone();
                    let name_of = |id: &str| -> Option<String> {
                        floors
                            .iter()
                            .find(|(i, _)| i.as_str() == id)
                            .map(|(_, n)| n.clone())
                    };
                    // 塔楼地图：名字含冒号，按冒号后楼层号解析为 i64 排序，解析失败排最后。
                    let mut tower: Vec<(String, String)> = floors
                        .iter()
                        .filter(|(_, n)| n.contains(':'))
                        .cloned()
                        .collect();
                    tower.sort_by(|a, b| {
                        let num = |n: &str| {
                            n.rsplit(':')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .parse::<i64>()
                                .ok()
                        };
                        match (num(&a.1), num(&b.1)) {
                            (Some(x), Some(y)) => x.cmp(&y),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => a.1.cmp(&b.1),
                        }
                    });
                    // 楼层行：文件图标＋name（hover 显示 id），当前楼高亮，点击走 open_floor。
                    // 返回是否被点击（调用方再调 open_floor，避免闭包借 self 冲突）。
                    let show_floor = |ui: &mut egui::Ui, cur: bool, id: &str, name: &str| -> bool {
                        let mut hit = false;
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Label::new(ICON_DESCRIPTION.rich_text().size(14.0))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text(id)
                                .clicked()
                            {
                                hit = true;
                            }
                            if ui.selectable_label(cur, name).on_hover_text(id).clicked() {
                                hit = true;
                            }
                        });
                        hit
                    };
                    for id in ["m02", "m01"] {
                        if let Some(name) = name_of(id) {
                            let cur = self.floor.id.as_str() == id;
                            if show_floor(ui, cur, id, &name) && !cur {
                                self.open_floor(id);
                            }
                        }
                    }
                    // 组行：m03 本体，可点击直达（-chevron/文件夹图标只折叠/展开）。
                    if name_of("m03").is_some() || !tower.is_empty() {
                        let group_name = name_of("m03").unwrap_or_else(|| "魔塔样板".to_string());
                        let chev = if self.tower_open {
                            ICON_EXPAND_MORE
                        } else {
                            ICON_CHEVRON_RIGHT
                        };
                        let mut toggle = false;
                        let mut open = false;
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Label::new(chev.rich_text().size(14.0))
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                toggle = true;
                            }
                            if ui
                                .add(
                                    egui::Label::new(ICON_FOLDER.rich_text().size(14.0))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text("m03")
                                .clicked()
                            {
                                toggle = true;
                            }
                            let cur_m03 = self.floor.id == "m03";
                            if ui
                                .selectable_label(cur_m03, &group_name)
                                .on_hover_text("m03")
                                .clicked()
                            {
                                open = true;
                            }
                        });
                        if toggle {
                            self.tower_open = !self.tower_open;
                        }
                        if open && name_of("m03").is_some() {
                            self.open_floor("m03");
                        }
                        if self.tower_open {
                            ui.indent("tower", |ui| {
                                for (id, name) in &tower {
                                    let cur = id.as_str() == self.floor.id.as_str();
                                    if show_floor(ui, cur, id, name) && !cur {
                                        self.open_floor(id);
                                    }
                                }
                            });
                        }
                    }
                    if let Some(name) = name_of("m07") {
                        let cur = self.floor.id == "m07";
                        if show_floor(ui, cur, "m07", &name) && !cur {
                            self.open_floor("m07");
                        }
                    }
                    // 未知楼层追加在最后顶层。
                    for (id, name) in &floors {
                        if ["m01", "m02", "m03", "m07"].contains(&id.as_str()) {
                            continue;
                        }
                        if tower.iter().any(|(t, _)| t.as_str() == id.as_str()) {
                            continue;
                        }
                        let cur = id.as_str() == self.floor.id.as_str();
                        if show_floor(ui, cur, id, name) && !cur {
                            self.open_floor(id);
                        }
                    }
                }
            });

        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("inspector")
                    .show(ui, |ui| {
                        inspector::show_inspector(
                            ui,
                            &mut self.floor,
                            self.selected,
                            &self.doors,
                            &mut self.status,
                        );
                    });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                canvas::show_canvas(
                    ui,
                    CanvasIn {
                        floor: &mut self.floor,
                        tex: &mut self.textures,
                        gfx_dir: &self.gfx_dir,
                        palette: &self.palette,
                        selected: &mut self.selected,
                        next_seq: &mut self.next_seq,
                        status: &mut self.status,
                        play: &mut self.play,
                        doors: &self.doors,
                        enemies: &self.enemies,
                        items: &self.items,
                        barriers: &self.barriers,
                        event_detail: &mut self.event_detail,
                        click_snap: &mut self.click_snap,
                        zoom: self.zoom,
                        show_grid: self.show_grid,
                    },
                );
            });
        });

        // 独立游戏窗口（试玩中常驻；主窗口关闭时 eframe 默认一起关）
        let ctx = ui.ctx().clone();
        self.draw_play_window(&ctx);

        // 双击事件打开的详情窗（RMXP 编辑事件式：一个格子里的实例都可改）
        if let Some((x, y)) = self.event_detail {
            let mut open = true;
            egui::Window::new(format!("事件编辑 - ({x},{y})"))
                .open(&mut open)
                .default_width(380.0)
                .max_height(600.0)
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("event_detail")
                        .show(ui, |ui| {
                            inspector::show_event_detail(
                                ui,
                                &mut self.floor,
                                x,
                                y,
                                &self.doors,
                                &mut self.status,
                            );
                        });
                });
            if !open {
                self.event_detail = None;
            }
        }

        if self.show_about {
            let mut open = true;
            egui::Window::new("关于")
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label("mota-rs：魔塔编辑器（egui 纯 Rust）");
                    ui.label("楼层/数值存 JSON，美术复用 7630 原 PNG。");
                });
            self.show_about = open;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidpi_defaults_to_1_5x() {
        assert_eq!(EditorApp::default_zoom(2.0), 1.5);
        assert_eq!(EditorApp::default_zoom(1.0), 1.0);
    }
}
