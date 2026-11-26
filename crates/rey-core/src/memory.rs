use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, Row, functions::FunctionFlags, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL,
 content TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messages_session_id_id ON messages(session_id, id);
CREATE TABLE IF NOT EXISTS memories (
 id INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL, value TEXT NOT NULL, source_message TEXT NOT NULL,
 importance INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
 updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS memory_events (
 id INTEGER PRIMARY KEY AUTOINCREMENT, event_type TEXT NOT NULL, payload TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS games (
 id TEXT PRIMARY KEY, record TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS chat_sessions (
 id TEXT PRIMARY KEY, scope TEXT NOT NULL DEFAULT 'chat'
);";

#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub markdown: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameRecord {
    pub id: Uuid,
    pub kind: String,
    pub user_side: String,
    pub moves: Vec<String>,
    pub initial_position: String,
    pub result: String,
    pub seconds: u64,
    pub engine: Option<String>,
    pub engine_settings: String,
    pub analysis: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

impl Store {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { path: path.into() };
        std::fs::create_dir_all(
            store
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let connection = store.connect()?;
        connection.execute_batch(SCHEMA)?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path).context("Не удалось открыть память Rey")?;
        conn.busy_timeout(Duration::from_secs(3))?;
        conn.create_scalar_function("rey_lower", 1, FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
            Ok(ctx.get::<String>(0)?.to_lowercase())
        })?;
        Ok(conn)
    }

    pub fn history(&self, session: &str, limit: usize) -> Result<Vec<Message>> {
        let conn = self.connect()?;
        let mut query = conn.prepare(
            "SELECT role, content FROM messages WHERE session_id=?1 AND role IN ('user','assistant') ORDER BY id DESC LIMIT ?2")?;
        let mut rows = query
            .query_map(params![session, limit], |row| {
                Ok(Message {
                    role: row.get(0)?,
                    content: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.reverse();
        Ok(rows)
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let conn = self.connect()?;
        let mut query = conn.prepare(
            "SELECT recent.session_id, (SELECT content FROM messages WHERE session_id=recent.session_id
                 AND role='user' ORDER BY id LIMIT 1)
             FROM (SELECT session_id, MAX(id) AS last FROM messages GROUP BY session_id) recent
             LEFT JOIN chat_sessions s ON s.id=recent.session_id
             WHERE s.scope IS NULL OR s.scope='chat' ORDER BY recent.last DESC")?;
        Ok(query
            .query_map([], |row| {
                let title: Option<String> = row.get(1)?;
                Ok(Session {
                    id: row.get(0)?,
                    title: title.unwrap_or_else(|| "Диалог".into()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE session_id=?1", [id])?;
        tx.execute("DELETE FROM chat_sessions WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn save_turn(
        &self,
        session: &str,
        scope: &str,
        user: &str,
        answer: &str,
        facts: &[(&str, String, u8)],
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO chat_sessions(id,scope) VALUES(?1,?2)",
            params![session, scope],
        )?;
        tx.execute("INSERT INTO messages(session_id,role,content) VALUES (?1,'user',?2),(?1,'assistant',?3)", params![session,user,answer])?;
        for (title, value, importance) in facts {
            tx.execute(
                "INSERT INTO memories(key,value,source_message,importance) SELECT ?1,?2,?3,?4
                 WHERE NOT EXISTS(SELECT 1 FROM memories WHERE key=?1 AND rey_lower(value)=rey_lower(?2))",
                params![title,value,user,importance],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn notes(&self, needle: &str, limit: usize) -> Result<Vec<Note>> {
        let conn = self.connect()?;
        let mut query = conn.prepare(
            "SELECT id,key,value FROM memories WHERE instr(rey_lower(key),?1)>0 OR instr(rey_lower(value),?1)>0
             ORDER BY importance DESC,updated_at DESC,id DESC LIMIT ?2")?;
        let needle = needle.trim().to_lowercase();
        Ok(query
            .query_map(params![needle, limit], note_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn note(&self, id: i64) -> Result<Note> {
        let conn = self.connect()?;
        Ok(conn.query_row(
            "SELECT id,key,value FROM memories WHERE id=?1",
            [id],
            note_from_row,
        )?)
    }

    pub fn save_note(&self, id: Option<i64>, title: &str, markdown: &str) -> Result<i64> {
        let title = title.trim();
        ensure!(
            !title.is_empty() && title.chars().count() <= 200 && !title.contains('\n'),
            "Название заметки должно занимать одну строку до 200 символов"
        );
        ensure!(
            !markdown.trim().is_empty() && markdown.len() <= 256 * 1024,
            "Текст заметки должен занимать до 256 КиБ"
        );
        let conn = self.connect()?;
        if let Some(id) = id {
            ensure!(
                conn.execute(
                    "UPDATE memories SET key=?1,value=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
                    params![title, markdown, id]
                )? == 1,
                "Заметка уже удалена"
            );
            return Ok(id);
        }
        conn.execute(
            "INSERT INTO memories(key,value,source_message) VALUES(?1,?2,'')",
            params![title, markdown],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM memories WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn save_game(&self, record: &GameRecord) -> Result<()> {
        let conn = self.connect()?;
        conn.execute("INSERT INTO games(id,record) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET record=excluded.record",
            params![record.id.to_string(), serde_json::to_string(record)?])?;
        Ok(())
    }

    pub fn games(&self, limit: usize) -> Result<Vec<GameRecord>> {
        let conn = self.connect()?;
        let mut query = conn.prepare(
            "SELECT record,created_at FROM games ORDER BY created_at DESC,rowid DESC LIMIT ?1",
        )?;
        query
            .query_map([limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|entry| {
                let (record, date) = entry?;
                let mut record: GameRecord = serde_json::from_str(&record)?;
                record.date = Some(date);
                Ok(record)
            })
            .collect()
    }
}

fn note_from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        markdown: row.get(2)?,
    })
}

pub fn copy_database(source: &Path, destination: &Path, legacy_import: bool) -> Result<()> {
    ensure!(!destination.exists(), "Файл назначения уже существует");
    let conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("Не удалось открыть исходную SQLite-БД")?;
    conn.busy_timeout(Duration::from_secs(3))?;
    if legacy_import {
        conn.prepare("SELECT id,session_id,role,content,created_at FROM messages LIMIT 0")?;
        conn.prepare("SELECT id,key,value,source_message,importance,created_at,updated_at FROM memories LIMIT 0")?;
        conn.prepare("SELECT id,event_type,payload,created_at FROM memory_events LIMIT 0")?;
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    conn.backup("main", temporary.path(), None)?;
    if legacy_import {
        Store::open(temporary.path())?;
    }
    temporary
        .persist_noclobber(destination)
        .context("Не удалось сохранить снимок БД")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_keeps_ids_events_and_history_and_refuses_overwrite() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("old.db");
        let target = folder.path().join("new.db");
        let conn = Connection::open(&source).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("DROP TABLE chat_sessions; DROP TABLE games;
            INSERT INTO messages(session_id,role,content) VALUES('old','user','Привет');
            INSERT INTO memories(id,key,value,source_message) VALUES(41,'Игра','СЁГИ','старый текст');
            INSERT INTO memory_events(event_type,payload) VALUES('memory_saved','{}');").unwrap();
        copy_database(&source, &target, true).unwrap();
        let store = Store::open(&target).unwrap();
        assert_eq!(store.notes("сёги", 10).unwrap()[0].id, 41);
        assert_eq!(store.history("old", 10).unwrap()[0].content, "Привет");
        assert_eq!(
            store
                .connect()
                .unwrap()
                .query_row("SELECT count(*) FROM memory_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(copy_database(&source, &target, true).is_err());
        store.delete_note(41).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM memories", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
