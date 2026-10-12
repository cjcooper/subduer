//! The `.trace` file: one SQLite database per trace, so it can be kept,
//! copied and opened later without the cluster. Usage is stored as
//! one-minute buckets (min/avg/max) per container; requests and limits, and
//! restart and OOM counts, as change points. While recording the file is in
//! WAL mode, so a crash keeps everything up to the last write; closing it
//! cleanly folds the WAL back in and leaves a single file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::types::{TraceEnd, TraceGap, TraceScope, TraceSummary};
use crate::{Error, Result};

/// Bumped when the schema changes in a way older readers can't handle.
pub const FORMAT: i64 = 1;
/// "SUBT", so other SQLite files are recognised as not being traces.
const APPLICATION_ID: i32 = 0x5355_4254;
pub const BUCKET_SECS: i64 = 60;

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE pods (
    id INTEGER PRIMARY KEY,
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    -- The top owner: Deployment, StatefulSet, DaemonSet, CronJob, Job,
    -- ReplicaSet, or Pod for a bare pod. NULL when unknown.
    workload_kind TEXT,
    workload_name TEXT,
    -- pod-template-hash for Deployments, so rollouts can be told apart.
    revision TEXT,
    UNIQUE (namespace, name)
);
CREATE TABLE containers (
    id INTEGER PRIMARY KEY,
    pod_id INTEGER NOT NULL REFERENCES pods(id),
    name TEXT NOT NULL,
    UNIQUE (pod_id, name)
);
-- t: bucket start, unix seconds. CPU in microcores, memory in bytes.
-- throttled: the worst share of CPU periods throttled in the bucket (0..1);
-- NULL without a CPU limit. source: 0 Prometheus, 1 sampled by the app.
CREATE TABLE usage (
    container_id INTEGER NOT NULL,
    t INTEGER NOT NULL,
    cpu_min INTEGER, cpu_avg INTEGER, cpu_max INTEGER,
    mem_min INTEGER, mem_avg INTEGER, mem_max INTEGER,
    throttled REAL,
    source INTEGER NOT NULL,
    PRIMARY KEY (container_id, t)
) WITHOUT ROWID;
-- A row each time requests or limits change (and the first time seen).
-- CPU in millicores, memory in bytes; NULL when not set.
CREATE TABLE resources (
    container_id INTEGER NOT NULL,
    t INTEGER NOT NULL,
    cpu_request REAL, cpu_limit REAL, mem_request INTEGER, mem_limit INTEGER,
    PRIMARY KEY (container_id, t)
) WITHOUT ROWID;
-- kind: 'restart' (reason: the last termination reason, e.g. OOMKilled) or
-- 'oom' (the kernel OOM-killed a process in the container).
CREATE TABLE events (
    container_id INTEGER NOT NULL,
    t INTEGER NOT NULL,
    kind TEXT NOT NULL,
    reason TEXT,
    count INTEGER NOT NULL,
    PRIMARY KEY (container_id, t, kind)
) WITHOUT ROWID;
CREATE TABLE gaps (start INTEGER NOT NULL, end INTEGER NOT NULL, reason TEXT NOT NULL);
CREATE TABLE pulls (
    at INTEGER NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL,
    ok INTEGER NOT NULL, series INTEGER NOT NULL, message TEXT,
    -- Response bytes, as received. Files from before it was counted lack
    -- the column.
    bytes INTEGER
);
";

/// One container in the cluster.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerKey {
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

impl ContainerKey {
    pub fn new(namespace: &str, pod: &str, container: &str) -> Self {
        Self { namespace: namespace.into(), pod: pod.into(), container: container.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub kind: String,
    pub name: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stat {
    pub min: f64,
    pub avg: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Prometheus = 0,
    Local = 1,
}

/// One container's minute.
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    pub t: i64,
    /// Millicores.
    pub cpu: Option<Stat>,
    /// Bytes.
    pub memory: Option<Stat>,
    pub throttled: Option<f64>,
}

/// CPU in millicores, memory in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Resources {
    pub cpu_request: Option<f64>,
    pub cpu_limit: Option<f64>,
    pub mem_request: Option<f64>,
    pub mem_limit: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Restart,
    Oom,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::Oom => "oom",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub t: i64,
    pub kind: EventKind,
    pub reason: Option<String>,
    pub count: i64,
}

/// Everything one pull or sample writes, in one transaction.
#[derive(Debug, Default)]
pub struct Batch {
    pub source: Option<Source>,
    pub usage: Vec<(ContainerKey, Bucket)>,
    pub resources: Vec<(ContainerKey, i64, Resources)>,
    pub events: Vec<(ContainerKey, Event)>,
    /// (namespace, pod) → owner.
    pub owners: Vec<((String, String), Owner)>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.usage.is_empty() && self.resources.is_empty() && self.events.is_empty() && self.owners.is_empty()
    }
}

pub struct TraceFile {
    conn: Connection,
    path: PathBuf,
    pod_ids: HashMap<(String, String), i64>,
    container_ids: HashMap<ContainerKey, i64>,
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Message(format!("trace file: {e}"))
    }
}

impl TraceFile {
    /// A new, empty trace at `path`. An existing file is only replaced with
    /// `overwrite` (the Save dialog has already asked).
    pub fn create(path: &Path, overwrite: bool, context: &str, scope: &TraceScope, started: f64) -> Result<Self> {
        if path.exists() {
            if !overwrite {
                return Err(Error::Message(format!("{} already exists", path.display())));
            }
            std::fs::remove_file(path).map_err(|e| Error::Message(format!("can't replace {}: {e}", path.display())))?;
            for suffix in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(sidecar(path, suffix));
            }
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "application_id", APPLICATION_ID)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let file = Self { conn, path: path.to_owned(), pod_ids: HashMap::new(), container_ids: HashMap::new() };
        let scope = serde_json::to_string(scope).map_err(|e| Error::Message(e.to_string()))?;
        for (key, value) in [
            ("format", FORMAT.to_string()),
            ("context", context.to_owned()),
            ("scope", scope),
            ("started", started.to_string()),
            ("bucket_secs", BUCKET_SECS.to_string()),
            ("app_version", env!("CARGO_PKG_VERSION").to_owned()),
        ] {
            file.set_meta(key, &value)?;
        }
        Ok(file)
    }

    /// Opens a trace read-only, refusing files that aren't traces or are
    /// from a newer version of the app.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| Error::Message(format!("can't open {}: {e}", path.display())))?;
        let not_a_trace = || Error::Message(format!("{} isn't a trace file", path.display()));
        let id: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0)).map_err(|_| not_a_trace())?;
        if id != APPLICATION_ID {
            return Err(not_a_trace());
        }
        let file = Self { conn, path: path.to_owned(), pod_ids: HashMap::new(), container_ids: HashMap::new() };
        let format: i64 = file.meta("format")?.and_then(|f| f.parse().ok()).ok_or_else(not_a_trace)?;
        if format > FORMAT {
            return Err(Error::Message(format!("{} needs a newer version of the app", path.display())));
        }
        Ok(file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn add_gap(&self, start: i64, end: i64, reason: &str) -> Result<()> {
        if end > start {
            self.conn.execute("INSERT INTO gaps (start, end, reason) VALUES (?1, ?2, ?3)", params![start, end, reason])?;
        }
        Ok(())
    }

    pub fn log_pull(&self, at: i64, start: i64, end: i64, series: usize, bytes: usize, error: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pulls (at, start, end, ok, series, message, bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![at, start, end, error.is_none(), series as i64, error, bytes as i64],
        )?;
        Ok(())
    }

    /// Writes a batch in one transaction. Prometheus usage replaces whatever
    /// a minute held; the app's own samples never replace Prometheus data.
    pub fn write(&mut self, batch: &Batch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        let mut ids = Ids { pod_ids: &mut self.pod_ids, container_ids: &mut self.container_ids };
        let source = batch.source.unwrap_or(Source::Prometheus);
        // kube-state-metrics knows a Job's CronJob; the app's guess from the
        // pod alone doesn't, so it only fills in owners nothing else gave.
        let owner_sql = match source {
            Source::Prometheus => "UPDATE pods SET workload_kind = ?2, workload_name = ?3, revision = ?4 WHERE id = ?1",
            Source::Local => {
                "UPDATE pods SET workload_kind = ?2, workload_name = ?3, revision = ?4
                 WHERE id = ?1 AND workload_kind IS NULL"
            }
        };
        for ((namespace, pod), owner) in &batch.owners {
            let id = ids.pod(&tx, namespace, pod)?;
            tx.execute(owner_sql, params![id, owner.kind, owner.name, owner.revision])?;
        }
        {
            let sql = match source {
                Source::Prometheus => {
                    "INSERT OR REPLACE INTO usage VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)"
                }
                Source::Local => {
                    "INSERT INTO usage VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
                     ON CONFLICT (container_id, t) DO UPDATE SET
                       cpu_min = excluded.cpu_min, cpu_avg = excluded.cpu_avg, cpu_max = excluded.cpu_max,
                       mem_min = excluded.mem_min, mem_avg = excluded.mem_avg, mem_max = excluded.mem_max,
                       throttled = excluded.throttled
                     WHERE usage.source = 1"
                }
            };
            let mut insert = tx.prepare_cached(sql)?;
            for (key, b) in &batch.usage {
                let id = ids.container(&tx, key)?;
                let cpu = |f: fn(&Stat) -> f64| b.cpu.as_ref().map(|s| (f(s) * 1000.0).round() as i64);
                let mem = |f: fn(&Stat) -> f64| b.memory.as_ref().map(|s| f(s).round() as i64);
                insert.execute(params![
                    id,
                    b.t,
                    cpu(|s| s.min),
                    cpu(|s| s.avg),
                    cpu(|s| s.max),
                    mem(|s| s.min),
                    mem(|s| s.avg),
                    mem(|s| s.max),
                    b.throttled,
                ])?;
            }
            let mut insert = tx.prepare_cached("INSERT OR REPLACE INTO resources VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for (key, t, r) in &batch.resources {
                let id = ids.container(&tx, key)?;
                let bytes = |v: Option<f64>| v.map(|v| v.round() as i64);
                insert.execute(params![id, t, r.cpu_request, r.cpu_limit, bytes(r.mem_request), bytes(r.mem_limit)])?;
            }
            let mut insert = tx.prepare_cached("INSERT OR IGNORE INTO events VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for (key, e) in &batch.events {
                let id = ids.container(&tx, key)?;
                insert.execute(params![id, e.t, e.kind.as_str(), e.reason, e.count])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Records the end and turns WAL off, leaving one self-contained file.
    pub fn finish(&self, ended: f64, reason: TraceEnd) -> Result<()> {
        self.set_meta("ended", &ended.to_string())?;
        let reason = serde_json::to_string(&reason).map_err(|e| Error::Message(e.to_string()))?;
        self.set_meta("end_reason", &reason)?;
        self.conn.pragma_update(None, "journal_mode", "DELETE")?;
        Ok(())
    }

    /// The file's header and counts. `recording` says whether this app is
    /// still writing it; an unended file that isn't is "unfinished".
    pub fn summary(&self, recording: bool) -> Result<TraceSummary> {
        let missing = |key: &str| Error::Message(format!("{} has no {key}", self.path.display()));
        let number = |key: &str| -> Result<Option<f64>> { Ok(self.meta(key)?.and_then(|v| v.parse().ok())) };
        let scope: TraceScope = serde_json::from_str(&self.meta("scope")?.ok_or_else(|| missing("scope"))?)
            .map_err(|e| Error::Message(format!("trace scope: {e}")))?;
        let end_reason = self.meta("end_reason")?.and_then(|r| serde_json::from_str(&r).ok());
        let ended = number("ended")?;
        let count = |sql: &str| -> Result<u64> { Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64) };
        let (data_start, data_end): (Option<i64>, Option<i64>) =
            self.conn.query_row("SELECT MIN(t), MAX(t) FROM usage", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut gaps = self.conn.prepare("SELECT start, end, reason FROM gaps ORDER BY start")?;
        let gaps = gaps
            .query_map([], |r| {
                Ok(TraceGap { start: r.get::<_, i64>(0)? as f64, end: r.get::<_, i64>(1)? as f64, reason: r.get(2)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let file_bytes = ["", "-wal"]
            .iter()
            .filter_map(|s| std::fs::metadata(sidecar(&self.path, s)).ok())
            .map(|m| m.len())
            .sum();
        let counts_bytes = count("SELECT COUNT(*) FROM pragma_table_info('pulls') WHERE name = 'bytes'")? > 0;
        let pulled_bytes = if counts_bytes { count("SELECT COALESCE(SUM(bytes), 0) FROM pulls")? } else { 0 };
        Ok(TraceSummary {
            path: self.path.display().to_string(),
            context: self.meta("context")?.ok_or_else(|| missing("context"))?,
            scope,
            started: number("started")?.ok_or_else(|| missing("start time"))?,
            ended,
            end_reason,
            unfinished: ended.is_none() && !recording,
            data_start: data_start.map(|t| t as f64),
            data_end: data_end.map(|t| (t + BUCKET_SECS) as f64),
            prometheus: self.meta("prometheus")?,
            retention_secs: number("retention_secs")?,
            pods: count("SELECT COUNT(*) FROM pods")?,
            containers: count("SELECT COUNT(*) FROM containers")?,
            prometheus_samples: count("SELECT COUNT(*) FROM usage WHERE source = 0")?,
            local_samples: count("SELECT COUNT(*) FROM usage WHERE source = 1")?,
            gaps,
            file_bytes,
            pull_every: number("pull_every_secs")?,
            pulled_bytes,
        })
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Notes are kept in their own table, made when the first note is written,
/// so files from before notes existed still open. Key "" is the trace's own
/// note; `namespace/kind/name` a workload's.
const NOTES: &str = "CREATE TABLE IF NOT EXISTS notes (key TEXT PRIMARY KEY, text TEXT NOT NULL, updated REAL NOT NULL) WITHOUT ROWID";

impl TraceFile {
    /// Every note, by key. Empty if none were ever written.
    pub fn notes(&self) -> Result<std::collections::HashMap<String, String>> {
        let exists: bool = self
            .conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'notes'", [], |r| r.get::<_, i64>(0))?
            > 0;
        if !exists {
            return Ok(Default::default());
        }
        let mut stmt = self.conn.prepare("SELECT key, text FROM notes")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// The key of a workload's note.
pub fn note_key(namespace: &str, kind: &str, name: &str) -> String {
    format!("{namespace}/{kind}/{name}")
}

/// Writes (or, when blank, removes) a note in a trace file, which may be
/// being recorded by this app at the same time.
pub fn set_note(path: &Path, key: &str, text: &str, now: f64) -> Result<()> {
    TraceFile::open(path)?;
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| Error::Message(format!("can't write to {}: {e}", path.display())))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(NOTES)?;
    let text = text.trim();
    if text.is_empty() {
        conn.execute("DELETE FROM notes WHERE key = ?1", [key])?;
    } else {
        conn.execute("INSERT OR REPLACE INTO notes (key, text, updated) VALUES (?1, ?2, ?3)", params![key, text, now])?;
    }
    Ok(())
}

/// `path` with `suffix` added to the file name (SQLite's `-wal`, `-shm`).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Row ids for pods and containers, created on first use and cached.
struct Ids<'a> {
    pod_ids: &'a mut HashMap<(String, String), i64>,
    container_ids: &'a mut HashMap<ContainerKey, i64>,
}

impl Ids<'_> {
    fn pod(&mut self, conn: &Connection, namespace: &str, pod: &str) -> Result<i64> {
        let key = (namespace.to_owned(), pod.to_owned());
        if let Some(id) = self.pod_ids.get(&key) {
            return Ok(*id);
        }
        conn.execute("INSERT OR IGNORE INTO pods (namespace, name) VALUES (?1, ?2)", params![namespace, pod])?;
        let id = conn.query_row("SELECT id FROM pods WHERE namespace = ?1 AND name = ?2", params![namespace, pod], |r| r.get(0))?;
        self.pod_ids.insert(key, id);
        Ok(id)
    }

    fn container(&mut self, conn: &Connection, key: &ContainerKey) -> Result<i64> {
        if let Some(id) = self.container_ids.get(key) {
            return Ok(*id);
        }
        let pod = self.pod(conn, &key.namespace, &key.pod)?;
        conn.execute("INSERT OR IGNORE INTO containers (pod_id, name) VALUES (?1, ?2)", params![pod, key.container])?;
        let id = conn.query_row("SELECT id FROM containers WHERE pod_id = ?1 AND name = ?2", params![pod, key.container], |r| r.get(0))?;
        self.container_ids.insert(key.clone(), id);
        Ok(id)
    }
}

#[cfg(test)]
#[path = "trace_file_tests.rs"]
mod tests;
