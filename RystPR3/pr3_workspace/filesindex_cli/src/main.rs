#![deny(clippy::missing_errors_doc)]
#![deny(clippy::result_large_err)]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use filesindex_core::{create_storage, StorageConfig};
use std::env;

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

fn parse_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn parse_storage_config() -> Result<StorageConfig> {
    let value = env::var("FILES_INDEX_PATH")
        .context("environment variable FILES_INDEX_PATH is not set")?;

    let (storage_type, path) = value
        .split_once(':')
        .context("FILES_INDEX_PATH must be in format type:path")?;

    match storage_type.to_lowercase().as_str() {
        "json" => Ok(StorageConfig::Json(path.to_string())),
        "sqlite" => Ok(StorageConfig::Sqlite(path.to_string())),
        _ => anyhow::bail!("supported storage types: json, sqlite"),
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let config = parse_storage_config()?;
    let mut storage = create_storage(config).context("failed to create storage backend")?;

    match cli.command {
        Commands::Add { path, tags } => {
            let parsed_tags = parse_tags(&tags);

            storage
                .add_file(path.clone(), parsed_tags)
                .with_context(|| format!("failed to add file '{path}'"))?;

            println!("File added successfully");
        }
        Commands::Get { tags } => {
            let parsed_tags = parse_tags(&tags);

            let files = storage
                .get_files(parsed_tags.clone())
                .with_context(|| format!("failed to search files by tags {:?}", parsed_tags))?;

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
    }

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}