use lab6_sync_primitives::{MyArc, MyMutex};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

fn bench_std_arc(threads: usize, iterations: usize) -> std::time::Duration {
    let start = Instant::now();
    let shared = Arc::new(0usize);
    let mut handles = Vec::new();

    for _ in 0..threads {
        let base = shared.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let cloned = base.clone();
                std::hint::black_box(cloned);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    start.elapsed()
}

fn bench_my_arc(threads: usize, iterations: usize) -> std::time::Duration {
    let start = Instant::now();
    let shared = MyArc::new(0usize);
    let mut handles = Vec::new();

    for _ in 0..threads {
        let base = shared.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let cloned = base.clone();
                std::hint::black_box(cloned);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    start.elapsed()
}

fn bench_std_mutex(threads: usize, iterations: usize) -> std::time::Duration {
    let start = Instant::now();
    let shared = Arc::new(Mutex::new(0usize));
    let mut handles = Vec::new();

    for _ in 0..threads {
        let counter = shared.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let mut guard = counter.lock().unwrap();
                *guard += 1;
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let value = *shared.lock().unwrap();
    std::hint::black_box(value);

    start.elapsed()
}

fn bench_my_mutex(threads: usize, iterations: usize) -> std::time::Duration {
    let start = Instant::now();
    let shared = MyArc::new(MyMutex::new(0usize));
    let mut handles = Vec::new();

    for _ in 0..threads {
        let counter = shared.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..iterations {
                let mut guard = counter.lock();
                *guard += 1;
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let value = *shared.lock();
    std::hint::black_box(value);

    start.elapsed()
}

fn pct_change(std_time: std::time::Duration, my_time: std::time::Duration) -> f64 {
    ((std_time.as_secs_f64() - my_time.as_secs_f64()) / std_time.as_secs_f64()) * 100.0
}

fn main() {
    let is_miri = cfg!(miri);

    let threads = if is_miri {
        2
    } else {
        thread::available_parallelism().map_or(4, usize::from)
    };

    let arc_iterations = if is_miri { 1_000 } else { 200_000 };
    let mutex_iterations = if is_miri { 1_000 } else { 50_000 };

    println!("Threads: {}", threads);
    println!("Arc iterations per thread: {}", arc_iterations);
    println!("Mutex iterations per thread: {}", mutex_iterations);
    println!();

    let std_arc_time = bench_std_arc(threads, arc_iterations);
    let my_arc_time = bench_my_arc(threads, arc_iterations);

    println!("=== Arc benchmark ===");
    println!("std::sync::Arc   : {:.3?}", std_arc_time);
    println!("MyArc            : {:.3?}", my_arc_time);

    let arc_pct = pct_change(std_arc_time, my_arc_time);
    if arc_pct >= 0.0 {
        println!("MyArc faster by  : {:.2}%", arc_pct);
    } else {
        println!("MyArc slower by  : {:.2}%", -arc_pct);
    }

    println!();

    let std_mutex_time = bench_std_mutex(threads, mutex_iterations);
    let my_mutex_time = bench_my_mutex(threads, mutex_iterations);

    println!("=== Mutex benchmark ===");
    println!("std::sync::Mutex : {:.3?}", std_mutex_time);
    println!("MyMutex          : {:.3?}", my_mutex_time);

    let mutex_pct = pct_change(std_mutex_time, my_mutex_time);
    if mutex_pct >= 0.0 {
        println!("MyMutex faster by: {:.2}%", mutex_pct);
    } else {
        println!("MyMutex slower by: {:.2}%", -mutex_pct);
    }
}