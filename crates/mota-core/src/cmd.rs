//! 指令流：可视化列表的存储形态，也是 Lua 生成的目标。
//!
//! 编辑器渲染成 RMXP 式彩色列表；`Lua` 块是唯一手写代码入口。

use crate::state::GameState;
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
    /// 高级用户逃生舱（如复杂条件）。
    Lua {
        code: String,
    },
}

fn one() -> i64 {
    1
}

/// 可视化指令的最小执行器（Lua 块不走这里，走 `lua` 模块）。
pub fn run_cmds(state: &mut GameState, cmds: &[Cmd]) {
    for c in cmds {
        match c {
            Cmd::Talk { lines } => state.messages.extend(lines.iter().cloned()),
            Cmd::Give { item, n } => *state.bag.entry(item.clone()).or_insert(0) += *n,
            Cmd::Take { item, n } => *state.bag.entry(item.clone()).or_insert(0) -= *n,
            Cmd::SetFlag { name, value } => {
                state.flags.insert(name.clone(), *value);
            }
            Cmd::AddVar { name, delta } => *state.vars.entry(name.clone()).or_insert(0) += *delta,
            Cmd::Fight { enemy } => state.messages.push(format!("<fight {enemy}>")),
            Cmd::Teleport { floor, landing } => {
                state.messages.push(format!("<teleport {floor}:{landing}>"));
            }
            Cmd::OpenShop { shop } => state.messages.push(format!("<shop {shop}>")),
            Cmd::CallCommon { name } => state.messages.push(format!("<common {name}>")),
            Cmd::Lua { code } => state.messages.push(format!("<lua {code}>")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talk_give_flag_roundtrip() {
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
        ];
        let json = serde_json::to_string(&cmds).unwrap();
        let back: Vec<Cmd> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 3);

        let mut st = GameState::default();
        run_cmds(&mut st, &back);
        assert_eq!(st.bag.get("gold"), Some(&50));
        assert_eq!(st.flags.get("救出仙子"), Some(&true));
    }
}
