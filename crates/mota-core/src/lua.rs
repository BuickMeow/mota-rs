//! Lua 桥：可视化指令生成 Lua / 手写 Lua 片段调回 Rust 状态。
//!
//! 只暴露魔塔 API（talk/give/set_flag），默认无 IO，保持沙箱。

use crate::state::GameState;
use mlua::{Lua, Result as LuaResult};
use std::cell::RefCell;
use std::rc::Rc;

/// 执行一段事件 Lua（如可视化生成的代码或 Lua 指令块）。
pub fn eval_snippet(state: &mut GameState, code: &str) -> LuaResult<()> {
    let lua = Lua::new();
    let msgs = Rc::new(RefCell::new(std::mem::take(&mut state.messages)));
    let bag = Rc::new(RefCell::new(std::mem::take(&mut state.bag)));
    let flags = Rc::new(RefCell::new(std::mem::take(&mut state.flags)));

    {
        let m = Rc::clone(&msgs);
        let f = lua.create_function(move |_: &Lua, text: String| {
            m.borrow_mut().push(text);
            Ok(())
        })?;
        lua.globals().set("talk", f)?;
    }
    {
        let b = Rc::clone(&bag);
        let f = lua.create_function(move |_: &Lua, (item, n): (String, i64)| {
            *b.borrow_mut().entry(item).or_insert(0) += n;
            Ok(())
        })?;
        lua.globals().set("give", f)?;
    }
    {
        let f2 = Rc::clone(&flags);
        let f = lua.create_function(move |_: &Lua, (name, value): (String, bool)| {
            f2.borrow_mut().insert(name, value);
            Ok(())
        })?;
        lua.globals().set("set_flag", f)?;
    }

    let out = lua.load(code).exec();
    state.messages = Rc::try_unwrap(msgs).map_or_else(|m| m.borrow().clone(), |m| m.into_inner());
    state.bag = Rc::try_unwrap(bag).map_or_else(|b| b.borrow().clone(), |b| b.into_inner());
    state.flags = Rc::try_unwrap(flags).map_or_else(|f| f.borrow().clone(), |f| f.into_inner());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_talk_give_flag() {
        let mut st = GameState::default();
        eval_snippet(
            &mut st,
            r#"talk("欢迎来到第5层") give("gold", 50) set_flag("救出仙子", true)"#,
        )
        .unwrap();
        assert_eq!(st.messages, vec!["欢迎来到第5层".to_string()]);
        assert_eq!(st.bag.get("gold"), Some(&50));
        assert_eq!(st.flags.get("救出仙子"), Some(&true));
    }
}
