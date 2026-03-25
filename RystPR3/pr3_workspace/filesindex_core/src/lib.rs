#![deny(clippy::missing_errors_doc)]
#![deny(clippy::result_large_err)]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum StorageConfig {
    Json(String),
    Sqlite(String),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to read JSON file '{path}': {source}")]
    ReadJsonFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write JSON file '{path}': {source}")]
    WriteJsonFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON file '{path}': {source}")]
    ParseJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to serialize JSON data: {0}")]
    SerializeJson(#[source] serde_json::Error),

    #[error("failed to open SQLite database '{path}': {source}")]
    OpenSqlite {
        path: String,
        #[source]
        source: rusqlite::Error,
    },

    #[error("failed to initialize SQLite schema: {0}")]
    InitSqlite(#[source] rusqlite::Error),

    #[error("failed to insert file record into SQLite: {0}")]
    InsertFile(#[source] rusqlite::Error),

    #[error("failed to insert tag into SQLite: {0}")]
    InsertTag(#[source] rusqlite::Error),

    #[error("failed to query files from SQLite: {0}")]
    QueryFiles(#[source] rusqlite::Error),

    #[error("failed to query tags from SQLite: {0}")]
    QueryTags(#[source] rusqlite::Error),
}

pub trait Storage {
    /// Adds a file path with its tags into the storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend cannot save the record.
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), Box<StorageError>>;

    /// Returns indexed files that contain all requested tags.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage backend cannot load or query records.
    fn get_files(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, Box<StorageError>>;
}

pub struct JsonStorage {
    file_path: String,
}

impl JsonStorage {
    pub fn new(file_path: String) -> Self {
        Self { file_path }
    }

    fn load_records(&self) -> Result<Vec<FileRecord>, Box<StorageError>> {
        if !Path::new(&self.file_path).exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path).map_err(|source| {
            Box::new(StorageError::ReadJsonFile {
                path: self.file_path.clone(),
                source,
            })
        })?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&content).map_err(|source| {
            Box::new(StorageError::ParseJson {
                path: self.file_path.clone(),
                source,
            })
        })
    }

    fn save_records(&self, records: &[FileRecord]) -> Result<(), Box<StorageError>> {
        let json = serde_json::to_string_pretty(records)
            .map_err(|source| Box::new(StorageError::SerializeJson(source)))?;

        fs::write(&self.file_path, json).map_err(|source| {
            Box::new(StorageError::WriteJsonFile {
                path: self.file_path.clone(),
                source,
            })
        })
    }
}

impl Storage for JsonStorage {
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), Box<StorageError>> {
        let mut records = self.load_records()?;
        records.push(FileRecord { path, tags });
        self.save_records(&records)
    }

    fn get_files(&self, search_tags: Vec<String>) -> Result<Vec<FileRecord>, Box<StorageError>> {
        let records = self.load_records()?;
        let search_set: HashSet<String> = search_tags.into_iter().collect();

        let result = records
            .into_iter()
            .filter(|record| {
                let record_set: HashSet<String> = record.tags.iter().cloned().collect();
                search_set.is_subset(&record_set)
            })
            .collect();

        Ok(result)
    }
}

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Creates SQLite storage and initializes the schema.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or initialized.
    pub fn new(db_path: String) -> Result<Self, Box<StorageError>> {
        let conn = Connection::open(&db_path).map_err(|source| {
            Box::new(StorageError::OpenSqlite {
                path: db_path.clone(),
                source,
            })
        })?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| Box::new(StorageError::InitSqlite(e)))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id)
            )",
            [],
        )
        .map_err(|e| Box::new(StorageError::InitSqlite(e)))?;

        Ok(Self { conn })
    }
}

impl Storage for SqliteStorage {
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), Box<StorageError>> {
        self.conn
            .execute("INSERT INTO files (path) VALUES (?1)", params![path])
            .map_err(|e| Box::new(StorageError::InsertFile(e)))?;

        let file_id = self.conn.last_insert_rowid();

        for tag in tags {
            self.conn
                .execute(
                    "INSERT INTO tags (file_id, tag) VALUES (?1, ?2)",
                    params![file_id, tag],
                )
                .map_err(|e| Box::new(StorageError::InsertTag(e)))?;
        }

        Ok(())
    }

    fn get_files(&self, search_tags: Vec<String>) -> Result<Vec<FileRecord>, Box<StorageError>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM files")
            .map_err(|e| Box::new(StorageError::QueryFiles(e)))?;

        let file_rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let path: String = row.get(1)?;
                Ok((id, path))
            })
            .map_err(|e| Box::new(StorageError::QueryFiles(e)))?;

        let mut result = Vec::new();
        let search_set: HashSet<String> = search_tags.into_iter().collect();

        for row in file_rows {
            let (file_id, path) = row.map_err(|e| Box::new(StorageError::QueryFiles(e)))?;

            let mut tag_stmt = self
                .conn
                .prepare("SELECT tag FROM tags WHERE file_id = ?1")
                .map_err(|e| Box::new(StorageError::QueryTags(e)))?;

            let tag_rows = tag_stmt
                .query_map(params![file_id], |row| {
                    let tag: String = row.get(0)?;
                    Ok(tag)
                })
                .map_err(|e| Box::new(StorageError::QueryTags(e)))?;

            let mut tags = Vec::new();
            for tag_row in tag_rows {
                tags.push(tag_row.map_err(|e| Box::new(StorageError::QueryTags(e)))?);
            }

            let record_set: HashSet<String> = tags.iter().cloned().collect();

            if search_set.is_subset(&record_set) {
                result.push(FileRecord { path, tags });
            }
        }

        Ok(result)
    }
}

/// Creates a storage backend from the provided configuration.
///
/// # Errors
///
/// Returns an error if the selected backend cannot be initialized.
pub fn create_storage(config: StorageConfig) -> Result<Box<dyn Storage>, Box<StorageError>> {
    match config {
        StorageConfig::Json(path) => Ok(Box::new(JsonStorage::new(path))),
        StorageConfig::Sqlite(path) => Ok(Box::new(SqliteStorage::new(path)?)),
    }
}