//! 字体：MiSans 全局第一优先级（中西文通吃，单二进制 include 进去）。

use eframe::egui;
use egui::epaint::text::{FontInsert, FontPriority};

/// 装一遍即可：编辑器与试玩副窗口共用同一个 Context，重复调用是空操作。
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "MiSans".to_owned(),
        egui::FontData::from_static(include_bytes!("../../../assets/fonts/MiSans-Regular.otf"))
            .into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(0, "MiSans".to_owned());
    }
    // Material Icons 直接并进 definitions：不能 set_fonts 后再 add_font——
    // set_fonts 整表替换，add_font 看到旧表里已有同名就跳过，图标家族会被丢。
    add_insert(&mut fonts, egui_material_icons::font_insert());
    ctx.set_fonts(fonts);
}

/// 等价 `ctx.add_font`，但直接写进 definitions，不依赖加载时序。
fn add_insert(fonts: &mut egui::FontDefinitions, insert: FontInsert) {
    for f in &insert.families {
        let list = fonts.families.entry(f.family.clone()).or_default();
        match f.priority {
            FontPriority::Highest => list.insert(0, insert.name.clone()),
            FontPriority::Lowest => list.push(insert.name.clone()),
        }
    }
    fonts
        .font_data
        .insert(insert.name.clone(), std::sync::Arc::new(insert.data));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：试玩副窗口共享同一个 Context，重装字体不能把 material-icons 家族挤掉。
    #[test]
    fn setup_fonts_twice_keeps_material_icons() {
        let ctx = egui::Context::default();
        setup_fonts(&ctx);
        setup_fonts(&ctx);
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 480.0),
                )),
                max_texture_side: Some(8192),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let _ = ui.button(egui_material_icons::icons::ICON_SAVE.rich_text());
            });
            // 无渲染后端：手动消费纹理增量，避免 Drop 的 debug_assert
            out.textures_delta.clear();
        }
        let family = egui::FontFamily::Name("material-icons".into());
        ctx.fonts(|f| {
            assert!(
                f.definitions().families.contains_key(&family),
                "material-icons 家族没绑上字体"
            );
        });
    }
}
