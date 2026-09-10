//! 7630 贴图引擎（从 magic-tower-rs `assets.rs` 移植的最小集）。
//!
//! - Tilesets/magictower.png：256x1216，32px 格，8 列；图块号规则
//!   `<48` 空格，`48..48+7*48` 为 autotile（slot/pattern），`>=384` 为普通图块。
//! - Characters/*.png：4列x4行均分，列=pattern，行=方向（下左上右）。
//! - PNG→RGBA→ColorImage→TextureHandle 常驻显存，切片只算 UV。

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;

pub const TILE: u32 = 32;
pub const TILESET_COLS: u32 = 8;

/// 7630 标配 7 个 autotile（Map004 实测名，单 tileset 工程写死即可）。
pub const AUTOTILE_NAMES: [&str; 7] = [
    "101-CF_Lava01",
    "102-CF_Lava02",
    "074-CW_Water01",
    "076-CW_Grass01",
    "002-G_Shadow01",
    "009-G2_Water01",
    "037-Tree02",
];

/// Character 单格拾取（列=pattern 0..3，行=方向）。
pub fn char_cell_px(col: u32, row: u32, img: (u32, u32)) -> (u32, u32, u32, u32) {
    assert!(col < 4 && row < 4);
    let (cw, ch) = (img.0.max(1) / 4, img.1.max(1) / 4);
    (col * cw, row * ch, cw, ch)
}

/// autotile pattern(0..48) × 象限(TL,TR,BL,BR) → 帧内 16px 源坐标
/// （RMXP 模板布局，与 mkxp `autotileRects` 同源的格式事实表）。
pub const AUTOTILE_QUADS: [[(u32, u32); 4]; 48] = [
    [(32, 64), (48, 64), (32, 80), (48, 80)],
    [(64, 0), (48, 64), (32, 80), (48, 80)],
    [(32, 64), (80, 0), (32, 80), (48, 80)],
    [(64, 0), (80, 0), (32, 80), (48, 80)],
    [(32, 64), (48, 64), (32, 80), (80, 16)],
    [(64, 0), (48, 64), (32, 80), (80, 16)],
    [(32, 64), (80, 0), (32, 80), (80, 16)],
    [(64, 0), (80, 0), (32, 80), (80, 16)],
    [(32, 64), (48, 64), (64, 16), (48, 80)],
    [(64, 0), (48, 64), (64, 16), (48, 80)],
    [(32, 64), (80, 0), (64, 16), (48, 80)],
    [(64, 0), (80, 0), (64, 16), (48, 80)],
    [(32, 64), (48, 64), (64, 16), (80, 16)],
    [(64, 0), (48, 64), (64, 16), (80, 16)],
    [(32, 64), (80, 0), (64, 16), (80, 16)],
    [(64, 0), (80, 0), (64, 16), (80, 16)],
    [(0, 64), (16, 64), (0, 80), (16, 80)],
    [(0, 64), (80, 0), (0, 80), (16, 80)],
    [(0, 64), (16, 64), (0, 80), (80, 16)],
    [(0, 64), (80, 0), (0, 80), (80, 16)],
    [(32, 32), (48, 32), (32, 48), (48, 48)],
    [(32, 32), (48, 32), (32, 48), (80, 16)],
    [(32, 32), (48, 32), (64, 16), (48, 48)],
    [(32, 32), (48, 32), (64, 16), (80, 16)],
    [(64, 64), (80, 64), (64, 80), (80, 80)],
    [(64, 64), (80, 64), (64, 16), (80, 80)],
    [(64, 0), (80, 64), (64, 80), (80, 80)],
    [(64, 0), (80, 64), (64, 16), (80, 80)],
    [(32, 96), (48, 96), (32, 112), (48, 112)],
    [(64, 0), (48, 96), (32, 112), (48, 112)],
    [(32, 96), (80, 0), (32, 112), (48, 112)],
    [(64, 0), (80, 0), (32, 112), (48, 112)],
    [(0, 64), (80, 64), (0, 80), (80, 80)],
    [(32, 32), (48, 32), (32, 112), (48, 112)],
    [(0, 32), (16, 32), (0, 48), (16, 48)],
    [(0, 32), (16, 32), (0, 48), (80, 16)],
    [(64, 32), (80, 32), (64, 48), (80, 48)],
    [(64, 32), (80, 32), (64, 16), (80, 48)],
    [(64, 96), (80, 96), (64, 112), (80, 112)],
    [(64, 0), (80, 96), (64, 112), (80, 112)],
    [(0, 96), (16, 96), (0, 112), (16, 112)],
    [(0, 96), (80, 0), (0, 112), (16, 112)],
    [(0, 32), (80, 32), (0, 48), (80, 48)],
    [(0, 32), (16, 32), (0, 112), (16, 112)],
    [(0, 96), (80, 96), (0, 112), (80, 112)],
    [(64, 32), (80, 32), (64, 112), (80, 112)],
    [(0, 32), (80, 32), (0, 112), (80, 112)],
    [(0, 0), (16, 0), (0, 16), (16, 16)],
];

/// 非标小 autotile（如 7630 岩浆 128x32 条带）：取首格 32x32 静态显示。
pub fn autotile_is_small(size: (u32, u32)) -> bool {
    size.0 < 96 || size.1 < 128
}

/// 图块号 → autotile (slot, pattern)。
pub fn autotile_slot_pattern(id: i32) -> Option<(usize, usize)> {
    if (48..48 + 7 * 48).contains(&id) {
        Some((((id - 48) / 48) as usize, ((id - 48) % 48) as usize))
    } else {
        None
    }
}

/// 普通图块号 → tileset 表内 (col, row)。
pub fn normal_tile_col_row(id: i32) -> Option<(u32, u32)> {
    if id >= 384 {
        let t = (id - 384) as u32;
        Some((t % TILESET_COLS, t / TILESET_COLS))
    } else {
        None
    }
}

/// UV 矩形（像素 → 0..1）。
pub fn uv(px: (u32, u32, u32, u32), img: (u32, u32)) -> egui::Rect {
    let (x, y, w, h) = (px.0 as f32, px.1 as f32, px.2 as f32, px.3 as f32);
    let (iw, ih) = (img.0 as f32, img.1 as f32);
    egui::Rect::from_min_max(
        egui::pos2(x / iw, y / ih),
        egui::pos2((x + w) / iw, (y + h) / ih),
    )
}

/// 当前地图的纹理集（egui 侧持有，缺文件对应槽为 None，绘制时跳过）。
pub struct Textures {
    pub tileset: egui::TextureHandle,
    pub tileset_tiles_high: u32,
    pub autotiles: Vec<Option<egui::TextureHandle>>,
    pub autotile_px: Vec<(u32, u32)>,
    chars: HashMap<(String, i64), egui::TextureHandle>,
    char_px: HashMap<(String, i64), (u32, u32)>,
}

fn load_rgba(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some((w, h, img.into_raw()))
}

fn to_tex(ctx: &egui::Context, name: String, w: u32, h: u32, px: Vec<u8>) -> egui::TextureHandle {
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
    ctx.load_texture(name, img, egui::TextureOptions::NEAREST)
}

/// 在 Characters 目录里大小写容错找行走图（.PNG/.png 混写）。
fn find_character(gfx: &Path, name: &str) -> Option<std::path::PathBuf> {
    let dir = gfx.join("Characters");
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .map(|s| s.eq_ignore_ascii_case(std::ffi::OsStr::new(name)))
                .unwrap_or(false)
        })
}

impl Textures {
    pub fn load(ctx: &egui::Context, gfx: &Path) -> Self {
        let (tw, th, tp) = load_rgba(&gfx.join("Tilesets/magictower.png")).unwrap_or((
            256,
            32,
            vec![0; 256 * 32 * 4],
        ));
        let tileset = to_tex(ctx, "ts:magictower".to_string(), tw, th, tp);
        let mut autotiles = Vec::new();
        let mut autotile_px = Vec::new();
        for n in AUTOTILE_NAMES {
            match load_rgba(&gfx.join(format!("Autotiles/{n}.png"))) {
                Some((w, h, px)) => {
                    autotiles.push(Some(to_tex(ctx, format!("at:{n}"), w, h, px)));
                    autotile_px.push((w, h));
                }
                None => {
                    autotiles.push(None);
                    autotile_px.push((0, 0));
                }
            }
        }
        Self {
            tileset,
            tileset_tiles_high: th / TILE,
            autotiles,
            autotile_px,
            chars: HashMap::new(),
            char_px: HashMap::new(),
        }
    }

    /// 行走图（hue 0–360，0＝原色）：按（名，色相）缓存，绿门 hue120 即走这里。
    pub fn character(
        &mut self,
        ctx: &egui::Context,
        gfx: &Path,
        name: &str,
        hue: i64,
    ) -> Option<egui::TextureHandle> {
        if name.is_empty() {
            return None;
        }
        let hue = hue.rem_euclid(360);
        let key = (name.to_owned(), hue);
        if let Some(t) = self.chars.get(&key) {
            return Some(t.clone());
        }
        let (w, h, mut px) = load_rgba(&find_character(gfx, name)?)?;
        if hue != 0 {
            shift_hue_in_place(&mut px, hue as i32);
        }
        let t = to_tex(ctx, format!("ch:{name}:{hue}"), w, h, px);
        self.char_px.insert(key.clone(), (w, h));
        self.chars.insert(key, t.clone());
        Some(t)
    }

    pub fn char_size(&self, name: &str, hue: i64) -> (u32, u32) {
        let hue = hue.rem_euclid(360);
        self.char_px
            .get(&(name.to_owned(), hue))
            .or_else(|| self.char_px.get(&(name.to_owned(), 0)))
            .copied()
            .unwrap_or((128, 128))
    }
}

/// 画一个图块号（普通/autotile/空格都处理，失败静默跳过）。
pub fn draw_tile_id(
    t: &Textures,
    painter: &egui::Painter,
    tid: i32,
    dst: egui::Rect,
    tint: egui::Color32,
) {
    if tid < 48 {
        return;
    }
    if let Some((slot, pat)) = autotile_slot_pattern(tid) {
        let Some(at) = t.autotiles.get(slot).and_then(|o| o.clone()) else {
            return;
        };
        let size = t.autotile_px.get(slot).copied().unwrap_or((96, 128));
        if autotile_is_small(size) {
            painter.image(at.id(), dst, uv((0, 0, 32, 32), size), tint);
            return;
        }
        // 动画帧仅取第 0 帧（编辑器静态显示）。
        let q = dst.width() / 2.0;
        for (i, (sx, sy)) in AUTOTILE_QUADS[pat].iter().enumerate() {
            let (ox, oy) = ((i % 2) as f32 * q, (i / 2) as f32 * q);
            painter.image(
                at.id(),
                egui::Rect::from_min_size(dst.min + egui::vec2(ox, oy), egui::vec2(q, q)),
                uv((*sx, *sy, 16, 16), size),
                tint,
            );
        }
    } else if let Some((c, r)) = normal_tile_col_row(tid) {
        if r >= t.tileset_tiles_high {
            return;
        }
        painter.image(
            t.tileset.id(),
            dst,
            uv((c * 32, r * 32, 32, 32), (256, t.tileset_tiles_high * 32)),
            tint,
        );
    }
}

/// 行走格 dst：按实际格高宽比，底边居中对齐 tile 底（RMXP 式，标准 32x48 伸出格上沿）。
pub fn char_dst(dst: egui::Rect, size: (u32, u32)) -> egui::Rect {
    let (cw, ch) = (size.0.max(1) as f32 / 4.0, size.1.max(1) as f32 / 4.0);
    let (w, h) = (dst.width() * cw / 32.0, dst.height() * ch / 32.0);
    egui::Rect::from_min_size(
        egui::pos2(dst.center().x - w / 2.0, dst.max.y - h),
        egui::vec2(w, h),
    )
}

/// 画一个行走图格（缺图画暗红块兜底，不静默，方便发现映射缺失）。
#[allow(clippy::too_many_arguments)]
pub fn draw_char(
    t: &mut Textures,
    ctx: &egui::Context,
    gfx: &Path,
    painter: &egui::Painter,
    name: &str,
    hue: i64,
    col: u32,
    row: u32,
    dst: egui::Rect,
    tint: egui::Color32,
) {
    let size = t.char_size(name, hue);
    if let Some(h) = t.character(ctx, gfx, name, hue) {
        painter.image(
            h.id(),
            char_dst(dst, size),
            uv(char_cell_px(col.min(3), row.min(3), size), size),
            tint,
        );
    } else {
        painter.rect_filled(dst, 0.0, egui::Color32::DARK_RED);
    }
}

/// 色相旋转（度）：RGB→HSL→旋 H→RGB，就地改 rgba 缓冲。
fn shift_hue_in_place(px: &mut [u8], hue: i32) {
    let shift = (hue.rem_euclid(360)) as f32;
    if shift == 0.0 {
        return;
    }
    for c in px.chunks_exact_mut(4) {
        let (r, g, b) = (
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        );
        let (mut h, s, l) = rgb_to_hsl(r, g, b);
        if s > 0.0 {
            h = (h + shift) % 360.0;
            let (nr, ng, nb) = hsl_to_rgb(h, s, l);
            c[0] = (nr * 255.0).round().clamp(0.0, 255.0) as u8;
            c[1] = (ng * 255.0).round().clamp(0.0, 255.0) as u8;
            c[2] = (nb * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = h / 360.0;
    let ch = |t: f32| {
        let mut t = t % 1.0;
        if t < 0.0 {
            t += 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (ch(hk + 1.0 / 3.0), ch(hk), ch(hk - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_math_matches_rmxp() {
        assert_eq!(normal_tile_col_row(384), Some((0, 0)));
        assert_eq!(normal_tile_col_row(384 + 8), Some((0, 1)));
        assert_eq!(normal_tile_col_row(384 + 303), Some((7, 37)));
        assert_eq!(normal_tile_col_row(48), None);
        assert_eq!(autotile_slot_pattern(48), Some((0, 0)));
        assert_eq!(autotile_slot_pattern(48 + 6 * 48 + 47), Some((6, 47)));
        assert_eq!(autotile_slot_pattern(384), None);
        assert_eq!(char_cell_px(3, 3, (128, 128)), (96, 96, 32, 32));
        assert_eq!(char_cell_px(1, 2, (96, 128)), (24, 64, 24, 32));
        assert!(autotile_is_small((128, 32)));
        assert!(!autotile_is_small((384, 128)));
    }

    #[test]
    fn hue_zero_is_identity() {
        let mut px = vec![255, 0, 0, 255, 0, 255, 0, 255];
        shift_hue_in_place(&mut px, 0);
        assert_eq!(px, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        // 灰色（s=0）任何色相都不变
        let mut gray = vec![128, 128, 128, 255];
        shift_hue_in_place(&mut gray, 120);
        assert_eq!(gray, vec![128, 128, 128, 255]);
    }

    #[test]
    fn bundled_graphics_exist() {
        let gfx = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/Graphics");
        assert!(gfx.join("Tilesets/magictower.png").exists());
        assert!(gfx.join("Characters/011-Braver01.png").exists());
        assert!(gfx.join("Autotiles/101-CF_Lava01.png").exists());
    }
}
