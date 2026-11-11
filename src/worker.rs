// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! Runs every reader call on a thread of its own, so a slow card or a reader that stops
//! answering never freezes the window.
//!
//! The thread owns the smart card service and the open card connection (`Devices`). The window
//! hands it a job (a closure over `Devices`) together with what to do with the answer; answers
//! come back in the order the jobs were sent, and the window applies them on its own thread.

use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
#[cfg(test)]
use std::{sync::mpsc::RecvTimeoutError, time::Duration};

use crate::reader::{CardLink, CardSystem, Error, SystemOpener};

/// What the worker thread owns.
pub struct Devices {
    open_system: SystemOpener,
    pub system: Option<Box<dyn CardSystem>>,
    pub card: Option<Box<dyn CardLink>>,
    /// The CCID escape code of the open connection, once looked up (`card::escape_code`).
    pub escape: Option<crate::card::EscapeCode>,
}

impl Devices {
    /// Opens the smart card service (again).
    pub fn open(&mut self) -> Result<(), Error> {
        self.system = Some((self.open_system)()?);
        Ok(())
    }

    /// The open connection, in the form the exchange helpers take.
    pub fn link(&mut self) -> Option<&mut dyn CardLink> {
        self.card.as_deref_mut().map(|c| c as &mut dyn CardLink)
    }
}

type Answer = Box<dyn Any + Send>;
type Job = Box<dyn FnOnce(&mut Devices) -> Answer + Send>;

/// Why a job sent to the reader thread got no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    /// The thread could not be started.
    NotRunning,
    /// The thread ended before answering.
    Stopped,
    /// The job panicked; the next jobs still run.
    Panicked(String),
    /// The answer was not of the type expected (a bug).
    UnexpectedAnswer,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::NotRunning => f.write_str("The reader thread isn't running."),
            WorkerError::Stopped => f.write_str("The reader thread stopped."),
            WorkerError::Panicked(why) => write!(f, "The reader thread failed: {why}"),
            WorkerError::UnexpectedAnswer => {
                f.write_str("The reader thread gave an unexpected answer.")
            }
        }
    }
}

/// A job that panicked on the worker thread: its answer, in place of the expected one.
struct JobPanicked(String);

/// What the window does with an answer: the answer, or the reason the job failed.
pub type Then<A> = Box<dyn FnOnce(&mut A, Result<Answer, WorkerError>)>;

/// A job sent whose answer is still expected.
struct Pending<A> {
    then: Then<A>,
    /// A background job (a poll): it doesn't make the window busy.
    quiet: bool,
}

pub struct Worker<A> {
    jobs: Option<Sender<Job>>,
    answers: Receiver<Answer>,
    /// What to do with the answers still expected, in the order the jobs were sent.
    pending: VecDeque<Pending<A>>,
    /// Jobs that could not be sent (no thread): they are answered with an error.
    failed: VecDeque<Then<A>>,
}

impl<A> Worker<A> {
    /// Starts the thread. It opens nothing until a job asks it to.
    pub fn spawn(open_system: SystemOpener) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (answer_tx, answer_rx) = mpsc::channel::<Answer>();
        let spawned = thread::Builder::new().name("reader".into()).spawn(move || {
            let mut devices = Devices {
                open_system,
                system: None,
                card: None,
                escape: None,
            };
            // Ends when the window drops its sender (the app closes).
            while let Ok(job) = job_rx.recv() {
                let answer = panic::catch_unwind(AssertUnwindSafe(|| job(&mut devices)))
                    .unwrap_or_else(|p| {
                        let why = p
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| p.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown error".into());
                        Box::new(JobPanicked(why))
                    });
                if answer_tx.send(answer).is_err() {
                    break;
                }
            }
        });
        Worker {
            // Without a thread no job can run; `call` then answers every job with an error.
            jobs: spawned.ok().map(|_| job_tx),
            answers: answer_rx,
            pending: VecDeque::new(),
            failed: VecDeque::new(),
        }
    }

    /// Sends `job` to the reader thread; `then` gets its answer on the window's thread. The
    /// window is busy until then (`busy`), so no second action starts meanwhile.
    pub fn call<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&mut Devices) -> T + Send + 'static,
        then: impl FnOnce(&mut A, Result<T, WorkerError>) + 'static,
    ) {
        self.send(job, then, false);
    }

    /// Like `call`, for a background job (polling for a card) that doesn't make the window
    /// busy: the controls stay as they are while it runs, and an action the person starts
    /// meanwhile simply runs after it.
    pub fn call_quiet<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&mut Devices) -> T + Send + 'static,
        then: impl FnOnce(&mut A, Result<T, WorkerError>) + 'static,
    ) {
        self.send(job, then, true);
    }

    fn send<T: Send + 'static>(
        &mut self,
        job: impl FnOnce(&mut Devices) -> T + Send + 'static,
        then: impl FnOnce(&mut A, Result<T, WorkerError>) + 'static,
        quiet: bool,
    ) {
        let wrapped: Job = Box::new(move |d: &mut Devices| Box::new(job(d)) as Answer);
        let then: Then<A> = Box::new(move |a: &mut A, answer: Result<Answer, WorkerError>| {
            let result = answer.and_then(|any| match any.downcast::<T>() {
                Ok(t) => Ok(*t),
                Err(other) => Err(match other.downcast::<JobPanicked>() {
                    Ok(p) => WorkerError::Panicked(p.0),
                    Err(_) => WorkerError::UnexpectedAnswer,
                }),
            });
            then(a, result);
        });
        let sent = self
            .jobs
            .as_ref()
            .is_some_and(|tx| tx.send(wrapped).is_ok());
        if sent {
            self.pending.push_back(Pending { then, quiet });
        } else {
            self.jobs = None;
            self.failed.push_back(then);
        }
    }

    /// Jobs the person started whose answer hasn't been applied yet (quiet jobs don't count).
    pub fn busy(&self) -> bool {
        self.pending.iter().any(|p| !p.quiet) || !self.failed.is_empty()
    }

    /// Any job whose answer hasn't been applied yet, quiet ones included.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.failed.is_empty()
    }

    /// The answers that have arrived, with what to do with each; never waits.
    pub fn ready(&mut self) -> Vec<(Then<A>, Result<Answer, WorkerError>)> {
        let mut out = self.take_failed();
        while !self.pending.is_empty() {
            match self.answers.try_recv() {
                Ok(answer) => {
                    if let Some(p) = self.pending.pop_front() {
                        out.push((p.then, Ok(answer)));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.extend(self.drain_lost());
                    break;
                }
            }
        }
        out
    }

    /// Waits up to `timeout` for the next answer (tests check what an action did).
    #[cfg(test)]
    pub fn wait(&mut self, timeout: Duration) -> Option<(Then<A>, Result<Answer, WorkerError>)> {
        if let Some(t) = self.failed.pop_front() {
            return Some((t, Err(WorkerError::NotRunning)));
        }
        self.pending.front()?;
        match self.answers.recv_timeout(timeout) {
            Ok(answer) => self.pending.pop_front().map(|p| (p.then, Ok(answer))),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => self.drain_lost().into_iter().next(),
        }
    }

    fn take_failed(&mut self) -> Vec<(Then<A>, Result<Answer, WorkerError>)> {
        self.failed
            .drain(..)
            .map(|t| (t, Err(WorkerError::NotRunning)))
            .collect()
    }

    fn drain_lost(&mut self) -> Vec<(Then<A>, Result<Answer, WorkerError>)> {
        self.jobs = None;
        self.pending
            .drain(..)
            .map(|p| (p.then, Err(WorkerError::Stopped)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::fake::FakeSystem;
    use crate::reader::{Proto, Share};
    use std::sync::{Arc, Barrier};
    use std::time::Instant;

    type Log = Vec<String>;

    fn worker() -> Worker<Log> {
        Worker::spawn(FakeSystem::with_card("R", &[0x3B]).opener())
    }

    /// Applies answers until nothing is pending (or 5 s pass).
    fn settle(w: &mut Worker<Log>, log: &mut Log) {
        let end = Instant::now() + Duration::from_secs(5);
        while w.busy() && Instant::now() < end {
            if let Some((then, answer)) = w.wait(Duration::from_millis(100)) {
                then(log, answer);
            }
        }
        assert!(!w.busy(), "answers still pending");
    }

    #[test]
    fn answers_come_back_in_order() {
        let mut w = worker();
        let mut log = Log::new();
        for i in 0..20u32 {
            w.call(
                move |_| {
                    if i % 3 == 0 {
                        thread::sleep(Duration::from_millis(5));
                    }
                    i
                },
                |log: &mut Log, r| log.push(format!("{:?}", r)),
            );
        }
        settle(&mut w, &mut log);
        let want: Vec<String> = (0..20u32).map(|i| format!("Ok({i})")).collect();
        assert_eq!(log, want);
    }

    #[test]
    fn a_slow_job_does_not_block_the_window() {
        let mut w = worker();
        let gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        w.call(move |_| g.wait(), |_: &mut Log, _| {});
        // The job is stuck until the barrier opens: the window can still ask and is told "not yet".
        let t = Instant::now();
        assert!(w.ready().is_empty());
        assert!(w.busy());
        assert!(t.elapsed() < Duration::from_millis(500));
        gate.wait();
        let mut log = Log::new();
        settle(&mut w, &mut log);
    }

    #[test]
    fn a_panicking_job_is_reported_and_the_next_one_still_runs() {
        let mut w = worker();
        let mut log = Log::new();
        w.call(
            |_| -> u8 { panic!("boom") },
            |log: &mut Log, r| log.push(format!("{:?}", r)),
        );
        w.call(|_| 7u8, |log: &mut Log, r| log.push(format!("{:?}", r)));
        settle(&mut w, &mut log);
        assert_eq!(
            log,
            vec!["Err(Panicked(\"boom\"))".to_string(), "Ok(7)".to_string()]
        );
        assert_eq!(
            WorkerError::Panicked("boom".into()).to_string(),
            "The reader thread failed: boom"
        );
    }

    #[test]
    fn the_thread_owns_the_connection() {
        let mut w = worker();
        let mut log = Log::new();
        w.call(
            |d| {
                d.open().map_err(|e| format!("{e:?}"))?;
                let s = d.system.as_mut().ok_or("no system")?;
                d.card = Some(
                    s.connect("R", Share::Shared, Proto::Any)
                        .map_err(|e| format!("{e:?}"))?,
                );
                Ok::<_, String>(())
            },
            |log: &mut Log, r| log.push(format!("{:?}", r)),
        );
        w.call(
            |d| d.link().map(|l| l.status().map(|s| s.atr)),
            |log: &mut Log, r| log.push(format!("{:?}", r)),
        );
        settle(&mut w, &mut log);
        assert_eq!(log, vec!["Ok(Ok(()))", "Ok(Some(Ok([59])))"]);
    }
}
