# RX 6700 XT findings: `game.exe` device removal

Scope: title `D:\kyoto-dump-gta\game.exe` (title ID 7952987A) on this machine, launched with `scripts\play.ps1`.

## Summary

- The host device was removed (`0x887A0005`, then `0x887A0007`) during the first world load, about 60–90 s after start, whenever submissions were sent ahead (`VICTORY_LATE_WAIT` > 0, which `VICTORY_RELAY` turns on).
- **Cause, from a GPU ETW trace:** the video memory manager runs out of room in a **2 GB aperture segment** (system memory the card maps; segment 2 to `D3DKMTQueryStatistics`, 3 in ETW). It holds the driver's command buffers (1 MB + 64 KB per D3D12 command list, growing with what a list records), our upload scratch chunks and our `D3D12_HEAP_TYPE_CUSTOM` write-back buffers. The next allocation that can only live there cannot be made resident (`ReserveResource` → `0xC0000001`, 46 tries), and the kernel scheduler marks our device as faulted (`VidSchMarkDeviceAsError`, reason 12). No GPU hang, no page fault, no timed-out packet.
- With send-ahead the host lists climbed from ~830 to ~1,150–1,250 at the world load: `land()` held the newest 64 flights of finished lists (up to 716 lists), and up to 240 title lists were in flight. With every submission waited for (`VICTORY_RELAY=0`) the count stays at 674 and the segment never fills.
- **Fixed** (commits `f173347`, `018d372`, `e3c0703` and the prelude pool): finished lists go back to the pool at once; preludes are shared from a pool instead of one per host list for life (101 made instead of ~950); and while the segment is above 80% every flight lands as it is sent and idle lists are trimmed. Default `play.ps1` settings now get through the world load: **5.5–7.4 fps (~145 ms/frame) in the world, no device removal in 180 s**, against 2.8 fps with `VICTORY_RELAY=0`. The back-off no longer fires.

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

On this card the PR hurts: the system cuts the budget to ~10.0–10.4 GB at the world load, and 1,536 MiB of card buffers put the card 1.6 GB over it, so the video memory manager pages and the card slows five-fold. Not committed to this fork.

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

## Next steps

1. Frame rate: the frame is now ~125 ms of preparing on the submitting thread and the card is barely waited for. Next is the CPU side of a submission (copies ~20–40 ms, the rest in steps and refreshes).
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
