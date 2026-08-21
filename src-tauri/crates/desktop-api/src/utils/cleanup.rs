//! 工具模块 - 清理队列、错误处理

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 全局清理队列 - 自动删除临时文件
pub struct CleanupQueue {
    sender: mpsc::UnboundedSender<PathBuf>,
}

impl CleanupQueue {
    pub fn new() -> Arc<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel::<PathBuf>();

        // 启动后台清理任务
        tokio::spawn(async move {
            let mut pending: VecDeque<(PathBuf, tokio::time::Instant)> = VecDeque::new();
            let cleanup_delay = tokio::time::Duration::from_secs(300); // 5分钟延迟
            let max_size_mb: usize = 100;

            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

            loop {
                interval.tick().await;

                // 接收新文件
                while let Ok(path) = receiver.try_recv() {
                    pending.push_back((path, tokio::time::Instant::now() + cleanup_delay));
                }

                // 执行到期的清理
                let now = tokio::time::Instant::now();
                while let Some((_path, deadline)) = pending.front() {
                    if *deadline > now {
                        break;
                    }
                    let (path, _) = pending.pop_front().unwrap();
                    let _ = std::fs::remove_file(&path);
                    tracing::debug!("cleaned up: {}", path.display());
                }

                // LRU: 如果总量超限，清理最早的（每次删除后重算）
                while Self::calc_size(&pending) > max_size_mb * 1024 * 1024 && pending.len() > 1 {
                    if let Some((path, _)) = pending.pop_front() {
                        let _ = std::fs::remove_file(&path);
                        tracing::debug!("LRU cleanup: removed {}", path.display());
                    }
                }
            }
        });

        Arc::new(Self { sender })
    }

    /// 注册文件待清理
    pub fn send(&self, path: PathBuf) {
        let _ = self.sender.send(path);
    }

    fn calc_size(pending: &VecDeque<(PathBuf, tokio::time::Instant)>) -> usize {
        // 粗略估算
        pending.len() * 500 * 1024 // 假设平均 500KB
    }
}
