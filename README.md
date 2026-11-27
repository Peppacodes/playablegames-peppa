# playablegames-peppa

**The game's world works on NVIDIA at 23 fps, confirmed on an RTX 5080
(2026-11-27).** This tree runs the game's own executable,
`game_scarlett_master.exe` (build 1.0.2.1, an Xbox Series title), from the
user's own copy: from a save into the open world, in-game and playable with a
controller, at 23.5 fps held over a 12 minute run standing in a house interior
(three shorter runs of the same build: 22.8 to 23.9 fps).

| Milestone | Status |
|---|---|
| The game starts, first frame, main menu | Works |
| In the world, from a save | Works |
| Playable with a controller | Works: a 55 minute session without a crash |
| Story progress | First missions: the first mission completed, side tasks completed, and driving around the world by day and by night (the owner's own play) |
| Performance | 23.5 fps average over 12 minutes in a house interior (three 5 minute runs: 22.8 to 23.9), on a named PC: RTX 5080 16 GB, Ryzen 7 5700X3D, 32 GB RAM, Windows 11; the goal is 30 |
| Rendering | Lit and whole, with known faults; ray tracing is off |
| Stability | A 55 minute session without a crash; a night of 61 scripted runs with no lost device and no driver error; memory at about 28 GB |

That is on an NVIDIA RTX 5080 with 32 GB of RAM. Figures, method and known
faults: [Where it stands](#where-it-stands-2026-11-27).

A fork of [VIctory](https://github.com/Marvisak/VIctory), a compatibility layer
that runs an Xbox Series title natively on Windows from files the user supplies.
It loads the title's executable and answers its calls to the console's own
libraries (`xgameruntime`, `d3d12_xs`, `xmem`) with Windows and Direct3D 12 on
the PC. It carries [fxceless/victory-fork](https://github.com/fxceless/victory-fork)
and [avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)
(speed on AMD RDNA 3), with fixes for NVIDIA cards and 32 GB of RAM on top.

**AMD and NVIDIA cards are both supported**: the AMD work this tree carries is
kept. **For now, all testing and new work here is on NVIDIA.** The goal is 30
real frames a second (a 33 ms frame) in the world on an RTX 5080. AMD results and
fixes are welcome, but they are not tested here.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## Where it stands (2026-11-27)

Measured on one PC: Windows 11, RTX 5080 16 GB (driver 621.24), Ryzen 7 5700X3D,
32 GB RAM. Series S profile, the title's own upscaler at 2/3 scale (720p drawn,
1080p shown), ray tracing left out. Loaded from a save into a house interior
(about 5,200 draws a frame), the character standing, a button pressed every 10 s
so the title keeps its gameplay camera, frames counted from 110 s after launch,
after the driver had finished compiling shaders.

| Build and settings | ms a frame (median) | fps |
|---|---|---|
| This tree with `Play-VIctory.cmd`'s settings, a 12 minute run | 42.5 (p10 41.1, p90 46.2) | 23.5 |
| The same, three 5 minute runs | 43.9, 41.8, 42.7 | 22.8-23.9 |
| The same before the title's submissions were merged (pull request 20), a 12 minute run; three 5 minute runs | 46.0; 46.5, 49.4, 49.7 | 21.7; 20.1-21.5 |
| The same before the allocator change and the copy-back skip (pull request 17), a 12 minute run | 53.5 (p10 51.2, p90 55.5) | 18.7 |
| The same code before the five switches and the 2026-11-27 changes (pull requests 1, 2 and 4), four runs | 68-113 | 9-15 |
| The tree as it was on 2026-11-25 (Series S with the title's FidelityFX upscaler), two runs | 138.0, 138.8 | 7.2 |

How to read these. They are one PC's figures. The same build lands on different
medians from load to load (three 4 minute runs of the fourth row's build: 56.3,
50.0 and 58.2 ms), so each switch was judged inside one run, flipped every 10 s
(`VICTORY_AB=<seconds>,<SWITCH>`, diagnostics builds), over 5 to 11 cycles. The
GPU crate's unit tests run with
`cargo test -p victory-gpu --lib --features "diagnostics mimalloc"`: 440 pass,
and four known ones fail (two of them on the base tree too). The
process holds about 28 GB of private memory over 12 minutes (27.9 GB after the
first minute, 28.9 at its highest, 28.4 at the end). Earlier figures (2026-11-26, Series X, the first room):
61-67 ms for this tree against 77.3 for avatarmanz's on the same PC.

The Rust side of the layer allocates through mimalloc, not the Windows process
heap it shared with the title and the driver. It is on by default in the build
(`VICTORY_ALLOC=system` goes back for one run): about 2 ms off the median over
three alternated pairs of runs (48.5 against 50.2 ms), the same in the time the
layer spends preparing a frame. The submitting thread's time inside ntdll, where
the process heap lives, fell from 42-44% to 32-34%, with about 3% now in
mimalloc's own code. A first figure of 5.6 ms, from two pairs, was mostly the
difference between loads.

The switches the launcher sets, all measured on this card with the in-run A/B:

| Switch | What it does | Effect |
|---|---|---|
| `VICTORY_BURY_ASIDE=1` | Host objects that waited for a flight are dropped on a thread of their own, not on the thread that tells the title the card is done | -23 ms mean |
| `VICTORY_FILL_TITLE=2` | Small constant fills do not wait for copy-back when no shader wrote that page in the last 600 frames (a constant a shader does write there could be read one flight late; none seen) | -11 ms |
| `VICTORY_PORT_GUARD=1` | The write guards' lock is shared by faults and refreshes | -3 to -4 ms |
| `VICTORY_LAND_ASIDE=1` | Landed lists are reset on a thread of their own | -2 to -4 ms |
| `VICTORY_PLACED_ORDER=1` | A copy at its place in a list holds back the card's steps only if they stand behind it, not the whole submission | -2 ms |
| `VICTORY_SKIP_WC_BACK=1` with `VICTORY_SKIP_WC_KEEP=1048576` | What shaders wrote to write-combining title memory stays on the card; declared ranges up to 1 MiB are still fetched back | -3.9 ms, in 44 of 44 cycles |
| `VICTORY_MERGE_SUBMISSIONS=1` | Consecutive submissions of the title go to the card as one: about 41 a frame become about 8 | -1.9 ms mean, in 10 of 11 cycles |

About the copy-back row: the 1 MiB keep is there because the title's CPU code reads
small ranges back (its exposure, counts and visibility lists); without the
second switch the picture is wrong (overexposed, blocks). Where the title's CPU
code writes part of a larger piece the card wrote, stale bytes could go up; none
showed in the captures of five runs, the longest 12 minutes. The 1 MiB figure
comes from Zignixx's RTX 5080 write-up in fxceless/victory-fork issue 1379.

What sets the frame now: one thread of ours, the one that prepares each
submission of the title's work for the card. It is busy the whole frame, and the
title, the list threads and the card (about 18 ms of the frame) all wait for it.
A census of that thread's work (`VICTORY_WORK_CENSUS`) puts most of what is left
in the bytes its refreshes copy, about 47 MB a frame, not in the pages walked or
in repeated requests.
Tried on this card without a gain, so that nobody has to again:
`VICTORY_PORT_CONSTANTS` (the card then reads the constants over the bus: 126 ms
a frame), `VICTORY_PORT_DESCRIPTORS`, a thin-LTO AVX2 build, 6 or 12 list threads
instead of 8, `VICTORY_LANE_CORE`, `VICTORY_CARD_BUFFERS_MIN_KB=256`,
`VICTORY_VIEW_BYTES=0`, `VICTORY_COLD_SKIP=1` with `VICTORY_FEED_ONE_CALL=1`
(0.6 ms together; both are in the tree, off), and two ways of giving that thread
a helper thread (draft pull requests 13 and 16: 7 and 2 ms slower, because the
processor is already near full). Also in the tree and off, each within half a
millisecond in its A/B: `VICTORY_SCRATCH`, `VICTORY_REFRESH_STAMP`,
`VICTORY_COMPARE_JOIN` with `VICTORY_READBACK_POOL` and
`VICTORY_WATCH_RESERVATION`, and `VICTORY_GUARD_BITS` with
`VICTORY_WATCH_TEXTURES_JOINED`, `VICTORY_SPARSE_REFRESH` and
`VICTORY_WRITE_JOBS`. `VICTORY_QUIET_WINDOWS=check` found no quiet window here.

What is known to be wrong or missing on NVIDIA:

- The same room loads bright in one run and dark in the next, in every build
  measured, the 2026-11-25 tree included. The title's own exposure settles on a
  different level each load (its read-back shows it); no cause on this side has
  been found yet.
- Every run logs a few hundred "title slot past the host heap" warnings: NVIDIA
  caps a view heap at 1,000,000 slots, the title's has 1,245,184, and 250,000 of
  them are fitted in place. `VICTORY_DYNAMIC_SLOTS=262144` goes back to about
  738,000 fitted and the warnings stop; the picture and the frame time were the
  same in two runs.

- Four compute programs written for 64-lane waves have no form for NVIDIA's
  32-lane waves and are left out (about one dispatch a frame each). What they
  draw is not known yet. The AMD tree's form of them is refused by the NVIDIA
  driver as well. Work on such programs elsewhere: avatarmanz/VIctory-7900XTX
  #152, #142 and #134; fxceless/victory-fork #1248, #1249 and #1261.
- Without the ray-tracing structure builder, the ray-tracing resolve and ray
  activation programs are left out on purpose: they have no hits to shade
  (`VICTORY_RT_RESOLVE=1` and `VICTORY_ACTIVATE_RAYS=1` bring them back).
- 32 GB of RAM is tight: the process used about 31.8 GB of private memory on
  Series X.
- The first runs of a new build or a new driver are slow while the driver
  compiles shaders in the background (20 minutes or more). The NVIDIA driver
  keeps them by the executable's path, so run a rebuilt exe from the same path.
- From the trees it carries: trees and far buildings flicker in places,
  rooms with little direct light are too dark without ray tracing, and textures
  that just came into view appear late.

## How to run it

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, Rust (the toolchain is pinned), the MSVC C++ build
tools with a Windows SDK, and your own dump of the title. There are no prebuilt
downloads: build it from source.

```powershell
git clone https://github.com/Peppacodes/playablegames-peppa
cd playablegames-peppa
cargo build --release --locked -p victory
$env:VICTORY_GAME_DIR = 'D:\Games\MyDump'    # your own dump, outside this folder
.\Play-VIctory.cmd pareto-residency
```

The launcher picks the Series S profile and the settings the figures above were
measured with (the title's own upscaler at 2/3 scale, the measured switches, card
buffers and pipeline settings from the RTX 4080 recipe in avatarmanz's issue
143), and keeps its shader cache under `%LOCALAPPDATA%\VIctory\cache-pareto`. A
setting you set yourself before starting it wins. If the title's own watchdog
ends a run (it can while frames are slow), set `VICTORY_NO_WATCHDOG=1` first.

The diagnostics build and avatarmanz's launcher work too:

```powershell
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
scripts\play.ps1 -Exe <path of your copy's executable>
```

`scripts\play.ps1 -Safe` leaves out the newest, faster ways; try it first when
something is wrong. Before the first run, `target\diag\victory.exe shaders <the
shader archives>` translates every shader ahead of time and
`target\diag\victory.exe warm --used` makes the pipelines earlier runs drew with.
Commands and debugging switches: [docs/README-fxceless.md](docs/README-fxceless.md).
Every `VICTORY_...` switch is described in a comment where it is read.

## Helping

Results, bug reports and pull requests are all welcome.

- **A result** (an issue): your card, CPU, RAM, driver version, console profile,
  resolution, the switches you set, the median ms a frame once the driver had
  stopped compiling, and whether the picture was right.
- **A pull request**: one change per pull request; say what it fixes, how you
  measured it (before and after, same build path, same place in the game) and on
  which card. When you port a change from another tree, name its commit and its
  author.
- Never add game files or anything made from them
  ([docs/asset-policy.md](docs/asset-policy.md)), and keep the title's name out of
  code, commits and pull requests.

## What is different here

- **NVIDIA and 32 GB fixes on avatarmanz's tree** (made before this repository;
  the history has their commits): dynamic descriptor slots given back at
  their flight's fence, which makes thin recording (`VICTORY_THIN`) work on
  NVIDIA; residency budgets for 32 GB hosts; wave32 forms of more programs for
  NVIDIA; the Series S two-plane black screen fixed; one oversized indirect record
  no longer empties the rest of its list (`VICTORY_INDIRECT_REST=0` by default);
  ray-tracing resolve left out without the structure builder; the
  `Play-VIctory.cmd` launcher. Notes: [docs/bug-journal.md](docs/bug-journal.md).
- **Added here**: an opt-in hook for the title's watchdog (`VICTORY_NO_WATCHDOG=1`,
  build 1.0.2.1), and 64 MB stacks for the pipeline workers, since NVIDIA's
  compiler overflowed the default 2 MB on a world compute program.
- **Added here on 2026-11-26 and 27**, each from someone's public work and
  named in its commit: avatarmanz's 25 commits after the base (heap records read
  in place, uploads on a copy queue, pooled textures); from
  trustno1-coder/VIctory-NVIDIA the title's own upscaler and render scale on
  every profile, faces that move (its port of fxceless pull request 927 by
  tippythetaidum), the intro video fix and the choice of card; avatarmanz pull
  requests 24 (UwUCammie), 42 (gomeaw), 82 (sawetco), 96 and 97 (kalijerry), 105,
  107 and 108 (SousaTrashBin), 109 (rivaldodev), 114 (secure-silicon) and 119
  (ESAT586); fxceless pull requests 868 (Chipsraine: the census and the
  write-combining copy-back experiment), 1224 (Steenax6: `VICTORY_BURY_ASIDE`)
  and 1146 (beamdotdev: `VICTORY_LAND_ASIDE`); avatarmanz's renderer patches
  (`VICTORY_PORT_CONSTANTS`, `VICTORY_PORT_DESCRIPTORS`, `VICTORY_PORT_GUARD`);
  trustno1-coder/VIctory-NVIDIA pull request 11 (Dastano: mimalloc); fxceless
  pull requests 1358 (TG0000: `VICTORY_COLD_SKIP`) and 1137
  (`VICTORY_FEED_ONE_CALL`); Marvisak/VIctory pull request 3384 (borosf: the
  sorted snapshot pool); and two small changes after ZigZagBoi1's and
  XaviFortes's public trees (`ask_ahead` returning early, a lock-free
  `has_pending`); LooperSalty/AnyXboxX pull requests 337 and 362 (Skyfrye:
  `VICTORY_MERGE_SUBMISSIONS`, `VICTORY_WRITE_JOBS`); fxceless pull requests
  1359 (fxceless), 1365 and 1406 (spk-c); Marvisak/VIctory pull request 3379
  (Texbirds); and changes after the public trees of DoubleGhosty (the joined
  compares), KevinPratap (`VICTORY_GUARD_BITS`, the textures' one asking),
  ka1qux (`VICTORY_QUIET_WINDOWS`) and XaviFortes (the last-ahead cache).
  Written here: the card copies' scans narrowed to the copies touched,
  `VICTORY_SKIP_WC_KEEP`, and timeline diagnostics that sample every thread
  (`VICTORY_TIMELINE_ALL`, `VICTORY_TIMELINE_FAST`).
- **From avatarmanz/VIctory-7900XTX**, in short (the reasons and measurements are
  in [docs/findings.md](docs/findings.md)): list calls written down on the
  title's threads and translated on eight of ours (`VICTORY_THIN`); submissions
  prepared on a thread of ours and joined (`VICTORY_ASYNC_SUBMIT`,
  `VICTORY_JOIN_PARTS`); what shaders wrote brought back beside the write guards
  (`VICTORY_RELAY`, `VICTORY_LAND`); buffer fills done on the card
  (`VICTORY_CARD_STEPS`); cheaper catching of the title's writes; textures uploaded
  on a copy queue of their own; textures that shaders write and the title reads
  sent back to title memory; frame record and replay ([docs/replay.md](docs/replay.md)).

## Credits

This is a tree of other people's work with more on top. The history is kept, and
commits taken from elsewhere name the commit and its author.

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end and
  the shader recompiler.
- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**
  by avatarmanz: the speed work this tree is built on, and the bringing together
  of the forks below.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**: the fork
  avatarmanz started from, and the people whose pull requests it carries.
- **[trustno1-coder/VIctory-NVIDIA](https://github.com/trustno1-coder/VIctory-NVIDIA)**:
  NVIDIA work on avatarmanz's tree (render scale, own upscaler, faces, the intro
  video), with patches it took from vitutu1's victory-perf and ZigZagBoi1's
  ScarlettPort.
- **Proudnoob4**, for the RTX 4080 recipe in avatarmanz's issue 143, which named
  most of the pull requests brought in here; and their authors: UwUCammie, gomeaw,
  sawetco, kalijerry, SousaTrashBin, rivaldodev, secure-silicon, ESAT586,
  Chipsraine, Steenax6, beamdotdev, tippythetaidum.
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  cube gradients, sparse textures.
- **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**:
  the ambient light idea, pipeline workers, and work by romainhedouin.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read in avatarmanz's
  tree for how the same title behaves under another layer.
- Changes in avatarmanz's tree and here were made with
  [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
