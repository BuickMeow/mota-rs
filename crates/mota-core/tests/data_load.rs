//! 真数据回归：data/ 下的所有文件必须能被 core 解析，且符合 schema。

use mota_core::{db, map, schema};
use std::collections::HashMap;

fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn read_json(dir: &str) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(data_dir().join(dir))
        .expect("read data dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    for n in names {
        let text = std::fs::read_to_string(data_dir().join(dir).join(&n)).expect("read file");
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        out.push((n, v));
    }
    out
}

#[test]
fn example_data_parses() {
    let enemy: db::Enemy =
        serde_json::from_str(include_str!("../../../data/enemies/electric_slime.json"))
            .expect("electric_slime.json");
    assert_eq!(enemy.id, "electric_slime");

    let item: db::Item = serde_json::from_str(include_str!("../../../data/items/iron_key.json"))
        .expect("iron_key.json");
    assert!(item.passive);

    let floor: map::Floor =
        serde_json::from_str(include_str!("../../../data/maps/1_1.json")).expect("1_1.json");
    assert!(!floor.instances.is_empty());
}

#[test]
fn all_tables_parse_and_match_schema() {
    let schema_text = std::fs::read_to_string(data_dir().join("schema.json")).expect("schema.json");
    let db_schema = schema::load_schema(&schema_text).expect("schema parses");
    assert_eq!(db_schema.tables.len(), 7);

    let tables = [
        ("enemies", "enemies"),
        ("items", "items"),
        ("shops", "shops"),
        ("skills", "skills"),
        ("doors", "doors"),
        ("barriers", "barriers"),
        ("tilesets", "tilesets"),
    ];
    let skill_ids: std::collections::HashSet<String> = read_json("skills")
        .iter()
        .filter_map(|(_, v)| v.get("id")?.as_str().map(str::to_string))
        .collect();
    for (dir, table) in tables {
        let ts = db_schema.get(table).expect("table in schema");
        let files = read_json(dir);
        assert!(!files.is_empty(), "{dir} 为空");
        for (name, v) in files {
            let obj = v.as_object().expect("record is object");
            let rec: HashMap<String, serde_json::Value> =
                obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            ts.validate_record(&rec)
                .unwrap_or_else(|e| panic!("{dir}/{name} 不符合 schema: {e}"));
            // 再按 struct 硬解析一遍，字段改名/改类型立刻能发现
            let text = serde_json::to_string(&v).unwrap();
            match table {
                "enemies" => {
                    let e: db::Enemy = serde_json::from_str(&text).unwrap();
                    for sk in &e.skills {
                        db::check_skill(sk).unwrap_or_else(|err| {
                            panic!("{dir}/{name} 特技串非法：{err}");
                        });
                        let (id, _) = db::parse_skill(sk).unwrap();
                        assert!(skill_ids.contains(&id), "{dir}/{name} 未知特技 {id}");
                    }
                }
                "items" => {
                    db::load_item(&text).unwrap();
                }
                "shops" => {
                    db::load_shop(&text).unwrap();
                }
                "skills" => {
                    db::load_skill(&text).unwrap();
                }
                "doors" => {
                    db::load_door(&text).unwrap();
                }
                "barriers" => {
                    db::load_barrier(&text).unwrap();
                }
                "tilesets" => {
                    mota_core::tiles::load_tileset(&text).unwrap();
                }
                _ => unreachable!(),
            }
        }
    }
}

/// 楼层交叉引用：怪/物/门/路障 id 必须在表里，商店必须存在，
/// 楼梯目标楼必须存在且落脚点在目标楼。
#[test]
fn maps_reference_known_ids() {
    use std::collections::HashSet;
    let ids = |dir: &str| -> HashSet<String> {
        read_json(dir)
            .iter()
            .filter_map(|(_, v)| v.get("id")?.as_str().map(str::to_string))
            .collect()
    };
    let enemies = ids("enemies");
    let items = ids("items");
    let doors = ids("doors");
    let barriers = ids("barriers");
    let shops = ids("shops");

    // 先把所有楼解析出来（id → 落脚点名集合）
    let floors = read_json("maps");
    assert!(!floors.is_empty());
    let mut landings: HashMap<String, HashSet<String>> = HashMap::new();
    let mut parsed: Vec<(String, map::Floor)> = Vec::new();
    for (name, v) in &floors {
        let text = serde_json::to_string(v).unwrap();
        let f: map::Floor =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} 解析失败：{e}"));
        let mut set = HashSet::new();
        for inst in &f.instances {
            if let map::TemplateKind::Landing { name } = &inst.template {
                set.insert(name.clone());
            }
        }
        landings.insert(f.id.clone(), set);
        parsed.push((name.clone(), f));
    }

    for (fname, f) in &parsed {
        for inst in &f.instances {
            let tag = format!("{fname} {}", inst.id);
            match &inst.template {
                map::TemplateKind::Monster { monster_id } => {
                    assert!(enemies.contains(monster_id), "{tag} 未知怪物 {monster_id}");
                }
                map::TemplateKind::Item { item_id } => {
                    assert!(items.contains(item_id), "{tag} 未知物品 {item_id}");
                }
                map::TemplateKind::Door { door_id } => {
                    assert!(doors.contains(door_id), "{tag} 未知门 {door_id}");
                }
                map::TemplateKind::Barrier { barrier_id } => {
                    assert!(barriers.contains(barrier_id), "{tag} 未知路障 {barrier_id}");
                }
                map::TemplateKind::Npc {
                    shop_id: Some(s), ..
                } => {
                    assert!(shops.contains(s), "{tag} 未知商店 {s}");
                }
                map::TemplateKind::Npc { .. } => {}
                map::TemplateKind::StairUp {
                    to_floor,
                    to_landing,
                }
                | map::TemplateKind::StairDown {
                    to_floor,
                    to_landing,
                } => {
                    let t = landings.get(to_floor).unwrap_or_else(|| {
                        panic!("{tag} 目标楼 {to_floor} 不存在");
                    });
                    assert!(
                        t.contains(to_landing),
                        "{tag} 目标楼 {to_floor} 无落脚点 {to_landing}"
                    );
                }
                _ => {}
            }
        }
    }
}
