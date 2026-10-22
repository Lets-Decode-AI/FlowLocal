pub mod migrations;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub id: i64,
    pub created_at: i64,
    pub text: String,
    pub raw_text: String,
    pub app_name: Option<String>,
    pub duration_ms: i64,
    pub word_count: i64,
    pub wpm: f64,
    pub model_id: String,
}

/// A transcript to insert: same as [`Transcript`] minus `id`.
#[derive(Debug, Clone)]
pub struct NewTranscript {
    pub created_at: i64,
    pub text: String,
    pub raw_text: String,
    pub app_name: Option<String>,
    pub duration_ms: i64,
    pub word_count: i64,
    pub wpm: f64,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub id: i64,
    pub term: String,
    pub replacement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadRow {
    pub file: String,
    pub bytes_done: i64,
    pub total: i64,
    pub status: String,
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        // journal_mode returns a row, so read it rather than execute.
        conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Store> {
        migrations::migrate(&mut conn)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("store mutex poisoned"))
    }

    pub fn schema_version(&self) -> Result<i64> {
        migrations::current_version(&*self.lock()?)
    }

    pub fn insert_transcript(&self, t: NewTranscript) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO transcripts
             (created_at, text, raw_text, app_name, duration_ms, word_count, wpm, model_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                t.created_at,
                t.text,
                t.raw_text,
                t.app_name,
                t.duration_ms,
                t.word_count,
                t.wpm,
                t.model_id
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Newest first. `query` filters with a case-insensitive `LIKE` on `text`.
    pub fn list_transcripts(
        &self,
        page: u32,
        page_size: u32,
        query: Option<&str>,
    ) -> Result<Vec<Transcript>> {
        let conn = self.lock()?;
        let pattern = query
            .filter(|q| !q.is_empty())
            .map(|q| {
                let esc = q
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                format!("%{esc}%")
            })
            .unwrap_or_else(|| "%".to_string());
        let mut stmt = conn.prepare(
            "SELECT id, created_at, text, raw_text, app_name, duration_ms, word_count, wpm, model_id
             FROM transcripts WHERE text LIKE ?1 ESCAPE '\\'
             ORDER BY created_at DESC, id DESC LIMIT ?2 OFFSET ?3",
        )?;
        let offset = i64::from(page) * i64::from(page_size);
        let rows = stmt.query_map(params![pattern, i64::from(page_size), offset], |r| {
            Ok(Transcript {
                id: r.get(0)?,
                created_at: r.get(1)?,
                text: r.get(2)?,
                raw_text: r.get(3)?,
                app_name: r.get(4)?,
                duration_ms: r.get(5)?,
                word_count: r.get(6)?,
                wpm: r.get(7)?,
                model_id: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_transcript(&self, id: i64) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM transcripts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT value_json FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        match rows.next()? {
            Some(row) => {
                let json: String = row.get(0)?;
                Ok(Some(serde_json::from_str(&json)?))
            }
            None => Ok(None),
        }
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)?;
        self.lock()?.execute(
            "INSERT INTO settings(key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, json],
        )?;
        Ok(())
    }

    pub fn dictionary_list(&self) -> Result<Vec<DictEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT id, term, replacement FROM dictionary ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(DictEntry {
                id: r.get(0)?,
                term: r.get(1)?,
                replacement: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn dictionary_add(&self, term: &str, replacement: Option<&str>) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO dictionary(term, replacement) VALUES (?1, ?2)",
            params![term, replacement],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn dictionary_delete(&self, id: i64) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM dictionary WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn download_upsert(
        &self,
        model_id: &str,
        file: &str,
        bytes_done: i64,
        total: i64,
        status: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO downloads(model_id, file, bytes_done, total, status)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(model_id, file) DO UPDATE SET
               bytes_done = excluded.bytes_done, total = excluded.total, status = excluded.status",
            params![model_id, file, bytes_done, total, status],
        )?;
        Ok(())
    }

    pub fn downloads_for(&self, model_id: &str) -> Result<Vec<DownloadRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT file, bytes_done, total, status FROM downloads
             WHERE model_id = ?1 ORDER BY file",
        )?;
        let rows = stmt.query_map([model_id], |r| {
            Ok(DownloadRow {
                file: r.get(0)?,
                bytes_done: r.get(1)?,
                total: r.get(2)?,
                status: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nt(ts: i64, text: &str) -> NewTranscript {
        NewTranscript {
            created_at: ts,
            text: text.into(),
            raw_text: text.into(),
            app_name: Some("Notes".into()),
            duration_ms: 1000,
            word_count: 2,
            wpm: 120.0,
            model_id: "m".into(),
        }
    }

    #[test]
    fn migrates_fresh_db() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
    }

    #[test]
    fn transcript_roundtrip_and_search() {
        let s = Store::open_in_memory().unwrap();
        s.insert_transcript(nt(1, "hello world")).unwrap();
        s.insert_transcript(nt(2, "goodbye")).unwrap();
        let id3 = s.insert_transcript(nt(3, "Hello again")).unwrap();
        let hits = s.list_transcripts(0, 10, Some("hello")).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, id3);
        assert_eq!(hits[0].text, "Hello again");
        assert_eq!(hits[1].text, "hello world");
        assert_eq!(s.list_transcripts(0, 10, None).unwrap().len(), 3);
        assert_eq!(s.list_transcripts(1, 2, None).unwrap().len(), 1);
        s.delete_transcript(id3).unwrap();
        assert_eq!(s.list_transcripts(0, 10, Some("hello")).unwrap().len(), 1);
    }

    #[test]
    fn settings_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.get_setting::<i32>("idle_unload_min").unwrap(), None);
        s.set_setting("idle_unload_min", &10).unwrap();
        assert_eq!(s.get_setting::<i32>("idle_unload_min").unwrap(), Some(10));
        s.set_setting("idle_unload_min", &20).unwrap();
        assert_eq!(s.get_setting::<i32>("idle_unload_min").unwrap(), Some(20));
    }

    #[test]
    fn dictionary_crud() {
        let s = Store::open_in_memory().unwrap();
        let a = s.dictionary_add("kubernetes", None).unwrap();
        s.dictionary_add("tauri", Some("Tauri")).unwrap();
        let l = s.dictionary_list().unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].replacement.as_deref(), Some("Tauri"));
        s.dictionary_delete(a).unwrap();
        assert_eq!(s.dictionary_list().unwrap().len(), 1);
    }

    #[test]
    fn downloads_upsert() {
        let s = Store::open_in_memory().unwrap();
        s.download_upsert("m", "a.onnx", 1, 10, "downloading")
            .unwrap();
        s.download_upsert("m", "a.onnx", 10, 10, "done").unwrap();
        let d = s.downloads_for("m").unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].status, "done");
        assert!(s.downloads_for("x").unwrap().is_empty());
    }

    #[test]
    fn reopen_does_not_remigrate() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("flow.db");
        let s = Store::open(&p).unwrap();
        s.set_setting("k", &1).unwrap();
        drop(s);
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
        assert_eq!(s.get_setting::<i32>("k").unwrap(), Some(1));
    }
}
