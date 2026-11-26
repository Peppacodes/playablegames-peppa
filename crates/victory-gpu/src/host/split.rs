//! The next submission's write-watch asking made ahead, on a thread of its
//! own (`VICTORY_SUBMIT_SPLIT=1`; off by default; flips with the in-run A/B
//! in diagnostics builds): slice 1a of the split of the submitting thread's
//! work (`overnight/DESIGN-submit-split.md` in the repository's working
//! notes).
//!
//! Why: the thread that prepares submissions is the frame's wall in the
//! house test on an RTX 5080 (no waits of 50 us or more; the title's Present
//! waits ~38 ms a frame for it). One of its steps, the one asking of the
//! kernel's write watch per submission (`crate::watch::ask_once`, ~7.7% of
//! its time), depends on no flight: it reads the write-watch bits of the
//! ranges the submission refreshes into the journal. A submission's asking
//! ends when its refreshes are done (`ask_once_end`), and what follows (its
//! snapshots, the copies in the card's memory, the sending, ~19% of the
//! thread) does not ask. So the next submission's asking is made then, on
//! another thread, while this one is finished.
//!
//! What may be asked ahead: the lists of submissions that stand first in
//! their queue in the line, nothing in front of them (a submission behind a
//! wait for a memory value may read what the title writes later). When the
//! next submission comes to its own asking it is skipped only if all of this
//! holds, else it asks as before (a newer asking only sees more):
//! - every list of it was asked ahead, and it has no list of card steps of
//!   its own (made from the flights in the air, unknown ahead);
//! - every range it would ask about lies in one asked;
//! - that asking still stands (no other asking and no end of one since);
//! - nothing of ours wrote title memory since the asking began
//!   (`crate::watch::wrote`, which notes our writes only while an asking
//!   stands).
//!
//! The submitting thread waits for an asking under way before its own (never
//! two at once), and an asking ahead is made only when the submitting thread
//! is done asking for its own submission. The writes the title makes after
//! the asking ahead to the next submission's ranges are the next asking's,
//! a submission later than before: the title made them after it submitted
//! that submission.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Condvar, Mutex, OnceLock};

use super::HostList;

/// See the file.
pub(super) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    let on = *ON.get_or_init(|| std::env::var_os("VICTORY_SUBMIT_SPLIT").is_some_and(|v| v == "1"));
    #[cfg(feature = "diagnostics")]
    let on = crate::ab::pick("VICTORY_SUBMIT_SPLIT").unwrap_or(on);
    on
}

/// The asking made ahead.
struct Ahead {
    /// 0: none; 1: being made; 2: made.
    stage: u8,
    /// The host lists it was made for (addresses).
    lists: Vec<usize>,
    /// The ranges it asked about.
    ranges: Vec<(usize, usize)>,
    /// The asking's number, and our writes to title memory, when it began.
    epoch: u64,
    wrote: u64,
}

static AHEAD: Mutex<Ahead> = Mutex::new(Ahead { stage: 0, lists: Vec::new(), ranges: Vec::new(), epoch: 0, wrote: 0 });
static DONE: Condvar = Condvar::new();

/// Askings made ahead; used; not used for: lists, ranges, an asking or a
/// write since, card steps of its own.
pub(super) static STATS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

fn ahead() -> std::sync::MutexGuard<'static, Ahead> {
    AHEAD.lock().unwrap_or_else(|e| e.into_inner())
}

/// The thread that asks ahead.
fn asker() -> Option<&'static Mutex<std::sync::mpsc::Sender<Vec<(usize, usize)>>>> {
    static TO: OnceLock<Option<Mutex<std::sync::mpsc::Sender<Vec<(usize, usize)>>>>> = OnceLock::new();
    TO.get_or_init(|| {
        let (to, from) = std::sync::mpsc::channel::<Vec<(usize, usize)>>();
        std::thread::Builder::new()
            .name("victory-ask-ahead".into())
            .spawn(move || {
                while let Ok(ranges) = from.recv() {
                    let _asking = crate::timeline::span("asking ahead for the next submission");
                    crate::watch::ask_once(&ranges);
                    let epoch = crate::watch::asking_epoch();
                    let mut a = ahead();
                    a.stage = 2;
                    a.epoch = epoch;
                    drop(a);
                    DONE.notify_all();
                }
            })
            .ok()
            .map(|_| Mutex::new(to))
    })
    .as_ref()
}

/// The submitting thread's asking has ended and its refreshes are done:
/// the next submission's asking is made ahead, if one stands ready.
pub(super) fn after_own_asking() {
    if !on() || !crate::watch::once() {
        return;
    }
    let lists = crate::queue::ready_runs();
    if lists.is_empty() {
        return;
    }
    // SAFETY: the lists stay in the line until this thread (the one that
    // serves it, inside a submission now) takes them.
    let refs: Vec<&HostList> = lists.iter().map(|&p| unsafe { &*(p as *const HostList) }).collect();
    let ranges = super::asked_spans(&refs);
    // Known by who they are: unfolding takes them off their title lists.
    let lists: Vec<usize> = refs.iter().map(|h| h.id()).collect();
    if ranges.is_empty() {
        return;
    }
    let Some(to) = asker() else {
        return;
    };
    let mut a = ahead();
    if a.stage == 1 {
        return;
    }
    *a = Ahead { stage: 1, lists, ranges: ranges.clone(), epoch: 0, wrote: crate::watch::wrote_count() };
    drop(a);
    if to.lock().unwrap_or_else(|e| e.into_inner()).send(ranges).is_err() {
        ahead().stage = 0;
        return;
    }
    STATS[0].fetch_add(1, Relaxed);
}

/// Whether the asking made ahead stands for this submission (`runs`, asking
/// about `ranges`; `own`: it has a list of card steps of its own), so that
/// its own asking is left out. Waits for one under way; forgets it either
/// way.
pub(super) fn asked_for(runs: &[&HostList], ranges: &[(usize, usize)], own: bool) -> bool {
    let mut a = ahead();
    while a.stage == 1 {
        a = DONE.wait(a).unwrap_or_else(|e| e.into_inner());
    }
    if a.stage == 0 {
        return false;
    }
    let lists: Vec<usize> = runs.iter().map(|r| r.id()).collect();
    let why = if own {
        Some(5)
    } else if !covers(&a.lists, &a.ranges, &lists, ranges).0 {
        Some(1)
    } else if !covers(&a.lists, &a.ranges, &lists, ranges).1 {
        Some(2)
    } else if !crate::watch::asking_stands(a.epoch) {
        Some(3)
    } else if crate::watch::wrote_count() != a.wrote {
        Some(4)
    } else {
        None
    };
    *a = Ahead { stage: 0, lists: Vec::new(), ranges: Vec::new(), epoch: 0, wrote: 0 };
    drop(a);
    match why {
        None => {
            STATS[1].fetch_add(1, Relaxed);
            on()
        }
        Some(i) => {
            STATS[i + 1].fetch_add(1, Relaxed);
            false
        }
    }
}

/// Whether every list of `lists` is one of `asked_lists`, and whether every
/// range of `ranges` lies in one of `asked`.
fn covers(asked_lists: &[usize], asked: &[(usize, usize)], lists: &[usize], ranges: &[(usize, usize)]) -> (bool, bool) {
    let all_lists = lists.iter().all(|l| asked_lists.contains(l));
    let all_ranges = ranges.iter().all(|&(from, to)| from >= to || asked.iter().any(|&(a, b)| a <= from && to <= b));
    (all_lists, all_ranges)
}

/// For the log: askings made ahead, used, and why the others were not.
pub(super) fn line() -> String {
    let s: Vec<u64> = STATS.iter().map(|s| s.swap(0, Relaxed)).collect();
    format!(
        "next submission's asking made ahead (VICTORY_SUBMIT_SPLIT): {} made, {} used; not used: {} lists not asked, {} ranges not asked, {} asking ended, {} title memory written by us, {} card steps of its own",
        s[0], s[1], s[2], s[3], s[4], s[5], s[6]
    )
}

#[cfg(test)]
mod tests {
    use super::covers;

    #[test]
    fn an_asking_ahead_stands_only_for_lists_and_ranges_it_covered() {
        let asked_lists = [10, 20];
        let asked = [(0x1000, 0x5000), (0x9000, 0xa000)];
        // The same lists, ranges inside: it stands.
        assert_eq!(covers(&asked_lists, &asked, &[10, 20], &[(0x1000, 0x2000), (0x9000, 0xa000)]), (true, true));
        // A list joined that was not asked for: not.
        assert_eq!(covers(&asked_lists, &asked, &[10, 30], &[(0x1000, 0x2000)]).0, false);
        // A range past what was asked: not.
        assert_eq!(covers(&asked_lists, &asked, &[10], &[(0x4000, 0x6000)]).1, false);
        // A range across two asked ones is not inside either: not.
        assert_eq!(covers(&asked_lists, &asked, &[10], &[(0x4000, 0x9800)]).1, false);
        // Fewer lists than asked for, ranges inside: it stands.
        assert_eq!(covers(&asked_lists, &asked, &[20], &[(0x9000, 0x9800)]), (true, true));
        // Nothing to ask about stands trivially for the ranges.
        assert_eq!(covers(&asked_lists, &asked, &[10], &[]).1, true);
    }
}
