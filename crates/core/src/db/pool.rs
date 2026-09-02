//! Connection strategy (DESIGN §5.1): one dedicated write connection behind
//! a `Mutex`, a small read pool for the UI. WAL mode lets readers proceed
//! while a scan writes continuously.

use std::path::Path;
use std::sync::{Arc, Mutex};

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OpenFlags};

use crate::error::{classify_sqlite, CoreError, CoreResult};

/// Read pool size. Four is enough to serve the UI's concurrent queries
/// without contending; writes never come through here.
const READ_POOL_SIZE: u32 = 4;

/// Pragmas applied to **every** connection, read or write (DESIGN §5.1).
/// `synchronous = NORMAL` is a deliberate durability trade: every byte in
/// this database is derived and re-scannable.
const PRAGMAS: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA foreign_keys = ON;
    PRAGMA busy_timeout = 5000;
";

pub type ReadPool = r2d2::Pool<SqliteConnectionManager>;
pub type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Owns both halves of the connection strategy.
pub struct Pools {
    write: Arc<Mutex<Connection>>,
    read: ReadPool,
    /// For an in-memory database, one extra open handle keeps the
    /// shared-cache database alive for the process's lifetime. Unused for
    /// file-backed databases. Wrapped in a `Mutex` only so `Pools` stays
    /// `Sync` — a bare `rusqlite::Connection` is `Send` but not `Sync`.
    _keepalive: Option<Mutex<Connection>>,
}

impl Pools {
    /// Open a file-backed database, creating it if absent.
    pub fn open_file(path: &Path) -> CoreResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let write = Connection::open(path).map_err(|e| classify_sqlite(e, path))?;
        write.execute_batch(PRAGMAS)?;

        let manager = SqliteConnectionManager::file(path).with_init(|c| c.execute_batch(PRAGMAS));
        let read = build_read_pool(manager)?;

        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read,
            _keepalive: None,
        })
    }

    /// Open a private in-memory database with shared cache, so the write
    /// connection and the read pool see the same data. Used by tests
    /// (DESIGN §16.5). The name is randomised so parallel tests do not
    /// collide.
    pub fn open_in_memory() -> CoreResult<Self> {
        let token = format!("file:rr-mem-{:x}?mode=memory&cache=shared", fastrand_u64());
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI;

        // Keepalive first: the shared-cache DB exists only while at least
        // one connection to it is open.
        let keepalive = Connection::open_with_flags(&token, flags)?;
        keepalive.execute_batch(PRAGMAS)?;

        let write = Connection::open_with_flags(&token, flags)?;
        write.execute_batch(PRAGMAS)?;

        let manager = SqliteConnectionManager::file(&token)
            .with_flags(flags)
            .with_init(|c| c.execute_batch(PRAGMAS));
        let read = build_read_pool(manager)?;

        Ok(Self {
            write: Arc::new(Mutex::new(write)),
            read,
            _keepalive: Some(Mutex::new(keepalive)),
        })
    }

    /// Run a closure with exclusive access to the write connection.
    pub fn with_write<T>(&self, f: impl FnOnce(&mut Connection) -> CoreResult<T>) -> CoreResult<T> {
        let mut guard = self.write.lock().expect("write connection mutex poisoned");
        f(&mut guard)
    }

    /// Check out a read connection from the pool.
    pub fn read(&self) -> CoreResult<PooledConn> {
        self.read.get().map_err(CoreError::from)
    }

    /// Clone the `Arc` to the write connection (for the scan writer thread).
    pub fn write_handle(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.write)
    }
}

fn build_read_pool(manager: SqliteConnectionManager) -> CoreResult<ReadPool> {
    r2d2::Pool::builder()
        .max_size(READ_POOL_SIZE)
        .build(manager)
        .map_err(CoreError::from)
}

/// Tiny xorshift PRNG — enough to name a temp in-memory DB uniquely without
/// pulling in a dependency. Seeded from the clock and a per-process atomic
/// counter so parallel test threads get distinct names.
fn fastrand_u64() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut x = seed
        ^ (COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x2545_F491_4F6C_DD1D)
            + 1);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}
