#![deny(clippy::missing_errors_doc)]
#![deny(clippy::result_large_err)]

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use clap::{Parser, Subcommand};
use crossbeam_channel::{unbounded, Receiver, Sender};
use rand::RngCore;
use rayon::prelude::*;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use walkdir::WalkDir;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(name = "pr5_threads")]
#[command(version = "1.0")]
#[command(about = "Practical work 5")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Matrix {
        #[arg(long, default_value_t = 4096)]
        size: usize,
        #[arg(long, default_value_t = 4)]
        count: usize,
    },
    EncryptDir {
        #[arg(long)]
        dir: String,
    },
}

type Matrix = Vec<Vec<u32>>;

#[derive(Debug)]
struct MatrixJob {
    id: usize,
    matrix: Matrix,
}

#[derive(Debug)]
struct FileJob {
    path: PathBuf,
    data: Vec<u8>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Matrix { size, count } => run_matrix_task(size, count)?,
        Commands::EncryptDir { dir } => run_encrypt_dir_task(PathBuf::from(dir))?,
    }

    Ok(())
}

fn run_matrix_task(size: usize, count: usize) -> AppResult<()> {
    let (tx, rx) = unbounded::<MatrixJob>();

    let generator = thread::spawn(move || -> AppResult<()> {
        for id in 1..=count {
            let matrix = generate_matrix(size);
            tx.send(MatrixJob { id, matrix })?;
            println!("Generated matrix #{id}");
        }
        Ok(())
    });

    let mut workers = Vec::new();

    for worker_id in 1..=2 {
        let rx_clone = rx.clone();

        let handle = thread::spawn(move || -> AppResult<()> {
            while let Ok(job) = rx_clone.recv() {
                let sum = parallel_matrix_sum(&job.matrix);
                println!(
                    "Worker {worker_id} processed matrix #{} => sum = {}",
                    job.id, sum
                );
            }
            Ok(())
        });

        workers.push(handle);
    }

    generator.join().map_err(|_| "generator thread panicked")??;

    drop(rx);

    for handle in workers {
        handle.join().map_err(|_| "worker thread panicked")??;
    }

    Ok(())
}

fn generate_matrix(size: usize) -> Matrix {
    let mut matrix = Vec::with_capacity(size);

    for i in 0..size {
        let mut row = Vec::with_capacity(size);
        for j in 0..size {
            row.push(((i + j) % 100) as u32);
        }
        matrix.push(row);
    }

    matrix
}

fn parallel_matrix_sum(matrix: &Matrix) -> u64 {
    matrix
        .par_iter()
        .map(|row| row.par_iter().map(|&value| value as u64).sum::<u64>())
        .sum()
}

fn run_encrypt_dir_task(root_dir: PathBuf) -> AppResult<()> {
    if !root_dir.exists() {
        return Err(format!("Directory does not exist: {}", root_dir.display()).into());
    }

    let (tx, rx) = unbounded::<FileJob>();
    let processed_counter = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let key_bytes = Arc::new([7u8; 32]);

    let producer_dir = root_dir.clone();
    let producer = {
        let tx = tx.clone();
        thread::spawn(move || -> AppResult<()> {
            read_files_recursively(&producer_dir, &tx)?;
            Ok(())
        })
    };

    drop(tx);

    let mut consumers = Vec::new();

    for consumer_id in 1..=3 {
        let rx_clone = rx.clone();
        let counter_clone = Arc::clone(&processed_counter);
        let key_clone = Arc::clone(&key_bytes);

        let handle = thread::spawn(move || -> AppResult<()> {
            while let Ok(job) = rx_clone.recv() {
                process_file(job, &key_clone)?;
                let new_value = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
                println!("Consumer {consumer_id} processed file. Total = {new_value}");
            }
            Ok(())
        });

        consumers.push(handle);
    }

    let monitor_counter = Arc::clone(&processed_counter);
    let monitor_done = Arc::clone(&done);

    let monitor = thread::spawn(move || {
        let mut last_seen = usize::MAX;

        loop {
            let current = monitor_counter.load(Ordering::SeqCst);

            if current != last_seen {
                println!("Processed files counter = {current}");
                last_seen = current;
            }

            if monitor_done.load(Ordering::SeqCst) {
                break;
            }

            thread::sleep(Duration::from_millis(200));
        }
    });

    producer.join().map_err(|_| "producer thread panicked")??;

    for handle in consumers {
        handle.join().map_err(|_| "consumer thread panicked")??;
    }

    done.store(true, Ordering::SeqCst);
    monitor.join().map_err(|_| "monitor thread panicked")?;

    Ok(())
}

fn read_files_recursively(root: &Path, tx: &Sender<FileJob>) -> AppResult<()> {
    for entry in WalkDir::new(root) {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if is_output_data_file(path) {
                continue;
            }

            let data = fs::read(path)?;
            tx.send(FileJob {
                path: path.to_path_buf(),
                data,
            })?;
        }
    }

    Ok(())
}

fn is_output_data_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("data"))
        .unwrap_or(false)
}

fn process_file(job: FileJob, key_bytes: &[u8; 32]) -> AppResult<()> {
    let encrypted = encrypt_bytes(&job.data, key_bytes)?;
    let output_path = build_output_path(&job.path);
    fs::write(output_path, encrypted)?;
    Ok(())
}

fn build_output_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");

    path.with_file_name(format!("{file_name}.data"))
}

fn encrypt_bytes(data: &[u8], key_bytes: &[u8; 32]) -> AppResult<Vec<u8>> {
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|_| "encryption failed")?;

    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&ciphertext);
    Ok(result)
}