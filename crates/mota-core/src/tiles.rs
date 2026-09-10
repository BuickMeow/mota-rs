//! 图块集通行数据（RMXP Tilesets.rxdata 三表原样导入）。
//!
//! - 通行 passages：低 4 位方向（0x01 下 / 0x02 左 / 0x04 右 / 0x08 上），
//!   0x0f 禁行（×），0x00 可通行（○）；0x40 草木繁茂（半身遮挡），
//!   0x80 柜台（隔桌对话）。后两者已导入，渲染/交互待接。
//! - 优先级 priorities：0 画在角色下方；1+ 画在角色上方（桥顶/树冠），渲染待接。
//! - 地形标志 terrains：脚本分支用（如伤害地形），规则引擎待接。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tileset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub passages: Vec<i64>,
    #[serde(default)]
    pub priorities: Vec<i64>,
    #[serde(default)]
    pub terrains: Vec<i64>,
}

/// RMXP 方向 → 通行位。
pub fn dir_bit(dir: i32) -> i64 {
    match dir {
        2 => 0x01,
        4 => 0x02,
        6 => 0x04,
        8 => 0x08,
        _ => 0,
    }
}

/// 反方向（进格检查用）。
pub fn reverse_dir(dir: i32) -> i32 {
    match dir {
        2 => 8,
        8 => 2,
        4 => 6,
        6 => 4,
        d => d,
    }
}

impl Tileset {
    fn passage_of(&self, tid: i32) -> Option<i64> {
        if tid < 0 {
            return None;
        }
        self.passages.get(tid as usize).copied()
    }

    fn priority_of(&self, tid: i32) -> i64 {
        if tid < 0 {
            return 0;
        }
        self.priorities.get(tid as usize).copied().unwrap_or(0)
    }

    /// 单格朝 dir 能否离开：None=空格继续看下层（调用方负责跨层）。
    fn exit_cell(&self, tid: i32, dir: i32) -> Option<bool> {
        if tid < 48 {
            return None;
        }
        let p = self.passage_of(tid)?;
        if p & 0x0f == 0x0f {
            return Some(false);
        }
        if self.priority_of(tid) == 0 {
            return Some(p & dir_bit(dir) == 0);
        }
        None
    }

    /// 整格（z 从高到低三层图块号）朝 dir 能否离开：首个有结论的为准，都没结论即可走。
    pub fn exit_at(&self, l2: i32, l1: i32, l0: i32, dir: i32) -> bool {
        for tid in [l2, l1, l0] {
            if let Some(ok) = self.exit_cell(tid, dir) {
                return ok;
            }
        }
        true
    }

    /// 是否草木格（0x40，半身遮挡，渲染待接）。
    pub fn is_bush(&self, tid: i32) -> bool {
        self.passage_of(tid)
            .map(|p| p & 0x40 == 0x40)
            .unwrap_or(false)
    }

    /// 是否柜台（0x80，隔桌对话，交互待接）。
    pub fn is_counter(&self, tid: i32) -> bool {
        self.passage_of(tid)
            .map(|p| p & 0x80 == 0x80)
            .unwrap_or(false)
    }
}

/// 从 JSON 文本解析（IO 由调用方负责）。
pub fn load_tileset(s: &str) -> Result<Tileset, serde_json::Error> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles() -> Tileset {
        // 395 地面可走，424 墙禁行（7630 实测值）。
        let mut t = Tileset {
            id: "magictower".to_string(),
            name: "魔塔".to_string(),
            passages: vec![0; 688],
            priorities: vec![0; 688],
            terrains: vec![0; 688],
        };
        t.passages[424] = 15;
        t
    }

    #[test]
    fn wall_blocks_and_ground_passes() {
        let t = tiles();
        assert!(t.exit_at(0, 0, 395, 2));
        assert!(!t.exit_at(0, 0, 424, 2));
        // 上层空格跳过看下层
        assert!(!t.exit_at(0, 424, 395, 6));
        assert!(t.exit_at(0, 0, 0, 2));
    }

    #[test]
    fn direction_bits_matter() {
        let mut t = tiles();
        t.passages[500] = 0x0E; // 只许下（其余三向禁行）
        assert!(t.exit_at(0, 0, 500, 2));
        assert!(!t.exit_at(0, 0, 500, 8));
        assert_eq!(reverse_dir(2), 8);
        assert_eq!(dir_bit(6), 0x04);
    }

    #[test]
    fn bush_and_counter_bits() {
        let mut t = tiles();
        t.passages[600] = 0x40;
        t.passages[601] = 0x80;
        assert!(t.is_bush(600));
        assert!(!t.is_bush(601));
        assert!(t.is_counter(601));
        assert!(!t.is_counter(600));
    }
}
