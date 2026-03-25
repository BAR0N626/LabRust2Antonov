//! Власні примітиви синхронізації для лабораторної роботи 6.
//!
//! Реалізовано:
//! - `MyArc<T>`
//! - `MyMutex<T>`
//!
//! Ці реалізації призначені для навчальних цілей.

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::result_large_err)]

use std::cell::UnsafeCell;
use std::hint::spin_loop;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering};

/// Внутрішній стан `MyArc`.
struct MyArcInner<T> {
    /// Лічильник посилань.
    ref_count: AtomicUsize,
    /// Значення.
    value: T,
}

/// Власний потокобезпечний reference-counted pointer.
pub struct MyArc<T> {
    /// Вказівник на виділену область.
    ptr: NonNull<MyArcInner<T>>,
    /// Маркер типу.
    _marker: PhantomData<MyArcInner<T>>,
}

impl<T> MyArc<T> {
    /// Створює новий `MyArc`.
    #[must_use]
    pub fn new(value: T) -> Self {
        let boxed = Box::new(MyArcInner {
            ref_count: AtomicUsize::new(1),
            value,
        });

        let ptr = NonNull::from(Box::leak(boxed));

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Повертає поточну кількість сильних посилань.
    #[must_use]
    pub fn strong_count(this: &Self) -> usize {
        this.inner().ref_count.load(Ordering::Acquire)
    }

    /// Повертає внутрішню структуру.
    fn inner(&self) -> &MyArcInner<T> {
        // SAFETY: `ptr` створюється з Box::leak і залишається валідним,
        // поки хоча б один `MyArc` існує.
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> Clone for MyArc<T> {
    fn clone(&self) -> Self {
        self.inner().ref_count.fetch_add(1, Ordering::Relaxed);

        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for MyArc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner().value
    }
}

impl<T> Drop for MyArc<T> {
    fn drop(&mut self) {
        if self.inner().ref_count.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);

            // SAFETY: ми останній власник, тому можемо звільнити пам'ять.
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}

// SAFETY: MyArc є Send, якщо T можна безпечно передавати та ділити між потоками.
unsafe impl<T: Send + Sync> Send for MyArc<T> {}
// SAFETY: MyArc є Sync, якщо T можна безпечно передавати та ділити між потоками.
unsafe impl<T: Send + Sync> Sync for MyArc<T> {}

/// Власний spin-based mutex.
pub struct MyMutex<T> {
    /// Прапорець захоплення lock.
    locked: AtomicBool,
    /// Захищене значення.
    value: UnsafeCell<T>,
}

impl<T> MyMutex<T> {
    /// Створює новий mutex.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Захоплює mutex і повертає guard.
    #[must_use]
    pub fn lock(&self) -> MyMutexGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }

            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }

        MyMutexGuard { mutex: self }
    }
}

// SAFETY: MyMutex можна передавати між потоками, якщо T: Send.
unsafe impl<T: Send> Send for MyMutex<T> {}
// SAFETY: MyMutex можна ділити між потоками, якщо T: Send.
unsafe impl<T: Send> Sync for MyMutex<T> {}

/// Guard для `MyMutex`.
pub struct MyMutexGuard<'a, T> {
    /// Посилання на mutex.
    mutex: &'a MyMutex<T>,
}

impl<T> Deref for MyMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: guard існує лише коли lock утримується.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MyMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: guard є унікальним доступом під lock.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MyMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::{MyArc, MyMutex};
    use std::thread;

    #[test]
    fn my_arc_counts_references() {
        let a = MyArc::new(10);
        let b = a.clone();

        assert_eq!(MyArc::strong_count(&a), 2);
        assert_eq!(*a, 10);
        assert_eq!(*b, 10);
    }

    #[test]
    fn my_mutex_protects_shared_value() {
        let value = MyArc::new(MyMutex::new(0usize));
        let mut handles = Vec::new();

        for _ in 0..4 {
            let shared = value.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    let mut guard = shared.lock();
                    *guard += 1;
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let guard = value.lock();
        assert_eq!(*guard, 4_000);
    }
}