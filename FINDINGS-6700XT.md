# RX 6700 XT findings: `game.exe` device removal

Scope: title `D:\kyoto-dump-gta\game.exe` (title ID 7952987A) on this machine, launched with `scripts\play.ps1`.

## Summary

- The host device was removed (`0x887A0005`, then `0x887A0007`) during the first world load, about 60–90 s after start, whenever submissions were sent ahead (`VICTORY_LATE_WAIT` > 0, which `VICTORY_RELAY` turns on).
- **Cause, from a GPU ETW trace:** the video memory manager runs out of room in a **2 GB aperture segment** (system memory the card maps; segment 2 to `D3DKMTQueryStatistics`, 3 in ETW). It holds the driver's command buffers (1 MB + 64 KB per D3D12 command list, growing with what a list records), our upload scratch chunks and our `D3D12_HEAP_TYPE_CUSTOM` write-back buffers. The next allocation that can only live there cannot be made resident (`ReserveResource` → `0xC0000001`, 46 tries), and the kernel scheduler marks our device as faulted (`VidSchMarkDeviceAsError`, reason 12). No GPU hang, no page fault, no timed-out packet.
- With send-ahead the host lists climbed from ~830 to ~1,150–1,250 at the world load: `land()` held the newest 64 flights of finished lists (up to 716 lists), and up to 240 title lists were in flight. With every submission waited for (`VICTORY_RELAY=0`) the count stays at 674 and the segment never fills.
- **Fixed** (commits `f173347`, `018d372`, `e3c0703`, `63a773e`): finished lists go back to the pool at once; preludes are shared from a pool instead of one per host list for life (101 made instead of ~950); and while the segment is above 80% every flight lands as it is sent and idle lists are trimmed. With the 10–12 GB card-buffer limits at 1 MiB / 1024 MiB (from PR #225 by @Zayfway, with its cap lowered for this card), default `play.ps1` settings get through the world load at **8.8–12.6 fps (~100 ms/frame) in the world, no device removal**, against 2.8 fps with `VICTORY_RELAY=0`. The back-off no longer fires.

## Hardware and system

- AMD Radeon RX 6700 XT, vendor 0x1002, device 0x73df, 12243 MB VRAM. Driver 32.0.21045.29003 (2026-11-17).
- Card memory budget 11475 MB at start, cut by the system to ~10400 MB at the world load in every run.
- Time zone GMT (UTC+0): Windows event times and Victory's `Z` times are the same clock.
- TDR registry values unset (TdrDelay default, 2 s). `HwSchMode` unset (system default).
- System log: unexpected shutdown 09:55:21 and unclean reboot 10:33:46 (Event 41, 6008). No BugCheck (1001), no WHEA, no display TDR (4101) in the last two days. Every device removal (11:49 onward) came after that reboot. Not linked to the removals.

## Runs

All runs were 150 s maximum, using this fork's `target\diag\victory.exe` (built 13:46Z from `c9c932c`) unless noted. Logs are in `target\run-*.log` (gitignored).

| Settings (`-More`) | Budget cut at | Removed at | Gap | Result |
|---|---|---|---|---|
| `VICTORY_RELAY=0` | 14:02:47 | — | — | Survives; 2.8 fps; 10.2 of 10.5 GB in use at the end |
| `VICTORY_RELAY=0`, `VICTORY_D3D12_DEBUG=1` | 13:48:46 | — | — | Survives; ~2 fps |
| defaults (relay 2), `VICTORY_D3D12_DEBUG=1` (7900XTX build) | 11:54:45 | 11:54:49 | 4.5 s | Removed |
| `VICTORY_RELAY=1` (7900XTX build) | — | 11:53:34 | — | Removed |
| `VICTORY_RELAY_COPY=0` | 13:51:26 | 13:51:30 | 3.3 s | Removed |
| `VICTORY_TEXTURE_SWEEP=0` | 13:53:06 | 13:53:09 | 3.0 s | Removed |
| `VICTORY_DISPLAY_MODE=1920x1080` | 13:55:04 | 13:55:07 | 2.5 s | Removed; memory use unchanged |
| `VICTORY_LATE_WAIT_LISTS=16` | 13:56:25 | 13:56:27 | 2.6 s | Removed |
| `VICTORY_CARD_AWAKE=0` | 13:59:45 | 13:59:49 | 3.3 s | Removed |
| `VICTORY_RELAY=0`, `VICTORY_LATE_WAIT=3` | 13:58:07 | 13:58:09 | 2.7 s | Removed |
| `VICTORY_RELAY=0`, `VICTORY_LATE_WAIT=1` | 14:01:12 | 14:01:26 | 14.0 s | Removed; 5 submissions (62 lists) in flight |
| defaults + experimental "settle every flight while card memory moves" patch (reverted) | 14:10:40 | 14:10:44 | 3.2 s | Removed; the patch was active the last 13 s, every flight waited for as soon as it was sent |
| code defaults, no `play.ps1` switches (only `VICTORY_D3D12_DRED=light`) | 14:14:10 | 14:14:24 | 14.0 s | Removed; 2 submissions in flight |

Ruled out as the trigger: the relay thread, its copy-queue fetch (`VICTORY_RELAY_COPY`), the texture host-copy sweep, the keep-awake compute queue (`VICTORY_CARD_AWAKE`), the number of lists in flight, the reported display size, the rest of `play.ps1`'s switches, and our work being queued on the card while memory moves (the patch run).

### Other fork: `codec-d--cool_fox` (`f4b48ad`, pareto-residency)

Release build (`cargo build --release --locked -p victory`), `Play-VIctory.cmd`'s settings (Series S, its switches) pointed at `game.exe` (the launcher only accepts `game_scarlett_master.exe`), `VICTORY_LOG=info`. Its 32 GB residency work does not avoid the failure:

| Settings | Budget cut at | Removed at | Result |
|---|---|---|---|
| launcher defaults (relay 2) | 14:18:02 (then back and forth) | 14:18:29 | Removed; 12 submissions (126 lists) in flight, nothing of ours running |
| launcher defaults + `VICTORY_RELAY=0` | 14:19:41 | — | Survives 150 s; 3.0–3.4 fps in the world (~70 ms preparing, ~120 ms on the GPU, ~118 ms copying back a frame) |

### PR #225 (card-buffer limits for 10–12 GB cards), applied uncommitted to `shadow.rs`

| Build / settings | Card buffer limits (log) | Result |
|---|---|---|
| before the fix, `play.ps1` defaults | min 64 KiB, max 1536 MiB | Removed 7 s after the budget cut; 1,173 host lists alive |
| with the holding back-off | min 64 KiB, max 1536 MiB | Survives 180 s; **1.2 fps** (796 ms a frame on the GPU): the card's memory 11,679 MB in use against a 10,097 MB budget |
| with the holding back-off | min 16 MiB, max 512 MiB (the overrides, as before the PR) | Survives 180 s; **4.7 fps** (127 ms on the GPU), 10,077 MB in use against 9,997 MB |

On this card the PR's cap hurts: the system cuts the budget to ~10.0–10.4 GB at the world load, and 1,536 MiB of card buffers put the card 1.6 GB over it, so the video memory manager pages and the card slows five-fold. Its tier is kept with a 1,024 MiB cap and a 1 MiB minimum (see below).

### Card-buffer limits on this card (prelude pool build, default `play.ps1` settings otherwise)

| Limits | Snapshots a frame | Preparing | fps in the world | Card memory in use / budget at the end |
|---|---|---|---|---|
| 16 MiB / 512 MiB (before) | 67.5 ms | ~125–140 ms | 5.5–7.4 | 10,347 / 10,085 MB |
| 1 MiB / 512 MiB | 67.8 ms | 111–148 ms | 5.6–7.0 | 10,339 / 10,146 MB |
| 1 MiB / 1024 MiB | 2.7–4.0 ms | 28–75 ms | **8.8–12.6** | 11,081 / 10,084 MB |
| 64 KiB / 1536 MiB (PR #225 as it was) | — | — | 1.2 (796 ms on the card) | 11,679 / 10,097 MB |

The minimum does not matter here (512 MiB is filled by the big copies either way); the cap does. A copy is made up to 97% of the budget at the moment, and the copies come in a burst right before the system cuts the budget, so the card ends over the cut budget by about the copies' bytes; at 1 GB that still holds steady (18–72 ms a frame on the card), at 1.5 GB it pages. Making copies only up to 88% of the budget changed nothing (the burst comes while the card is far below it), so that was not kept.

## GPU ETW trace (`wpr -start GPU -filemode`, default settings)

Trace start 14:24:29.459 UTC. Budget cut at +80.71 s, removal logged at +84.73 s. Exported +79.5..+85.0 s with `xperf -a dumper`.

- `VidSchMarkDeviceAsError` on our device: +84.7253 s (System, reason 12), +84.7257 s (victory.exe, user mode, reason 6), +84.7435 s (reason 14).
- Right before the first: 46 `ReserveResource` calls for allocation `0xffffd78a1e84f1a0` in segment 3, all `0xC0000001`, inside a paging-queue packet.
- That allocation: 1 MB, flags `0xA001`, write-segment mask 4 (segment 3 only), made by victory.exe 4.5 ms earlier. Stack: `Host::submit` → `HostList` prelude (`OnceLock::get_or_init`) → `Host::make_prelude` → `ID3D12Device::CreateCommandList` → `amdxc64.dll` (symbolized with `dbghelp` against `target\diag\victory.pdb`).
- Segment 3 resident bytes: 1,253 MB at +79.5 s, 1,651 MB at +82.5 s, 2,038 MB at +84.5 s (max seen 2,046 MB). Segment 2 (card memory) ~9.9 GB, segment 5 ~5.3 GB.
- Segment-3-only allocations made in those 5.5 s (1,829 MB in all):

| Count | Size | Flags | Made by |
|---|---|---|---|
| 899 | 1 MB | `0xA001` | `Host::new_list` (command list) |
| 900 | 64 KB | `0x20100015` | `Host::new_list` (command list) |
| 151 + 13 | 4 MB, 8 MB | `0x2005` | `Host::shared_buffer` (`CUSTOM`, write-back, L0) from `shadow::loose` |
| 184 + 15 | 256 KB, 4 MB | `0x2001` | `Host::upload_buffer` from `HostList::scratch_at` |
| 1 | 32 MB | `0x2005` | `Host::shared_buffer` from `Host::relay_land` |

- No `DmaPacket` with a faulting address, no `QueuePacket` stop with `bTimeouted`.

Host lists alive (memory line): relay 0 holds 674. Every send-ahead run: ~830 before the world load, 1,127–1,198 at the removal, including the run where every flight was settled as soon as it was sent. Preludes are few (239 for 1,127 lists), so the 1 MB allocations are the lists themselves, made again and again.

## Fix progress

Where the host lists are (a temporary log at each 25th new host list, default settings, run `run-lists-diag`): before the fix, idle 0, in flight 0–32 title lists, queued submissions 12–34, and **landed but not given back up to 716**. `land()` kept the newest `bury_lag()` (64) entries of the landed line, and an entry is a whole flight of lists. The rest (about 650) are the title's own lists, one host list each, as with relay 0.

| Build / settings | Budget cut at | Removed at | Lists alive at the end | fps in the world |
|---|---|---|---|---|
| before, `VICTORY_BURY_LAG=8` | 14:50:45 | 14:51:29 | 1,256 (180 title lists in flight) | 0.6–30 |
| landed lists given back at once (`land()` → `list::recycle_landed`, what they held kept through `retire`) | 14:54:49 | 14:55:15 | 1,235 (224 in flight) | 3.7–30 |
| + back-off on a full aperture segment (`host/segments.rs`, `018d372`) | 15:00:25 | 15:01:13 | 1,208 (239 in flight) | 6.7–30 |
| + back-off on every send, all flights land while tight, idle pool trimmed to 64, on at 80% / off at 70% (`e3c0703`) | 15:18:59 | — (180 s) | 1,052 | **4.5–4.7 steady** (with PR #225 card limits: 1.2) |
| + preludes from a pool (taken at submission, given back at the list's reset; 128 kept idle, 16 while tight) | 15:46:55 | — (180 s) | 1,218; **101 preludes** made (was ~950) | **5.5–7.4** (~145 ms/frame: ~125 ms preparing, the card waited for ~0); the back-off never fired |

Segments as the graphics kernel reports them on this PC (`D3DKMTQueryStatistics`, logged at the start):

| id | aperture | limit | resident at start |
|---|---|---|---|
| 0 | no | 241 MB | 75 MB |
| 1 | no | 12,001 MB | 1,510 MB |
| 2 | yes | **2,048 MB** | 20 MB |
| 3 | yes | 30,610 MB | 0 MB |
| 4 | yes | unlimited | 90 MB |

Segment 2 (ETW's segment 3) is the one that fills. With `018d372` the back-off first fired at 1,911 MB (93%): it looked at the segment only when the idle pool was empty, and the in-flight count kept growing. With the back-off on every send it fires at ~1,650 MB, trimming the idle pool frees ~250–300 MB (1,650 → 1,350–1,430 MB), and the segment stayed under 1,700 MB.

## What every failing run shows

- Memory ramp at the world load: card memory in use goes from ~3 GB to ~9.2 GB, and shared system memory from ~1.7 GB to ~7 GB, in about 10 s. Then the system cuts the budget by ~1 GB. The surviving relay-0 run goes through the same ramp and cut.
- Breadcrumbs empty. The removal report says nothing of ours was executing ("between submissions" or "preparing a submission"), except the late-1 run ("waiting for the card", 5 submissions in flight).
- Debug layer (`VICTORY_D3D12_DEBUG=1`, failing run): `DXGI_ERROR_DEVICE_RESET`: "the hardware took an unreasonable amount of time to execute a command on a different Device Context... The current Device Context was NOT executing commands when the hang occurred", plus "Kernel memory failure. There might be a memory leak" (id 834). The closed-command-list error (id 547) appears only in the same millisecond as the removal: it is a consequence (the prelude list could not be reset on the removed device).

## Hypothesis status

Confirmed by the trace: segment-3 exhaustion (above). Two earlier hypotheses were wrong:

- A paging timeout because our work is queued on the card: a patch that waited for every flight as soon as it was sent, whenever card memory moved, still lost the device 3.2 s after the budget cut (reverted).
- The relay, the sweep, the keep-awake queue, lists in flight: ruled out by the runs above.

The debug layer's "different Device Context" / TDR text was misleading; its "Kernel memory failure" (id 834) was the real signal.

Why only on this machine: segment 3's 2 GB cap. [INFERENCE] The driver sizes it from system RAM (32 GB here); the 7900 XTX reference machine has 64 GB. Not measured there.

## Debug-layer errors present in the surviving run too

These are real bugs but not the trigger (they appear with relay 0, which survives):

- id 646: `SetComputeRootDescriptorTable`: UAV descriptors placed in a range declared SRV (145 times in 150 s).
- id 538: `SetComputeRootShaderResourceView` on a buffer in `COPY_DEST` (promoted from COMMON): resource `title buffer 0xdb790df00+0x17ebf00 on the card` (222 times; only with relay 0).
- id 971: a pipeline library load for a name that is not stored (2 times).

## 1080p output and the 7900 XTX fork's renderer patches

`VICTORY_OUTPUT=1920x1080` (the title renders 1080p in the Series X profile), default settings otherwise:

| Run | World samples (fps, `scripts\fps-milestones.ps1`) | Card memory in use / budget |
|---|---|---|
| `run-out1080` | 10.0 5.7 10.4 10.7 11.0 8.9 15.1 13.8 15.0 16.2 12.9 11.6 14.5 14.6 13.2 13.7 13.1 13.3 12.5 9.9 8.3 | 8,967 / 10,191 MB |
| `run-out1080-b` | 10.5 5.5 9.0 6.9 4.3 12.7 16.0 16.3 16.4 16.6 16.3 15.7 6.5 13.6 14.7 14.0 14.4 13.7 12.7 13.4 12.4 | 9,315 / 10,175 MB |

The card is under its budget and about 57–67% busy; the frame (62–80 ms) is set by preparing on the submitting thread (~60 ms: refreshes 24, waiting for the heaps' records 13, title bytes under flights 9, card copies 7, the write watch 6). Range 15 met; steady 10 not (81% and 76% of samples).

The renderer patches (off by default), each alone at 1080p, 150 s:

| Switch | Result |
|---|---|
| all five together | the title stopped on an access violation in its own code (read of 0x2c at image+0x164a298) before the world |
| `VICTORY_PORT_GUARD=1` | **the title stops on its own `int3`** (0x80000003 at image+0x1e6c29f) before the world: the one that breaks the five |
| `VICTORY_PORT_CBV=1` | runs, 1–3 fps in the world |
| `VICTORY_PORT_CONSTANTS=1` | runs, 0.4–16 fps over 50 s of world: no clear change |
| `VICTORY_PORT_DESCRIPTORS=1` | runs, 7–16 fps over 55 s: no clear change |
| `VICTORY_PORT_JOURNAL=1` | runs, 5–15.5 fps over 70 s: no clear change |

They stay off here.

## The Series S profile and its upscaler

`-Console series-s` (`play.ps1` passes `--console`) runs the steadiest, but its picture was soft, ghosted and stippled. The title's upscaler picker (`victory-loader/src/resolution.rs`) returns 6 (its own) on Series X and 7 (AMD FidelityFX) on Series S; both render at 2/3 of the output (1280x720 for 1080p). On Series S the title loads `amd_fidelityfx_upscaler_xbox.dll` from its own files (the host policy refuses a host copy); a window capture at 150 s showed a checkerboard-dithered copy of Lucia's head and shirt trailing her, and stippled walls: history that should have been rejected. The Series X runs never load it and show none of it. With `VICTORY_UPSCALER=6` (from pull request #53) the Series S profile uses the title's own upscaler: no ghost, no stipple, and no slower.

| Run | Upscaler | World samples at 10 fps or more | Range |
|---|---|---|---|
| `run-series-s` | FidelityFX | 31 of 33 (94%) | 3.3–15.6 |
| `run-series-s-b` | FidelityFX | 30 of 34 (88%) | 0.3–17.4 |
| `run-look-seriess-up6` | own | 29 of 32 (91%) | 6.0–18.0 |
| `run-seriess-up6-b` | own (`play.ps1 -Console series-s`) | 32 of 34 (94%) | 6.9–19.7 |

`play.ps1 -Console series-s` now sets `VICTORY_UPSCALER=6`; steady 10 is reached with it. Also tried, no gain: `VICTORY_SUBMIT_FRAMES=2` (the title two frames ahead of the submitting thread; 78%), and the three renderer patches that run here together (88%).

## List-translation threads (2026-11-27)

Every run from one copy of the saves (restored before each), `-Console series-s`, a street where the title makes about 52,500 list calls a frame. Upstream translates them on 8 threads of ours (`VICTORY_THIN_THREADS`); on this 8-core, 16-thread CPU they compete with the title's own threads.

| Threads | Runs | Median fps | ≥10 fps | CPU per list call | Landing thread, ms a frame | Card busy |
|---|---|---|---|---|---|---|
| 8 (upstream) | `run-base-a`, `-base-b`, `-port-a`, `-port-b` | 4.6–6.5 | 3–16% | 21–30 µs | 122–183 | 18–28% |
| 6 | `run-thin6` | 10.9 | 65% | 9.5 µs | 52 | 55% |
| 4 | `run-thin4`, `-thin4b` | 13.1, 12.9 | 74%, 98% | 5.8 µs | 42–44 | 60–63% |
| 3 | `run-thin3` | 11.9 | 97% | 4.7 µs | 32 | 58% |
| 2 | `run-thin2` | 9.8 | 26% | 3.9 µs | 28 | 47% |
| 4, the new default (a quarter of the logical cores) | `run-new-a`, `-new-b` | 13.0, 13.3 | 97%, 98% | 5.3–5.5 µs | 40–42 | 66% |

The four 8-thread runs are two of the build before the port of avatarmanz's five items (`dcdae92`) and two that were meant to be of the port but ran the earlier binary (a stray `CARGO_TARGET_DIR`). The `run-new` runs are of the port with the new default: against the earlier build at 4 threads, no change in frame rate (13.0 and 13.3 against 13.1 and 12.9).

## Next steps

1. Frame rate: at 1080p output the submitting thread's preparing (~60 ms) sets the frame; at native output (~100 ms a frame) the card (~80 ms of work, ~1 GB over the cut budget). Next: the refreshes and the wait for the heaps' records on the submitting thread. In the heavier street with 4 list threads the frame is about 77 ms and the card 66% busy.
2. The card's memory runs slightly over the cut budget late in runs (10,077 / 9,997 MB); keep an eye on the GPU time if it creeps up.
3. Fix the id 646 / id 538 validation errors separately.

## Relay and late-wait semantics (from code)

- `crates/victory-gpu/src/shadow.rs:924-931`: `shadow::relay()`: `VICTORY_RELAY` unset, `1` or `2` enables relay (and not with kept snapshots); `0` disables it.
- `crates/victory-gpu/src/host.rs:2497-2511`: `late_stage()`: `VICTORY_LATE_WAIT=1/2/3` picks the stage. Unset: 3 if relay is on, else 0 (send-ahead off).
- `host.rs:2587-2590`: `relay()` = `late_stage() > 0 && shadow::relay()`. `host.rs:2598-2604`: `relay_exact()` (stage 2) unless `VICTORY_RELAY=1`.
- `host.rs:2726-2731`: `VICTORY_LATE_WAIT_LISTS` (default 256).
- `host.rs:157-183`: `vram_keep()`: the texture sweep lets host copies go above 88% of the budget (down to 85%).

## Reproduce

```powershell
# fails ~60-70 s in (world load)
powershell -NoProfile -Command "& '.\scripts\play.ps1' -Exe 'D:\kyoto-dump-gta\game.exe'"
powershell -NoProfile -Command "& '.\scripts\play.ps1' -Exe 'D:\kyoto-dump-gta\game.exe' -More @{VICTORY_RELAY='0'; VICTORY_LATE_WAIT='3'}"
# survives
powershell -NoProfile -Command "& '.\scripts\play.ps1' -Exe 'D:\kyoto-dump-gta\game.exe' -More @{VICTORY_RELAY='0'}"
```

Code defaults without `play.ps1`'s switches (PowerShell, from the repo root):

```powershell
Get-ChildItem Env:VICTORY_* | Remove-Item; $env:VICTORY_D3D12_DRED = 'light'
.\target\diag\victory.exe run 'D:\kyoto-dump-gta\game.exe' --execute
```

Read the memory lines with:

```bash
grep -E "changed the budget|ERROR" target/run-*.log
```
