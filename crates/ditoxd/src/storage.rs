use std::borrow::Cow;
use std::sync::Mutex;
use std::time::Duration;

use ditox_rpc::{Entry, Filter, Kind};
use rusqlite::functions::FunctionFlags;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::blob;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::fuzzy;
use crate::image::{self, ImageSize};
use crate::text;

pub const SCHEMA_VERSION: i64 = 5;

const COMPACT_CHARS: u32 = 400;

const ENTRY_COLUMNS: &str = "entries.id, entries.kind, entries.mime, entries.content, entries.preview, entries.hash, entries.favorite, \
     entries.created_at_ms, entries.byte_len, entries.source_app, entries.blob_path, entries.image_width, entries.image_height, \
     entries.last_used_at_ms, entries.sticky";

const NOW_MS: &str = "CAST(strftime('%s','now') AS INTEGER) * 1000";

const V1_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK (kind IN ('text', 'image')),
  mime TEXT NOT NULL,
  content TEXT NOT NULL,
  preview TEXT NOT NULL,
  hash TEXT NOT NULL,
  favorite INTEGER NOT NULL DEFAULT 0,
  created_at_ms INTEGER NOT NULL,
  byte_len INTEGER NOT NULL,
  source_app TEXT
);
CREATE INDEX IF NOT EXISTS idx_entries_hash ON entries(hash);
CREATE INDEX IF NOT EXISTS idx_entries_created ON entries(created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_entries_favorite ON entries(favorite);
CREATE TABLE IF NOT EXISTS runtime_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
";

const REST_SCHEMA: &str = "
CREATE INDEX IF NOT EXISTS idx_entries_hash ON entries(hash);
CREATE INDEX IF NOT EXISTS idx_entries_created ON entries(created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_entries_favorite ON entries(favorite);
CREATE TABLE IF NOT EXISTS pending_blob_prunes (
  path TEXT PRIMARY KEY,
  created_at_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
);
CREATE VIRTUAL TABLE IF NOT EXISTS entry_fts USING fts5(content, preview, source_app, content='entries', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entry_fts(rowid, content, preview, source_app) VALUES (new.id, new.content, new.preview, new.source_app);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entry_fts(entry_fts, rowid, content, preview, source_app) VALUES ('delete', old.id, old.content, old.preview, old.source_app);
END;
CREATE TRIGGER IF NOT EXISTS entries_au AFTER UPDATE ON entries BEGIN
  INSERT INTO entry_fts(entry_fts, rowid, content, preview, source_app) VALUES ('delete', old.id, old.content, old.preview, old.source_app);
  INSERT INTO entry_fts(rowid, content, preview, source_app) VALUES (new.id, new.content, new.preview, new.source_app);
END;
CREATE TRIGGER IF NOT EXISTS entries_blob_ad AFTER DELETE ON entries
WHEN old.kind = 'image' AND old.blob_path IS NOT NULL AND old.blob_path != ''
BEGIN
  INSERT OR IGNORE INTO pending_blob_prunes(path) VALUES (old.blob_path);
END;
CREATE TRIGGER IF NOT EXISTS entry_formats_blob_ad AFTER DELETE ON entry_formats
WHEN old.blob_path IS NOT NULL AND old.blob_path != ''
BEGIN
  INSERT OR IGNORE INTO pending_blob_prunes(path) VALUES (old.blob_path);
END;
CREATE TABLE IF NOT EXISTS runtime_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferFormat<'a> {
    pub mime: String,
    pub bytes: Cow<'a, [u8]>,
    pub image: bool,
}

pub struct Offer<'a> {
    pub kind: Kind,
    pub mime: &'a str,
    pub bytes: &'a [u8],
    pub preview: &'a str,
    pub source_app: Option<&'a str>,
    pub formats: &'a [OfferFormat<'a>],
    pub size: ImageSize,
}

pub struct ListQuery<'a> {
    pub query: &'a str,
    pub filter: Filter,
    pub limit: u32,
    pub compact: bool,
}

impl<'a> ListQuery<'a> {
    pub fn new(query: &'a str, filter: Filter, limit: u32) -> Self {
        ListQuery {
            query,
            filter,
            limit,
            compact: false,
        }
    }
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(cfg: &Config) -> Result<Storage> {
        let conn = Connection::open(&cfg.db_path)
            .map_err(|err| Error::Sqlite(format!("{}: {err}", cfg.db_path)))?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.create_scalar_function(
            "ditox_fuzzy_score",
            2,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            fuzzy_udf(),
        )?;
        let storage = Storage { conn };
        storage.migrate(cfg)?;
        Ok(storage)
    }

    #[cfg(test)]
    pub fn add_text(&self, cfg: &Config, text: &str) -> Result<i64> {
        self.add_text_from(cfg, text, None)
    }

    pub fn add_text_from(&self, cfg: &Config, raw: &str, source_app: Option<&str>) -> Result<i64> {
        let sanitized = text::sanitize_text(raw.as_bytes());
        let content = text::cap_utf8(&sanitized, cfg.max_text_bytes);
        let preview = text::preview(content, cfg.max_preview_chars);
        let hash = text::sha256_hex(content.as_bytes());
        self.immediate(|| {
            if !cfg.allow_duplicates
                && let Some(existing) = self.find_id_by_hash(&hash)? {
                    self.touch_last_used(existing, source_app)?;
                    return Ok(existing);
                }
            self.conn.execute(
                &format!(
                    "INSERT INTO entries (kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, source_app) \
                     VALUES ('text', 'text/plain;charset=utf-8', ?, ?, ?, 0, {NOW_MS}, ?, ?)"
                ),
                params![content, preview, hash, content.len() as i64, source_app],
            )?;
            let id = self.conn.last_insert_rowid();
            self.insert_format(id, "text/plain", content.len() as i64, Some(content), None)?;
            self.apply_retention(cfg)?;
            Ok(id)
        })
    }

    #[cfg(test)]
    pub fn add_image(
        &self,
        cfg: &Config,
        mime: &str,
        bytes: &[u8],
        size: ImageSize,
    ) -> Result<i64> {
        self.add_image_from(cfg, mime, bytes, size, None)
    }

    pub fn add_image_from(
        &self,
        cfg: &Config,
        mime: &str,
        bytes: &[u8],
        size: ImageSize,
        source_app: Option<&str>,
    ) -> Result<i64> {
        if !cfg.allow_duplicates
            && let Some(existing) = self.find_id_by_hash(&text::sha256_hex(bytes))?
        {
            self.touch_last_used(existing, source_app)?;
            return Ok(existing);
        }
        let format = OfferFormat {
            mime: mime.to_string(),
            bytes: Cow::Borrowed(bytes),
            image: true,
        };
        self.add_offer(
            cfg,
            &Offer {
                kind: Kind::Image,
                mime,
                bytes,
                preview: "",
                source_app,
                formats: std::slice::from_ref(&format),
                size,
            },
        )
    }

    pub fn add_offer(&self, cfg: &Config, offer: &Offer<'_>) -> Result<i64> {
        let hash = text::sha256_hex(offer.bytes);
        let is_image = offer.kind == Kind::Image;
        self.immediate(|| {
            if !cfg.allow_duplicates
                && let Some(existing) = self.find_id_by_hash(&hash)? {
                    self.replace_formats(cfg, existing, offer.formats)?;
                    if !offer.preview.is_empty() {
                        let preview = text::preview(offer.preview, cfg.max_preview_chars);
                        self.conn.execute("UPDATE entries SET preview = ? WHERE id = ?", params![preview, existing])?;
                    }
                    self.touch_last_used(existing, offer.source_app)?;
                    self.apply_retention(cfg)?;
                    return Ok(existing);
                }

            let blob_path = if is_image {
                let path = blob::write_image_blob(&cfg.image_dir, &hash, image::extension(offer.mime), offer.bytes)?;
                self.cancel_pending_blob_prune(&path)?;
                Some(path)
            } else {
                None
            };
            let body = String::from_utf8_lossy(offer.bytes);
            let preview = if is_image && offer.preview.is_empty() {
                image::preview(offer.mime, offer.bytes.len(), offer.size)
            } else {
                text::preview(if offer.preview.is_empty() { &body } else { offer.preview }, cfg.max_preview_chars)
            };
            let content: &str = if is_image { &hash } else { &body };
            self.conn.execute(
                &format!(
                    "INSERT INTO entries \
                     (kind, mime, content, preview, hash, favorite, created_at_ms, byte_len, blob_path, image_width, image_height, source_app) \
                     VALUES (?, ?, ?, ?, ?, 0, {NOW_MS}, ?, ?, ?, ?, ?)"
                ),
                params![
                    offer.kind.as_str(),
                    offer.mime,
                    content,
                    preview,
                    hash,
                    offer.bytes.len() as i64,
                    blob_path,
                    offer.size.width.filter(|_| is_image),
                    offer.size.height.filter(|_| is_image),
                    offer.source_app,
                ],
            )?;
            let id = self.conn.last_insert_rowid();
            self.replace_formats(cfg, id, offer.formats)?;
            self.apply_retention(cfg)?;
            Ok(id)
        })
    }

    fn replace_formats(&self, cfg: &Config, id: i64, formats: &[OfferFormat<'_>]) -> Result<()> {
        self.conn
            .execute("DELETE FROM entry_formats WHERE entry_id = ?", params![id])?;
        for format in formats {
            let len = format.bytes.len() as i64;
            if format.image {
                let hash = text::sha256_hex(&format.bytes);
                let path = blob::write_image_blob(
                    &cfg.image_dir,
                    &hash,
                    image::extension(&format.mime),
                    &format.bytes,
                )?;
                self.cancel_pending_blob_prune(&path)?;
                self.insert_format(id, &format.mime, len, None, Some(&path))?;
            } else {
                self.insert_format(
                    id,
                    &format.mime,
                    len,
                    Some(&String::from_utf8_lossy(&format.bytes)),
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn insert_format(
        &self,
        id: i64,
        mime: &str,
        byte_len: i64,
        content: Option<&str>,
        blob_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO entry_formats(entry_id, mime, byte_len, content, blob_path) VALUES (?, ?, ?, ?, ?)",
            params![id, mime, byte_len, content, blob_path],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn format_mimes(&self, id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT mime FROM entry_formats WHERE entry_id = ? ORDER BY id ASC")?;
        let rows = stmt.query_map(params![id], |row| text_column(row, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn format_text(&self, id: i64, mime: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT content FROM entry_formats WHERE entry_id = ? AND mime = ? AND content IS NOT NULL LIMIT 1",
                params![id, mime],
                |row| text_column(row, 0),
            )
            .optional()?)
    }

    fn touch_last_used(&self, id: i64, source_app: Option<&str>) -> Result<()> {
        match source_app {
            Some(app) => self.conn.execute(
                &format!(
                    "UPDATE entries SET last_used_at_ms = {NOW_MS}, source_app = ? WHERE id = ?"
                ),
                params![app, id],
            )?,
            None => self.conn.execute(
                &format!("UPDATE entries SET last_used_at_ms = {NOW_MS} WHERE id = ?"),
                params![id],
            )?,
        };
        Ok(())
    }

    pub fn list(&self, query: &ListQuery<'_>) -> Result<Vec<Entry>> {
        let trimmed = query.query.trim();
        let fts = build_fts_query(trimmed);
        let has_query = !fts.is_empty();
        let contains = build_like_contains(trimmed);
        let prefix = build_like_prefix(trimmed);
        let mut binds: Vec<Value> = Vec::new();
        let bind = |binds: &mut Vec<Value>, value: &str| binds.push(Value::Text(value.to_string()));

        let columns = if query.compact {
            ENTRY_COLUMNS.replace(
                "entries.content,",
                &format!("substr(entries.content, 1, {COMPACT_CHARS}),"),
            )
        } else {
            ENTRY_COLUMNS.to_string()
        };
        let mut sql = format!("SELECT {columns} FROM entries WHERE 1 = 1");
        if has_query {
            sql.push_str(
                " AND (
                   entries.id IN (SELECT rowid FROM entry_fts WHERE entry_fts MATCH ?)
                   OR LOWER(entries.content) LIKE LOWER(?) ESCAPE '\\'
                   OR LOWER(entries.preview) LIKE LOWER(?) ESCAPE '\\'
                   OR ditox_fuzzy_score(entries.content, ?) > 0
                   OR ditox_fuzzy_score(entries.preview, ?) > 0
                 )",
            );
            for value in [
                fts.as_str(),
                contains.as_str(),
                contains.as_str(),
                trimmed,
                trimmed,
            ] {
                bind(&mut binds, value);
            }
        }
        sql.push_str(match query.filter {
            Filter::All => "",
            Filter::Text => " AND kind = 'text'",
            Filter::Images => " AND kind = 'image'",
            Filter::Favorites => " AND favorite = 1",
            Filter::Today => " AND created_at_ms >= (CAST(strftime('%s','now','start of day') AS INTEGER) * 1000)",
        });
        if has_query {
            sql.push_str(
                " ORDER BY sticky DESC, favorite DESC,
                 CASE
                   WHEN LOWER(entries.content) = LOWER(?) OR LOWER(entries.preview) = LOWER(?) THEN 0
                   WHEN LOWER(entries.content) LIKE LOWER(?) ESCAPE '\\' OR LOWER(entries.preview) LIKE LOWER(?) ESCAPE '\\' THEN 1
                   WHEN LOWER(entries.content) LIKE LOWER(?) ESCAPE '\\' THEN 2
                   WHEN LOWER(entries.preview) LIKE LOWER(?) ESCAPE '\\' THEN 3
                   WHEN ditox_fuzzy_score(entries.content, ?) > 0 OR ditox_fuzzy_score(entries.preview, ?) > 0 THEN 4
                   ELSE 5
                 END ASC,
                 max(ditox_fuzzy_score(entries.content, ?), ditox_fuzzy_score(entries.preview, ?)) DESC,
                 max(entries.created_at_ms, COALESCE(entries.last_used_at_ms, 0)) DESC, id DESC LIMIT ?",
            );
            for value in [
                trimmed,
                trimmed,
                prefix.as_str(),
                prefix.as_str(),
                contains.as_str(),
                contains.as_str(),
                trimmed,
                trimmed,
                trimmed,
                trimmed,
            ] {
                bind(&mut binds, value);
            }
        } else {
            sql.push_str(" ORDER BY sticky DESC, favorite DESC, max(created_at_ms, COALESCE(last_used_at_ms, 0)) DESC, id DESC LIMIT ?");
        }
        binds.push(Value::Integer(i64::from(query.limit)));

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(binds), read_entry)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE id = ?"),
                params![id],
                read_entry,
            )
            .optional()?)
    }

    pub fn delete(&self, id: i64) -> Result<bool> {
        let changed = self
            .conn
            .execute("DELETE FROM entries WHERE id = ?", params![id])?;
        if changed > 0 {
            self.prune_pending_blobs()?;
        }
        Ok(changed > 0)
    }

    pub fn set_sticky(&self, id: i64, sticky: bool) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE entries SET sticky = ? WHERE id = ?",
            params![sticky, id],
        )? > 0)
    }

    pub fn favorite(&self, id: i64, value: bool) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE entries SET favorite = ? WHERE id = ?",
            params![value, id],
        )? > 0)
    }

    pub fn mark_used(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE entries SET last_used_at_ms = CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) WHERE id = ?",
            params![id],
        )? > 0)
    }

    pub fn schema_version(&self) -> Result<i64> {
        self.scalar("PRAGMA user_version")
    }

    pub fn apply_retention(&self, cfg: &Config) -> Result<i64> {
        let mut changed = 0;
        if cfg.delete_after_seconds > 0 {
            changed += self.delete_expired(cfg.delete_after_seconds)?;
        }
        changed += self.prune(cfg.max_entries)?;
        changed += self.prune_images_by_bytes(cfg.max_image_bytes)?;
        if changed > 0 {
            self.prune_pending_blobs()?;
        }
        Ok(changed)
    }

    pub fn pause_watcher(&self, duration_ms: u32) -> Result<()> {
        let until = if duration_ms == 0 {
            -1
        } else {
            self.now_ms()? + i64::from(duration_ms)
        };
        self.set_runtime("paused_until_ms", &until.to_string())
    }

    pub fn resume_watcher(&self) -> Result<()> {
        self.set_runtime("paused_until_ms", "0")
    }

    pub fn is_watcher_paused(&self) -> Result<bool> {
        let Some(until) = self
            .runtime("paused_until_ms")?
            .and_then(|value| value.parse::<i64>().ok())
        else {
            return Ok(false);
        };
        Ok(until < 0 || until > self.now_ms()?)
    }

    pub fn set_watcher_error(&self, message: &str) -> Result<()> {
        self.set_runtime("watcher_last_error", message)
    }

    pub fn clear_watcher_error(&self) -> Result<()> {
        self.delete_runtime("watcher_last_error")
    }

    pub fn watcher_last_error(&self) -> Result<Option<String>> {
        self.runtime("watcher_last_error")
    }

    pub fn mark_self_write(&self, hash: &str) -> Result<()> {
        self.set_runtime("last_self_hash", hash)
    }

    pub fn self_write_hash(&self) -> Result<Option<String>> {
        self.runtime("last_self_hash")
    }

    pub fn clear_self_write_hash(&self) -> Result<()> {
        self.delete_runtime("last_self_hash")
    }

    pub fn now_ms(&self) -> Result<i64> {
        self.scalar(&format!("SELECT {NOW_MS}"))
    }

    fn migrate(&self, cfg: &Config) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
        )?;

        let version = self.schema_version()?;
        if version > SCHEMA_VERSION {
            return Err(Error::Sqlite(format!(
                "the history was written by a newer ditox (schema {version})"
            )));
        }
        let rebuild_fts = version < SCHEMA_VERSION || !self.table_exists("entry_fts")?;
        if (1..5).contains(&version) {
            self.keep_a_copy(&format!("{}.before-v5", cfg.db_path))?;
        }

        if version < SCHEMA_VERSION {
            self.immediate(|| {
                if version < 1 {
                    self.conn.execute_batch(V1_SCHEMA)?;
                    self.ensure_column("source_app", "TEXT")?;
                }
                if version < 2 {
                    self.ensure_column("blob_path", "TEXT")?;
                    self.ensure_column("image_width", "INTEGER")?;
                    self.ensure_column("image_height", "INTEGER")?;
                    self.conn.execute_batch(
                        "DROP TRIGGER IF EXISTS entries_ai; DROP TRIGGER IF EXISTS entries_ad; \
                         DROP TRIGGER IF EXISTS entries_au; DROP TABLE IF EXISTS entry_fts;",
                    )?;
                }
                if version < 3 {
                    self.ensure_column("last_used_at_ms", "INTEGER")?;
                }
                if version < 4 {
                    self.migrate_to_v4()?;
                }
                if version < 5 {
                    self.migrate_to_v5(cfg)?;
                }
                self.conn
                    .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
                Ok(())
            })?;
        }

        self.conn.execute_batch(REST_SCHEMA)?;
        if rebuild_fts {
            self.conn
                .execute_batch("INSERT INTO entry_fts(entry_fts) VALUES('rebuild')")?;
        }
        Ok(())
    }

    /// The history as it was, for going back to the daemon that wrote it: an
    /// older daemon refuses a newer schema.
    fn keep_a_copy(&self, path: &str) -> Result<()> {
        if !blob::file_exists(path) {
            self.conn.execute("VACUUM INTO ?", params![path])?;
        }
        Ok(())
    }

    /// Until schema 4 the watcher read text with `wl-paste --type`, which adds
    /// a line break, so every captured text carries one it never had and
    /// pastes it back. Take that one line break off.
    fn migrate_to_v5(&self, cfg: &Config) -> Result<()> {
        let rows: Vec<(i64, Vec<u8>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, content FROM entries WHERE kind = 'text' AND substr(content, -1) = char(10)",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, bytes_column(row, 1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (id, mut bytes) in rows {
            bytes.pop();
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };
            if content.is_empty() {
                continue;
            }
            self.conn.execute(
                "UPDATE entries SET content = ?, preview = ?, hash = ?, byte_len = ? WHERE id = ?",
                params![
                    content,
                    text::preview(&content, cfg.max_preview_chars),
                    text::sha256_hex(content.as_bytes()),
                    content.len() as i64,
                    id
                ],
            )?;
        }
        if self.table_exists("entry_formats")? {
            self.conn.execute_batch(
                "UPDATE entry_formats
                 SET content = substr(content, 1, length(content) - 1), byte_len = byte_len - 1
                 WHERE content IS NOT NULL AND length(content) > 1 AND substr(content, -1) = char(10);",
            )?;
        }
        Ok(())
    }

    fn migrate_to_v4(&self) -> Result<()> {
        self.ensure_column("sticky", "INTEGER NOT NULL DEFAULT 0")?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS groups (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               name TEXT NOT NULL,
               position INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        self.ensure_column(
            "group_id",
            "INTEGER REFERENCES groups(id) ON DELETE SET NULL",
        )?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entry_formats (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
               mime TEXT NOT NULL,
               byte_len INTEGER NOT NULL,
               content TEXT,
               blob_path TEXT,
               CHECK ((content IS NULL) <> (blob_path IS NULL))
             );
             CREATE INDEX IF NOT EXISTS idx_entry_formats_entry ON entry_formats(entry_id);
             INSERT INTO entry_formats(entry_id, mime, byte_len, content, blob_path)
             SELECT id,
               CASE WHEN kind = 'image' THEN mime ELSE 'text/plain' END,
               byte_len,
               CASE WHEN kind = 'text' THEN content ELSE NULL END,
               CASE WHEN kind = 'image' THEN blob_path ELSE NULL END
             FROM entries
             WHERE id NOT IN (SELECT entry_id FROM entry_formats)
               AND ((kind = 'text') OR (kind = 'image' AND blob_path IS NOT NULL AND blob_path != ''));",
        )?;
        Ok(())
    }

    fn ensure_column(&self, column: &str, definition: &str) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(entries)")?;
        let names = stmt
            .query_map([], |row| text_column(row, 1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !names.iter().any(|name| name == column) {
            self.conn.execute_batch(&format!(
                "ALTER TABLE entries ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    fn table_exists(&self, table: &str) -> Result<bool> {
        self.exists("SELECT 1 FROM sqlite_master WHERE type IN ('table', 'virtual table') AND name = ? LIMIT 1", table)
    }

    fn set_runtime(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO runtime_state(key, value, updated_at_ms) VALUES (?, ?, {NOW_MS}) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at_ms = excluded.updated_at_ms"
            ),
            params![key, value],
        )?;
        Ok(())
    }

    fn runtime(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM runtime_state WHERE key = ?",
                params![key],
                |row| text_column(row, 0),
            )
            .optional()?)
    }

    fn delete_runtime(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM runtime_state WHERE key = ?", params![key])?;
        Ok(())
    }

    fn prune(&self, max_entries: u32) -> Result<i64> {
        if self.scalar("SELECT count(*) FROM entries")? <= i64::from(max_entries) {
            return Ok(0);
        }
        Ok(self.conn.execute(
            "DELETE FROM entries WHERE favorite = 0 AND id NOT IN (
               SELECT id FROM entries ORDER BY favorite DESC, max(created_at_ms, COALESCE(last_used_at_ms, 0)) DESC, id DESC LIMIT ?
             )",
            params![max_entries],
        )? as i64)
    }

    fn prune_images_by_bytes(&self, max_bytes: u32) -> Result<i64> {
        if max_bytes == 0 {
            return Ok(0);
        }
        let images: Vec<(i64, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT e.id,
                   COALESCE((SELECT SUM(f.byte_len) FROM entry_formats f WHERE f.entry_id = e.id AND f.mime LIKE 'image/%'), e.byte_len)
                 FROM entries e
                 WHERE e.favorite = 0 AND e.kind = 'image'
                 ORDER BY max(e.created_at_ms, COALESCE(e.last_used_at_ms, 0)) ASC, e.id ASC",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut total: i64 = images.iter().map(|(_, len)| (*len).max(0)).sum();
        let mut removed = 0;
        for (id, len) in images {
            if total <= i64::from(max_bytes) {
                break;
            }
            if self.conn.execute(
                "DELETE FROM entries WHERE id = ? AND favorite = 0",
                params![id],
            )? == 0
            {
                continue;
            }
            removed += 1;
            total -= len.max(0);
        }
        Ok(removed)
    }

    fn delete_expired(&self, delete_after_seconds: u32) -> Result<i64> {
        let cutoff = self.now_ms()? - i64::from(delete_after_seconds) * 1000;
        Ok(self.conn.execute(
            "DELETE FROM entries WHERE favorite = 0 AND max(created_at_ms, COALESCE(last_used_at_ms, 0)) < ?",
            params![cutoff],
        )? as i64)
    }

    fn prune_pending_blobs(&self) -> Result<i64> {
        let paths: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path FROM pending_blob_prunes ORDER BY created_at_ms ASC")?;
            let rows = stmt.query_map([], |row| text_column(row, 0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut pruned = 0;
        for path in paths {
            if !self.blob_path_referenced(&path)? {
                blob::delete_file(&path);
            }
            pruned += self.conn.execute(
                "DELETE FROM pending_blob_prunes WHERE path = ?",
                params![path],
            )? as i64;
        }
        Ok(pruned)
    }

    fn blob_path_referenced(&self, path: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM entries WHERE blob_path = ? UNION SELECT 1 FROM entry_formats WHERE blob_path = ? LIMIT 1",
                params![path, path],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn cancel_pending_blob_prune(&self, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM pending_blob_prunes WHERE path = ?",
            params![path],
        )?;
        Ok(())
    }

    fn find_id_by_hash(&self, hash: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM entries WHERE hash = ? ORDER BY id DESC LIMIT 1",
                params![hash],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn scalar(&self, sql: &str) -> Result<i64> {
        Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
    }

    fn exists(&self, sql: &str, value: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(sql, params![value], |_| Ok(()))
            .optional()?
            .is_some())
    }

    fn immediate<T>(&self, body: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match body() {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn raw(&self) -> &Connection {
        &self.conn
    }
}

/// The search calls the score several times per row with the same arguments,
/// in WHERE and again in ORDER BY. The last two answers are kept.
fn fuzzy_udf()
-> impl Fn(&rusqlite::functions::Context<'_>) -> rusqlite::Result<i64> + Send + 'static {
    type Slot = Option<(Vec<u8>, Vec<u8>, i64)>;
    let memo: Mutex<[Slot; 2]> = Mutex::new([None, None]);
    move |ctx| {
        let haystack = ctx.get_raw(0).as_bytes().unwrap_or(b"");
        let needle = ctx.get_raw(1).as_bytes().unwrap_or(b"");
        let mut memo = memo.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for slot in memo.iter().flatten() {
            if slot.0 == haystack && slot.1 == needle {
                return Ok(slot.2);
            }
        }
        let score = fuzzy::score(haystack, needle);
        memo.swap(0, 1);
        memo[0] = Some((haystack.to_vec(), needle.to_vec(), score));
        Ok(score)
    }
}

fn read_entry(row: &Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        kind: Kind::parse(&text_column(row, 1)?).unwrap_or(Kind::Text),
        mime: text_column(row, 2)?,
        content: text_column(row, 3)?,
        preview: text_column(row, 4)?,
        hash: text_column(row, 5)?,
        favorite: row.get::<_, i64>(6)? != 0,
        created_at_ms: row.get(7)?,
        byte_len: row.get(8)?,
        source_app: optional_text_column(row, 9)?,
        blob_path: optional_text_column(row, 10)?,
        image_width: row.get(11)?,
        image_height: row.get(12)?,
        last_used_at_ms: row.get(13)?,
        sticky: row.get::<_, i64>(14)? != 0,
    })
}

fn bytes_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Vec<u8>> {
    Ok(match row.get_ref(index)? {
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.to_vec(),
        ValueRef::Integer(value) => value.to_string().into_bytes(),
        ValueRef::Real(value) => value.to_string().into_bytes(),
        ValueRef::Null => Vec::new(),
    })
}

fn text_column(row: &Row<'_>, index: usize) -> rusqlite::Result<String> {
    Ok(String::from_utf8_lossy(&bytes_column(row, index)?).into_owned())
}

fn optional_text_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<String>> {
    match row.get_ref(index)? {
        ValueRef::Null => Ok(None),
        _ => text_column(row, index).map(Some),
    }
}

fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| token.trim_matches(['"', '\'']))
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn escape_like(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    for ch in query.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn build_like_contains(query: &str) -> String {
    format!("%{}%", escape_like(query))
}

fn build_like_prefix(query: &str) -> String {
    format!("{}%", escape_like(query))
}

#[cfg(test)]
mod tests;
