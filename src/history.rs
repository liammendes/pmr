use anyhow::{anyhow, bail, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub name: String,
    pub namespace: String,
    pub env: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(name: &str) -> CommandSpec {
        CommandSpec {
            id: uuid::Uuid::new_v4().to_string(),
            program: "echo".into(),
            args: vec![name.into()],
            cwd: "/tmp".into(),
            name: name.into(),
            namespace: "test".into(),
            env: BTreeMap::new(),
        }
    }
    #[test]
    fn durable_queue_idempotency_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite3");
        let store = Store::open(&path).unwrap();
        let first = spec("first");
        let id = first.id.clone();
        let a = store.submit(first.clone(), "run").unwrap();
        assert_eq!(store.submit(first, "run").unwrap().seq, a.seq);
        store.submit(spec("second"), "run").unwrap();
        assert_eq!(store.claim().unwrap().unwrap().1.id, id);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert_eq!(store.get(&id).unwrap().status, "interrupted");
        assert_eq!(store.claim().unwrap().unwrap().1.name, "second");
        assert!(store.claim().unwrap().is_none());
    }
    #[test]
    fn indexed_pagination_search_and_output_tail() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("h.db")).unwrap();
        let mut ids = Vec::new();
        for name in ["alpha", "beta", "gamma"] {
            ids.push(store.submit(spec(name), "run").unwrap());
        }
        let page = store
            .query(Query {
                limit: 2,
                ..Query::default()
            })
            .unwrap();
        assert_eq!(page["records"][0]["name"], "gamma");
        let next = store
            .query(Query {
                cursor: page["next_cursor"].as_i64(),
                ..Query::default()
            })
            .unwrap();
        assert_eq!(next["records"][0]["name"], "alpha");
        let found = store
            .query(Query {
                search: Some("beta".into()),
                ..Query::default()
            })
            .unwrap();
        assert_eq!(found["records"].as_array().unwrap().len(), 1);
        assert_eq!(found["records"][0]["name"], "beta");
        let filtered = store
            .query(Query {
                name: Some("alpha".into()),
                ascending: true,
                ..Query::default()
            })
            .unwrap();
        assert_eq!(filtered["records"][0]["seq"], ids[0].seq);
        store
            .append(ids[0].seq, "out", &vec![b'x'; 4096], 4096)
            .unwrap();
        store
            .append(ids[0].seq, "out", b"one\ntwo\nthree\n", 4096)
            .unwrap();
        let output = store.output(&ids[0].id, 2).unwrap();
        assert_eq!(output["text"], "two\nthree");
        assert_eq!(output["truncated"], true);
        assert_eq!(store.get(&ids[0].id).unwrap().dropped_bytes, 4096);
    }
    #[test]
    fn retention_is_opt_in_and_only_removes_finished_commands() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("h.db")).unwrap();
        let a = store.submit(spec("old"), "run").unwrap();
        let b = store.submit(spec("new"), "run").unwrap();
        let queued = store.submit(spec("queued"), "run").unwrap();
        store.finish(a.seq, "succeeded", Some(0)).unwrap();
        store.finish(b.seq, "succeeded", Some(0)).unwrap();
        store.append(a.seq, "out", b"output", 4096).unwrap();
        store
            .db
            .lock()
            .unwrap()
            .execute("UPDATE runs SET finished_at=1 WHERE seq=?", [a.seq])
            .unwrap();
        assert_eq!(store.prune(100).unwrap()["records_removed"], 0);
        store
            .configure(SettingsPatch {
                output_days: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(store.prune(100).unwrap()["outputs_removed"], 1);
        assert!(store.get(&a.id).is_ok());
        store
            .configure(SettingsPatch {
                max_records: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(store.prune(1).unwrap()["records_removed"], 1);
        assert!(store.get(&a.id).is_err());
        assert!(store.get(&b.id).is_ok());
        assert!(store.get(&queued.id).is_ok());
        assert_eq!(
            store
                .query(Query {
                    search: Some("old".into()),
                    ..Default::default()
                })
                .unwrap()["records"],
            serde_json::json!([])
        );
    }
    #[test]
    fn queue_limits_and_cancel_release_space() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("h.db")).unwrap();
        store
            .configure(SettingsPatch {
                max_queued: Some(1),
                ..Default::default()
            })
            .unwrap();
        let first = spec("one");
        store.submit(first.clone(), "run").unwrap();
        assert!(store.submit(spec("two"), "run").is_err());
        assert!(store.cancel_queued(&first.id).unwrap());
        assert!(!store.cancel_queued(&first.id).unwrap());
        store.submit(spec("two"), "run").unwrap();
        assert!(store
            .configure(SettingsPatch {
                max_running: Some(0),
                ..Default::default()
            })
            .is_err());
    }
    #[test]
    #[ignore = "explicit million-record disk/index benchmark"]
    fn million_record_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.db");
        let store = Store::open(&path).unwrap();
        let start = std::time::Instant::now();
        {
            let mut db = store.db.lock().unwrap();
            for batch in 0..100 {
                let tx = db.transaction().unwrap();
                {
                    let mut insert=tx.prepare("INSERT INTO runs(id,name,namespace,command,cwd,status,submitted_at,finished_at,source) VALUES(?,?,'bench','echo benchmark','/tmp','succeeded',1,1,'run')").unwrap();
                    for n in batch * 10_000..(batch + 1) * 10_000 {
                        insert
                            .execute(params![format!("bench-{n}"), format!("command-{n}")])
                            .unwrap();
                    }
                }
                tx.commit().unwrap();
            }
        }
        let seed = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert_eq!(
                store.query(Query::default()).unwrap()["records"]
                    .as_array()
                    .unwrap()
                    .len(),
                25
            );
            assert_eq!(
                store
                    .query(Query {
                        name: Some("command-500000".into()),
                        ..Default::default()
                    })
                    .unwrap()["records"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
        let elapsed = start.elapsed();
        let db = store.db.lock().unwrap();
        let plan:String=db.query_row("EXPLAIN QUERY PLAN SELECT seq FROM runs WHERE name='command-500000' AND seq<900000 ORDER BY seq DESC LIMIT 25",[],|r|r.get(3)).unwrap();
        assert!(plan.contains("runs_name_seq"), "{plan}");
        println!("1,000,000 records: seed={seed:?}; 200 bounded queries={elapsed:?}; database_bytes={}; plan={plan}",std::fs::metadata(path).unwrap().len());
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub max_running: usize,
    pub max_queued: usize,
    pub history_days: u32,
    pub output_days: u32,
    pub max_records: u64,
    pub output_bytes: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            max_running: 64,
            max_queued: 10_000,
            history_days: 0,
            output_days: 0,
            max_records: 0,
            output_bytes: 1_048_576,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    pub max_running: Option<usize>,
    pub max_queued: Option<usize>,
    pub history_days: Option<u32>,
    pub output_days: Option<u32>,
    pub max_records: Option<u64>,
    pub output_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub limit: usize,
    pub cursor: Option<i64>,
    pub ascending: bool,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            limit: 25,
            cursor: None,
            ascending: false,
            name: None,
            namespace: None,
            status: None,
            search: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Submit(CommandSpec),
    List(Query),
    Get {
        id: String,
    },
    Output {
        id: String,
        lines: usize,
        #[serde(default)]
        stream: Option<String>,
    },
    Cancel {
        id: String,
    },
    Configure(SettingsPatch),
    Prune {
        batch: usize,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub seq: i64,
    pub id: String,
    pub name: String,
    pub namespace: String,
    pub command: String,
    pub cwd: String,
    pub status: String,
    pub submitted_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
    pub source: String,
    pub output_bytes: u64,
    pub dropped_bytes: u64,
}
fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        seq: row.get(0)?,
        id: row.get(1)?,
        name: row.get(2)?,
        namespace: row.get(3)?,
        command: row.get(4)?,
        cwd: row.get(5)?,
        status: row.get(6)?,
        submitted_at: row.get::<_, i64>(7)? as u64,
        started_at: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        finished_at: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        exit_code: row.get(10)?,
        pid: row.get(11)?,
        source: row.get(12)?,
        output_bytes: row.get::<_, i64>(13)? as u64,
        dropped_bytes: row.get::<_, i64>(14)? as u64,
    })
}
const COLUMNS: &str = "r.seq,r.id,r.name,r.namespace,r.command,r.cwd,r.status,r.submitted_at,r.started_at,r.finished_at,r.exit_code,r.pid,r.source,r.output_bytes,r.dropped_bytes";

pub struct Store {
    db: Mutex<Connection>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?;
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            bail!("history database requires a newer PMR version");
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            PRAGMA foreign_keys=ON; PRAGMA cache_size=-8192; PRAGMA wal_autocheckpoint=1000;
            CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runs (
                seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL, namespace TEXT NOT NULL, command TEXT NOT NULL,
                cwd TEXT NOT NULL, status TEXT NOT NULL, submitted_at INTEGER NOT NULL,
                started_at INTEGER, finished_at INTEGER, exit_code INTEGER, pid INTEGER,
                source TEXT NOT NULL, output_bytes INTEGER NOT NULL DEFAULT 0,
                dropped_bytes INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS runs_source_seq ON runs(source,seq);
            CREATE INDEX IF NOT EXISTS runs_name_source_seq ON runs(name,source,seq);
            CREATE INDEX IF NOT EXISTS runs_namespace_source_seq ON runs(namespace,source,seq);
            CREATE INDEX IF NOT EXISTS runs_status_seq ON runs(status,seq);
            CREATE INDEX IF NOT EXISTS runs_name_seq ON runs(name,seq);
            CREATE INDEX IF NOT EXISTS runs_namespace_seq ON runs(namespace,seq);
            CREATE INDEX IF NOT EXISTS runs_finished ON runs(finished_at) WHERE finished_at IS NOT NULL;
            CREATE INDEX IF NOT EXISTS runs_completed_seq ON runs(seq) WHERE finished_at IS NOT NULL;
            CREATE TABLE IF NOT EXISTS pending (
                seq INTEGER PRIMARY KEY REFERENCES runs(seq) ON DELETE CASCADE,
                spec TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS output (
                chunk INTEGER PRIMARY KEY AUTOINCREMENT,
                run INTEGER NOT NULL REFERENCES runs(seq) ON DELETE CASCADE,
                stream TEXT NOT NULL, data BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS output_run_chunk ON output(run,chunk);
            CREATE VIRTUAL TABLE IF NOT EXISTS run_search USING fts5(name,namespace,command,content='runs',content_rowid='seq');
            CREATE TRIGGER IF NOT EXISTS runs_insert AFTER INSERT ON runs BEGIN
              INSERT INTO run_search(rowid,name,namespace,command) VALUES(new.seq,new.name,new.namespace,new.command); END;
            CREATE TRIGGER IF NOT EXISTS runs_delete AFTER DELETE ON runs BEGIN
              INSERT INTO run_search(run_search,rowid,name,namespace,command) VALUES('delete',old.seq,old.name,old.namespace,old.command); END;
            PRAGMA user_version=1;")?;
        db.execute(
            "INSERT OR IGNORE INTO settings VALUES(1,?)",
            [serde_json::to_string(&Settings::default())?],
        )?;
        Ok(Arc::new(Self { db: Mutex::new(db) }))
    }
    pub async fn call<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || f(&store)).await?
    }
    pub fn settings(&self) -> Result<Settings> {
        let db = self.db.lock().unwrap();
        Ok(serde_json::from_str(&db.query_row(
            "SELECT value FROM settings WHERE id=1",
            [],
            |r| r.get::<_, String>(0),
        )?)?)
    }
    pub fn configure(&self, patch: SettingsPatch) -> Result<Settings> {
        if patch.max_running.is_none()
            && patch.max_queued.is_none()
            && patch.history_days.is_none()
            && patch.output_days.is_none()
            && patch.max_records.is_none()
            && patch.output_bytes.is_none()
        {
            return self.settings();
        }
        // One lock protects read-modify-write against concurrent updates.
        let db = self.db.lock().unwrap();
        let mut s: Settings = serde_json::from_str(&db.query_row(
            "SELECT value FROM settings WHERE id=1",
            [],
            |r| r.get::<_, String>(0),
        )?)?;
        if let Some(v) = patch.max_running {
            s.max_running = v;
        }
        if let Some(v) = patch.max_queued {
            s.max_queued = v;
        }
        if let Some(v) = patch.history_days {
            s.history_days = v;
        }
        if let Some(v) = patch.output_days {
            s.output_days = v;
        }
        if let Some(v) = patch.max_records {
            s.max_records = v;
        }
        if let Some(v) = patch.output_bytes {
            s.output_bytes = v;
        }
        if !(1..=100_000).contains(&s.max_running) || !(1..=10_000_000).contains(&s.max_queued) {
            bail!("running must be 1..100000 and queued must be 1..10000000");
        }
        if !(4096..=1_073_741_824).contains(&s.output_bytes) || s.max_records > i64::MAX as u64 {
            bail!("output bytes must be 4096..1073741824; max records must fit a signed 64-bit integer");
        }
        db.execute(
            "UPDATE settings SET value=? WHERE id=1",
            [serde_json::to_string(&s)?],
        )?;
        Ok(s)
    }
    pub fn recover(&self) -> Result<usize> {
        // Never replay a command which might already have performed a side effect.
        Ok(self.db.lock().unwrap().execute(
            "UPDATE runs SET status='interrupted',finished_at=?,pid=NULL WHERE status='running'",
            [crate::god::now_ms() as i64],
        )?)
    }
    pub fn submit(&self, spec: CommandSpec, source: &str) -> Result<Record> {
        uuid::Uuid::parse_str(&spec.id)?;
        if spec.program.is_empty() || spec.name.len() > 1024 || spec.namespace.len() > 1024 {
            bail!("a program and bounded name/namespace are required");
        }
        let serialized = serde_json::to_string(&spec)?;
        if serialized.len() > 1_048_576 {
            bail!("command specification exceeds 1 MiB");
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        if let Some(existing) = tx
            .query_row(
                &format!("SELECT {COLUMNS} FROM runs r WHERE id=?"),
                [&spec.id],
                record,
            )
            .optional()?
        {
            if existing.command
                != serde_json::to_string(&(spec.program.clone(), spec.args.clone()))?
                || existing.name != spec.name
                || existing.namespace != spec.namespace
                || existing.cwd != spec.cwd
            {
                bail!("command UUID already belongs to a different command");
            }
            return Ok(existing);
        }
        if source == "run" {
            let queued: i64 = tx.query_row("SELECT count(*) FROM pending", [], |r| r.get(0))?;
            let settings: Settings = serde_json::from_str(&tx.query_row(
                "SELECT value FROM settings WHERE id=1",
                [],
                |r| r.get::<_, String>(0),
            )?)?;
            if queued >= settings.max_queued as i64 {
                bail!("command queue is full (limit {})", settings.max_queued);
            }
        }
        let command = serde_json::to_string(&(spec.program.clone(), spec.args.clone()))?;
        let now = crate::god::now_ms() as i64;
        tx.execute("INSERT INTO runs(id,name,namespace,command,cwd,status,submitted_at,started_at,source) VALUES(?,?,?,?,?,?,?,?,?)",
            params![spec.id,spec.name,spec.namespace,command,spec.cwd,
                if source=="run" {"queued"} else {"running"},now,if source=="run" {None} else {Some(now)},source])?;
        let seq = tx.last_insert_rowid();
        if source == "run" {
            tx.execute("INSERT INTO pending VALUES(?,?)", params![seq, serialized])?;
        }
        let result = tx.query_row(
            &format!("SELECT {COLUMNS} FROM runs r WHERE seq=?"),
            [seq],
            record,
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn claim(&self) -> Result<Option<(i64, CommandSpec)>> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let next = tx
            .query_row(
                "SELECT seq,spec FROM pending ORDER BY seq LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((seq, raw)) = next else {
            return Ok(None);
        };
        let spec = serde_json::from_str(&raw)?;
        tx.execute(
            "UPDATE runs SET status='running',started_at=? WHERE seq=?",
            params![crate::god::now_ms() as i64, seq],
        )?;
        tx.execute("DELETE FROM pending WHERE seq=?", [seq])?;
        tx.commit()?;
        Ok(Some((seq, spec)))
    }
    pub fn set_pid(&self, seq: i64, pid: u32) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE runs SET pid=? WHERE seq=?", params![pid, seq])?;
        Ok(())
    }
    pub fn finish(&self, seq: i64, status: &str, code: Option<i32>) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE runs SET status=?,finished_at=?,exit_code=?,pid=NULL WHERE seq=? AND finished_at IS NULL",
            params![status,crate::god::now_ms() as i64,code,seq])?;
        Ok(())
    }
    pub fn append(&self, seq: i64, stream: &str, data: &[u8], cap: u64) -> Result<()> {
        if data.len() > 8192 {
            bail!("output chunks must be at most 8192 bytes");
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO output(run,stream,data) VALUES(?,?,?)",
            params![seq, stream, data],
        )?;
        tx.execute(
            "UPDATE runs SET output_bytes=output_bytes+? WHERE seq=?",
            params![data.len() as i64, seq],
        )?;
        let mut kept: i64 =
            tx.query_row("SELECT output_bytes FROM runs WHERE seq=?", [seq], |r| {
                r.get(0)
            })?;
        while kept > cap as i64 {
            let (chunk, len): (i64, i64) = tx.query_row(
                "SELECT chunk,length(data) FROM output WHERE run=? ORDER BY chunk LIMIT 1",
                [seq],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            tx.execute("DELETE FROM output WHERE chunk=?", [chunk])?;
            kept -= len;
            tx.execute(
                "UPDATE runs SET output_bytes=?,dropped_bytes=dropped_bytes+? WHERE seq=?",
                params![kept, len, seq],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Record> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT {COLUMNS} FROM runs r WHERE id=?"),
                [id],
                record,
            )
            .optional()?
            .ok_or_else(|| anyhow!("history record not found: {id}"))
    }
    pub fn query(&self, q: Query) -> Result<Value> {
        if !(1..=1000).contains(&q.limit) {
            bail!("history limit must be 1..1000");
        }
        let mut clauses = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(cursor) = q.cursor {
            clauses.push(format!("r.seq {} ?", if q.ascending { ">" } else { "<" }));
            values.push(cursor.into());
        }
        for (field, value) in [
            ("name", q.name),
            ("namespace", q.namespace),
            ("status", q.status),
        ] {
            if let Some(v) = value {
                clauses.push(format!("r.{field}=?"));
                values.push(v.into());
            }
        }
        if let Some(search) = q.search {
            clauses.push("r.seq IN (SELECT rowid FROM run_search WHERE run_search MATCH ?)".into());
            values.push(search.into());
        }
        let filter = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT {COLUMNS} FROM runs r{filter} ORDER BY r.seq {} LIMIT ?",
            if q.ascending { "ASC" } else { "DESC" }
        );
        values.push(((q.limit + 1) as i64).into());
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare(&sql)?;
        let mut rows = statement
            .query_map(params_from_iter(values), record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > q.limit;
        rows.truncate(q.limit);
        let cursor = if more {
            rows.last().map(|r| r.seq)
        } else {
            None
        };
        Ok(serde_json::json!({"records":rows,"next_cursor":cursor}))
    }
    pub fn output(&self, id: &str, lines: usize) -> Result<Value> {
        self.output_filtered(id, lines, None)
    }
    pub fn latest_run(&self, target: &str) -> Result<Option<Record>> {
        let db = self.db.lock().unwrap();
        if target == "all" || target.is_empty() {
            return Ok(db
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM runs r WHERE source='run' ORDER BY seq DESC LIMIT 1"
                    ),
                    [],
                    record,
                )
                .optional()?);
        }
        let mut latest: Option<Record> = None;
        for field in ["id", "name", "namespace"] {
            let row=db.query_row(&format!("SELECT {COLUMNS} FROM runs r WHERE source='run' AND {field}=? ORDER BY seq DESC LIMIT 1"),[target],record).optional()?;
            if let Some(row) = row {
                if latest.as_ref().is_none_or(|old| row.seq > old.seq) {
                    latest = Some(row);
                }
            }
        }
        Ok(latest)
    }
    pub fn output_filtered(&self, id: &str, lines: usize, stream: Option<&str>) -> Result<Value> {
        if !(1..=100_000).contains(&lines) {
            bail!("output lines must be 1..100000");
        }
        let row = self.get(id)?;
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare(
            "SELECT data FROM output WHERE run=? AND (? IS NULL OR stream=?) ORDER BY chunk DESC",
        )?;
        let mut chunks = Vec::new();
        let mut bytes = 0usize;
        let mut newline_count = 0;
        let mut rows = statement.query(params![row.seq, stream, stream])?;
        while let Some(r) = rows.next()? {
            let data: Vec<u8> = r.get(0)?;
            newline_count += data.iter().filter(|&&b| b == b'\n').count();
            bytes += data.len();
            chunks.push(data);
            if bytes >= 1_048_576 || newline_count > lines {
                break;
            }
        }
        chunks.reverse();
        let data: Vec<u8> = chunks.into_iter().flatten().collect();
        let text = String::from_utf8_lossy(&data);
        let mut tail = text.lines().rev().take(lines).collect::<Vec<_>>();
        tail.reverse();
        Ok(
            serde_json::json!({"id":id,"status":row.status,"exit_code":row.exit_code,
            "text":tail.join("\n"),"truncated":row.dropped_bytes>0 || row.output_bytes>bytes as u64}),
        )
    }
    pub fn cancel_queued(&self, id: &str) -> Result<bool> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let count = tx.execute(
            "UPDATE runs SET status='cancelled',finished_at=? WHERE id=? AND status='queued'",
            params![crate::god::now_ms() as i64, id],
        )?;
        if count > 0 {
            tx.execute(
                "DELETE FROM pending WHERE seq=(SELECT seq FROM runs WHERE id=?)",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(count > 0)
    }
    pub fn prune(&self, batch: usize) -> Result<Value> {
        if !(1..=1000).contains(&batch) {
            bail!("prune batch must be 1..1000");
        }
        let settings = self.settings()?;
        let now = crate::god::now_ms();
        let cutoff = |days: u32| now.saturating_sub(u64::from(days) * 86_400_000);
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let mut expired_output = 0;
        if settings.output_days > 0 {
            let ids=tx.prepare("SELECT seq FROM runs WHERE finished_at<? AND output_bytes>0 ORDER BY finished_at LIMIT ?")?
                .query_map(params![cutoff(settings.output_days) as i64,batch as i64],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for seq in ids {
                tx.execute("DELETE FROM output WHERE run=?", [seq])?;
                tx.execute("UPDATE runs SET dropped_bytes=dropped_bytes+output_bytes,output_bytes=0 WHERE seq=?",[seq])?;
                expired_output += 1;
            }
        }
        let mut removed = 0;
        if settings.history_days > 0 {
            removed+=tx.execute("DELETE FROM runs WHERE seq IN (SELECT seq FROM runs WHERE finished_at<? ORDER BY finished_at LIMIT ?)",
                params![cutoff(settings.history_days) as i64,batch as i64])?;
        }
        if settings.max_records > 0 && removed < batch {
            let threshold:Option<i64>=tx.query_row("SELECT seq FROM runs WHERE finished_at IS NOT NULL ORDER BY seq DESC LIMIT 1 OFFSET ?",
                [settings.max_records as i64],|r|r.get(0)).optional()?;
            if let Some(threshold) = threshold {
                removed+=tx.execute("DELETE FROM runs WHERE seq IN (SELECT seq FROM runs WHERE finished_at IS NOT NULL AND seq<=? ORDER BY seq LIMIT ?)",
                    params![threshold,(batch-removed) as i64])?;
            }
        }
        tx.commit()?;
        Ok(serde_json::json!({"records_removed":removed,"outputs_removed":expired_output}))
    }
}
