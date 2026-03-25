use clap::{Parser, Subcommand};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "filesindex")]
#[command(version = "1.0")]
#[command(about = "CLI utility for indexing and searching files by tags")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add {
        #[arg(long)]
        path: String,
        #[arg(long)]
        tags: String,
    },
    Get {
        #[arg(long)]
        tags: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileRecord {
    path: String,
    tags: Vec<String>,
}

trait Storage {
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), String>;
    fn get_files(&self, tags: Vec<String>) -> Result<Vec<FileRecord>, String>;
}

struct JsonStorage {
    file_path: String,
}

impl JsonStorage {
    fn new(file_path: String) -> Self {
        Self { file_path }
    }

    fn load_records(&self) -> Result<Vec<FileRecord>, String> {
        if !Path::new(&self.file_path).exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.file_path).map_err(|e| e.to_string())?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    fn save_records(&self, records: &[FileRecord]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
        fs::write(&self.file_path, json).map_err(|e| e.to_string())
    }
}

impl Storage for JsonStorage {
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), String> {
        let mut records = self.load_records()?;

        records.push(FileRecord { path, tags });

        self.save_records(&records)
    }

    fn get_files(&self, search_tags: Vec<String>) -> Result<Vec<FileRecord>, String> {
        let records = self.load_records()?;
        let search_set: HashSet<String> = search_tags.into_iter().collect();

        let result: Vec<FileRecord> = records
            .into_iter()
            .filter(|record| {
                let record_set: HashSet<String> = record.tags.iter().cloned().collect();
                search_set.is_subset(&record_set)
            })
            .collect();

        Ok(result)
    }
}

struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    fn new(db_path: String) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL
            )",
            [],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                tag TEXT NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id)
            )",
            [],
        )
        .map_err(|e| e.to_string())?;

        Ok(Self { conn })
    }
}

impl Storage for SqliteStorage {
    fn add_file(&mut self, path: String, tags: Vec<String>) -> Result<(), String> {
        self.conn
            .execute("INSERT INTO files (path) VALUES (?1)", params![path])
            .map_err(|e| e.to_string())?;

        let file_id = self.conn.last_insert_rowid();

        for tag in tags {
            self.conn
                .execute(
                    "INSERT INTO tags (file_id, tag) VALUES (?1, ?2)",
                    params![file_id, tag],
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn get_files(&self, search_tags: Vec<String>) -> Result<Vec<FileRecord>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM files")
            .map_err(|e| e.to_string())?;

        let file_rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let path: String = row.get(1)?;
                Ok((id, path))
            })
            .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        let search_set: HashSet<String> = search_tags.into_iter().collect();

        for row in file_rows {
            let (file_id, path) = row.map_err(|e| e.to_string())?;

            let mut tag_stmt = self
                .conn
                .prepare("SELECT tag FROM tags WHERE file_id = ?1")
                .map_err(|e| e.to_string())?;

            let tag_rows = tag_stmt
                .query_map(params![file_id], |row| {
                    let tag: String = row.get(0)?;
                    Ok(tag)
                })
                .map_err(|e| e.to_string())?;

            let mut tags = Vec::new();
            for tag_row in tag_rows {
                tags.push(tag_row.map_err(|e| e.to_string())?);
            }

            let record_set: HashSet<String> = tags.iter().cloned().collect();

            if search_set.is_subset(&record_set) {
                result.push(FileRecord { path, tags });
            }
        }

        Ok(result)
    }
}

enum StorageType {
    Json(String),
    Sqlite(String),
}

fn parse_storage_config() -> Result<StorageType, String> {
    let value = env::var("FILES_INDEX_PATH")
        .map_err(|_| "Environment variable FILES_INDEX_PATH is not set".to_string())?;

    let (storage_type, path) = value
        .split_once(':')
        .ok_or("FILES_INDEX_PATH must be in format type:path".to_string())?;

    match storage_type.to_lowercase().as_str() {
        "json" => Ok(StorageType::Json(path.to_string())),
        "sqlite" => Ok(StorageType::Sqlite(path.to_string())),
        _ => Err("Supported storage types: json, sqlite".to_string()),
    }
}

fn create_storage() -> Result<Box<dyn Storage>, String> {
    match parse_storage_config()? {
        StorageType::Json(path) => Ok(Box::new(JsonStorage::new(path))),
        StorageType::Sqlite(path) => Ok(Box::new(SqliteStorage::new(path)?)),
    }
}

fn parse_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn main() {
    let cli = Cli::parse();

    let mut storage = match create_storage() {
        Ok(storage) => storage,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    match cli.command {
        Commands::Add { path, tags } => {
            let parsed_tags = parse_tags(&tags);

            match storage.add_file(path, parsed_tags) {
                Ok(_) => println!("File added successfully"),
                Err(e) => eprintln!("Error: {}", e),
            }
        }
        Commands::Get { tags } => {
            let parsed_tags = parse_tags(&tags);

            match storage.get_files(parsed_tags) {
                Ok(files) => {
                    if files.is_empty() {
                        println!("No files found");
                    } else {
                        for file in files {
                            println!("Path: {}", file.path);
                            println!("Tags: {}", file.tags.join(", "));
                            println!();
                        }
                    }
                }
                Err(e) => eprintln!("Error: {}", e),
            }
        }
    }
}