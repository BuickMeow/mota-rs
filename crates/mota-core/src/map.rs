//! 地图实例：笔刷模板 + 摆上去的格子。
//!
//! 勇士不进笔刷（出生点/落脚点配置）；移动 NPC = Npc + route。
//! 显示：实例 `visual`（Some=开发者换的骗人贴图）优先，否则用 DB 默认贴图。

use serde::{Deserialize, Serialize};

use crate::db::SpriteRef;

/// 生命周期：一次性 / 多次不消失 / 重进恢复 / 按开关显隐。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// 触发一次后消失（如血瓶、门）。
    Once,
    /// 多次触发，不消失（如商人、传送器）。
    Persistent,
    /// 消失后离开重进恢复（如可刷新的路障）。
    RespawnOnReenter,
    /// 由开关决定显隐（如花门/暗墙剧情）。
    ByFlag(String),
}

/// 事件如何发生。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// 撞上/踩上（门、路障、物品）。
    Touch,
    /// 面向按确认（NPC、商店、传送器）。
    Interact,
    /// 进层自动执行。
    Auto,
    /// 可穿透背景装饰。
    PassableBg,
}

/// 可摆放的笔刷种类（编辑器调色板一行一种）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TemplateKind {
    Npc {
        dialog: Vec<String>,
        #[serde(default)]
        shop_id: Option<String>,
        /// 巡逻路线（格子序列），无则站定不动。
        #[serde(default)]
        route: Vec<(i32, i32)>,
    },
    Monster {
        monster_id: String,
    },
    Item {
        item_id: String,
    },
    /// 路障（data/barriers 表，Common015 变量 21 编码）。
    Barrier {
        barrier_id: String,
    },
    Door {
        door_id: String,
    },
    /// 上楼：去某层某落脚点（图块 633）。
    StairUp {
        to_floor: String,
        to_landing: String,
    },
    /// 下楼：去某层某落脚点（图块 632）。
    StairDown {
        to_floor: String,
        to_landing: String,
    },
    /// 落脚点命名（楼梯的另一端）。
    Landing {
        name: String,
    },
    /// 推箱子（7630 EV017：接触后沿面向方向推一格，撞墙则跳过）。
    Box,
    /// 压力板：有箱子压在上面时 `flag` 为真（推箱子解谜用）。
    Plate {
        flag: String,
    },
}

impl TemplateKind {
    pub fn default_lifecycle(&self) -> Lifecycle {
        match self {
            Self::Npc { .. } | Self::Landing { .. } | Self::Box | Self::Plate { .. } => {
                Lifecycle::Persistent
            }
            Self::Monster { .. } | Self::Item { .. } | Self::Barrier { .. } | Self::Door { .. } => {
                Lifecycle::Once
            }
            Self::StairUp { .. } | Self::StairDown { .. } => Lifecycle::Persistent,
        }
    }

    pub fn default_trigger(&self) -> Trigger {
        match self {
            Self::Npc { .. } => Trigger::Interact,
            Self::Monster { .. }
            | Self::Door { .. }
            | Self::StairUp { .. }
            | Self::StairDown { .. }
            | Self::Box
            | Self::Plate { .. } => Trigger::Touch,
            Self::Item { .. } | Self::Barrier { .. } => Trigger::Touch,
            Self::Landing { .. } => Trigger::PassableBg,
        }
    }

    /// 箱子能否被推进该格：箱子/门/怪/NPC/楼梯/路障挡路，其余可进。
    pub fn blocks_push(&self) -> bool {
        matches!(
            self,
            Self::Box
                | Self::Door { .. }
                | Self::Monster { .. }
                | Self::Npc { .. }
                | Self::StairUp { .. }
                | Self::StairDown { .. }
                | Self::Barrier { .. }
        )
    }
}

/// 地图上摆好的一个格子：99% 用模板默认，只存覆盖。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub template: TemplateKind,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub trigger: Option<Trigger>,
    /// 按开关显隐的覆盖（优先级高于模板默认）。
    #[serde(default)]
    pub visible_flag: Option<String>,
    /// 换贴图（None = 用 DB 默认贴图；Some = 开发者指定的，包括骗人物件）。
    #[serde(default)]
    pub visual: Option<SpriteRef>,
    /// RMXP 事件页「移动时动画」：走路时 4 帧循环。
    #[serde(default, skip_serializing_if = "is_false")]
    pub walk_anime: bool,
    /// RMXP 事件页「停止时动画」：站定也 4 帧循环。
    #[serde(default, skip_serializing_if = "is_false")]
    pub step_anime: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Instance {
    pub fn lifecycle(&self) -> Lifecycle {
        if let Some(flag) = &self.visible_flag {
            return Lifecycle::ByFlag(flag.clone());
        }
        self.lifecycle
            .clone()
            .unwrap_or_else(|| self.template.default_lifecycle())
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger
            .unwrap_or_else(|| self.template.default_trigger())
    }
}

/// 一层楼。`layers` 为 RMXP 式三层图块（z,y,x），缺层/缺格按 0（空格）处理。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Floor {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub layers: Vec<Vec<Vec<i32>>>,
    #[serde(default)]
    pub instances: Vec<Instance>,
    /// 勇士出生/重进落点。
    #[serde(default)]
    pub spawn: Option<(i32, i32)>,
    /// 进层自动剧情（开始地图用）：先自动播对话，播完切层。
    #[serde(default)]
    pub intro: Option<FloorIntro>,
    /// 塔号（None = 特殊地图：开始地图/模板/空白等）。
    #[serde(default)]
    pub tower: Option<i64>,
    /// 层号（>0 地上，0 = 0 层，<0 地下；None = 特殊地图）。
    #[serde(default)]
    pub level: Option<i64>,
    /// 地图树父节点 id（None = 顶层；来自 RMXP MapInfos）。
    #[serde(default)]
    pub parent: Option<String>,
    /// 地图树排序（来自 RMXP MapInfos.order）。
    #[serde(default)]
    pub order: Option<i64>,
}

/// 自动剧情：开始地图（start）那段无法跳过的开场白 + 结束后的落点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorIntro {
    /// 自动播放的对话（原文每一行一条）。
    pub lines: Vec<String>,
    /// 播完切到哪层（None = 原地不切）。
    #[serde(default)]
    pub to_floor: Option<String>,
    #[serde(default)]
    pub to_x: i32,
    #[serde(default)]
    pub to_y: i32,
}

impl Floor {
    /// 取图块号，越界或缺层返回 0。
    pub fn tile(&self, x: i32, y: i32, z: usize) -> i32 {
        if x < 0 || y < 0 {
            return 0;
        }
        self.layers
            .get(z)
            .and_then(|l| l.get(y as usize))
            .and_then(|r| r.get(x as usize))
            .copied()
            .unwrap_or(0)
    }

    /// 写图块号；越界或缺层静默跳过（破坏地形用）。
    pub fn set_tile(&mut self, x: i32, y: i32, z: usize, tid: i32) {
        if x < 0 || y < 0 {
            return;
        }
        if let Some(layer) = self.layers.get_mut(z)
            && let Some(row) = layer.get_mut(y as usize)
            && let Some(cell) = row.get_mut(x as usize)
        {
            *cell = tid;
        }
    }

    /// 找出某格的全部实例下标。
    pub fn instances_at(&self, x: i32, y: i32) -> Vec<usize> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| inst.x == x && inst.y == y)
            .map(|(i, _)| i)
            .collect()
    }

    /// 清除某格全部实例，返回清掉的数量。
    pub fn erase_at(&mut self, x: i32, y: i32) -> usize {
        let before = self.instances.len();
        self.instances.retain(|inst| !(inst.x == x && inst.y == y));
        before - self.instances.len()
    }

    /// 图块落笔：自动扩层，越界格跳过。
    pub fn paint_tile(&mut self, x: i32, y: i32, z: usize, tid: i32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        while self.layers.len() <= z {
            self.layers
                .push(vec![vec![0; self.width as usize]; self.height as usize]);
        }
        self.layers[z][y as usize][x as usize] = tid;
    }

    /// 按名找落脚点（楼梯的另一端）；没有就返回 None，不回落（显式摆放优先）。
    pub fn landing_pos(&self, name: &str) -> Option<(i32, i32)> {
        self.instances.iter().find_map(|inst| match &inst.template {
            TemplateKind::Landing { name: n } if n == name => Some((inst.x, inst.y)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_door_once_touch() {
        let inst = Instance {
            id: "d1".to_string(),
            template: TemplateKind::Door {
                door_id: "yellow".to_string(),
            },
            x: 3,
            y: 4,
            lifecycle: None,
            trigger: None,
            visible_flag: None,
            visual: None,
            walk_anime: false,
            step_anime: false,
        };
        assert_eq!(inst.lifecycle(), Lifecycle::Once);
        assert_eq!(inst.trigger(), Trigger::Touch);
    }

    #[test]
    fn visible_flag_overrides() {
        let inst = Instance {
            id: "w1".to_string(),
            template: TemplateKind::Door {
                door_id: "flower".to_string(),
            },
            x: 0,
            y: 0,
            lifecycle: None,
            trigger: None,
            visible_flag: Some("flags.打败花门守卫".to_string()),
            visual: None,
            walk_anime: false,
            step_anime: false,
        };
        assert_eq!(
            inst.lifecycle(),
            Lifecycle::ByFlag("flags.打败花门守卫".to_string())
        );
    }

    #[test]
    fn box_and_plate_are_persistent_touch() {
        for t in [
            TemplateKind::Box,
            TemplateKind::Plate {
                flag: "箱子到位".to_string(),
            },
        ] {
            assert_eq!(t.default_lifecycle(), Lifecycle::Persistent);
            assert_eq!(t.default_trigger(), Trigger::Touch);
        }
        assert!(TemplateKind::Box.blocks_push());
        assert!(
            TemplateKind::Door {
                door_id: "yellow".to_string()
            }
            .blocks_push()
        );
        assert!(
            !TemplateKind::Plate {
                flag: "x".to_string()
            }
            .blocks_push()
        );
        assert!(
            !TemplateKind::Landing {
                name: "入口".to_string()
            }
            .blocks_push()
        );
    }

    #[test]
    fn landing_pos_strict_no_fallback() {
        let f = Floor {
            id: "f".to_string(),
            name: "F".to_string(),
            width: 5,
            height: 5,
            layers: Vec::new(),
            instances: vec![Instance {
                id: "l".to_string(),
                template: TemplateKind::Landing {
                    name: "入口".to_string(),
                },
                x: 2,
                y: 3,
                lifecycle: None,
                trigger: None,
                visible_flag: None,
                visual: None,
                walk_anime: false,
                step_anime: false,
            }],
            spawn: Some((0, 0)),
            intro: None,
            tower: None,
            level: None,
            parent: None,
            order: None,
        };
        assert_eq!(f.landing_pos("入口"), Some((2, 3)));
        // 没摆就是 None，不回落到 spawn
        assert_eq!(f.landing_pos("出口"), None);
    }
}
