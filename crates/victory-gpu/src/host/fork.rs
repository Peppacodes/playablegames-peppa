//! A submission's refreshes in front done on a thread of their own while the
//! submitting thread goes through the lists' steps (`VICTORY_SUBMIT_FORK=1`;
//! off by default; flips with the in-run A/B in diagnostics builds).
//!
//! Why: the thread that prepares submissions is the frame's wall in the house
//! test on an RTX 5080 (everything else waits on it). Of its ~51 ms a frame,
//! the refreshes in front take ~17 ms (`Batch::refresh_spans`: the host
//! copies a submission's lists read brought up to date from title memory,
//! spread on the wide helpers) and the walk over the lists' steps that
//! follows another ~13 ms (`HostList::run_deferred`), one after the other.
//!
//! What the steps need of the refreshes: the plan of them (which copy, which
//! range, `Batch::ahead`), which stays on the submitting thread, before the
//! fork. The work itself only before a step that writes title memory or
//! runs work of ours (all of it: [`Gate::All`]), and before a step that gives
//! a copy to shaders to write (that copy's: [`Gate::Copy`]). Refreshes the
//! steps ask for, fills, textures and planes touch no host copy the fork
//! writes ([`Gate::Free`]). And all of it before the batch's last fills and
//! its last refreshes. The work spreads on the wide helpers as before; the
//! thread that hands it out runs above normal priority, as the submitting
//! thread does. Without the thread the submitting thread does it itself.

use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// See the file.
pub(super) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    let on = *ON.get_or_init(|| std::env::var_os("VICTORY_SUBMIT_FORK").is_some_and(|v| v == "1"));
    #[cfg(feature = "diagnostics")]
    let on = crate::ab::pick("VICTORY_SUBMIT_FORK").unwrap_or(on);
    on
}

/// For the log: the step at which a submission waited for all of its fork,
/// by kind: the batch's end, a copy, a copy at its place, work that may
/// write title memory, work of ours.
pub(super) static FIRST_ALL: [std::sync::atomic::AtomicU64; 5] = [const { std::sync::atomic::AtomicU64::new(0) }; 5];

/// The line for the log.
pub(super) fn line() -> String {
    let s: Vec<u64> = FIRST_ALL.iter().map(|s| s.swap(0, std::sync::atomic::Ordering::Relaxed)).collect();
    format!(
        "forked refreshes in front (VICTORY_SUBMIT_FORK): waited for all of a fork at the batch's end {}, at a title buffer copy {}, at a copy at its place {}, at work that may write title memory {}, at work of ours {}",
        s[0], s[1], s[2], s[3], s[4]
    )
}

/// What a step of the lists must wait for of a fork.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Gate {
    /// Nothing.
    Free,
    /// That copy's refresh (its address).
    Copy(usize),
    /// The textures it brings up to date (work of ours on textures).
    Textures,
    /// All of the fork.
    All,
}

/// What is left of a fork's work: per copy, its spans not done yet.
struct Left {
    finished: bool,
    copies: std::collections::HashMap<usize, u32>,
    /// Textures not done yet.
    textures: usize,
}

/// A fork's work being done, as its owner sees it.
pub(super) struct Fork {
    left: Arc<(Mutex<Left>, Condvar)>,
}

/// Handed to the work: says that a span of a copy is done.
#[derive(Clone)]
pub(super) struct Done(Arc<(Mutex<Left>, Condvar)>);

impl Done {
    pub(super) fn texture(&self) {
        let mut left = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        left.textures = left.textures.saturating_sub(1);
        if left.textures == 0 {
            drop(left);
            self.0.1.notify_all();
        }
    }

    pub(super) fn copy(&self, copy: usize) {
        let mut left = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = left.copies.get_mut(&copy) {
            *n -= 1;
            if *n == 0 {
                left.copies.remove(&copy);
                drop(left);
                self.0.1.notify_all();
            }
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// The fork's thread.
fn thread() -> Option<&'static Mutex<std::sync::mpsc::Sender<Job>>> {
    static TO: OnceLock<Option<Mutex<std::sync::mpsc::Sender<Job>>>> = OnceLock::new();
    TO.get_or_init(|| {
        let (to, from) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("victory-submit-fork".into())
            .spawn(move || {
                use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};
                // SAFETY: this thread's own priority.
                unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
                crate::cores::keep_helper_on_fast_cores("victory-submit-fork");
                while let Ok(job) = from.recv() {
                    job();
                }
            })
            .ok()
            .map(|_| Mutex::new(to))
    })
    .as_ref()
}

impl Fork {
    /// Starts `work` on the fork's thread; `spans`: the copy (address) of each
    /// span it brings up to date, once per span, which it tells `Done` of as
    /// it goes. Without the thread it is done here, now.
    pub(super) fn start(spans: impl Iterator<Item = usize>, textures: usize, work: impl FnOnce(Done) + Send + 'static) -> Fork {
        let mut copies = std::collections::HashMap::new();
        for copy in spans {
            *copies.entry(copy).or_insert(0u32) += 1;
        }
        let left = Arc::new((Mutex::new(Left { finished: false, copies, textures }), Condvar::new()));
        let done = Done(left.clone());
        let end = left.clone();
        let job: Job = Box::new(move || {
            let _work = crate::timeline::span("refreshes in front, forked");
            work(done);
            let mut left = end.0.lock().unwrap_or_else(|e| e.into_inner());
            left.finished = true;
            left.copies.clear();
            drop(left);
            end.1.notify_all();
        });
        match thread() {
            Some(to) => {
                if let Err(back) = to.lock().unwrap_or_else(|e| e.into_inner()).send(job) {
                    (back.0)();
                }
            }
            None => job(),
        }
        Fork { left }
    }

    /// Waits until the spans of `copy` are done (or all are).
    pub(super) fn wait_copy(&self, copy: usize) {
        let mut left = self.left.0.lock().unwrap_or_else(|e| e.into_inner());
        if left.finished || !left.copies.contains_key(&copy) {
            return;
        }
        let _waiting = crate::timeline::span("a shader write waits for its copy's forked refresh");
        while !left.finished && left.copies.contains_key(&copy) {
            left = self.left.1.wait(left).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Waits until its textures are done (or all is).
    pub(super) fn wait_textures(&self) {
        let mut left = self.left.0.lock().unwrap_or_else(|e| e.into_inner());
        if left.finished || left.textures == 0 {
            return;
        }
        let _waiting = crate::timeline::span("work of ours waits for the forked textures");
        while !left.finished && left.textures > 0 {
            left = self.left.1.wait(left).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Waits until all of it is done.
    pub(super) fn join(&self) {
        let mut left = self.left.0.lock().unwrap_or_else(|e| e.into_inner());
        if left.finished {
            return;
        }
        let _waiting = crate::timeline::span("waiting for the forked refreshes");
        while !left.finished {
            left = self.left.1.wait(left).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for Fork {
    /// Never past its batch: what it brings up to date is that submission's.
    fn drop(&mut self) {
        self.join();
    }
}

#[cfg(test)]
mod tests {
    use super::{Done, Fork};

    #[test]
    fn a_fork_lets_a_copy_go_when_its_spans_are_done_and_joins_at_its_end() {
        let (to_work, work_gets) = std::sync::mpsc::channel::<()>();
        let (work_did, did) = std::sync::mpsc::channel::<usize>();
        // Copy 1 has two spans, copy 2 one: the work does copy 2, then
        // copy 1's two, each when told to.
        let fork = Fork::start([1, 1, 2].into_iter(), 0, move |done: Done| {
            for copy in [2, 1, 1] {
                work_gets.recv().unwrap();
                done.copy(copy);
                work_did.send(copy).unwrap();
            }
        });
        to_work.send(()).unwrap();
        assert_eq!(did.recv().unwrap(), 2);
        // Copy 2 is done: no wait.
        fork.wait_copy(2);
        // Copy 1 has one span left: it waits for the second.
        to_work.send(()).unwrap();
        assert_eq!(did.recv().unwrap(), 1);
        let waiting = std::thread::spawn({
            let to_work = to_work.clone();
            move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                to_work.send(()).unwrap();
            }
        });
        fork.wait_copy(1);
        assert_eq!(did.recv().unwrap(), 1);
        fork.join();
        waiting.join().unwrap();
        // A copy the fork never had does not wait.
        fork.wait_copy(9);
    }
}
