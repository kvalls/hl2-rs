//! Cooperative owned work: cancellation never blocks the render thread on a join.
use anyhow::{Result, bail};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled() {
            bail!("owned resource loading cancelled");
        }
        Ok(())
    }
}

pub(crate) struct Work<T> {
    result: Mutex<Receiver<Result<T, String>>>,
    pub(crate) cancel: Cancellation,
}
impl<T: Send + 'static> Work<T> {
    pub(crate) fn spawn(
        name: &str,
        work: impl FnOnce(Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        let token = cancel.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    token.check()?;
                    let value = work(token.clone())?;
                    token.check()?;
                    Ok(value)
                }))
                .map_err(|_| "owned resource worker panicked".to_owned())
                .and_then(|r: Result<T>| r.map_err(|e| format!("{e:#}")));
                // The receiver can disappear after replacement. Dispose on this worker.
                let _ = sender.send(result);
            })
            .map_err(|e| format!("Cannot start owned resource worker: {e}"))?;
        Ok(Self {
            result: Mutex::new(receiver),
            cancel,
        })
    }
    pub(crate) fn poll(&self) -> Option<Result<T, String>> {
        match self.result.lock().expect("owned work result").try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("owned resource worker disconnected".into()))
            }
        }
    }
    pub(crate) fn wait(self) -> Result<T> {
        let result = self
            .result
            .lock()
            .expect("owned work result")
            .recv()
            .map_err(|_| anyhow::anyhow!("owned resource worker disconnected"))?;
        result.map_err(anyhow::Error::msg)
    }
}
impl<T> Drop for Work<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
impl<T> Work<T> {
    pub(crate) fn ready(value: T) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(sender.send(Ok(value)).is_ok(), "ready work receiver");
        Self {
            result: Mutex::new(receiver),
            cancel: Cancellation::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cancellation, Work};
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn cancelled_token_refuses_work_and_drop_signals_running_work() {
        let token = Cancellation::default();
        token.cancel();
        assert!(token.check().is_err());
        let (started_tx, started_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let work = Work::spawn("cancellation-test", move |cancel| {
            started_tx.send(()).unwrap();
            while cancel.check().is_ok() {
                std::thread::yield_now();
            }
            exit_tx.send(()).unwrap();
            cancel.check()?;
            Ok(())
        })
        .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(work);
        exit_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn cancellation_during_work_rejects_an_otherwise_successful_result() {
        let (start_tx, start_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let work = Work::spawn("cancel-result-test", move |_| {
            start_tx.send(()).unwrap();
            finish_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(42)
        })
        .unwrap();
        start_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        work.cancel.cancel();
        finish_tx.send(()).unwrap();
        assert!(work.wait().unwrap_err().to_string().contains("cancelled"));
    }

    #[test]
    fn worker_panics_are_reported_instead_of_leaving_loading_stuck() {
        let result = Work::<()>::spawn("panic-result-test", |_| panic!("test panic"))
            .unwrap()
            .wait();
        assert!(result.unwrap_err().to_string().contains("panicked"));
    }
    #[test]
    fn disconnected_worker_reports_failure_instead_of_waiting_forever() {
        let (sender, receiver) = mpsc::channel::<Result<(), String>>();
        drop(sender);
        let work = Work {
            result: std::sync::Mutex::new(receiver),
            cancel: Cancellation::default(),
        };
        assert!(work.poll().unwrap().unwrap_err().contains("disconnected"));
    }
}
