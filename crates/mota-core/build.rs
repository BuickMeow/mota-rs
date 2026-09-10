//! 构建期把 `scripts/rules/**/*.lua` 编成内置兜底，新增脚本不用改 Rust。

use std::path::{Path, PathBuf};

fn collect(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, out);
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // 下划线/点开头视为停用；只收 .lua
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        if path.extension().map(|x| x == "lua").unwrap_or(false)
            && let Ok(rel) = path.strip_prefix(base)
        {
            out.push(rel.to_path_buf());
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/rules");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    files.sort();

    let mut out = String::from("/// 构建期自动收集的规则脚本（相对路径, 源码）。\n");
    out.push_str("pub static EMBEDDED_RULES: &[(&str, &str)] = &[\n");
    for rel in &files {
        let abs = root.join(rel);
        println!("cargo:rerun-if-changed={}", abs.display());
        out.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            rel.to_string_lossy(),
            abs.to_string_lossy()
        ));
    }
    out.push_str("];\n");

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("rules_embedded.rs");
    std::fs::write(dest, out).unwrap();
}
