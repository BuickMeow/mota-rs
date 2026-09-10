//! 表结构：每张表列出全部字段（enemy 列全部数值字段，不只列扩展项）。
//!
//! `data/schema.json` 是 `{ "tables": [...] }`，与 `db.rs` 的 struct 一一对应。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 字段类型（够配表用；嵌套结构用 List / Object 兜底到一层）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Int,
    Float,
    Str,
    Bool,
    List,
    Object,
}

/// 单个字段定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub field_type: FieldType,
    #[serde(default)]
    pub default: serde_json::Value,
    #[serde(default)]
    pub description: String,
}

/// 一张表的全部字段（如 enemies / items / shops）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableSchema {
    pub table: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
}

impl TableSchema {
    fn type_ok(ft: FieldType, v: &serde_json::Value) -> bool {
        match ft {
            FieldType::Int => v.is_i64() || v.is_u64(),
            FieldType::Float => v.is_f64() || v.is_i64() || v.is_u64(),
            FieldType::Str => v.is_string(),
            FieldType::Bool => v.is_boolean(),
            FieldType::List => v.is_array(),
            FieldType::Object => v.is_object(),
        }
    }

    /// 校验一条完整记录（JSON 对象）：未知字段报错，已知字段类型不符报错；缺字段不管（走 serde 默认）。
    pub fn validate_record(
        &self,
        record: &HashMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        for (k, v) in record {
            let def = self
                .fields
                .iter()
                .find(|f| &f.name == k)
                .ok_or_else(|| format!("未知字段: {k}"))?;
            if !Self::type_ok(def.field_type, v) {
                return Err(format!("字段 {k} 类型不符，应为 {:?}", def.field_type));
            }
        }
        Ok(())
    }
}

/// 整个 DB 的表集合，对应 `data/schema.json`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbSchema {
    #[serde(default)]
    pub tables: Vec<TableSchema>,
}

impl DbSchema {
    pub fn get(&self, table: &str) -> Option<&TableSchema> {
        self.tables.iter().find(|t| t.table == table)
    }
}

/// 从 JSON 文本解析（IO 由调用方负责）。
pub fn load_schema(s: &str) -> Result<DbSchema, serde_json::Error> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enemies_schema() -> TableSchema {
        TableSchema {
            table: "enemies".to_string(),
            fields: vec![
                FieldDef {
                    name: "id".to_string(),
                    field_type: FieldType::Str,
                    default: json!(""),
                    description: String::new(),
                },
                FieldDef {
                    name: "hp".to_string(),
                    field_type: FieldType::Int,
                    default: json!(0),
                    description: String::new(),
                },
            ],
        }
    }

    #[test]
    fn full_record_ok_type_mismatch_and_unknown() {
        let s = enemies_schema();
        let mut good = HashMap::new();
        good.insert("id".to_string(), json!("slime"));
        good.insert("hp".to_string(), json!(50));
        assert!(s.validate_record(&good).is_ok());

        let mut bad = HashMap::new();
        bad.insert("hp".to_string(), json!("高"));
        assert!(s.validate_record(&bad).is_err());

        let mut unknown = HashMap::new();
        unknown.insert("不存在".to_string(), json!(1));
        assert!(s.validate_record(&unknown).is_err());
    }

    #[test]
    fn missing_fields_are_allowed() {
        let s = enemies_schema();
        let mut partial = HashMap::new();
        partial.insert("id".to_string(), json!("slime"));
        assert!(s.validate_record(&partial).is_ok());
    }
}
