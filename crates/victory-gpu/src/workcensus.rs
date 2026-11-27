//! Diagnostics: what the submitting thread does that could be left out
//! (`VICTORY_WORK_CENSUS=<from s>,<seconds>`), counted over a window of the
//! run and logged once when it closes.
//!
//! - Blocks the submitting thread allocates (the counting allocator tells
//!   them), with the call sites of one in [`ALLOC_EVERY`] (stacks captured
//!   in the allocator, named when the window closes).
//! - Refreshes of host copies (`Shadow::update`) on every thread: how many,
//!   how many copied nothing, how many asked again for the same copy and
//!   range in the same submission, the bytes and the time, by memory class
//!   (the title's 32-bit graphics memory, renderer port 1's region, below
//!   4 GiB, or above; a copy in the card's memory or not), with the callers
//!   of one in [`REFRESH_EVERY`]; and the requests answered "nothing new"
//!   without asking (`Shadow::nothing_new`).
//! - The processors the submitting thread runs its submissions on.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};

/// Stack depth kept per sample.
const DEPTH: usize = 14;
/// Stacks kept per kind.
const RING: usize = 8192;
/// One allocation of the submitting thread in so many has its stack kept.
const ALLOC_EVERY: u64 = 16;
/// One refresh in so many has its stack kept.
const REFRESH_EVERY: u64 = 32;

static ON: AtomicBool = AtomicBool::new(false);

thread_local! {
    static SUBMITTING: Cell<bool> = const { Cell::new(false) };
}

/// The thread that prepares and sends submissions says so at its start.
pub(crate) fn mark_submitting_thread() {
    SUBMITTING.set(true);
}

fn submitting() -> bool {
    SUBMITTING.try_with(Cell::get).unwrap_or(false)
}

struct Stacks {
    at: AtomicUsize,
    frames: [[AtomicUsize; DEPTH]; RING],
}

impl Stacks {
    const fn new() -> Self {
        Stacks { at: AtomicUsize::new(0), frames: [const { [const { AtomicUsize::new(0) }; DEPTH] }; RING] }
    }

    /// This thread's stack, into the next slot (no allocation).
    fn keep(&self) {
        use windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
        let mut frames = [std::ptr::null_mut(); DEPTH];
        // SAFETY: a buffer of DEPTH entries, DEPTH asked for.
        let n = unsafe { RtlCaptureStackBackTrace(2, DEPTH as u32, frames.as_mut_ptr(), std::ptr::null_mut()) } as usize;
        let slot = &self.frames[self.at.fetch_add(1, Relaxed) % RING];
        for (i, f) in slot.iter().enumerate() {
            f.store(if i < n { frames[i] as usize } else { 0 }, Relaxed);
        }
    }

    fn taken(&self) -> Vec<Vec<usize>> {
        let n = self.at.load(Relaxed).min(RING);
        self.frames[..n].iter().map(|s| s.iter().map(|f| f.load(Relaxed)).take_while(|&f| f != 0).collect()).collect()
    }

    fn clear(&self) {
        self.at.store(0, Relaxed);
    }
}

static ALLOC_STACKS: Stacks = Stacks::new();
static REFRESH_STACKS: Stacks = Stacks::new();
/// Blocks and bytes the submitting thread allocated (grown blocks count once).
static ALLOCS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// The counting allocator: a block of `size` bytes was made or grown.
#[inline]
pub(crate) fn allocated(size: usize) {
    if !ON.load(Relaxed) || !submitting() {
        return;
    }
    let n = ALLOCS[0].fetch_add(1, Relaxed);
    ALLOCS[1].fetch_add(size as u64, Relaxed);
    if n % ALLOC_EVERY == 0 {
        ALLOC_STACKS.keep();
    }
}

/// Per memory class (below 4 GiB or not, times in the card's memory or not)
/// and per thread kind (the submitting thread, others): requests, nothing
/// copied, repeats in the same submission, bytes, nanoseconds.
static REFRESH: [[[AtomicU64; 5]; 2]; 4] = [const { [const { [const { AtomicU64::new(0) }; 5] }; 2] }; 4];
/// Requests answered "nothing new" without asking, per class.
static KNOWN: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// The submission under way (the repeats are counted within one).
static SUBMISSION: AtomicU64 = AtomicU64::new(0);
/// Hashes of (copy, range, submission) seen: a lossy set.
static SEEN: [AtomicU64; 1 << 16] = [const { AtomicU64::new(0) }; 1 << 16];
/// Submissions begun on each processor.
static CORES: [AtomicU32; 64] = [const { AtomicU32::new(0) }; 64];
static FRAMES: AtomicU64 = AtomicU64::new(0);

fn class(address: usize, card: bool) -> usize {
    usize::from(address < 1 << 32) * 2 + usize::from(card)
}

/// Whether a refresh is to be counted now (the caller times it).
#[inline]
pub(crate) fn counting() -> bool {
    ON.load(Relaxed)
}

/// `Shadow::update` of `[address, address + len)` of copy `copy` copied
/// `copied` bytes in `ns`.
pub(crate) fn refreshed(copy: usize, address: usize, len: usize, card: bool, copied: usize, ns: u64) {
    let c = &REFRESH[class(address, card)][usize::from(!submitting())];
    let n = c[0].fetch_add(1, Relaxed);
    if copied == 0 {
        c[1].fetch_add(1, Relaxed);
    }
    let mut key = (copy as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (address as u64).rotate_left(17) ^ (len as u64).rotate_left(41);
    key ^= SUBMISSION.load(Relaxed).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    key |= 1;
    if SEEN[(key >> 7) as usize % SEEN.len()].swap(key, Relaxed) == key {
        c[2].fetch_add(1, Relaxed);
    }
    c[3].fetch_add(copied as u64, Relaxed);
    c[4].fetch_add(ns, Relaxed);
    if n % REFRESH_EVERY == 0 {
        REFRESH_STACKS.keep();
    }
}

/// A request for `address` answered "nothing new" without asking.
pub(crate) fn known_nothing(address: usize, card: bool) {
    if ON.load(Relaxed) {
        KNOWN[class(address, card)].fetch_add(1, Relaxed);
    }
}

/// A submission begins on this thread.
pub(crate) fn submission_begins() {
    SUBMISSION.fetch_add(1, Relaxed);
    if ON.load(Relaxed) {
        // SAFETY: a plain query of this thread's processor.
        let core = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessorNumber() } as usize;
        CORES[core % CORES.len()].fetch_add(1, Relaxed);
    }
}

/// A frame was shown.
pub(crate) fn presented() {
    if ON.load(Relaxed) {
        FRAMES.fetch_add(1, Relaxed);
    }
}

/// The window asked for, in seconds from the start.
fn window() -> Option<(f64, f64)> {
    static W: std::sync::OnceLock<Option<(f64, f64)>> = std::sync::OnceLock::new();
    *W.get_or_init(|| {
        let v = std::env::var("VICTORY_WORK_CENSUS").ok()?;
        let (a, b) = v.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    })
}

/// Opens and closes the window (the frame-rate line calls it).
pub(crate) fn tick() {
    let Some((from, seconds)) = window() else { return };
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static DONE: AtomicBool = AtomicBool::new(false);
    let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
    if DONE.load(Relaxed) {
        return;
    }
    if !ON.load(Relaxed) && t >= from {
        ON.store(true, Relaxed);
        tracing::info!(from, seconds, "the work census begins (VICTORY_WORK_CENSUS)");
    } else if ON.load(Relaxed) && t >= from + seconds {
        ON.store(false, Relaxed);
        DONE.store(true, Relaxed);
        report();
    }
}

fn report() {
    let frames = FRAMES.load(Relaxed).max(1) as f64;
    let per = |v: u64| v as f64 / frames;
    let skip = ["copystat::", "heap::", "workcensus::", "mi_", "_mi_", "__rust", "__rdl", "__rg_", "mimalloc", "hashbrown::"];
    let alloc_sites = crate::timeline::census_sites(&ALLOC_STACKS.taken(), &skip);
    let taken: usize = alloc_sites.iter().map(|s| s.1).sum::<usize>().max(1);
    let top: Vec<String> = alloc_sites.iter().take(14).map(|(s, n)| format!("{:.1}% {s}", 100.0 * *n as f64 / taken as f64)).collect();
    tracing::info!(
        "work census over {frames:.0} frames: the submitting thread allocated {:.0} blocks a frame ({:.2} MB); by site (one in {ALLOC_EVERY} sampled): {}",
        per(ALLOCS[0].load(Relaxed)),
        per(ALLOCS[1].load(Relaxed)) / 1e6,
        top.join("; ")
    );
    const CLASSES: [&str; 4] = ["above 4 GiB, host copy", "above 4 GiB, card copy", "below 4 GiB (port 1 region), host copy", "below 4 GiB (port 1 region), card copy"];
    for (c, name) in CLASSES.iter().enumerate() {
        for (k, kind) in ["on the submitting thread", "on other threads"].iter().enumerate() {
            let r = &REFRESH[c][k];
            let n = r[0].load(Relaxed);
            if n == 0 {
                continue;
            }
            tracing::info!(
                "work census, refreshes of {name} {kind}, a frame: {:.0} requests, {:.0} copied nothing, {:.0} repeated a request of the same submission, {:.2} MB copied, {:.2} ms",
                per(n),
                per(r[1].load(Relaxed)),
                per(r[2].load(Relaxed)),
                per(r[3].load(Relaxed)) / 1e6,
                per(r[4].load(Relaxed)) / 1e6
            );
        }
        let known = KNOWN[c].load(Relaxed);
        if known > 0 {
            tracing::info!("work census, requests of {name} answered nothing new without asking, a frame: {:.0}", per(known));
        }
    }
    let callers = crate::timeline::census_sites(&REFRESH_STACKS.taken(), &["shadow::Shadow::update", "workcensus::"]);
    let taken: usize = callers.iter().map(|s| s.1).sum::<usize>().max(1);
    let top: Vec<String> = callers.iter().take(12).map(|(s, n)| format!("{:.1}% {s}", 100.0 * *n as f64 / taken as f64)).collect();
    tracing::info!("work census, refreshes by caller (one in {REFRESH_EVERY} sampled): {}", top.join("; "));
    let cores: Vec<String> = CORES.iter().enumerate().filter(|c| c.1.load(Relaxed) > 0).map(|(i, c)| format!("{i}: {}", c.load(Relaxed))).collect();
    tracing::info!("work census, submissions begun by processor: {}", cores.join(", "));
    copies_report(frames);
    ALLOC_STACKS.clear();
    REFRESH_STACKS.clear();
}

/// Per copy, in a lossy table: id, start, bytes, window (1) and card (2)
/// bits, bytes refreshed, requests, nanoseconds.
static COPIES: [[AtomicU64; 7]; 4096] = [const { [const { AtomicU64::new(0) }; 7] }; 4096];

/// [`refreshed`]'s copy, for the table of the copies refreshed most.
pub(crate) fn copy_refreshed(id: u64, start: u64, len: u64, window: bool, card: bool, copied: usize, ns: u64) {
    let mut i = (id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 52) as usize % COPIES.len();
    for _ in 0..16 {
        let slot = &COPIES[i];
        let there = slot[0].compare_exchange(0, id + 1, Relaxed, Relaxed).unwrap_or_else(|v| v);
        if there == 0 || there == id + 1 {
            slot[1].store(start, Relaxed);
            slot[2].store(len, Relaxed);
            slot[3].store(u64::from(window) | u64::from(card) << 1, Relaxed);
            slot[4].fetch_add(copied as u64, Relaxed);
            slot[5].fetch_add(1, Relaxed);
            slot[6].fetch_add(ns, Relaxed);
            return;
        }
        i = (i + 1) % COPIES.len();
    }
}

/// The copies that took the most refresh time, and the sizes of all of
/// them, by class (logged with the census).
fn copies_report(frames: f64) {
    let mut all: Vec<[u64; 6]> = COPIES
        .iter()
        .filter(|s| s[0].load(Relaxed) != 0)
        .map(|s| [s[1].load(Relaxed), s[2].load(Relaxed), s[3].load(Relaxed), s[4].load(Relaxed), s[5].load(Relaxed), s[6].load(Relaxed)])
        .collect();
    all.sort_unstable_by(|a, b| b[5].cmp(&a[5]));
    let top: Vec<String> = all
        .iter()
        .take(16)
        .map(|c| {
            format!(
                "{:#x} {:.2} MB {}{}: {:.2} MB {:.1} requests {:.2} ms",
                c[0],
                c[1] as f64 / 1e6,
                if c[2] & 2 != 0 { "card" } else { "host" },
                if c[2] & 1 != 0 { " window" } else { "" },
                c[3] as f64 / frames / 1e6,
                c[4] as f64 / frames,
                c[5] as f64 / frames / 1e6
            )
        })
        .collect();
    tracing::info!("work census, the copies refreshed longest, a frame: {}", top.join("; "));
    // Host copies above 4 GiB by size: copies, MB refreshed, ms.
    let mut by = [[0f64; 3]; 5];
    for c in all.iter().filter(|c| c[2] & 2 == 0 && c[0] >= 1 << 32) {
        let b = match c[1] {
            0..0x1_0000 => 0,
            0x1_0000..0x10_0000 => 1,
            0x10_0000..0x100_0000 => 2,
            0x100_0000..0x1000_0000 => 3,
            _ => 4,
        };
        by[b][0] += 1.0;
        by[b][1] += c[3] as f64 / frames / 1e6;
        by[b][2] += c[5] as f64 / frames / 1e6;
    }
    let names = ["under 64 KiB", "64 KiB-1 MiB", "1-16 MiB", "16-256 MiB", "256 MiB and more"];
    let parts: Vec<String> = names.iter().zip(by).map(|(n, b)| format!("{n}: {:.0} copies, {:.2} MB, {:.2} ms", b[0], b[1], b[2])).collect();
    tracing::info!("work census, host copies above 4 GiB by their size, a frame: {}", parts.join("; "));
}
