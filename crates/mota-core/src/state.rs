//! 最小可玩状态：命名开关/变量 + 背包 + 消息（供试玩与单测）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameState {
    #[serde(default)]
    pub flags: HashMap<String, bool>,
    #[serde(default)]
    pub vars: HashMap<String, i64>,
    #[serde(default)]
    pub bag: HashMap<String, i64>,
    #[serde(default)]
    pub messages: Vec<String>,
}
