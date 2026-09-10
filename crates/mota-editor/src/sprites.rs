//! 模板 → 7630 贴图：实例换贴图优先，否则查 DB 默认贴图，都没有则标红。
//!
//! - NPC/路障/楼梯/落脚/箱子走固定映射；怪/物/门查 `data/` 默认贴图。
//! - 楼梯是图块（上633/下632）；路障画岩浆 autotile 48；箱子用 EV017 同款 582。

use std::collections::HashMap;

use mota_core::db::{Barrier, Door, Enemy, Item, SpriteRef};
use mota_core::map::TemplateKind;

/// 行走图精灵描述（pattern=列，row=行，hue 色相，opacity 不透明度）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharSpec {
    pub file: String,
    pub hue: i64,
    pub col: u32,
    pub row: u32,
    pub opacity: i64,
}

/// 一个模板在画布上的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sprite {
    Char(CharSpec),
    /// 图块号（楼梯/路障这类 tile 图形）。
    Tile(i32),
}

impl From<&SpriteRef> for CharSpec {
    fn from(v: &SpriteRef) -> Self {
        Self {
            file: v.file.clone(),
            hue: v.hue,
            col: v.col,
            row: v.row,
            opacity: v.opacity,
        }
    }
}

/// 勇士（011-Braver01，站立帧 0 朝下；行走动画由 playtest 按朝向/帧号切）。
pub fn hero_sprite() -> CharSpec {
    CharSpec {
        file: "011-Braver01".to_string(),
        hue: 0,
        col: 0,
        row: 0,
        opacity: 255,
    }
}

fn ch(file: &str, col: u32, row: u32) -> Sprite {
    Sprite::Char(CharSpec {
        file: file.to_string(),
        hue: 0,
        col,
        row,
        opacity: 255,
    })
}

/// 解析实例显示：换贴图 > DB 默认 > 固定映射；DB 无此 id 返回 None（画红块警示）。
pub fn sprite_of(
    template: &TemplateKind,
    visual: Option<&SpriteRef>,
    enemies: &HashMap<String, Enemy>,
    items: &HashMap<String, Item>,
    doors: &HashMap<String, Door>,
    barriers: &HashMap<String, Barrier>,
) -> Option<Sprite> {
    if let Some(v) = visual {
        return Some(Sprite::Char(CharSpec::from(v)));
    }
    match template {
        TemplateKind::Npc { .. } => Some(ch("001-npc01", 1, 0)),
        TemplateKind::Monster { monster_id } => enemies
            .get(monster_id)
            .map(|e| Sprite::Char(CharSpec::from(&e.sprite))),
        TemplateKind::Item { item_id } => items
            .get(item_id)
            .map(|i| Sprite::Char(CharSpec::from(&i.sprite))),
        TemplateKind::Barrier { barrier_id } => barriers
            .get(barrier_id)
            .map(|b| Sprite::Char(CharSpec::from(&b.sprite))),
        TemplateKind::Door { door_id } => doors
            .get(door_id)
            .map(|d| Sprite::Char(CharSpec::from(&d.sprite))),
        TemplateKind::StairUp { .. } => Some(Sprite::Tile(633)),
        TemplateKind::StairDown { .. } => Some(Sprite::Tile(632)),
        // 落脚点按名区分图块：含“上”为上楼落点，含“下”为下楼落点，其余兜底。
        TemplateKind::Landing { name } if name.contains('上') => Some(Sprite::Tile(639)),
        TemplateKind::Landing { name } if name.contains('下') => Some(Sprite::Tile(631)),
        TemplateKind::Landing { .. } => Some(Sprite::Tile(632)),
        // 箱子用 7630 EV017 同款图块 582；压力板是纯逻辑标记，不画。
        TemplateKind::Box => Some(Sprite::Tile(582)),
        TemplateKind::Plate { .. } => Some(Sprite::Tile(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mota_core::map::TemplateKind as T;

    fn enemies() -> HashMap<String, Enemy> {
        [("electric_slime", "048-Monster18")]
            .into_iter()
            .map(|(id, file)| {
                (
                    id.to_string(),
                    Enemy {
                        id: id.to_string(),
                        name: id.to_string(),
                        hp: 1,
                        atk: 1,
                        def: 0,
                        mdef: 0,
                        gold: 0,
                        exp: 0,
                        battler: file.to_string(),
                        hue: 0,
                        sprite: SpriteRef {
                            file: file.to_string(),
                            hue: 0,
                            col: 0,
                            row: 0,
                            opacity: 255,
                        },
                        skills: Vec::new(),
                        extra: Default::default(),
                    },
                )
            })
            .collect()
    }

    type Maps = (
        HashMap<String, Enemy>,
        HashMap<String, Item>,
        HashMap<String, Door>,
        HashMap<String, Barrier>,
    );

    fn empty_maps() -> Maps {
        (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }

    #[test]
    fn monster_uses_db_default_and_override_wins() {
        let enemies = enemies();
        let (_, items, doors, barriers) = empty_maps();
        let t = T::Monster {
            monster_id: "electric_slime".to_string(),
        };
        assert_eq!(
            sprite_of(&t, None, &enemies, &items, &doors, &barriers),
            Some(Sprite::Char(CharSpec {
                file: "048-Monster18".to_string(),
                hue: 0,
                col: 0,
                row: 0,
                opacity: 255,
            }))
        );
        // 换贴图优先（骗人物件）
        let fake = SpriteRef {
            file: "001-npc01".to_string(),
            hue: 0,
            col: 0,
            row: 0,
            opacity: 128,
        };
        assert_eq!(
            sprite_of(&t, Some(&fake), &enemies, &items, &doors, &barriers),
            Some(Sprite::Char(CharSpec::from(&fake)))
        );
        // DB 无此怪：标红，不兜底
        let ghost = T::Monster {
            monster_id: "???".to_string(),
        };
        assert_eq!(
            sprite_of(&ghost, None, &enemies, &items, &doors, &barriers),
            None
        );
    }

    #[test]
    fn landing_tile_follows_name() {
        let (enemies, items, doors, barriers) = empty_maps();
        // 名字含“上”→639，含“下”→631，否则兜底 632。
        for (name, tile) in [("上楼梯", 639), ("下楼梯", 631), ("入口", 632)] {
            assert_eq!(
                sprite_of(
                    &T::Landing {
                        name: name.to_string()
                    },
                    None,
                    &enemies,
                    &items,
                    &doors,
                    &barriers
                ),
                Some(Sprite::Tile(tile))
            );
        }
    }

    #[test]
    fn static_kinds_still_resolve() {
        let (enemies, items, doors, barriers) = empty_maps();
        assert_eq!(
            sprite_of(&T::Box, None, &enemies, &items, &doors, &barriers),
            Some(Sprite::Tile(582))
        );
        assert_eq!(
            sprite_of(
                &T::StairUp {
                    to_floor: "f".to_string(),
                    to_landing: "l".to_string()
                },
                None,
                &enemies,
                &items,
                &doors,
                &barriers
            ),
            Some(Sprite::Tile(633))
        );
        assert_eq!(
            sprite_of(
                &T::StairDown {
                    to_floor: "f".to_string(),
                    to_landing: "l".to_string()
                },
                None,
                &enemies,
                &items,
                &doors,
                &barriers
            ),
            Some(Sprite::Tile(632))
        );
    }
}
