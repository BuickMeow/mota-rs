//! 右侧检查器：改选中格实例的参数、生命周期/触发覆盖、显隐开关。

use std::collections::HashMap;

use eframe::egui;
use mota_core::db::{Door, SpriteRef};
use mota_core::map::{Floor, Instance, Lifecycle, Trigger};

use crate::canvas::instances_at;

fn lifecycle_label(lc: &Option<Lifecycle>) -> &'static str {
    match lc {
        None => "默认",
        Some(Lifecycle::Once) => "一次",
        Some(Lifecycle::Persistent) => "多次",
        Some(Lifecycle::RespawnOnReenter) => "重进恢复",
        Some(Lifecycle::ByFlag(_)) => "按开关",
    }
}

fn cycle_lifecycle(lc: &mut Option<Lifecycle>) {
    *lc = match lc.take() {
        None => Some(Lifecycle::Once),
        Some(Lifecycle::Once) => Some(Lifecycle::Persistent),
        Some(Lifecycle::Persistent) => Some(Lifecycle::RespawnOnReenter),
        Some(Lifecycle::RespawnOnReenter) | Some(Lifecycle::ByFlag(_)) => None,
    };
}

fn trigger_label(tr: &Option<Trigger>) -> &'static str {
    match tr {
        None => "默认",
        Some(Trigger::Touch) => "接触",
        Some(Trigger::Interact) => "对话",
        Some(Trigger::Auto) => "自动",
        Some(Trigger::PassableBg) => "穿透",
    }
}

fn cycle_trigger(tr: &mut Option<Trigger>) {
    *tr = match tr.take() {
        None => Some(Trigger::Touch),
        Some(Trigger::Touch) => Some(Trigger::Interact),
        Some(Trigger::Interact) => Some(Trigger::Auto),
        Some(Trigger::Auto) | Some(Trigger::PassableBg) => None,
    }
}

fn kind_label(inst: &Instance) -> String {
    use mota_core::map::TemplateKind as T;
    match &inst.template {
        T::Npc { .. } => "NPC".to_string(),
        T::Monster { monster_id, .. } => format!("怪物 {monster_id}"),
        T::Item { item_id } => format!("物品 {item_id}"),
        T::Barrier { barrier_id } => format!("路障 {barrier_id}"),
        T::Door { door_id } => format!("门 {door_id}"),
        T::StairUp {
            to_floor,
            to_landing,
        } => format!("上楼 {to_floor}:{to_landing}"),
        T::StairDown {
            to_floor,
            to_landing,
        } => format!("下楼 {to_floor}:{to_landing}"),
        T::Landing { name } => format!("落脚 {name}"),
        T::Box => "箱子".to_string(),
        T::Plate { flag } => format!("压力板 {flag}"),
    }
}

/// 编辑模板自带参数（ID 类文本框改完即生效）。
fn edit_template_params(ui: &mut egui::Ui, inst: &mut Instance) {
    use mota_core::map::TemplateKind as T;
    match &mut inst.template {
        T::Monster { monster_id, .. } => {
            ui.horizontal(|ui| {
                ui.label("monster_id");
                ui.text_edit_singleline(monster_id);
            });
        }
        T::Item { item_id } => {
            ui.horizontal(|ui| {
                ui.label("item_id");
                ui.text_edit_singleline(item_id);
            });
        }
        T::Door { door_id } => {
            ui.horizontal(|ui| {
                ui.label("door_id");
                ui.text_edit_singleline(door_id);
            });
        }
        T::Barrier { barrier_id } => {
            ui.horizontal(|ui| {
                ui.label("barrier_id");
                ui.text_edit_singleline(barrier_id);
            });
        }
        T::StairUp {
            to_floor,
            to_landing,
        }
        | T::StairDown {
            to_floor,
            to_landing,
        } => {
            ui.horizontal(|ui| {
                ui.label("去楼层");
                ui.text_edit_singleline(to_floor);
            });
            ui.horizontal(|ui| {
                ui.label("落脚点");
                ui.text_edit_singleline(to_landing);
            });
        }
        T::Landing { name } => {
            ui.horizontal(|ui| {
                ui.label("名字");
                ui.text_edit_singleline(name);
            });
        }
        T::Box => {
            ui.label("推：撞墙不动，目标格被挡不动");
        }
        T::Plate { flag } => {
            ui.horizontal(|ui| {
                ui.label("开关");
                ui.text_edit_singleline(flag);
            });
        }
        T::Npc {
            dialog, shop_id, ..
        } => {
            ui.label("对话（每行一句）");
            let mut text = dialog.join("\n");
            let resp = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if resp.changed() {
                *dialog = text.lines().map(str::to_string).collect();
            }
            ui.horizontal(|ui| {
                ui.label("商店");
                let mut shop = shop_id.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut shop).changed() {
                    let t = shop.trim();
                    *shop_id = if t.is_empty() {
                        None
                    } else {
                        Some(t.to_owned())
                    };
                }
            });
        }
    }
}

/// 单个实例的详情编辑块（右侧检查器与双击事件窗共用）。
/// 返回是否点了“删除此实例”。
pub fn edit_instance(
    ui: &mut egui::Ui,
    inst: &mut Instance,
    doors: &HashMap<String, Door>,
) -> bool {
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong(kind_label(inst));
        ui.label(format!("id={}", inst.id));
    });
    ui.label(format!(
        "实际：{} / {}",
        lifecycle_label(&Some(inst.lifecycle())),
        trigger_label(&Some(inst.trigger()))
    ));
    edit_template_params(ui, inst);
    if let mota_core::map::TemplateKind::Door { door_id } = &inst.template {
        match doors.get(door_id) {
            Some(d) if d.free => {
                ui.label("徒手门：一撞就开");
            }
            Some(d) => match &d.key {
                Some(k) => {
                    ui.label(format!(
                        "钥匙：{k}（{}）",
                        if d.consume { "消耗" } else { "不消耗" }
                    ));
                }
                None => {
                    if d.description.is_empty() {
                        ui.label("条件墙：走剧情开关，不吃钥匙");
                    } else {
                        ui.label(&d.description);
                    }
                }
            },
            None => {
                ui.label("门表无此门（会一直挡路）");
            }
        }
    }
    // 换贴图：默认用 DB 贴图；填了就是覆盖（含骗人物件），文件名空画红块警示。
    ui.horizontal(|ui| {
        ui.label("换贴图");
        if inst.visual.is_some() {
            if ui.button("恢复默认").clicked() {
                inst.visual = None;
            }
        } else if ui.button("覆盖默认").clicked() {
            inst.visual = Some(SpriteRef {
                file: String::new(),
                hue: 0,
                col: 0,
                row: 0,
                opacity: 255,
            });
        }
    });
    if let Some(v) = inst.visual.as_mut() {
        ui.horizontal(|ui| {
            ui.label("文件");
            ui.text_edit_singleline(&mut v.file);
        });
        ui.horizontal(|ui| {
            ui.label("hue");
            ui.add(egui::DragValue::new(&mut v.hue));
            ui.label("col");
            ui.add(egui::DragValue::new(&mut v.col));
            ui.label("row");
            ui.add(egui::DragValue::new(&mut v.row));
        });
        ui.horizontal(|ui| {
            ui.label("不透明");
            ui.add(egui::DragValue::new(&mut v.opacity).range(0..=255));
        });
    }
    ui.horizontal(|ui| {
        if ui
            .button(format!("生命周期：{}", lifecycle_label(&inst.lifecycle)))
            .clicked()
        {
            cycle_lifecycle(&mut inst.lifecycle);
        }
        if ui
            .button(format!("触发：{}", trigger_label(&inst.trigger)))
            .clicked()
        {
            cycle_trigger(&mut inst.trigger);
        }
    });
    ui.horizontal(|ui| {
        ui.label("动画");
        ui.checkbox(&mut inst.walk_anime, "移动时");
        ui.checkbox(&mut inst.step_anime, "停止时");
    });
    ui.horizontal(|ui| {
        ui.label("显隐开关");
        let mut flag = inst.visible_flag.clone().unwrap_or_default();
        if ui.text_edit_singleline(&mut flag).changed() {
            let t = flag.trim();
            inst.visible_flag = if t.is_empty() {
                None
            } else {
                Some(t.to_owned())
            };
        }
    });
    ui.button(format!(
        "{} 删除此实例",
        String::from(egui_material_icons::icons::ICON_DELETE)
    ))
    .clicked()
}

/// 按下标删除实例：删前校验仍是同一格，防止借用期间漂移。
fn delete_at(floor: &mut Floor, i: usize, x: i32, y: i32, status: &mut String) {
    if i < floor.instances.len() && floor.instances[i].x == x && floor.instances[i].y == y {
        let id = floor.instances[i].id.clone();
        floor.instances.remove(i);
        *status = format!("删除 {id}");
    }
}

pub fn show_inspector(
    ui: &mut egui::Ui,
    floor: &mut Floor,
    selected: Option<(i32, i32)>,
    doors: &HashMap<String, Door>,
    status: &mut String,
) {
    ui.heading("检查器");
    let Some((x, y)) = selected else {
        ui.label("点画布选中一格");
        return;
    };
    ui.label(format!("选中 ({x},{y}) · 双击看详情"));
    let idxs = instances_at(floor, x, y);
    if idxs.is_empty() {
        ui.label("本格无实例");
        return;
    }
    let mut delete: Option<usize> = None;
    for i in idxs {
        let Some(inst) = floor.instances.get_mut(i) else {
            continue;
        };
        if edit_instance(ui, inst, doors) {
            delete = Some(i);
        }
    }
    if let Some(i) = delete {
        delete_at(floor, i, x, y, status);
    }
}

/// 双击事件打开的详情窗内容：该格全部实例都可改（RMXP 编辑事件式）。
pub fn show_event_detail(
    ui: &mut egui::Ui,
    floor: &mut Floor,
    x: i32,
    y: i32,
    doors: &HashMap<String, Door>,
    status: &mut String,
) {
    let idxs = instances_at(floor, x, y);
    ui.label(format!("格子 ({x},{y}) · 实例 {} 个", idxs.len()));
    if idxs.is_empty() {
        ui.weak("本格已无实例");
        return;
    }
    let mut delete: Option<usize> = None;
    for i in idxs {
        let Some(inst) = floor.instances.get_mut(i) else {
            continue;
        };
        if edit_instance(ui, inst, doors) {
            delete = Some(i);
        }
    }
    if let Some(i) = delete {
        delete_at(floor, i, x, y, status);
    }
}
