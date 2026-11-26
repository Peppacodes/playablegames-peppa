//! A submission's one asking ([`super::ask_once`]) by the Windows
//! allocations its ranges lie in (`VICTORY_WATCH_RESERVATION=1`; off by
//! default; flips with the in-run A/B in diagnostics builds).
//!
//! From fxceless/victory-fork pull request 1365 (spk-c, from a kelvaroq
//! patch), with the follow-up for Wine pushed after a tester's failure. The
//! ranges are grouped by the allocation they lie in (one allocation query
//! where a range could join the next), and each group is one bounded kernel
//! asking that never crosses an allocation's end: [`super::ask_ahead`]'s runs
//! may, fail, and are asked again by halves, and the ranges it leaves alone
//! are asked one by one after it ([`super::ask_rest`]). A group the kernel
//! refuses has its ranges asked one by one; what lies in memory that is not
//! private (views) is left to `writes`, as before.
//!
//! The same journal: every page found written goes through
//! [`Journal::note`] (and so moves `VICTORY_COLD_SKIP`'s chunk marks); the
//! pages between a group's ranges, which the asking reset too, are noted as
//! well, and the whole group's range stands asked, as `ask_ahead`'s runs do.
//! The asking stands no longer at a map change of title memory
//! ([`super::title_memory_remapped`], [`super::title_memory_allocated`]), and
//! a write told done ([`super::title_memory_written`]) has its pages that lie
//! under the asking noted written after it.
//!
//! Upstream measured one pair: 10,000 ranges in 100 allocations 15.6 ->
//! 8.8 ms of asking, but ranges in one big allocation 6.0 -> 9.4 ms (the
//! allocation queries cost more than they save there).
use super::*;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Since the last line: groups asked, groups the kernel refused, allocation
/// queries, ranges asked.
static STATS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

pub(super) fn enabled() -> bool {
    #[cfg(feature = "diagnostics")]
    if let Some(on) = crate::ab::pick("VICTORY_WATCH_RESERVATION") {
        return on;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        // (Not with every guard asked again: that mode wants `writes` alone.)
        crate::switch::on("VICTORY_WATCH_RESERVATION") && !std::env::var_os("VICTORY_GUARD_ASK").is_some_and(|v| v == "1")
    })
}

/// Per frame, when the path ran.
pub(crate) fn line(frames: f64) {
    let [groups, refused, queries, ranges] = [0, 1, 2, 3].map(|i| STATS[i].swap(0, Relaxed) as f64 / frames.max(1.0));
    if groups + ranges > 0.0 {
        tracing::info!(
            "write watch by allocation per frame (VICTORY_WATCH_RESERVATION): {ranges:.0} ranges asked in {groups:.0} kernel askings \
             ({refused:.0} refused, their ranges asked alone), {queries:.0} allocation queries"
        );
    }
}

/// The allocation that page `page` lies in, as pages `[first, end)`, and
/// whether it is private memory (not a view, which the kernel cannot watch).
fn region(page: usize) -> Option<(usize, usize, bool)> {
    use windows_sys::Win32::System::Memory::{
        MEM_PRIVATE, MEMORY_BASIC_INFORMATION, MemoryRegionInfo, QueryVirtualMemoryInformation, VirtualQuery,
        WIN32_MEMORY_REGION_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let address = page.checked_mul(PAGE)? as *const std::ffi::c_void;
    STATS[2].fetch_add(1, Relaxed);
    // SAFETY: plain data, all zeros valid.
    let mut info: WIN32_MEMORY_REGION_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a question only; the answer's room and its size.
    let asked = unsafe {
        QueryVirtualMemoryInformation(
            GetCurrentProcess(),
            address,
            MemoryRegionInfo,
            (&raw mut info).cast(),
            size_of_val(&info),
            std::ptr::null_mut(),
        )
    };
    if asked == 0 {
        return None;
    }
    let base = info.AllocationBase as usize;
    let end = base.checked_add(info.RegionSize)?;
    // SAFETY: a successful answer has set the flags.
    let flags = unsafe { info.Anonymous.Flags };
    let private = if flags == 0 {
        // Wine gives the allocation's bounds but leaves its kind unset: the
        // address must be private memory of the same allocation.
        STATS[2].fetch_add(1, Relaxed);
        // SAFETY: plain data, all zeros valid.
        let mut basic: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let size = size_of_val(&basic);
        // SAFETY: a question only; the answer's room and its size.
        let returned = unsafe { VirtualQuery(address, &mut basic, size) };
        returned == size && basic.AllocationBase == info.AllocationBase && basic.Type == MEM_PRIVATE
    } else {
        flags & 1 != 0
    };
    Some((base.div_ceil(PAGE), end / PAGE, private))
}

/// Whether page range `next` joins the group `[first, last)`: the group stays
/// within [`AHEAD_MAX`], and the gap is within [`AHEAD_GAP`] or the smaller
/// of the two sizes.
fn can_join(first: usize, last: usize, next: (usize, usize)) -> bool {
    last.max(next.1).saturating_sub(first) <= AHEAD_MAX
        && next.0.saturating_sub(last) <= AHEAD_GAP.max((last - first).min(next.1 - next.0))
}

/// [`super::ask_once`]'s asking of `ranges` (title addresses), by
/// allocation. The caller holds everything ([`Asking::hold_all`]) and has
/// ended the asking that stood ([`ask_once_end`]).
pub(super) fn ask(ranges: &[(usize, usize)]) {
    // As `ask_ahead` and `ask_rest` take them: not view memory known by its
    // bits (the kernel would refuse the asking).
    let mut pieces: Vec<(usize, usize)> = ranges
        .iter()
        .copied()
        .filter(|r| r.0 < r.1 && !published(r.0, r.1) && !refused_as_view(r.0, r.1))
        .map(|(start, end)| (start / PAGE, end.div_ceil(PAGE)))
        .collect();
    pieces.sort_unstable();
    pieces.dedup();
    // The allocations do not change while they are looked at and asked
    // about (not waited for: no asking stands then, and `writes` asks).
    let Some(_mapping) = title_memory_reading_now() else {
        return;
    };
    let (asked, written) = run(&pieces);
    note_hot(&written);
    let mut j = journal_mut();
    // A number of its own, as `ask_rest` gives it.
    let now = next_asking();
    j.ahead_asked = now;
    for (page, parts) in written {
        j.note(page, parts, now);
    }
    j.kept = !asked.is_empty();
    j.ahead = asked;
}

/// The askings for `pieces` (page ranges, in order): the page ranges that
/// stand asked, in order, and the pages found written.
fn run(mut pieces: &[(usize, usize)]) -> (Vec<(usize, usize)>, Vec<(usize, u16)>) {
    let (mut asked, mut written) = (Vec::new(), Vec::new());
    let mut allocation: Option<(usize, usize, bool)> = None;
    STATS[3].fetch_add(pieces.len() as u64, Relaxed);
    while let Some(&(first, end)) = pieces.first() {
        if allocation.is_none_or(|(a, b, _)| first < a || first >= b) {
            // (Asked only where the range could join the next.)
            allocation = if pieces.get(1).is_some_and(|&n| can_join(first, end, n)) { region(first) } else { None };
        }
        let (_, limit, private) = allocation.unwrap_or((first, end, true));
        let mut last = end;
        let mut n = 1;
        while let Some(&next) = pieces.get(n) {
            if last > limit || next.1 > limit || !can_join(first, last, next) {
                break;
            }
            last = last.max(next.1);
            n += 1;
        }
        if private && last <= limit && last - first <= AHEAD_MAX {
            STATS[0].fetch_add(1, Relaxed);
            if let Some(pages) = kernel_written(first, last) {
                written.extend(pages);
                asked.push((first, last));
            } else if n > 1 {
                STATS[1].fetch_add(1, Relaxed);
                for &(a, b) in &pieces[..n] {
                    if b - a <= AHEAD_MAX
                        && !asked.iter().any(|&(x, y)| x <= a && b <= y)
                        && let Some(pages) = kernel_written(a, b)
                    {
                        written.extend(pages);
                        asked.push((a, b));
                    }
                }
            }
        }
        pieces = &pieces[n..];
    }
    // (In order of their first page, as the groups come; the ranges asked
    // alone after a refusal lie inside their group.)
    asked.sort_unstable();
    (asked, written)
}

/// A write that the kernel may not have seen is done: title memory
/// `[at, at + len)`. Of its pages, those under the asking that stands are
/// noted written after it, so that the refreshes that follow copy them.
pub(super) fn completed_write(at: usize, len: usize) {
    if len == 0 || !enabled() {
        return;
    }
    let (first, last) = (at / PAGE, at.saturating_add(len).div_ceil(PAGE));
    let mut j = journal_mut();
    if !j.kept {
        return;
    }
    let under: Vec<(usize, usize)> =
        j.ahead.iter().map(|&(a, b)| (a.max(first), b.min(last))).filter(|&(a, b)| a < b).collect();
    if under.is_empty() {
        return;
    }
    let now = next_asking();
    j.ahead_asked = now;
    for (a, b) in under {
        for page in a..b {
            j.note(page, WHOLE, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree};
    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    /// Write-watched memory of ours, `pages` of [`PAGE`], committed where
    /// asked.
    struct Memory(usize);

    impl Memory {
        fn new(pages: usize) -> Self {
            // SAFETY: a reservation of ours.
            let at = unsafe { VirtualAlloc(std::ptr::null(), pages * PAGE, MEM_RESERVE | MEM_WRITE_WATCH, PAGE_READWRITE) };
            assert!(!at.is_null());
            Self(at as usize)
        }

        fn commit(&self, page: usize) -> usize {
            let at = self.0 + page * PAGE;
            // SAFETY: a page of our reservation.
            assert_eq!(unsafe { VirtualAlloc(at as *const _, KERNEL_PAGE, MEM_COMMIT, PAGE_READWRITE) } as usize, at);
            at
        }
    }

    impl Drop for Memory {
        fn drop(&mut self) {
            // SAFETY: our reservation.
            assert_ne!(unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) }, 0);
        }
    }

    /// Two ranges of one allocation with a written page between them: one
    /// asking, which notes the page between too; ranges in two allocations:
    /// two askings, none refused.
    #[test]
    fn ranges_are_asked_by_allocation_and_the_pages_between_noted() {
        let memory = Memory::new(5);
        let a = memory.commit(0);
        let gap = memory.commit(2);
        let b = memory.commit(4);
        // SAFETY: committed pages of ours.
        unsafe { (gap as *mut u8).write_volatile(1) };
        let pieces = [(a / PAGE, a / PAGE + 1), (b / PAGE, b / PAGE + 1)];
        let (asked, written) = run(&pieces);
        assert_eq!(asked, vec![(a / PAGE, b / PAGE + 1)]);
        assert!(written.iter().any(|&(page, _)| page == gap / PAGE));
        // Asked again: the bit was reset.
        assert!(run(&pieces).1.iter().all(|&(page, _)| page != gap / PAGE));

        let other = Memory::new(1);
        let c = other.commit(0);
        let mut two = vec![(a / PAGE, a / PAGE + 1), (c / PAGE, c / PAGE + 1)];
        two.sort_unstable();
        let (asked, _) = run(&two);
        assert_eq!(asked.len(), 2, "{asked:?}");
    }

    #[test]
    fn a_range_joins_within_the_gap_or_the_smaller_size() {
        assert!(can_join(0, 1, (1 + AHEAD_GAP, 2 + AHEAD_GAP)));
        assert!(!can_join(0, 1, (2 + AHEAD_GAP, 3 + AHEAD_GAP)));
        let big = 4 * AHEAD_GAP;
        assert!(can_join(0, big, (2 * big, 3 * big)));
        assert!(!can_join(0, AHEAD_MAX, (AHEAD_MAX, AHEAD_MAX + 1)));
    }
}
