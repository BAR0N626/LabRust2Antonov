use image::imageops::FilterType;
use reqwest::blocking::Client;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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

    let output_dir =
        env::var("MYME_FILES_PATH").map_err(|_| "Не задана змінна середовища MYME_FILES_PATH".to_string())?;

    let output_dir_path = Path::new(&output_dir);

    if !output_dir_path.exists() {
        fs::create_dir_all(output_dir_path)
            .map_err(|e| format!("Не вдалося створити директорію: {}", e))?;
    }

    let content = fs::read_to_string(&input_file)
        .map_err(|e| format!("Не вдалося прочитати файл {}: {}", input_file, e))?;

    let client = Client::new();

    for (index, line) in content.lines().enumerate() {
        let source = line.trim();

        if source.is_empty() {
            continue;
        }

        match process_one(&client, source, width, height, output_dir_path, index + 1) {
            Ok(path) => println!("Збережено: {}", path.display()),
            Err(err) => eprintln!("Рядок {}: {} -> {}", index + 1, source, err),
        }
    }

    Ok(())
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
    output_dir: &Path,
    index: usize,
) -> Result<PathBuf, String> {
    let bytes = if is_url(source) {
        download_image_bytes(client, source)?
    } else {
        fs::read(source).map_err(|e| format!("Не вдалося прочитати локальний файл: {}", e))?
    };

    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("Не вдалося відкрити зображення: {}", e))?;

    let resized = img.resize_exact(width, height, FilterType::Lanczos3);

    let output_path = build_output_path(source, output_dir, index);

    resized
        .save(&output_path)
        .map_err(|e| format!("Не вдалося зберегти файл: {}", e))?;

    Ok(output_path)
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

fn build_output_path(source: &str, output_dir: &Path, index: usize) -> PathBuf {
    let extension = detect_extension(source);
    let filename = format!("resized_{}.{}", index, extension);
    output_dir.join(filename)
}

fn detect_extension(source: &str) -> String {
    let clean_source = source.split('?').next().unwrap_or(source);

    if let Some(ext) = Path::new(clean_source).extension().and_then(|e| e.to_str()) {
        let ext = ext.to_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tiff" | "webp" => return ext,
            _ => {}
        }
    }

    "png".to_string()
}