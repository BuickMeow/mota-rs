//! mota-rs 核心数据模型：Schema(DB字段) x 模板(笔刷) x 实例(地图格) x 指令流.
//!
//! 约定：数值/地图/事件存 JSON（开发期易读易 diff），发行期可转二进制；
//! 脚本只写逻辑（Lua），不存大量数据。

pub mod battle;
pub mod cmd;
pub mod db;
pub mod lua;
pub mod map;
pub mod schema;
pub mod state;
pub mod tiles;
