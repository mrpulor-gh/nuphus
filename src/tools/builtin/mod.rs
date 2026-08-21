//! 内置工具子模块
//!
//! 按工具类别拆分:
//! - file   — file_list_dir / file_stat / file_append (目录浏览衔接链)
//! - web    — web::search / web::extract (HTTP 搜索与抓取)
//! - generation — image_generate / video_generate (MiniMax 生成 API 通用调用器)

pub mod diff;
pub mod experience;
pub mod file;
pub mod generation;
pub mod http;
pub mod planner;
pub mod tenet;
pub mod ui_maps;
pub mod user_input;
pub mod video;
pub mod web;

/// 在 tokio 运行时中执行阻塞任务。
///
/// 优先使用 `tokio::task::block_in_place`（不创建额外线程，调度器感知阻塞），
/// 不在 tokio 上下文中时 fallback 到 `std::thread::spawn`。
pub fn run_blocking<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_rt) => {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tokio::task::block_in_place(f)
            })) {
                Ok(result) => result,
                Err(_) => Err("blocking task panicked".to_string()),
            }
        }
        Err(_) => match std::thread::spawn(f).join() {
            Ok(result) => result,
            Err(e) => Err(format!("thread panicked: {:?}", e)),
        },
    }
}
