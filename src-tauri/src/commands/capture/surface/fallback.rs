//! 三平台之外的稀有目标：无原生遮罩实现（占位）。
//!
//! `surface/mod.rs` 现按三平台分派：Windows → `windows.rs`、macOS → `macos.rs`、
//! Linux → `linux.rs`，三者实现同一契约 `run_overlay(OverlayParams) -> OverlayOutcome`。
//!
//! **本文件不是 macOS/Linux 的兜底**（那两条链路是各自的原生实现）。它只覆盖三平台
//! 之外的目标：这些目标上 `capture/session.rs` 的 native 分支不参与编译，
//! 而 `commands/toolbar.rs` 的 `#[cfg(windows)]` 路由块同样不编译 → 实际由
//! 跨平台 WebView 链路（`frontend/src/capture-overlay/`）承接，无需原生实现。
//!
//! 三平台原生实现真机验证通过后，WebView 链路整体退役（见 `surface/mod.rs` 的退役计划）。
#![allow(dead_code)]
