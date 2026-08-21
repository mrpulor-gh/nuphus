//! 快速验证：索引 docs/knowledge 目录 + 搜索
//!
//! cargo run --example quick_test
use nuphus_index::{IndexConfig, IndexEngine, QueryRequest};

fn main() {
    let engine = IndexEngine::new(IndexConfig {
        docs_root: "./docs/knowledge".to_string(),
        index_path: "./.nuphus/index/test_index.json".to_string(),
    });

    // 搜索测试
    let results = engine.search(&QueryRequest {
        query: "PowerShell".to_string(),
        max_results: 5,
        ..Default::default()
    });

    println!("\nSearch 'PowerShell': {} hits", results.len());
    for hit in &results {
        println!("  {} ({})  tags={:?}", hit.title, hit.rel_path, hit.tags);
        if !hit.snippet.is_empty() {
            println!("    {}", &hit.snippet[..hit.snippet.len().min(80)]);
        }
    }

    // 清理测试索引
    let _ = std::fs::remove_file("./.nuphus/index/test_index.json");
}
