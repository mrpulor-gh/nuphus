//! 跨进程自动化锁（与 nuphus-mcp 侧批次A 加固后的实现一致）。
//!
//! 桌面（鼠标/键盘/屏幕）与浏览器自动化操作的是**独占机器资源**——任意时刻
//! 至多一个 Agent 能执行自动化操作。主仓库直连通道与各 nuphus-mcp 实例
//! （每个 Agent 一个 stdio 实例）通过**同一把锁文件**协调互斥。
//!
//! 语义（产品决策，与 nuphus-mcp 保持一致）：
//! - **全包**：`browser_*` 与 `desktop_*` 直连工具都获取锁。
//! - **短暂持锁**：锁仅在单次工具调用期间持有，调用结束立即释放（Drop guard），
//!   绝不绑定 Agent 会话；空闲的进程不持有任何锁，任何 Agent 都能获取。
//! - **占用拒绝**：存在活持有者时获取失败并返回明确的 busy 提示，不阻塞，
//!   调用方应稍后重试。
//! - **崩溃自愈**：崩溃进程遗留的过期锁通过 TTL（90s）自动回收。
//!
//! 加固（nuphus-mcp 审计批次A 同步）：
//! - **原子发布**：记录先写临时文件再 hard_link 就位——hard_link 在锁存在时
//!   原子失败，任何进程都不会读到半截锁记录，且保持 create-if-absent 互斥语义。
//! - **持有期心跳续期**：持有者每 TTL/3 刷新 expires_at，长工具调用
//!   （如 browser_wait_for 大超时）不会突破自己的锁。崩溃残留仍被 TTL 回收——
//!   心跳随进程死亡，每次只把 expires_at 推进有限区间。
//! - **token 所有权 + rename-before-delete**：每次获取写入唯一 token。
//!   释放/回收先把锁文件 rename 到一边（原子捕获该路径当时的文件），校验捕获
//!   记录的 token，匹配才删除——迟到的 Drop 绝不会删掉新持有者的锁。
//!
//! 锁文件位置：`{data_dir}/Nuphus/nuphus-mcp/automation.lock` —— 与 nuphus-mcp
//! 侧**完全同一路径**，因此主仓库直连与 MCP 通道真正互斥。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 过期锁允许存活的时长（崩溃自愈）。持有期由心跳刷新，只有死亡/挂起的持有者才会让它过期。
const LOCK_TTL_SECS: u64 = 90;

/// 心跳节奏：在 TTL 过期前充分刷新 expires_at。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(LOCK_TTL_SECS / 3);

/// 锁文件名（位于 `{data_dir}/Nuphus/nuphus-mcp/`）。
const LOCK_FILE_NAME: &str = "automation.lock";

/// 锁 token 的进程内唯一性来源（pid 可能被系统回收复用）。
static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 写入锁文件的持有者信息。
#[derive(Debug, Serialize, Deserialize)]
struct LockRecord {
    pid: u32,
    /// 每次获取唯一——释放/回收时的所有权证明。旧版本写入的记录没有该字段，
    /// 反序列化为空串，不会通过所有权校验，只经 TTL 过期路径回收。
    #[serde(default)]
    token: String,
    tool: String,
    acquired_at: u64,
    expires_at: u64,
}

/// 跨进程自动化锁句柄。
pub struct AutomationLock {
    path: PathBuf,
}

/// RAII guard：Drop 释放锁（并停止心跳线程）。
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
    token: String,
    /// Drop sender 即断开心跳的 recv_timeout → 心跳线程退出。
    heartbeat_stop: Option<mpsc::Sender<()>>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

impl AutomationLock {
    /// 解析共享锁文件路径（跨平台 data dir）。
    ///
    /// `#[cfg(test)]` 下锁落在 temp 目录，单元/集成测试不触碰真实用户数据目录
    ///（并行测试否则会争抢生产锁文件）。
    pub fn new() -> Self {
        #[cfg(test)]
        {
            let dir = std::env::temp_dir().join("nuphus_automation_lock_test");
            let _ = fs::create_dir_all(&dir);
            Self {
                path: dir.join(LOCK_FILE_NAME),
            }
        }
        #[cfg(not(test))]
        {
            let base =
                dirs::data_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let dir = base.join("Nuphus").join("nuphus-mcp");
            let _ = fs::create_dir_all(&dir);
            Self {
                path: dir.join(LOCK_FILE_NAME),
            }
        }
    }

    /// 尝试为单次工具调用获取锁。
    ///
    /// 锁空闲（或过期锁被回收）时返回 `Ok(guard)`；
    /// 存在活持有者时返回 `Err(busy)`——调用方将其作为语义失败返回给上层。
    pub fn acquire(&self, tool: &str) -> Result<LockGuard, String> {
        for _attempt in 0..3 {
            let now = unix_now();
            let record = LockRecord {
                pid: std::process::id(),
                token: new_token(),
                tool: tool.to_string(),
                acquired_at: now,
                expires_at: now + LOCK_TTL_SECS,
            };
            if publish_new(&self.path, &record)? {
                tracing::debug!(
                    "[automation-lock] acquired for '{tool}' (pid={}, token={})",
                    record.pid,
                    record.token
                );
                return Ok(LockGuard::new(self.path.clone(), record.token));
            }

            // 锁文件存在：检查持有者。
            match read_lock_record(&self.path) {
                Some(rec) if rec.expires_at > unix_now() => {
                    // 活持有者 → busy。报告持有者（工具 + pid）便于上层决定何时重试。
                    return Err(format!(
                        "Automation is busy: another Agent (pid={}) is running '{}'. \
                         Only one Agent can operate the desktop/browser at a time; retry later.",
                        rec.pid, rec.tool
                    ));
                }
                _ => {
                    // 过期或不可读的锁 → 原子回收后重试。
                    tracing::warn!(
                        "[automation-lock] stale lock detected, reclaiming ({})",
                        self.path.display()
                    );
                    reclaim_stale(&self.path);
                }
            }
        }
        Err(
            "automation lock contention: could not acquire after reclaiming a stale lock; \
             retry later."
                .to_string(),
        )
    }

    /// 锁文件路径（测试/诊断用）。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl LockGuard {
    fn new(path: PathBuf, token: String) -> Self {
        let (tx, rx) = mpsc::channel();
        let heartbeat = spawn_heartbeat(
            path.clone(),
            token.clone(),
            LOCK_TTL_SECS,
            HEARTBEAT_INTERVAL,
            rx,
        );
        Self {
            path,
            token,
            heartbeat_stop: Some(tx),
            heartbeat: Some(heartbeat),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // 断开 sender 立即唤醒心跳的 recv_timeout；join 保证释放后不会有
        // 在途的续期写把记录写回来。
        drop(self.heartbeat_stop.take());
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        release_owned(&self.path, &self.token);
    }
}

/// 每次获取唯一的 token：pid + 计数器 + 纳秒（进程重启复用 pid 也无法伪造所有权）。
fn new_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed),
        nanos
    )
}

/// 原子发布锁记录（**仅当锁文件不存在**）：先把内容写到同目录临时文件，
/// 再 hard_link 就位。hard_link 在锁被持有时以 AlreadyExists 原子失败——
/// 不存在"先创建后写"的半截记录窗口。
///
/// 返回 `Ok(true)` = 发布成功，`Ok(false)` = 锁已存在。
fn publish_new(path: &Path, record: &LockRecord) -> Result<bool, String> {
    let payload = serde_json::to_string(record)
        .map_err(|e| format!("automation lock serialize failed: {e}"))?;
    let tmp = scratch_path(path, "tmp", &record.token);
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| format!("automation lock tmp create failed: {e}"))?;
        if let Err(e) = file
            .write_all(payload.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&tmp);
            return Err(format!("automation lock write failed: {e}"));
        }
    }
    let result = match fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(format!("automation lock publish failed: {e}")),
    };
    let _ = fs::remove_file(&tmp);
    result
}

/// 原子回收过期锁：先 rename 到一边，再复查捕获的记录。只有我们检查过的那一份
/// 记录会被删除——如果在检查与搬移之间它变成活锁（心跳续期），原样放回。
fn reclaim_stale(path: &Path) {
    let claim = scratch_path(path, "reclaim", &new_token());
    if fs::rename(path, &claim).is_err() {
        return; // 已消失/被搬走——其他进程在处理
    }
    let still_stale = match read_lock_record(&claim) {
        Some(rec) => rec.expires_at <= unix_now(),
        None => true, // 不可读/垃圾 → 视为过期
    };
    if still_stale {
        let _ = fs::remove_file(&claim);
    } else {
        // 竞态窗口内变成活锁——放回去（若期间已有新锁就位则失败也无妨，记录本来就是活的）。
        let _ = fs::rename(&claim, path);
    }
}

/// 只释放**自己的**记录：先把锁文件 rename 到一边（原子捕获该路径当时的文件），
/// 校验捕获记录的 token，匹配才删除。如果记录是他人的（我们 TTL 过期后被回收重建），
/// 原样放回，绝不误删。
fn release_owned(path: &Path, token: &str) {
    let claim = scratch_path(path, "release", token);
    if let Ok(()) = fs::rename(path, &claim) {
        match read_lock_record(&claim) {
            Some(rec) if rec.token == token => {
                let _ = fs::remove_file(&claim);
                tracing::debug!(
                    "[automation-lock] released (pid={}, token={token})",
                    std::process::id()
                );
            }
            _ => {
                // 不是我们的：放回合法持有者的锁。若第三方已在该路径发布新锁，
                // 被捕获记录的持有者反正已失去路径——删掉捕获文件避免残留。
                if fs::rename(&claim, path).is_err() {
                    let _ = fs::remove_file(&claim);
                }
            }
        }
    }
    // Err(_) 分支为空：锁文件已消失（被回收）——无需释放
}

/// 心跳循环：持有期间每隔 interval 刷新 expires_at。停止条件：stop 通道断开
/// （guard Drop）或失去所有权。崩溃安全：线程随进程死亡，每次续期只把
/// expires_at 推进 ttl_secs，崩溃残留仍按正常 TTL 路径回收。
fn spawn_heartbeat(
    path: PathBuf,
    token: String,
    ttl_secs: u64,
    interval: Duration,
    stop: mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(interval) {
            if !renew(&path, &token, ttl_secs) {
                tracing::debug!("[automation-lock] heartbeat stopped: lock ownership lost");
                break;
            }
        }
        // 停止信号，或 sender 断开（guard 被 Drop / panic 展开）→ 退出循环
    })
}

/// 仍持有记录时刷新 expires_at。锁已丢失（记录消失或属于他人）返回 false——心跳停止。
fn renew(path: &Path, token: &str, ttl_secs: u64) -> bool {
    let Some(mut rec) = read_lock_record(path) else {
        return false;
    };
    if rec.token != token {
        return false;
    }
    rec.expires_at = unix_now() + ttl_secs;
    let Ok(payload) = serde_json::to_string(&rec) else {
        return true; // 暂时性失败：下一拍重试
    };
    let tmp = scratch_path(path, "renew", token);
    {
        use std::io::Write;
        let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
        else {
            return true; // 暂时性失败：下一拍重试
        };
        if file.write_all(payload.as_bytes()).is_err() {
            let _ = fs::remove_file(&tmp);
            return true;
        }
    }
    // 原子替换；读者永远看不到半截记录。
    let ok = fs::rename(&tmp, path).is_ok();
    if !ok {
        let _ = fs::remove_file(&tmp);
    }
    ok
}

/// 原子发布/续期/释放编舞使用的同目录临时文件路径。
fn scratch_path(path: &Path, kind: &str, token: &str) -> PathBuf {
    path.with_extension(format!("{kind}.{token}"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_lock_record(path: &std::path::Path) -> Option<LockRecord> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 使用 temp 锁路径，避免测试污染真实 data dir。
    fn temp_lock(dir_name: &str) -> AutomationLock {
        let path = std::env::temp_dir()
            .join("nuphus_automation_lock_test")
            .join(dir_name);
        let _ = fs::create_dir_all(&path);
        AutomationLock {
            path: path.join(LOCK_FILE_NAME),
        }
    }

    #[test]
    fn acquire_release_cycle() {
        let lock = temp_lock("cycle");
        let _ = fs::remove_file(lock.path());
        {
            let guard = lock.acquire("browser_navigate").expect("first acquire ok");
            assert!(guard.path.exists());
        } // guard dropped → released
        assert!(!lock.path().exists(), "lock file removed after guard drop");
    }

    #[test]
    fn busy_while_held() {
        let lock = temp_lock("busy");
        let _ = fs::remove_file(lock.path());
        let _guard = lock.acquire("desktop_mouse").expect("first acquire ok");
        let err = lock
            .acquire("browser_click")
            .expect_err("second acquire busy");
        assert!(err.contains("busy"), "error mentions busy: {err}");
        assert!(
            err.contains("desktop_mouse"),
            "error names the holding tool"
        );
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let lock = temp_lock("stale");
        let _ = fs::remove_file(lock.path());
        let stale = LockRecord {
            pid: 999_999,
            token: "stale-token".into(),
            tool: "browser_navigate".into(),
            acquired_at: 0,
            expires_at: unix_now().saturating_sub(1), // 已过期
        };
        fs::write(lock.path(), serde_json::to_string(&stale).unwrap()).unwrap();
        let guard = lock
            .acquire("browser_snapshot")
            .expect("stale lock reclaimed");
        // 新持有者必须是本进程。
        let rec = read_lock_record(lock.path()).expect("record readable");
        assert_eq!(rec.pid, std::process::id());
        assert_eq!(rec.token, guard.token);
        drop(guard);
    }

    #[test]
    fn guard_drop_does_not_delete_others_lock() {
        let lock = temp_lock("owner_check");
        let _ = fs::remove_file(lock.path());
        // 模拟：我们持有锁，随后另一个进程回收并写入自己的记录（如我们 TTL 过期后）。
        // Drop 我们的 guard 绝不能删除新持有者的锁。
        {
            let guard = lock.acquire("desktop_screenshot").expect("acquire ok");
            let stolen = LockRecord {
                pid: 888_888,
                token: "other-owner-token".into(),
                tool: "desktop_mouse".into(),
                acquired_at: unix_now(),
                expires_at: unix_now() + LOCK_TTL_SECS,
            };
            fs::write(lock.path(), serde_json::to_string(&stolen).unwrap()).unwrap();
            drop(guard); // 必须放回他人记录，而不是删除
            assert!(lock.path().exists(), "new owner's lock preserved");
            let rec = read_lock_record(lock.path()).expect("foreign record intact");
            assert_eq!(rec.token, "other-owner-token");
            assert_eq!(rec.pid, 888_888);
        }
        let _ = fs::remove_file(lock.path());
    }

    #[test]
    fn publish_leaves_no_scratch_files() {
        let lock = temp_lock("no_scratch");
        let _ = fs::remove_file(lock.path());
        {
            let _guard = lock.acquire("browser_navigate").expect("acquire ok");
        }
        let dir = lock.path().parent().expect("lock has parent dir");
        let leftovers: Vec<_> = fs::read_dir(dir)
            .expect("dir readable")
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.contains(".tmp.") || name.contains(".renew.") || name.contains(".release.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "scratch files leaked: {:?}",
            leftovers
        );
    }

    #[test]
    fn tokens_are_unique_per_acquisition() {
        let lock = temp_lock("tokens");
        let _ = fs::remove_file(lock.path());
        let t1 = {
            let g = lock.acquire("desktop_mouse").expect("acquire 1");
            g.token.clone()
        };
        let t2 = {
            let g = lock.acquire("desktop_mouse").expect("acquire 2");
            g.token.clone()
        };
        assert_ne!(t1, t2, "each acquisition must stamp a unique token");
    }

    #[test]
    fn renew_extends_expiry_while_owned() {
        let lock = temp_lock("renew");
        let _ = fs::remove_file(lock.path());
        let guard = lock.acquire("browser_wait_for").expect("acquire ok");
        let before = read_lock_record(lock.path()).expect("record").expires_at;
        assert!(renew(lock.path(), &guard.token, LOCK_TTL_SECS));
        let after = read_lock_record(lock.path()).expect("record").expires_at;
        assert!(after >= before, "renew must not move expiry backwards");
        // 错误 token 不得改写记录。
        assert!(!renew(lock.path(), "forged-token", LOCK_TTL_SECS));
        drop(guard);
    }

    #[test]
    fn heartbeat_thread_refreshes_expiry_until_stopped() {
        let lock = temp_lock("heartbeat");
        let _ = fs::remove_file(lock.path());
        let guard = lock.acquire("browser_wait_for").expect("acquire ok");
        let initial = read_lock_record(lock.path()).expect("record").expires_at;

        // 差异化 TTL 让续期效果无歧义：expires_at 是秒级时间戳，
        // 同 TTL 的心跳若与获取落在同一秒，效果无法与"未续期"区分。
        let (tx, rx) = mpsc::channel();
        let handle = spawn_heartbeat(
            lock.path().to_path_buf(),
            guard.token.clone(),
            LOCK_TTL_SECS + 100,
            Duration::from_millis(20),
            rx,
        );
        // 轮询直到心跳可见地刷新记录（有界）。
        let mut renewed = initial;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            renewed = read_lock_record(lock.path()).expect("record").expires_at;
            if renewed > initial {
                break;
            }
        }
        drop(tx); // 停止信号
        handle.join().expect("heartbeat joins");

        assert!(
            renewed > initial,
            "heartbeat must refresh expires_at ({renewed} > {initial})"
        );
        drop(guard);
        assert!(!lock.path().exists(), "lock released after guard drop");
    }

    #[test]
    fn heartbeat_stops_when_lock_lost() {
        let lock = temp_lock("heartbeat_lost");
        let _ = fs::remove_file(lock.path());
        let guard = lock.acquire("desktop_mouse").expect("acquire ok");
        // 模拟被他进程回收：我们的记录消失。
        fs::remove_file(lock.path()).expect("remove lock");

        let (_tx, rx) = mpsc::channel();
        let handle = spawn_heartbeat(
            lock.path().to_path_buf(),
            guard.token.clone(),
            LOCK_TTL_SECS,
            Duration::from_millis(20),
            rx,
        );
        // 心跳必须自己发现失去所有权并退出——无需停止信号。
        handle.join().expect("heartbeat exits when lock lost");
        // guard Drop 必须容忍文件已不存在。
        drop(guard);
    }

    #[test]
    fn concurrent_acquire_exactly_one_wins() {
        use std::sync::{Arc, Barrier};
        let lock = temp_lock("race");
        let _ = fs::remove_file(lock.path());
        let path = lock.path().to_path_buf();
        let threads = 8;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::new();
        for _ in 0..threads {
            let barrier = barrier.clone();
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let lock = AutomationLock { path };
                barrier.wait();
                // 把 guard 本体返回：统计时每个竞速者要么仍持有要么已被拒——
                // 提前释放后的顺序获取不能证明互斥。
                lock.acquire("desktop_screenshot")
            }));
        }
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("racer thread joins"))
            .collect();
        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "exactly one racer may hold the lock at a time");
        for r in &results {
            if let Err(e) = r {
                assert!(e.contains("busy"), "losers see busy, not errors: {e}");
            }
        }
        drop(results); // 全部 guard 在此释放
        assert!(
            !lock.path().exists(),
            "winning guard released the lock on drop"
        );
    }
}
