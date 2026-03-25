#![deny(clippy::missing_errors_doc)]
#![deny(clippy::result_large_err)]

use clap::Parser;
use reqwest::Client;
use sanitize_filename::sanitize;
use std::error::Error;
use std::io::{self, BufRead};
use std::path::PathBuf;
use tokio::fs;
use tokio::sync::Semaphore;
use url::Url;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser, Debug)]
#[command(name = "web-downloader")]
#[command(version = "1.0")]
#[command(about = "Asynchronously downloads web pages and saves them to files")]
struct Cli {
    #[arg(long)]
    max_threads: Option<usize>,

    file: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cli = Cli::parse();

    let worker_count = cli.max_threads.unwrap_or_else(num_cpus::get);

    let urls = match cli.file {
        Some(file_path) => read_urls_from_file(&file_path).await?,
        None => read_urls_from_stdin()?,
    };

    if urls.is_empty() {
        println!("No URLs provided");
        return Ok(());
    }

   let client = Client::builder()
    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
    .build()?;
    let semaphore = std::sync::Arc::new(Semaphore::new(worker_count));
    let mut handles = Vec::new();

    for (index, url) in urls.into_iter().enumerate() {
        let client = client.clone();
        let semaphore = semaphore.clone();

        let handle = tokio::spawn(async move {
            let permit = semaphore.acquire_owned().await?;
            let result = download_and_save(&client, &url, index + 1).await;
            drop(permit);
            result
        });

        handles.push(handle);
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("Download error: {error}"),
            Err(error) => eprintln!("Task join error: {error}"),
        }
    }

    Ok(())
}

/// Reads URLs from a text file asynchronously.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
async fn read_urls_from_file(path: &str) -> AppResult<Vec<String>> {
    let content = fs::read_to_string(path).await?;
    Ok(parse_urls(&content))
}

/// Reads URLs from standard input.
///
/// # Errors
///
/// Returns an error if stdin cannot be read.
fn read_urls_from_stdin() -> AppResult<Vec<String>> {
    let stdin = io::stdin();
    let lines: Result<Vec<String>, io::Error> = stdin.lock().lines().collect();
    Ok(parse_urls(&lines?.join("\n")))
}

fn parse_urls(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Downloads one web page and saves it into a local file.
///
/// # Errors
///
/// Returns an error if the URL is invalid, the request fails,
/// or the output file cannot be written.
async fn download_and_save(client: &Client, url: &str, index: usize) -> AppResult<()> {
    let parsed_url = Url::parse(url)?;
    let response = client.get(url).send().await?;
    let response = response.error_for_status()?;
    let body = response.text().await?;

    let output_path = build_output_path(&parsed_url, index);
    fs::write(&output_path, body).await?;

    println!("Saved {url} -> {}", output_path.display());
    Ok(())
}

fn build_output_path(url: &Url, index: usize) -> PathBuf {
    let host = url.host_str().unwrap_or("unknown-host");
    let path = url.path().trim_matches('/');

    let file_stem = if path.is_empty() {
        format!("{}_index", sanitize(host))
    } else {
        format!("{}_{}", sanitize(host), sanitize(path.replace('/', "_")))
    };

    PathBuf::from(format!("{index:03}_{file_stem}.html"))
}