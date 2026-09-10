//! 事件指令：可视化列表的存储形态。
//!
//! 执行在 `scripts/rules/commands.lua`（`Rules::run_cmds`），这里只管数据与序列化；
//! `Lua` 块是唯一手写代码入口，跑在规则引擎沙箱里。

use serde::{Deserialize, Serialize};

/// 单条指令（全部命名引用，杜绝 0009 魔法数字）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Cmd {
    Talk {
        lines: Vec<String>,
    },
    Give {
        item: String,
        #[serde(default = "one")]
        n: i64,
    },
    Take {
        item: String,
        #[serde(default = "one")]
        n: i64,
    },
    SetFlag {
        name: String,
        value: bool,
    },
    AddVar {
        name: String,
        delta: i64,
    },
    Fight {
        enemy: String,
    },
    Teleport {
        floor: String,
        landing: String,
    },
    OpenShop {
        shop: String,
    },
    CallCommon {
        name: String,
    },
    /// 高级用户逃生舱（如复杂条件），在规则沙箱里执行。
    Lua {
        code: String,
    },
}

fn one() -> i64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_roundtrip() {
        let cmds = vec![
            Cmd::Talk {
                lines: vec!["勇者，你还上不去。".to_string()],
            },
            Cmd::Give {
                item: "gold".to_string(),
                n: 50,
            },
            Cmd::SetFlag {
                name: "救出仙子".to_string(),
                value: true,
            },
            Cmd::Teleport {
                floor: "m05".to_string(),
                landing: "下楼梯".to_string(),
            },
        ];
        let json = serde_json::to_string(&cmds).unwrap();
        let back: Vec<Cmd> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 4);
        assert!(matches!(&back[0], Cmd::Talk { lines } if lines.len() == 1));
        assert!(matches!(&back[3], Cmd::Teleport { floor, .. } if floor == "m05"));
    }

    #[test]
    fn give_n_defaults_to_one() {
        let back: Cmd = serde_json::from_str(r#"{"op":"give","item":"yellow_key"}"#).unwrap();
        assert!(matches!(back, Cmd::Give { n, .. } if n == 1));
    }
}
