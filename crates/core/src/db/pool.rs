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
/// `journal_size_limit` caps the write-ahead log at 64 MiB.
///
/// An advisory sync ingests a whole ecosystem inside one transaction — for npm
/// that is ~230k advisories and >1M affected-version rows — so the WAL has to
/// grow to hold the entire transaction before it commits. Without a size
/// limit SQLite *resets* the WAL after checkpointing but never shrinks the
/// file, so that peak became permanent: a 259 MiB `repo-radar.db-wal` sitting
/// next to a 497 MiB database, for a database whose real content is a few
/// hundred MiB. `journal_size_limit` makes the post-checkpoint truncation
/// actually give the space back.
const PRAGMAS: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA foreign_keys = ON;
    PRAGMA busy_timeout = 5000;
    PRAGMA journal_size_limit = 67108864;
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
    ///
    /// # Poison recovery
    ///
    /// A panic inside a previous closure poisons this mutex. Propagating that
    /// poison (the old `.expect(...)`) made a single panic **permanently**
    /// fatal: every later write — every scan, every advisory sync, every
    /// settings change — panicked for the rest of the process's life, so the
    /// app looked like it was crashing constantly when one repository had
    /// tripped one bug once.
    ///
    /// Recovering is sound here because the guarded value is a SQLite
    /// connection, not an invariant-bearing data structure: `rusqlite`'s
    /// `Transaction` rolls back in its `Drop`, so a panic during a
    /// transaction unwinds to a clean connection. The only residue possible
    /// is a transaction opened without an RAII guard, which
    /// [`clear_stale_transaction`] closes before the connection is reused.
    pub fn with_write<T>(&self, f: impl FnOnce(&mut Connection) -> CoreResult<T>) -> CoreResult<T> {
        let mut guard = match self.write.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!(
                    "write connection mutex was poisoned by an earlier panic; recovering"
                );
                let mut g = poisoned.into_inner();
                clear_stale_transaction(&mut g);
                g
            }
        };
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

/// Roll back a transaction a panicking closure may have left open.
///
/// `rusqlite::Transaction` rolls back on drop, so this is a belt-and-braces
/// check for a `BEGIN` issued through raw SQL with no RAII guard. Leaving one
/// open would make the *next* write fail with "cannot start a transaction
/// within a transaction" and hold a write lock on the database file.
fn clear_stale_transaction(conn: &mut Connection) {
    if conn.is_autocommit() {
        return;
    }
    tracing::warn!("rolling back a transaction left open by a panicking write");
    if let Err(e) = conn.execute_batch("ROLLBACK") {
        tracing::error!(error = %e, "could not roll back the stale transaction");
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
