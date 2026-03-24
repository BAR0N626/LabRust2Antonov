//! CLI-програма для зчитування зображень із файлу, зміни їх розміру
//! та подальшого відвантаження або у локальну файлову систему,
//! або у S3-compatible сховище.
//!
//! # Підтримувані режими
//!
//! - `MYME_UPLOADER=fs` — збереження у директорію `MYME_FILES_PATH`.
//! - `MYME_UPLOADER=s3` — завантаження у bucket через S3 API.
//!
//! # Приклад запуску
//!
//! ```text
//! cargo run -- --files images.txt --resize 300x200
//! ```

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::result_large_err)]

use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;
use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use image::imageops::FilterType;
use image::ImageFormat;
use reqwest::blocking::Client;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use thiserror::Error;
use tokio::runtime::Runtime;

/// Псевдонім для результату застосунку.
type AppResult<T> = Result<T, Box<AppError>>;

/// Перелік помилок застосунку.
#[derive(Debug, Error)]
enum AppError {
    /// Неправильні аргументи командного рядка.
    #[error("неправильні аргументи командного рядка: {0}")]
    InvalidArguments(String),

    /// Неправильний формат параметра зміни розміру.
    #[error("неправильний формат resize: {0}")]
    InvalidResize(String),

    /// Відсутня обов'язкова змінна середовища.
    #[error("не задана змінна середовища {0}")]
    MissingEnv(&'static str),

    /// Помилка введення-виведення.
    #[error("помилка вводу/виводу: {0}")]
    Io(#[from] std::io::Error),

    /// Помилка HTTP-запиту.
    #[error("помилка HTTP-запиту: {0}")]
    Http(#[from] reqwest::Error),

    /// Помилка обробки зображення.
    #[error("помилка зображення: {0}")]
    Image(#[from] image::ImageError),

    /// Помилка перетворення числа.
    #[error("помилка парсингу числа: {0}")]
    ParseInt(#[from] std::num::ParseIntError),

    /// Помилка створення Tokio runtime.
    #[error("не вдалося створити Tokio runtime: {0}")]
    TokioRuntime(std::io::Error),

    /// Помилка конфігурації або роботи S3.
    #[error("помилка S3: {0}")]
    S3(String),
}

/// Спільний інтерфейс для відвантаження готових файлів.
trait Uploader {
    /// Відвантажує байти файлу за вказаною назвою.
    ///
    /// # Errors
    ///
    /// Повертає помилку, якщо файл не вдалося зберегти локально
    /// або завантажити у S3-compatible сховище.
    fn upload(&self, data: Vec<u8>, filename: &str) -> AppResult<()>;
}

/// Відвантажувач у локальну файлову систему.
struct FsUploader {
    /// Директорія для збереження файлів.
    output_dir: PathBuf,
}

impl Uploader for FsUploader {
    fn upload(&self, data: Vec<u8>, filename: &str) -> AppResult<()> {
        if !self.output_dir.exists() {
            fs::create_dir_all(&self.output_dir).map_err(|e| Box::new(AppError::from(e)))?;
        }

        let output_path = self.output_dir.join(filename);
        fs::write(&output_path, data).map_err(|e| Box::new(AppError::from(e)))?;

        println!("Збережено: {}", output_path.display());
        Ok(())
    }
}

/// Відвантажувач у S3-compatible сховище.
struct S3Uploader {
    /// Назва bucket.
    bucket: String,
    /// Клієнт AWS S3.
    client: aws_sdk_s3::Client,
    /// Tokio runtime для виконання async SDK у синхронній програмі.
    runtime: Runtime,
}

impl Uploader for S3Uploader {
    fn upload(&self, data: Vec<u8>, filename: &str) -> AppResult<()> {
        let bucket = self.bucket.clone();
        let key = filename.to_string();
        let client = self.client.clone();
        let body = ByteStream::from(data);

        self.runtime
            .block_on(async move {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&key)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| {
                        Box::new(AppError::S3(format!(
                            "помилка завантаження '{}': {e:?}",
                            key
                        )))
                    })?;
                Ok::<(), Box<AppError>>(())
            })?;

        println!("Завантажено в S3: {}", filename);
        Ok(())
    }
}

/// Точка входу програми.
fn main() {
    if let Err(error) = run() {
        eprintln!("Помилка: {}", error);
        std::process::exit(1);
    }
}

/// Запускає основну логіку застосунку.
///
/// # Errors
///
/// Повертає помилку, якщо:
/// - передані неправильні аргументи;
/// - відсутні необхідні змінні середовища;
/// - не вдалося прочитати вхідний файл;
/// - не вдалося обробити або відвантажити зображення.
fn run() -> AppResult<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 5 {
        return Err(Box::new(AppError::InvalidArguments(
            "використання: cargo run -- --files <path_to_file> --resize widthxheight".to_string(),
        )));
    }

    let mut files_path_arg: Option<String> = None;
    let mut resize_arg: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--files" => {
                if i + 1 >= args.len() {
                    return Err(Box::new(AppError::InvalidArguments(
                        "після --files потрібно вказати шлях до файлу".to_string(),
                    )));
                }
                files_path_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--resize" => {
                if i + 1 >= args.len() {
                    return Err(Box::new(AppError::InvalidArguments(
                        "після --resize потрібно вказати розмір widthxheight".to_string(),
                    )));
                }
                resize_arg = Some(args[i + 1].clone());
                i += 2;
            }
            other => {
                return Err(Box::new(AppError::InvalidArguments(format!(
                    "невідомий аргумент: {other}"
                ))));
            }
        }
    }

    let input_file = files_path_arg.ok_or_else(|| {
        Box::new(AppError::InvalidArguments(
            "аргумент --files не переданий".to_string(),
        ))
    })?;
    let resize_value = resize_arg.ok_or_else(|| {
        Box::new(AppError::InvalidArguments(
            "аргумент --resize не переданий".to_string(),
        ))
    })?;
    let (width, height) = parse_resize(&resize_value)?;

    let uploader = build_uploader()?;
    let content = fs::read_to_string(&input_file).map_err(|e| Box::new(AppError::from(e)))?;
let client = Client::builder()
    .timeout(Duration::from_secs(15))
    .build()
    .map_err(|e| Box::new(AppError::from(e)))?;

    let mut had_errors = false;

for (index, line) in content.lines().enumerate() {
    let source = line.trim();

    if source.is_empty() {
        continue;
    }

    match process_one(&client, source, width, height, index + 1) {
        Ok((bytes, filename)) => {
            if let Err(err) = uploader.upload(bytes, &filename) {
                eprintln!("Рядок {}: {} -> {}", index + 1, source, err);
                had_errors = true;
            }
        }
        Err(err) => {
            eprintln!("Рядок {}: {} -> {}", index + 1, source, err);
            had_errors = true;
        }
    }
}

if had_errors {
    return Err(Box::new(AppError::InvalidArguments(
        "одна або більше операцій завершились помилкою".to_string(),
    )));
}

Ok(())
}

/// Створює потрібний тип відвантажувача за значенням `MYME_UPLOADER`.
///
/// # Errors
///
/// Повертає помилку, якщо:
/// - не задана змінна `MYME_UPLOADER`;
/// - відсутні додаткові змінні для `fs` або `s3`;
/// - не вдалося ініціалізувати S3-клієнт.
fn build_uploader() -> AppResult<Box<dyn Uploader>> {
    let uploader_type =
        env::var("MYME_UPLOADER").map_err(|_| Box::new(AppError::MissingEnv("MYME_UPLOADER")))?;

    match uploader_type.as_str() {
        "fs" => {
            let output_dir = env::var("MYME_FILES_PATH")
                .map_err(|_| Box::new(AppError::MissingEnv("MYME_FILES_PATH")))?;

            Ok(Box::new(FsUploader {
                output_dir: PathBuf::from(output_dir),
            }))
        }
        "s3" => {
            let bucket =
                env::var("MYME_S3_BUCKET").map_err(|_| Box::new(AppError::MissingEnv("MYME_S3_BUCKET")))?;

            let endpoint = env::var("MYME_S3_ENDPOINT")
                .map_err(|_| Box::new(AppError::MissingEnv("MYME_S3_ENDPOINT")))?;

            let runtime = Runtime::new().map_err(|e| Box::new(AppError::TokioRuntime(e)))?;

            let shared_config = runtime.block_on(async {
                aws_config::defaults(BehaviorVersion::latest())
                    .endpoint_url(endpoint)
                    .load()
                    .await
            });

            let client = aws_sdk_s3::Client::new(&shared_config);

            Ok(Box::new(S3Uploader {
                bucket,
                client,
                runtime,
            }))
        }
        _ => Err(Box::new(AppError::InvalidArguments(
            "MYME_UPLOADER має бути fs або s3".to_string(),
        ))),
    }
}

/// Розбирає значення `widthxheight`.
///
/// # Errors
///
/// Повертає помилку, якщо формат не відповідає `widthxheight`
/// або хоча б одне зі значень не є додатним числом.
fn parse_resize(value: &str) -> AppResult<(u32, u32)> {
    let parts: Vec<&str> = value.split('x').collect();

    if parts.len() != 2 {
        return Err(Box::new(AppError::InvalidResize(
            "очікується формат widthxheight, наприклад 300x200".to_string(),
        )));
    }

    let width: u32 = parts[0].parse().map_err(|e| Box::new(AppError::from(e)))?;
    let height: u32 = parts[1].parse().map_err(|e| Box::new(AppError::from(e)))?;

    if width == 0 || height == 0 {
        return Err(Box::new(AppError::InvalidResize(
            "ширина і висота мають бути більше 0".to_string(),
        )));
    }

    Ok((width, height))
}

/// Завантажує, зчитує та змінює розмір одного зображення.
///
/// # Errors
///
/// Повертає помилку, якщо:
/// - локальний файл не вдалося прочитати;
/// - HTTP-запит завершився помилкою;
/// - зображення не вдалося декодувати або перекодувати.
fn process_one(
    client: &Client,
    source: &str,
    width: u32,
    height: u32,
    index: usize,
) -> AppResult<(Vec<u8>, String)> {
    let bytes = if is_url(source) {
        download_image_bytes(client, source)?
    } else {
        fs::read(source).map_err(|e| Box::new(AppError::from(e)))?
    };

    let img = image::load_from_memory(&bytes).map_err(|e| Box::new(AppError::from(e)))?;
    let resized = img.resize_exact(width, height, FilterType::Lanczos3);

    let mut buffer = Vec::new();
    resized
        .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
        .map_err(|e| Box::new(AppError::from(e)))?;

    let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_millis();

let filename = format!("resized_{index}_{now}.png");
    Ok((buffer, filename))
}

/// Перевіряє, чи є рядок HTTP або HTTPS URL.
fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// Завантажує зображення за URL.
///
/// # Errors
///
/// Повертає помилку, якщо HTTP-запит не виконався
/// або сервер повернув неуспішний статус.
fn download_image_bytes(client: &Client, url: &str) -> AppResult<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| Box::new(AppError::from(e)))?;

    let response = response
        .error_for_status()
        .map_err(|e| Box::new(AppError::from(e)))?;

    let bytes = response
        .bytes()
        .map_err(|e| Box::new(AppError::from(e)))?;

    Ok(bytes.to_vec())
}