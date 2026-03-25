#![deny(clippy::missing_errors_doc)]
#![deny(clippy::result_large_err)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread;
use std::time::{Duration, Instant};

struct MeasurableFuture<Fut> {
    inner_future: Fut,
    started_at: Option<Instant>,
}

impl<Fut> MeasurableFuture<Fut> {
    fn new(inner_future: Fut) -> Self {
        Self {
            inner_future,
            started_at: None,
        }
    }
}

impl<Fut> Future for MeasurableFuture<Fut>
where
    Fut: Future,
{
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        if this.started_at.is_none() {
            this.started_at = Some(Instant::now());
        }

        let inner = unsafe { Pin::new_unchecked(&mut this.inner_future) };

        match inner.poll(cx) {
            Poll::Ready(output) => {
                if let Some(started_at) = this.started_at {
                    println!("Execution time: {:?}", started_at.elapsed());
                }
                Poll::Ready(output)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

struct SleepFuture {
    shared_state: Arc<Mutex<SharedState>>,
    duration: Duration,
    started: bool,
}

struct SharedState {
    completed: bool,
    waker: Option<Waker>,
}

impl SleepFuture {
    fn new(milliseconds: u64) -> Self {
        Self {
            shared_state: Arc::new(Mutex::new(SharedState {
                completed: false,
                waker: None,
            })),
            duration: Duration::from_millis(milliseconds),
            started: false,
        }
    }
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        {
            let mut shared = this.shared_state.lock().expect("mutex poisoned");

            if shared.completed {
                return Poll::Ready(());
            }

            shared.waker = Some(cx.waker().clone());
        }

        if !this.started {
            this.started = true;

            let shared_state = Arc::clone(&this.shared_state);
            let duration = this.duration;

            thread::spawn(move || {
                thread::sleep(duration);

                let mut shared = shared_state.lock().expect("mutex poisoned");
                shared.completed = true;

                if let Some(waker) = shared.waker.take() {
                    waker.wake();
                }
            });
        }

        Poll::Pending
    }
}

fn main() {
    let future = MeasurableFuture::new(async {
        println!("Task started");
        SleepFuture::new(2000).await;
        println!("Task finished");
        42
    });

    let result = block_on(future);
    println!("Result: {result}");
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let waker = create_waker();
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::sleep(Duration::from_millis(1)),
        }
    }
}

fn create_waker() -> Waker {
    unsafe { Waker::from_raw(raw_waker()) }
}

fn raw_waker() -> RawWaker {
    RawWaker::new(std::ptr::null(), &VTABLE)
}

unsafe fn clone_raw_waker(_: *const ()) -> RawWaker {
    raw_waker()
}

unsafe fn wake_raw_waker(_: *const ()) {}

unsafe fn wake_by_ref_raw_waker(_: *const ()) {}

unsafe fn drop_raw_waker(_: *const ()) {}

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_raw_waker,
    wake_raw_waker,
    wake_by_ref_raw_waker,
    drop_raw_waker,
);