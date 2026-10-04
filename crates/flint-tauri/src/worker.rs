use std::future::Future;
use std::pin::Pin;
use std::thread;

use tokio::sync::{mpsc, oneshot};

/// Runs a value that must stay on one thread (the app service holds SQLite connections across `await`
/// points, so its futures are not `Send`) and lets any thread call into it. Calls run one at a time, in
/// the order they arrive.
pub struct Worker<T: 'static> {
    jobs: mpsc::UnboundedSender<Job<T>>,
}

type Job<T> = Box<dyn for<'a> FnOnce(&'a mut T) -> Pin<Box<dyn Future<Output = ()> + 'a>> + Send>;

impl<T: 'static> Worker<T> {
    /// Starts the thread. `make` runs on it, so the value itself never has to be `Send`.
    pub fn start(make: impl FnOnce() -> T + Send + 'static) -> Self {
        let (jobs, mut inbox) = mpsc::unbounded_channel::<Job<T>>();
        thread::Builder::new()
            .name("flint-service".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a tokio runtime");
                let mut value = make();
                runtime.block_on(async move {
                    while let Some(job) = inbox.recv().await {
                        job(&mut value).await;
                    }
                });
            })
            .expect("a thread");
        Self { jobs }
    }

    /// Runs `job` on the worker's thread and returns what it returns.
    pub async fn call<R, F>(&self, job: F) -> R
    where
        R: Send + 'static,
        F: for<'a> FnOnce(&'a mut T) -> Pin<Box<dyn Future<Output = R> + 'a>> + Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let wrapped: Job<T> = Box::new(move |value| {
            Box::pin(async move {
                let _ = reply.send(job(value).await);
            })
        });
        self.jobs
            .send(wrapped)
            .unwrap_or_else(|_| panic!("the service thread is gone"));
        answer.await.expect("the service thread dropped a call")
    }
}

/// Runs `job` on its own thread and waits at most `timeout` for its answer. For calls into code that may
/// never answer (the Android fingerprint prompt): the thread is left behind if it does not, but the caller,
/// and everything queued behind it on the worker, is not stuck with it.
#[cfg(any(target_os = "android", test))]
pub fn answer_within<T: Send + 'static>(
    timeout: std::time::Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (reply, answer) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = reply.send(job());
    });
    answer.recv_timeout(timeout).ok()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn a_call_that_never_answers_gives_up_instead_of_blocking() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let stuck = answer_within(Duration::from_millis(100), || {
            thread::park(); // never answers
            1
        });
        assert_eq!(stuck, None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(answer_within(Duration::from_secs(5), || 7), Some(7));
    }

    #[test]
    fn runs_calls_in_order_on_one_thread_with_a_value_that_is_not_send() {
        let worker = Worker::start(|| Rc::new(std::cell::Cell::new(0))); // Rc is not Send
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (first, second) = runtime.block_on(async {
            let a = worker
                .call(|n| {
                    Box::pin(async move {
                        n.set(n.get() + 1);
                        tokio::task::yield_now().await; // a real await point inside the job
                        (n.get(), thread::current().name().map(str::to_owned))
                    })
                })
                .await;
            let b = worker
                .call(|n| {
                    Box::pin(async move { (n.get(), thread::current().name().map(str::to_owned)) })
                })
                .await;
            (a, b)
        });
        assert_eq!(first.0, 1);
        assert_eq!(second.0, 1, "state is kept between calls");
        assert_eq!(first.1.as_deref(), Some("flint-service"));
        assert_eq!(second.1.as_deref(), Some("flint-service"));
    }
}
