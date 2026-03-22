//! SQLite database for credential storage and operation logging.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SavedCredential {
    pub id: i64,
    pub launcher: String,
    pub username: String,
    pub synced_at: String,
    pub file_count: i32,
    pub total_size: i64,
    pub notes: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginAccount {
    pub id: i64,
    pub launcher: String,
    pub label: String,
    pub username: String,
    pub password: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    pub id: i64,
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS credentials (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                launcher TEXT NOT NULL,
                username TEXT NOT NULL,
                synced_at TEXT NOT NULL DEFAULT (datetime('now')),
                registry_data BLOB,
                file_data BLOB,
                file_count INTEGER DEFAULT 0,
                total_size INTEGER DEFAULT 0,
                notes TEXT DEFAULT '',
                UNIQUE(launcher, username)
            );

            CREATE TABLE IF NOT EXISTS operation_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                level TEXT NOT NULL DEFAULT 'info',
                message TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS login_accounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                launcher TEXT NOT NULL,
                label TEXT NOT NULL,
                username TEXT NOT NULL,
                password TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(launcher, username)
            );
        ").map_err(|e| format!("Failed to create tables: {}", e))?;

        Ok(Database { conn })
    }

    // ── Credential CRUD ─────────────────────────────────────────────

    pub fn save_credential(
        &self,
        launcher: &str,
        username: &str,
        registry_data: Option<&[u8]>,
        file_data: Option<&[u8]>,
        file_count: i32,
        total_size: i64,
    ) -> Result<i64, String> {
        // Use ON CONFLICT UPDATE to preserve row ID (INSERT OR REPLACE deletes + re-inserts,
        // which changes the ID and breaks frontend references)
        self.conn.execute(
            "INSERT INTO credentials (launcher, username, synced_at, registry_data, file_data, file_count, total_size)
             VALUES (?1, ?2, datetime('now'), ?3, ?4, ?5, ?6)
             ON CONFLICT(launcher, username) DO UPDATE SET
                synced_at = datetime('now'),
                registry_data = ?3,
                file_data = ?4,
                file_count = ?5,
                total_size = ?6",
            params![launcher, username, registry_data, file_data, file_count, total_size],
        ).map_err(|e| format!("Failed to save credential: {}", e))?;

        // Return the ID (works for both insert and update)
        let id = self.conn.query_row(
            "SELECT id FROM credentials WHERE launcher = ?1 AND username = ?2",
            params![launcher, username],
            |row| row.get(0),
        ).map_err(|e| format!("Failed to get credential ID: {}", e))?;

        Ok(id)
    }

    pub fn list_credentials(&self, launcher: Option<&str>) -> Result<Vec<SavedCredential>, String> {
        let mut stmt = if let Some(l) = launcher {
            let mut s = self.conn.prepare(
                "SELECT id, launcher, username, synced_at, file_count, total_size, notes FROM credentials WHERE launcher = ?1 ORDER BY synced_at DESC"
            ).map_err(|e| e.to_string())?;
            let rows = s.query_map(params![l], |row| {
                Ok(SavedCredential {
                    id: row.get(0)?,
                    launcher: row.get(1)?,
                    username: row.get(2)?,
                    synced_at: row.get(3)?,
                    file_count: row.get(4)?,
                    total_size: row.get(5)?,
                    notes: row.get(6)?,
                })
            }).map_err(|e| e.to_string())?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string());
        } else {
            let mut s = self.conn.prepare(
                "SELECT id, launcher, username, synced_at, file_count, total_size, notes FROM credentials ORDER BY launcher, synced_at DESC"
            ).map_err(|e| e.to_string())?;
            let rows = s.query_map([], |row| {
                Ok(SavedCredential {
                    id: row.get(0)?,
                    launcher: row.get(1)?,
                    username: row.get(2)?,
                    synced_at: row.get(3)?,
                    file_count: row.get(4)?,
                    total_size: row.get(5)?,
                    notes: row.get(6)?,
                })
            }).map_err(|e| e.to_string())?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string());
        };
    }

    pub fn get_credential_data(&self, id: i64) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), String> {
        self.conn.query_row(
            "SELECT registry_data, file_data FROM credentials WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|e| format!("Credential not found: {}", e))
    }

    pub fn get_credential(&self, id: i64) -> Result<SavedCredential, String> {
        self.conn.query_row(
            "SELECT id, launcher, username, synced_at, file_count, total_size, notes FROM credentials WHERE id = ?1",
            params![id],
            |row| Ok(SavedCredential {
                id: row.get(0)?,
                launcher: row.get(1)?,
                username: row.get(2)?,
                synced_at: row.get(3)?,
                file_count: row.get(4)?,
                total_size: row.get(5)?,
                notes: row.get(6)?,
            }),
        ).map_err(|e| format!("Credential not found: {}", e))
    }

    pub fn remove_credential(&self, id: i64) -> Result<bool, String> {
        let affected = self.conn.execute(
            "DELETE FROM credentials WHERE id = ?1",
            params![id],
        ).map_err(|e| format!("Failed to delete: {}", e))?;
        Ok(affected > 0)
    }

    // ── Login Accounts (username/password for auto-login) ──────────

    pub fn save_login_account(
        &self,
        launcher: &str,
        label: &str,
        username: &str,
        password: &str,
    ) -> Result<i64, String> {
        self.conn.execute(
            "INSERT INTO login_accounts (launcher, label, username, password)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(launcher, username) DO UPDATE SET
                label = ?2,
                password = ?4",
            params![launcher, label, username, password],
        ).map_err(|e| format!("Failed to save login account: {}", e))?;

        let id = self.conn.query_row(
            "SELECT id FROM login_accounts WHERE launcher = ?1 AND username = ?2",
            params![launcher, username],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;

        Ok(id)
    }

    pub fn list_login_accounts(&self, launcher: &str) -> Result<Vec<LoginAccount>, String> {
        let mut s = self.conn.prepare(
            "SELECT id, launcher, label, username, password, created_at FROM login_accounts WHERE launcher = ?1 ORDER BY label"
        ).map_err(|e| e.to_string())?;
        let rows = s.query_map(params![launcher], |row| {
            Ok(LoginAccount {
                id: row.get(0)?,
                launcher: row.get(1)?,
                label: row.get(2)?,
                username: row.get(3)?,
                password: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_login_account(&self, id: i64) -> Result<LoginAccount, String> {
        self.conn.query_row(
            "SELECT id, launcher, label, username, password, created_at FROM login_accounts WHERE id = ?1",
            params![id],
            |row| Ok(LoginAccount {
                id: row.get(0)?,
                launcher: row.get(1)?,
                label: row.get(2)?,
                username: row.get(3)?,
                password: row.get(4)?,
                created_at: row.get(5)?,
            }),
        ).map_err(|e| format!("Login account not found: {}", e))
    }

    pub fn remove_login_account(&self, id: i64) -> Result<bool, String> {
        let affected = self.conn.execute(
            "DELETE FROM login_accounts WHERE id = ?1",
            params![id],
        ).map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    // ── Operation Log ───────────────────────────────────────────────

    pub fn log(&self, level: &str, message: &str) {
        let _ = self.conn.execute(
            "INSERT INTO operation_log (level, message) VALUES (?1, ?2)",
            params![level, message],
        );
    }

    pub fn get_recent_logs(&self, limit: i32) -> Result<Vec<LogEntry>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, level, message FROM operation_log ORDER BY id DESC LIMIT ?1"
        ).map_err(|e| e.to_string())?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(LogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                level: row.get(2)?,
                message: row.get(3)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut entries: Vec<LogEntry> = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        entries.reverse(); // Oldest first
        Ok(entries)
    }
}
