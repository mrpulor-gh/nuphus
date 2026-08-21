//! SQLite 数据库连接管理
//!
//! 使用 rusqlite 管理数据库连接，自动创建数据库文件和表结构。
//! 数据库路径从配置或默认路径确定。
//!
//! 自建轻量连接池（4 连接），替代全局单例 Mutex<Connection>，
//! 消除多线程并发瓶颈。WAL 模式下多连接安全。

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

/// 连接池大小（可配置）
const POOL_SIZE: usize = 4;

/// 全局数据库路径（一次初始化）
static DB_PATH: OnceLock<PathBuf> = OnceLock::new();

/// 连接池：Mutex 保护 Vec<Connection>，acquire() 弹出，PoolGuard drop 时归还
struct ConnectionPool {
    free: Mutex<Vec<Connection>>,
}

/// 全局连接池
static POOL: OnceLock<ConnectionPool> = OnceLock::new();

/// 获取或初始化数据库路径
fn db_path() -> &'static PathBuf {
    DB_PATH.get_or_init(|| {
        let path = default_db_path();
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        path
    })
}

/// 默认数据库路径：~/.nuphus/nuphus.db
fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .map(|d| d.join("nuphus").join("nuphus.db"))
        .unwrap_or_else(|| PathBuf::from(".").join(".nuphus").join("nuphus.db"))
}

// ══════════════════════════════════════════════════════════════════════════
// 连接池
// ══════════════════════════════════════════════════════════════════════════

/// RAII guard：持有从池中取出的连接，drop 时自动归还
pub struct PoolGuard {
    conn: Option<Connection>,
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            if let Some(pool) = POOL.get() {
                if let Ok(mut free) = pool.free.lock() {
                    free.push(conn);
                }
            }
        }
    }
}

impl std::ops::Deref for PoolGuard {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("PoolGuard conn is None")
    }
}

impl std::ops::DerefMut for PoolGuard {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn.as_mut().expect("PoolGuard conn is None")
    }
}

/// 初始化连接池（懒加载，首次调用时执行）
fn get_or_init_pool() -> crate::Result<&'static ConnectionPool> {
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    // 先构建（失败可重试），成功后再塞入 OnceLock，
    // 避免初始化失败 panic 永久污染 OnceLock 导致进程不可恢复
    let pool = build_pool()?;
    Ok(POOL.get_or_init(|| pool))
}

/// 构建连接池（纯构造，错误以 Result 返回，调用方可重试）
fn build_pool() -> crate::Result<ConnectionPool> {
    let path = db_path();
    tracing::info!(
        "[Store] 初始化连接池 (size={}): {}",
        POOL_SIZE,
        path.display()
    );
    let mut connections = Vec::with_capacity(POOL_SIZE);
    for i in 0..POOL_SIZE {
        let conn = Connection::open(path).map_err(|e| {
            crate::StoreError::ConnectionFailed(format!(
                "无法打开 SQLite 数据库 {}: {}",
                path.display(),
                e
            ))
        })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| {
                crate::StoreError::ConnectionFailed(format!("无法设置 SQLite PRAGMA: {e}"))
            })?;
        // 仅首个连接执行建表/迁移（避免并发 DDL 冲突）
        if i == 0 {
            init_tables(&conn).map_err(|e| {
                crate::StoreError::MigrationFailed(format!("无法初始化数据库表结构: {e}"))
            })?;
        }
        connections.push(conn);
    }
    Ok(ConnectionPool {
        free: Mutex::new(connections),
    })
}

/// 从连接池获取一个连接（阻塞直到有可用连接，30s 超时兜底）
///
/// 返回的 `PoolGuard` 实现了 `Deref<Target = Connection>`，
/// 可直接调用 `Connection` 的方法。
/// drop 时自动归还连接到池中。
pub fn acquire() -> crate::Result<PoolGuard> {
    let pool = get_or_init_pool()?;
    // 自旋等待可用连接（业务代码持有时间短，自旋即可）；超时防止连接泄漏导致永久挂起
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        {
            // 锁中毒时恢复数据继续使用（池中 Vec<Connection> 无一致性风险）
            let mut free = pool.free.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(conn) = free.pop() {
                return Ok(PoolGuard { conn: Some(conn) });
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(crate::StoreError::ConnectionFailed(
                "连接池获取超时（30s）：连接可能泄漏".to_string(),
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 获取池中当前可用连接数（用于监控/诊断）
pub fn pool_available() -> usize {
    POOL.get()
        .and_then(|p| p.free.lock().ok())
        .map(|free| free.len())
        .unwrap_or(0)
}

// ══════════════════════════════════════════════════════════════════════════
// 表结构初始化
// ══════════════════════════════════════════════════════════════════════════

/// 幂等加列：已存在则跳过，不存在则 ALTER TABLE ADD COLUMN。
/// 用于给已存量的旧表补充新增列（CREATE TABLE IF NOT EXISTS 不会改旧表）。
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    col_type: &str,
) -> rusqlite::Result<()> {
    let exists: bool = conn.query_row(
        &format!(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('{}') WHERE name = '{}'",
            table, column
        ),
        [],
        |r| r.get(0),
    )?;
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table, column, col_type
        ))?;
    }
    Ok(())
}

/// 初始化数据库表结构（纯声明式建表，无任何历史数据迁移/兼容逻辑）
fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
    // 记忆条目表 v4：一维 kind 分类（替代 source 三维混乱）+ goal_type 白名单约束
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS memory_entries (
            id              TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL,
            turn_id         TEXT NOT NULL,
            sequence        INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL,
            wall_clock_ms   INTEGER NOT NULL,
            agent_type      TEXT NOT NULL,
            kind            TEXT NOT NULL CHECK(kind IN ('conversation','task_trace','distill','pattern','snapshot')),
            task_chain_id   TEXT,
            chain_step      INTEGER,
            goal_type       TEXT CHECK(goal_type IS NULL OR goal_type IN ('project_analysis','code_generation','debug_diagnose','file_operation','research_query','scripting_exec','general','session_refine','memory_snapshot','workflow_memory_snapshot','workflow_turn')),
            tags            TEXT DEFAULT '',
            intent          TEXT NOT NULL,
            summary         TEXT DEFAULT '',
            user_message    TEXT DEFAULT '',
            assistant_message TEXT DEFAULT '',
            tools_used      TEXT DEFAULT '',
            success         INTEGER NOT NULL DEFAULT 0,
            output          TEXT,
            artifacts       TEXT DEFAULT '',
            is_marked       INTEGER NOT NULL DEFAULT 0,
            execution_steps TEXT DEFAULT '[]',
            parent_id       TEXT,
            children_ids    TEXT DEFAULT '',
            pattern         TEXT,
            custom_agent_id TEXT
        );
    ")?;

    // ── 幂等列迁移：custom_agent_id（Custom 记忆隔离，V1 新增）──
    // CREATE TABLE IF NOT EXISTS 不会给已存在的旧表加列，需显式 ALTER。
    ensure_column(conn, "memory_entries", "custom_agent_id", "TEXT")?;

    // ── B-tree 索引 ──
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_memory_session ON memory_entries(session_id);
        CREATE INDEX IF NOT EXISTS idx_memory_kind ON memory_entries(kind);
        CREATE INDEX IF NOT EXISTS idx_memory_goal_type ON memory_entries(goal_type);
        CREATE INDEX IF NOT EXISTS idx_memory_created_at ON memory_entries(created_at);
        CREATE INDEX IF NOT EXISTS idx_memory_custom_agent ON memory_entries(custom_agent_id);
    ",
    )?;

    // FTS5 v4 全文索引（新增 kind 列）— content-less 模式，全文检索由 Rust 层管理
    // 中文内容通过 jieba 分词后再插入，解决 unicode61 单字 token 无法匹配短语的问题
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts_v4 USING fts5(
            id UNINDEXED,
            kind,
            intent,
            summary,
            tags,
            pattern,
            output,
            tokenize='unicode61'
        );
    ",
    )?;

    // sessions 表：记录会话元数据，new_chat_session 时 upsert
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            id              TEXT PRIMARY KEY,
            parent_id       TEXT,
            depth           INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL,
            updated_at      TEXT NOT NULL,
            message_count   INTEGER NOT NULL DEFAULT 0,
            token_count     INTEGER NOT NULL DEFAULT 0,
            summary         TEXT DEFAULT ''
        );
    ",
    )?;

    // embedding 向量表：存储记忆条目的 embedding
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memory_embeddings (
            id          TEXT PRIMARY KEY,
            embedding   BLOB NOT NULL
        );
    ",
    )?;

    Ok(())
}

/// 获取数据库文件大小（字节）
pub fn db_size() -> u64 {
    let path = db_path();
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
