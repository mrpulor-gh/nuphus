use nuphus::skill::*;
use std::path::PathBuf;

fn test_skill_path() -> PathBuf {
    let root = std::env::var("CARGO_MANIFEST_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    root.join("src-tauri").join("tasks").join("__test_skills__").join("ui_ux_test_skill")
}

#[test]
fn test_skill_install_and_query() {
    // 1. 安装
    let mut registry = SkillRegistry::new();
    let path = test_skill_path();
    let manifest = registry
        .install_from_path(path.to_str().unwrap_or(""))
        .expect("安装失败");
    assert_eq!(manifest.name, "ui-ux-pro-max");

    // 2. 列表
    let list = registry.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "ui-ux-pro-max");
    assert!(list[0].active);

    // 3. 查询 SKILL.md 内容
    let md = registry.get_skill_md("ui-ux-pro-max");
    assert!(md.is_some());
    assert!(md.unwrap().contains("UI/UX Pro Max"));

    // 4. 搜索技能
    let results = registry.search("design");
    assert!(!results.is_empty());

    // 5. 查询知识
    let output = registry.query(&SkillQueryInput {
        query: "design".into(),
        skill: None,
        domain: None,
    });
    assert!(output.total > 0);

    // 6. 卸载
    registry.remove("ui-ux-pro-max").expect("卸载失败");
    assert!(registry.get("ui-ux-pro-max").is_none());

    println!("All tests passed!");
}
