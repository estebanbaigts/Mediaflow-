//! Local SQLite store: grants, sessions, durable operation journal, hash cache
//! and library index. Explicit, numbered, forward-only migrations.
//!
//! Durability: WAL with `synchronous=NORMAL` — the journal is always
//! consistent, and per-file entries become durable at the next checkpoint.
//! Every session state change forces a full checkpoint (fsync). Losing the
//! last few per-file entries on power loss is safe by design: on resume the
//! executor re-validates every non-final entry against the disk and
//! re-verifies (SHA-256) any copy it finds, so nothing is ever declared done
//! without proof.

use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{MfError, Result};
use crate::hash::Sha256Hex;
use crate::scan::FileStamp;
use crate::state::{OpState, SessionState};

const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
CREATE TABLE grants (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('source','destination')),
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(path, kind)
);
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    project TEXT NOT NULL,
    profile_json TEXT NOT NULL,
    source_root TEXT NOT NULL,
    source_label TEXT NOT NULL,
    source_scan_complete INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    active_seconds REAL NOT NULL DEFAULT 0,
    last_error TEXT,
    scan_issues_json TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE session_destinations (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    root TEXT NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (session_id, idx)
);
CREATE TABLE files (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL,
    rel_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    inode INTEGER,
    category TEXT NOT NULL,
    confidence TEXT NOT NULL,
    excluded_pattern TEXT,
    excluded_reason TEXT,
    source_sha256 TEXT,
    PRIMARY KEY (session_id, file_id)
);
CREATE TABLE ops (
    session_id TEXT NOT NULL,
    file_id INTEGER NOT NULL,
    dest_idx INTEGER NOT NULL,
    target_rel TEXT NOT NULL,
    final_rel TEXT,
    action TEXT NOT NULL,
    action_ref TEXT,
    action_level TEXT,
    state TEXT NOT NULL,
    temp_name TEXT,
    sha256 TEXT,
    bytes INTEGER NOT NULL DEFAULT 0,
    final_stamp_json TEXT,
    error_code TEXT,
    error_message TEXT,
    notes_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (session_id, file_id, dest_idx),
    FOREIGN KEY (session_id, file_id) REFERENCES files(session_id, file_id) ON DELETE CASCADE
);
CREATE INDEX ops_state ON ops(session_id, state);
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT,
    at TEXT NOT NULL,
    level TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    rel_path TEXT
);
CREATE INDEX events_session ON events(session_id);
CREATE TABLE hash_cache (
    path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    inode INTEGER,
    sha256 TEXT NOT NULL,
    computed_at TEXT NOT NULL
);
CREATE TABLE library (
    dest_root TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    category TEXT NOT NULL,
    project TEXT NOT NULL,
    source_label TEXT NOT NULL,
    source_rel TEXT NOT NULL,
    session_id TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    PRIMARY KEY (dest_root, rel_path)
);
CREATE INDEX library_sha ON library(sha256);
CREATE TABLE checks (
    id TEXT PRIMARY KEY,
    at TEXT NOT NULL,
    source_root TEXT NOT NULL,
    summary_json TEXT NOT NULL
);
"#,
)];

pub struct Db {
    pub(crate) conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub path: String,
    pub kind: GrantKind,
    pub label: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    Source,
    Destination,
}

impl GrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantKind::Source => "source",
            GrantKind::Destination => "destination",
        }
    }
    fn parse(s: &str) -> GrantKind {
        if s == "source" {
            GrantKind::Source
        } else {
            GrantKind::Destination
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub state: SessionState,
    pub project: String,
    pub profile_json: String,
    pub source_root: String,
    pub source_label: String,
    pub source_scan_complete: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub active_seconds: f64,
    pub last_error: Option<String>,
    pub scan_issues_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestRow {
    pub idx: i64,
    pub root: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRow {
    pub file_id: i64,
    pub rel_path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub inode: Option<u64>,
    pub category: String,
    pub confidence: String,
    pub excluded_pattern: Option<String>,
    pub excluded_reason: Option<String>,
    pub source_sha256: Option<String>,
}

impl FileRow {
    pub fn stamp(&self) -> FileStamp {
        FileStamp {
            size: self.size,
            mtime_ns: self.mtime_ns,
            inode: self.inode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpRow {
    pub file_id: i64,
    pub dest_idx: i64,
    pub target_rel: String,
    pub final_rel: Option<String>,
    pub action: String,
    pub action_ref: Option<String>,
    pub action_level: Option<String>,
    pub state: OpState,
    pub temp_name: Option<String>,
    pub sha256: Option<String>,
    pub bytes: u64,
    pub final_stamp_json: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub notes_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub at: DateTime<Utc>,
    pub level: String,
    pub code: String,
    pub message: String,
    pub rel_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryRow {
    pub dest_root: String,
    pub rel_path: String,
    pub sha256: String,
    pub size: u64,
    pub category: String,
    pub project: String,
    pub source_label: String,
    pub source_rel: String,
    pub session_id: String,
    pub verified_at: DateTime<Utc>,
}

fn ts(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_default()
}

fn ts_opt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.map(ts)
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MfError::io(parent, e))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        )?;
        let current = self.schema_version()?;
        let latest = MIGRATIONS.last().map(|m| m.0).unwrap_or(0);
        if current > latest {
            return Err(MfError::InvalidInput(format!(
                "database schema v{current} is newer than this version of MediaFlow (v{latest}); refusing to modify it"
            )));
        }
        for (v, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![v, crate::util::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- grants

    pub fn add_grant(&self, path: &str, kind: GrantKind, label: &str) -> Result<Grant> {
        if let Some(g) = self
            .list_grants()?
            .into_iter()
            .find(|g| g.path == path && g.kind == kind)
        {
            return Ok(g);
        }
        let g = Grant {
            id: uuid::Uuid::new_v4().to_string(),
            path: path.to_string(),
            kind,
            label: label.to_string(),
            created_at: crate::util::now(),
        };
        self.conn.execute(
            "INSERT INTO grants(id,path,kind,label,created_at) VALUES (?1,?2,?3,?4,?5)",
            params![
                g.id,
                g.path,
                kind.as_str(),
                g.label,
                g.created_at.to_rfc3339()
            ],
        )?;
        Ok(g)
    }

    pub fn list_grants(&self) -> Result<Vec<Grant>> {
        let mut st = self
            .conn
            .prepare("SELECT id,path,kind,label,created_at FROM grants ORDER BY created_at")?;
        let rows = st.query_map([], |r| {
            Ok(Grant {
                id: r.get(0)?,
                path: r.get(1)?,
                kind: GrantKind::parse(&r.get::<_, String>(2)?),
                label: r.get(3)?,
                created_at: ts(r.get(4)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn get_grant(&self, id: &str) -> Result<Grant> {
        self.list_grants()?
            .into_iter()
            .find(|g| g.id == id)
            .ok_or_else(|| MfError::NotFound(format!("authorized folder {id}")))
    }

    pub fn revoke_grant(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM grants WHERE id=?1", [id])?;
        Ok(())
    }

    // -------------------------------------------------------------- sessions

    pub fn insert_session(
        &mut self,
        s: &SessionRow,
        dests: &[DestRow],
        files: &[FileRow],
        ops: &[OpRow],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sessions(id,state,project,profile_json,source_root,source_label,source_scan_complete,created_at,scan_issues_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                s.id,
                s.state.as_str(),
                s.project,
                s.profile_json,
                s.source_root,
                s.source_label,
                s.source_scan_complete,
                s.created_at.to_rfc3339(),
                s.scan_issues_json
            ],
        )?;
        for d in dests {
            tx.execute(
                "INSERT INTO session_destinations(session_id,idx,root,label) VALUES (?1,?2,?3,?4)",
                params![s.id, d.idx, d.root, d.label],
            )?;
        }
        {
            let mut st = tx.prepare(
                "INSERT INTO files(session_id,file_id,rel_path,size,mtime_ns,inode,category,confidence,excluded_pattern,excluded_reason,source_sha256)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            )?;
            for f in files {
                st.execute(params![
                    s.id,
                    f.file_id,
                    f.rel_path,
                    f.size as i64,
                    f.mtime_ns,
                    f.inode.map(|i| i as i64),
                    f.category,
                    f.confidence,
                    f.excluded_pattern,
                    f.excluded_reason,
                    f.source_sha256
                ])?;
            }
            let now = crate::util::now().to_rfc3339();
            let mut st = tx.prepare(
                "INSERT INTO ops(session_id,file_id,dest_idx,target_rel,action,action_ref,action_level,state,notes_json,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            for o in ops {
                st.execute(params![
                    s.id,
                    o.file_id,
                    o.dest_idx,
                    o.target_rel,
                    o.action,
                    o.action_ref,
                    o.action_level,
                    o.state.as_str(),
                    o.notes_json,
                    now
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<SessionRow> {
        self.conn
            .query_row(
                "SELECT id,state,project,profile_json,source_root,source_label,source_scan_complete,created_at,started_at,finished_at,active_seconds,last_error,scan_issues_json
                 FROM sessions WHERE id=?1",
                [id],
                session_from_row,
            )
            .optional()?
            .ok_or_else(|| MfError::NotFound(format!("session {id}")))
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let mut st = self.conn.prepare(
            "SELECT id,state,project,profile_json,source_root,source_label,source_scan_complete,created_at,started_at,finished_at,active_seconds,last_error,scan_issues_json
             FROM sessions ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], session_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn set_session_state(
        &self,
        id: &str,
        state: SessionState,
        error: Option<&str>,
    ) -> Result<()> {
        let now = crate::util::now().to_rfc3339();
        let finished = if state.is_terminal() {
            Some(now.clone())
        } else {
            None
        };
        self.conn.execute(
            "UPDATE sessions SET state=?2, last_error=COALESCE(?3, last_error),
                 started_at=CASE WHEN ?2='copying' AND started_at IS NULL THEN ?4 ELSE started_at END,
                 finished_at=CASE WHEN ?5 IS NOT NULL THEN ?5 WHEN ?2='copying' THEN NULL ELSE finished_at END
             WHERE id=?1",
            params![id, state.as_str(), error, now, finished],
        )?;
        // Make the session state (and every entry before it) durable now.
        self.conn
            .query_row("PRAGMA wal_checkpoint(FULL)", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn clear_session_error(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE sessions SET last_error=NULL WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn add_active_seconds(&self, id: &str, secs: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET active_seconds=active_seconds+?2 WHERE id=?1",
            params![id, secs],
        )?;
        Ok(())
    }

    pub fn destinations(&self, id: &str) -> Result<Vec<DestRow>> {
        let mut st = self.conn.prepare(
            "SELECT idx,root,label FROM session_destinations WHERE session_id=?1 ORDER BY idx",
        )?;
        let rows = st.query_map([id], |r| {
            Ok(DestRow {
                idx: r.get(0)?,
                root: r.get(1)?,
                label: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn files(&self, id: &str) -> Result<Vec<FileRow>> {
        let mut st = self.conn.prepare(
            "SELECT file_id,rel_path,size,mtime_ns,inode,category,confidence,excluded_pattern,excluded_reason,source_sha256
             FROM files WHERE session_id=?1 ORDER BY file_id",
        )?;
        let rows = st.query_map([id], |r| {
            Ok(FileRow {
                file_id: r.get(0)?,
                rel_path: r.get(1)?,
                size: r.get::<_, i64>(2)? as u64,
                mtime_ns: r.get(3)?,
                inode: r.get::<_, Option<i64>>(4)?.map(|i| i as u64),
                category: r.get(5)?,
                confidence: r.get(6)?,
                excluded_pattern: r.get(7)?,
                excluded_reason: r.get(8)?,
                source_sha256: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn ops(&self, id: &str) -> Result<Vec<OpRow>> {
        let mut st = self.conn.prepare(
            "SELECT file_id,dest_idx,target_rel,final_rel,action,action_ref,action_level,state,temp_name,sha256,bytes,final_stamp_json,error_code,error_message,notes_json
             FROM ops WHERE session_id=?1 ORDER BY file_id, dest_idx",
        )?;
        let rows = st.query_map([id], |r| {
            Ok(OpRow {
                file_id: r.get(0)?,
                dest_idx: r.get(1)?,
                target_rel: r.get(2)?,
                final_rel: r.get(3)?,
                action: r.get(4)?,
                action_ref: r.get(5)?,
                action_level: r.get(6)?,
                state: OpState::parse(&r.get::<_, String>(7)?).unwrap_or(OpState::Failed),
                temp_name: r.get(8)?,
                sha256: r.get(9)?,
                bytes: r.get::<_, i64>(10)? as u64,
                final_stamp_json: r.get(11)?,
                error_code: r.get(12)?,
                error_message: r.get(13)?,
                notes_json: r.get(14)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Persist an op transition. Called *before* the corresponding disk action
    /// when entering `copying` (so the temp name is journaled first), and
    /// after it otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn update_op(
        &self,
        session: &str,
        file_id: i64,
        dest_idx: i64,
        state: OpState,
        temp_name: Option<&str>,
        final_rel: Option<&str>,
        sha256: Option<&str>,
        bytes: Option<u64>,
        final_stamp: Option<&FileStamp>,
        error: Option<(&str, &str)>,
    ) -> Result<()> {
        let stamp_json = final_stamp.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "UPDATE ops SET state=?4, temp_name=?5, final_rel=COALESCE(?6, final_rel), sha256=COALESCE(?7, sha256),
                 bytes=COALESCE(?8, bytes), final_stamp_json=COALESCE(?9, final_stamp_json),
                 error_code=?10, error_message=?11, updated_at=?12
             WHERE session_id=?1 AND file_id=?2 AND dest_idx=?3",
            params![
                session,
                file_id,
                dest_idx,
                state.as_str(),
                temp_name,
                final_rel,
                sha256,
                bytes.map(|b| b as i64),
                stamp_json,
                error.map(|e| e.0),
                error.map(|e| e.1),
                crate::util::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn set_op_target(
        &self,
        session: &str,
        file_id: i64,
        dest_idx: i64,
        target_rel: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE ops SET target_rel=?4 WHERE session_id=?1 AND file_id=?2 AND dest_idx=?3",
            params![session, file_id, dest_idx, target_rel],
        )?;
        Ok(())
    }

    pub fn set_file_hash(&self, session: &str, file_id: i64, sha: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET source_sha256=?3 WHERE session_id=?1 AND file_id=?2",
            params![session, file_id, sha],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- events

    pub fn log_event(
        &self,
        session: Option<&str>,
        level: &str,
        code: &str,
        message: &str,
        rel_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events(session_id,at,level,code,message,rel_path) VALUES (?1,?2,?3,?4,?5,?6)",
            params![session, crate::util::now().to_rfc3339(), level, code, message, rel_path],
        )?;
        Ok(())
    }

    pub fn events(&self, session: &str) -> Result<Vec<EventRow>> {
        let mut st = self.conn.prepare(
            "SELECT at,level,code,message,rel_path FROM events WHERE session_id=?1 ORDER BY id",
        )?;
        let rows = st.query_map([session], |r| {
            Ok(EventRow {
                at: ts(r.get(0)?),
                level: r.get(1)?,
                code: r.get(2)?,
                message: r.get(3)?,
                rel_path: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ------------------------------------------------------------ hash cache

    /// Returns a cached digest only if size, mtime and inode are unchanged.
    pub fn cached_hash(&self, path: &str, stamp: &FileStamp) -> Result<Option<Sha256Hex>> {
        let row: Option<(i64, i64, Option<i64>, String)> = self
            .conn
            .query_row(
                "SELECT size,mtime_ns,inode,sha256 FROM hash_cache WHERE path=?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(match row {
            Some((size, mtime, inode, sha))
                if size as u64 == stamp.size
                    && mtime == stamp.mtime_ns
                    && mtime != 0
                    && inode.map(|i| i as u64) == stamp.inode =>
            {
                Sha256Hex::parse(&sha).ok()
            }
            _ => None,
        })
    }

    pub fn put_hash(&self, path: &str, stamp: &FileStamp, sha: &Sha256Hex) -> Result<()> {
        self.conn.execute(
            "INSERT INTO hash_cache(path,size,mtime_ns,inode,sha256,computed_at) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns, inode=excluded.inode,
                 sha256=excluded.sha256, computed_at=excluded.computed_at",
            params![
                path,
                stamp.size as i64,
                stamp.mtime_ns,
                stamp.inode.map(|i| i as i64),
                sha.as_str(),
                crate::util::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    // --------------------------------------------------------------- library

    pub fn upsert_library(&self, row: &LibraryRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO library(dest_root,rel_path,sha256,size,category,project,source_label,source_rel,session_id,verified_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(dest_root, rel_path) DO UPDATE SET sha256=excluded.sha256, size=excluded.size,
                category=excluded.category, project=excluded.project, source_label=excluded.source_label,
                source_rel=excluded.source_rel, session_id=excluded.session_id, verified_at=excluded.verified_at",
            params![
                row.dest_root,
                row.rel_path,
                row.sha256,
                row.size as i64,
                row.category,
                row.project,
                row.source_label,
                row.source_rel,
                row.session_id,
                row.verified_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn library(&self, query: &str, limit: usize) -> Result<Vec<LibraryRow>> {
        let like = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut st = self.conn.prepare(
            "SELECT dest_root,rel_path,sha256,size,category,project,source_label,source_rel,session_id,verified_at
             FROM library WHERE rel_path LIKE ?1 ESCAPE '\\' OR project LIKE ?1 ESCAPE '\\'
             ORDER BY verified_at DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![like, limit as i64], |r| {
            Ok(LibraryRow {
                dest_root: r.get(0)?,
                rel_path: r.get(1)?,
                sha256: r.get(2)?,
                size: r.get::<_, i64>(3)? as u64,
                category: r.get(4)?,
                project: r.get(5)?,
                source_label: r.get(6)?,
                source_rel: r.get(7)?,
                session_id: r.get(8)?,
                verified_at: ts(r.get(9)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn save_check(&self, id: &str, source_root: &str, summary_json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO checks(id,at,source_root,summary_json) VALUES (?1,?2,?3,?4)",
            params![
                id,
                crate::util::now().to_rfc3339(),
                source_root,
                summary_json
            ],
        )?;
        Ok(())
    }

    pub fn last_check(&self, source_root: &str) -> Result<Option<(DateTime<Utc>, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT at, summary_json FROM checks WHERE source_root=?1 ORDER BY at DESC LIMIT 1",
                [source_root],
                |r| Ok((ts(r.get(0)?), r.get(1)?)),
            )
            .optional()?)
    }
}

fn session_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        state: SessionState::parse(&r.get::<_, String>(1)?).unwrap_or(SessionState::Failed),
        project: r.get(2)?,
        profile_json: r.get(3)?,
        source_root: r.get(4)?,
        source_label: r.get(5)?,
        source_scan_complete: r.get(6)?,
        created_at: ts(r.get(7)?),
        started_at: ts_opt(r.get(8)?),
        finished_at: ts_opt(r.get(9)?),
        active_seconds: r.get(10)?,
        last_error: r.get(11)?,
        scan_issues_json: r.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("mf.sqlite");
        let v = Db::open(&p).unwrap().schema_version().unwrap();
        assert_eq!(v, 1);
        assert_eq!(Db::open(&p).unwrap().schema_version().unwrap(), 1);
    }

    #[test]
    fn refuses_newer_schema() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("mf.sqlite");
        let db = Db::open(&p).unwrap();
        db.conn
            .execute("INSERT INTO schema_migrations VALUES (99, 'x')", [])
            .unwrap();
        drop(db);
        assert!(Db::open(&p).is_err());
    }

    #[test]
    fn hash_cache_invalidated_on_change() {
        let db = Db::open_in_memory().unwrap();
        let s = FileStamp {
            size: 10,
            mtime_ns: 5,
            inode: Some(1),
        };
        let h = crate::hash::hash_bytes(b"x");
        db.put_hash("/a", &s, &h).unwrap();
        assert_eq!(db.cached_hash("/a", &s).unwrap(), Some(h.clone()));
        assert_eq!(
            db.cached_hash("/a", &FileStamp { mtime_ns: 6, ..s })
                .unwrap(),
            None
        );
        assert_eq!(
            db.cached_hash("/a", &FileStamp { size: 11, ..s }).unwrap(),
            None
        );
        assert_eq!(
            db.cached_hash(
                "/a",
                &FileStamp {
                    inode: Some(2),
                    ..s
                }
            )
            .unwrap(),
            None
        );
        // mtime unknown → never trusted
        let s0 = FileStamp { mtime_ns: 0, ..s };
        db.put_hash("/b", &s0, &h).unwrap();
        assert_eq!(db.cached_hash("/b", &s0).unwrap(), None);
    }

    #[test]
    fn grants_unique() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_grant("/x", GrantKind::Source, "x").unwrap();
        let b = db.add_grant("/x", GrantKind::Source, "x").unwrap();
        assert_eq!(a.id, b.id);
        db.revoke_grant(&a.id).unwrap();
        assert!(db.get_grant(&a.id).is_err());
    }

    #[test]
    fn library_like_escapes() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.library("100%_", 10).unwrap().is_empty());
    }
}
