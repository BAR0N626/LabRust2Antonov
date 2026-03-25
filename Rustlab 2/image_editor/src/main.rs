use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use image::imageops::FilterType;
use image::ImageFormat;
use reqwest::blocking::Client;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tokio::runtime::Runtime;

trait Uploader {
    fn upload(&self, data: Vec<u8>, filename: &str) -> Result<(), String>;
}

struct FsUploader {
    output_dir: PathBuf,
}

impl Uploader for FsUploader {
    fn upload(&self, data: Vec<u8>, filename: &str) -> Result<(), String> {
        if !self.output_dir.exists() {
            fs::create_dir_all(&self.output_dir)
                .map_err(|e| format!("Не вдалося створити директорію: {}", e))?;
        }

        let output_path = self.output_dir.join(filename);

        fs::write(&output_path, data)
            .map_err(|e| format!("Не вдалося записати файл: {}", e))?;

        println!("Збережено: {}", output_path.display());
        Ok(())
    }
}

struct S3Uploader {
    bucket: String,
    client: aws_sdk_s3::Client,
    runtime: Runtime,
}

impl Uploader for S3Uploader {
    fn upload(&self, data: Vec<u8>, filename: &str) -> Result<(), String> {
        let bucket = self.bucket.clone();
        let client = self.client.clone();
        let body = ByteStream::from(data);

        self.runtime
            .block_on(async move {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(filename)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| format!("Помилка завантаження в S3: {}", e))?;
                Ok::<(), String>(())
            })?;

        println!("Завантажено в S3: {}", filename);
        Ok(())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Помилка: {}", error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 5 {
        return Err(
            "Використання: cargo run -- --files <path_to_file> --resize widthxheight".to_string(),
        );
    }

    let mut files_path_arg: Option<String> = None;
    let mut resize_arg: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--files" => {
                if i + 1 >= args.len() {
                    return Err("Після --files потрібно вказати шлях до файлу".to_string());
                }
                files_path_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--resize" => {
                if i + 1 >= args.len() {
                    return Err("Після --resize потрібно вказати розмір widthxheight".to_string());
                }
                resize_arg = Some(args[i + 1].clone());
                i += 2;
            }
            other => {
                return Err(format!("Невідомий аргумент: {}", other));
            }
        }
    }

    let input_file = files_path_arg.ok_or("Аргумент --files не переданий")?;
    let resize_value = resize_arg.ok_or("Аргумент --resize не переданий")?;
    let (width, height) = parse_resize(&resize_value)?;

    let uploader = build_uploader()?;

    let content = fs::read_to_string(&input_file)
        .map_err(|e| format!("Не вдалося прочитати файл {}: {}", input_file, e))?;

    let client = Client::new();

    for (index, line) in content.lines().enumerate() {
        let source = line.trim();

        if source.is_empty() {
            continue;
        }

        match process_one(&client, source, width, height, index + 1) {
            Ok((bytes, filename)) => {
                if let Err(err) = uploader.upload(bytes, &filename) {
                    eprintln!("Рядок {}: {} -> {}", index + 1, source, err);
                }
            }
            Err(err) => {
                eprintln!("Рядок {}: {} -> {}", index + 1, source, err);
            }
        }
    }

    Ok(())
}

fn build_uploader() -> Result<Box<dyn Uploader>, String> {
    let uploader_type =
        env::var("MYME_UPLOADER").map_err(|_| "Не задана змінна MYME_UPLOADER".to_string())?;

    match uploader_type.as_str() {
        "fs" => {
            let output_dir = env::var("MYME_FILES_PATH")
                .map_err(|_| "Не задана змінна MYME_FILES_PATH".to_string())?;

            Ok(Box::new(FsUploader {
                output_dir: PathBuf::from(output_dir),
            }))
        }
        "s3" => {
            let bucket = env::var("MYME_S3_BUCKET")
                .map_err(|_| "Не задана змінна MYME_S3_BUCKET".to_string())?;

            let endpoint = env::var("MYME_S3_ENDPOINT").ok();

            let runtime =
                Runtime::new().map_err(|e| format!("Не вдалося створити Tokio runtime: {}", e))?;

            let shared_config = runtime.block_on(async {
                let loader = aws_config::defaults(BehaviorVersion::latest());
                let loader = if let Some(endpoint_url) = endpoint {
                    loader.endpoint_url(endpoint_url)
                } else {
                    loader
                };
                loader.load().await
            });

            let client = aws_sdk_s3::Client::new(&shared_config);

            Ok(Box::new(S3Uploader {
                bucket,
                client,
                runtime,
            }))
        }
        _ => Err("MYME_UPLOADER має бути fs або s3".to_string()),
    }
}

fn parse_resize(value: &str) -> Result<(u32, u32), String> {
    let parts: Vec<&str> = value.split('x').collect();

    if parts.len() != 2 {
        return Err("Розмір має бути у форматі widthxheight, наприклад 300x200".to_string());
    }

    let width = parts[0]
        .parse::<u32>()
        .map_err(|_| "Ширина має бути числом".to_string())?;

    let height = parts[1]
        .parse::<u32>()
        .map_err(|_| "Висота має бути числом".to_string())?;

    if width == 0 || height == 0 {
        return Err("Ширина і висота мають бути більше 0".to_string());
    }

    Ok((width, height))
}

fn process_one(
    client: &Client,
    source: &str,
    width: u32,
    height: u32,
    index: usize,
) -> Result<(Vec<u8>, String), String> {
    let bytes = if is_url(source) {
        download_image_bytes(client, source)?
    } else {
        fs::read(source).map_err(|e| format!("Не вдалося прочитати локальний файл: {}", e))?
    };

    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("Не вдалося відкрити зображення: {}", e))?;

    let resized = img.resize_exact(width, height, FilterType::Lanczos3);

    let mut buffer = Vec::new();
    resized
        .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
        .map_err(|e| format!("Не вдалося підготувати файл до збереження: {}", e))?;

    let filename = format!("resized_{}.png", index);

    Ok((buffer, filename))
}

fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn download_image_bytes(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Помилка HTTP-запиту: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Сервер повернув статус {}", response.status()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("Не вдалося прочитати відповідь: {}", e))?;

    Ok(bytes.to_vec())
}