use anyhow::Result;
use rusqlite::Connection;

/// Ordered list of migrations; index + 1 is the schema version it produces.
const MIGRATIONS: &[&str] = &[
    // v1
    "CREATE TABLE transcripts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at INTEGER NOT NULL,
        text TEXT NOT NULL,
        raw_text TEXT NOT NULL,
        app_name TEXT,
        duration_ms INTEGER NOT NULL,
        word_count INTEGER NOT NULL,
        wpm REAL NOT NULL,
        model_id TEXT NOT NULL
    );
    CREATE INDEX idx_transcripts_created_at ON transcripts(created_at);
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value_json TEXT NOT NULL
    );
    CREATE TABLE dictionary (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        term TEXT NOT NULL,
        replacement TEXT
    );
    CREATE TABLE downloads (
        model_id TEXT NOT NULL,
        file TEXT NOT NULL,
        bytes_done INTEGER NOT NULL,
        total INTEGER NOT NULL,
        status TEXT NOT NULL,
        PRIMARY KEY (model_id, file)
    );",
];

pub fn current_version(conn: &Connection) -> Result<i64> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);")?;
    let v: Option<i64> =
        conn.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))?;
    Ok(v.unwrap_or(0))
}

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = i as i64 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute("INSERT INTO schema_version(version) VALUES (?1)", [version])?;
        tx.commit()?;
    }
    Ok(())
}
