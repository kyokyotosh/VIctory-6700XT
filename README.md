# VIctory on an RX 6700 XT

A fork of [avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX),
itself a fork of [VIctory](https://github.com/Marvisak/VIctory), a compatibility
layer that runs an Xbox Series title natively on Windows from files the user
supplies. The 7900 XTX fork is avatarmanz's work and holds almost everything
here; this fork takes it to a 12 GB card in a 32 GB PC.

## The goal

**Make the title playable on an AMD RX 6700 XT (12 GB) with 32 GB of RAM**: first
no device losses, then a steady frame rate, step by step up to 30 fps (see
[Milestones](#milestones)). Ray tracing stays off.

The way there is the 7900 XTX fork's: **porting, not emulating**, patching the
title's renderer so it behaves the way a PC renderer does rather than having
the layer watch and copy the console's shared memory every frame. On top of
that, this fork fits the layer into what a 12 GB card and a 32 GB PC give it.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## The machine

Every number in this README and in [FINDINGS-6700XT.md](FINDINGS-6700XT.md) comes
from one PC:

| | |
|---|---|
| OS | Windows 11 Pro (build 26300) |
| CPU | AMD Ryzen 7 5800X3D (8 cores, 16 threads) |
| RAM | 32 GB |
| GPU | AMD Radeon RX 6700 XT, 12 GB (WDDM budget 11,475 MB, cut to about 10,080 MB at the first world load) |
| Driver | 32.0.21045.29003 (2026-11-17) |
| Display | 2560x1440 |
| Settings | `scripts\play.ps1` defaults (Series X profile, ray tracing off), diagnostics build |

## Where it stands (2026-11-27)

| | |
|---|---|
| In the world, default settings | **8.8-12.6 fps, about 100 ms a frame** (about 45 ms preparing, 40 ms on the card), no device loss in 100+ s |
| In the world, `-More @{VICTORY_OUTPUT='1920x1080'}` | **12-16 fps, 62-80 ms a frame**, the card about 60% busy and under its budget; the frame is set by preparing (about 60 ms) |
| In the world, `-Console series-s` | **12-20 fps, steady 10 reached** (91% and 94% of samples at 10 fps or more), the card under its budget; the Series S profile with the title's own upscaler (its FidelityFX one leaves ghosts here, see below) |
| A heavier street, `-Console series-s` (52,500 list calls a frame) | **12.9-13.1 fps** with list translation on 4 threads (now the default here), against 4.6-6.5 with the 8 of before; the card 60% busy, was 18-28% |
| Before this fork's changes | the device was lost about 60-90 s in, at the first world load, every run; only `VICTORY_RELAY=0` ran (2.8 fps, 360 ms a frame) |
| Menus, videos, loading screens | 30 fps |
| Memory | about 27 GB of private memory; the card about 1 GB over the budget the system cuts to, which it holds |

What changed, in order, with the runs behind each step:
[FINDINGS-6700XT.md](FINDINGS-6700XT.md).

## Milestones

The world's frame rate on [the machine](#the-machine), from the frame-rate line
the diagnostics build writes every 5 seconds (`N fps, M ms/frame`, `D draws/frame`);
a sample is in the world when it draws 1,000 times a frame or more (menus,
videos and loading screens draw a few dozen):

- **Range N fps**: at least one 5-second sample in the world reaches N fps.
- **Steady N fps**: at least 90% of the 5-second samples in the world are N fps
  or more, over 90 s or more in the world.

Either one counts only with no device loss, in two runs of the same build and
settings, and is marked here with the runs, the settings and the commit.
`scripts\fps-milestones.ps1 <log> <log>` scores run logs by these rules.
`scripts\press-start.ps1 -Log <log>` gets an unattended run past the account
picker (it presses Enter, the keyboard's A, while the title is in its menus).

| Target | Range | Steady |
|---|---|---|
| 5 fps | **reached** (`b4ad9c9`, defaults) | **reached** (`b4ad9c9`, defaults: 91% and 95% of samples) |
| 10 fps | **reached** (`b4ad9c9`, defaults: 12.6 and 11.1 fps) | **reached** (`-Console series-s`, which now uses the title's own upscaler: 91% and 94% of samples; 1080p output 81% and 76%; defaults 22% and 32%) |
| 15 fps | **reached** (`3c5f249`, `VICTORY_OUTPUT=1920x1080`: 16.2 and 16.6 fps) | not yet |
| 20 fps | not yet | not yet |
| 25 fps | not yet | not yet |
| 30 fps | not yet | not yet |

Evidence (`scripts\fps-milestones.ps1`; "defaults" is `scripts\play.ps1` as it is,
with the 1 MiB / 1024 MiB card-buffer limits of `b4ad9c9`):

| Run | Settings | World samples (fps) |
|---|---|---|
| `run-defaults-1024` | defaults | 1.2 6.9 6.4 8.8 7.6 4.6 9.3 9.2 10.7 9.1 8.9 9.8 12.6 10.0 10.5 9.7 6.3 9.5 10.2 8.9 9.8 9.9 9.7 |
| `run-cards-D` | defaults (with a card-copy line setting that changed nothing and was not kept) | 5.7 7.2 8.4 5.9 8.8 9.4 9.4 10.4 2.5 11.1 10.7 8.8 10.8 10.8 8.9 9.7 10.3 8.9 9.8 10.9 9.2 9.1 |
| `run-out1080` | defaults + `VICTORY_OUTPUT=1920x1080` | 10.0 5.7 10.4 10.7 11.0 8.9 15.1 13.8 15.0 16.2 12.9 11.6 14.5 14.6 13.2 13.7 13.1 13.3 12.5 9.9 8.3 |
| `run-out1080-b` | defaults + `VICTORY_OUTPUT=1920x1080` | 10.5 5.5 9.0 6.9 4.3 12.7 16.0 16.3 16.4 16.6 16.3 15.7 6.5 13.6 14.7 14.0 14.4 13.7 12.7 13.4 12.4 |
| `run-series-s` | defaults + `-Console series-s -Overlay`, FidelityFX upscaler (as before) | 3.3 7.8 10.0 13.1 11.6 11.4 13.5 13.6 13.4 13.9 13.3 13.7 13.5 13.9 14.2 12.9 13.4 12.8 11.8 14.6 12.7 13.9 13.5 13.6 13.3 14.5 13.1 13.4 14.2 15.4 15.2 15.6 15.3 |
| `run-series-s-b` | defaults + `-Console series-s -Overlay`, FidelityFX upscaler (as before) | 0.3 8.5 7.8 12.9 11.6 12.0 13.9 14.4 14.3 14.4 14.2 14.1 14.8 9.9 12.3 13.2 12.6 13.2 13.3 13.6 13.6 15.9 15.1 14.5 13.9 14.4 12.9 16.5 15.2 15.1 15.0 15.1 16.8 17.4 |
| `run-look-seriess-up6` | defaults + `-Console series-s` + `VICTORY_UPSCALER=6` (what `-Console series-s` now sets) | 9.5 6.0 7.6 11.2 14.2 14.0 14.7 15.0 13.5 13.6 14.5 13.6 13.5 13.9 14.9 13.8 14.4 14.2 13.7 14.3 14.7 12.5 15.1 13.7 15.6 13.7 18.0 17.6 17.9 17.2 17.4 18.0 |
| `run-seriess-up6-b` | defaults + `-Console series-s` | 7.6 6.9 12.1 13.5 12.0 14.4 15.8 15.2 14.7 13.0 11.9 14.0 15.7 16.8 14.2 14.6 14.3 14.5 14.0 13.3 13.8 14.3 13.2 14.3 15.3 15.7 15.8 16.2 16.2 17.2 17.5 17.9 19.1 19.7 |

The misses left are the world's first frames: the frame that leaves the loading
screen, then 10-20 s of streaming in which the title itself takes 50-65 ms
after a present before it records again.

## What this fork changed

In short; the reasons, the GPU trace and the measurements are in
[FINDINGS-6700XT.md](FINDINGS-6700XT.md).

- **The device loss at the first world load.** A GPU ETW trace showed the video
  memory manager running out of room in a 2 GB aperture segment (system memory
  the card maps) that holds the driver's command buffers, upload heaps and
  CPU-cached buffers; it then marked the device as faulted. With submissions
  sent ahead the layer held some 1,200 host command lists at once there.
  - Lists whose flight has landed go back to the pool at once; what they named
    still waits the bury lag (`f173347`).
  - The adapter's segments are read with `D3DKMTQueryStatistics`, logged at the
    start, and while an aperture segment is above 80% of its limit every flight
    lands as it is sent and idle lists are trimmed (`VICTORY_SEGMENT_TIGHT`;
    `018d372`, `e3c0703`).
  - Preludes come from a pool instead of one per list for its life: 101 made
    instead of about 950 (`63a773e`).
- **Card-buffer limits for 10-12 GB cards**: 1 MiB and up, 1024 MiB, from
  [PR #225](https://github.com/avatarmanz/VIctory-7900XTX/pull/225) by @Zayfway
  with its cap lowered for this card (`b4ad9c9`). The CPU looked for shader
  writes in 400 MB a frame of copies left in the PC's memory (67 ms); now 3 ms.

What is known to be wrong here:

- The card's memory ends about 1 GB over the budget the system cuts to at the
  world load (the card copies come in a burst just before the cut).
- Two debug-layer errors repeat in every run (a UAV descriptor in an SRV range,
  and a buffer read in the copy-destination state); not the cause of anything
  seen yet.
- The 7900 XTX fork's list below holds here too.

## Inherited from the 7900 XTX fork

Everything below is [avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)'s
work, kept as it was. Its measurements were made on an RX 7900 XTX with 64 GB of
RAM, not on this machine.

### Renderer patches (off by default while they are tested)

| Switch | What it does |
|---|---|
| `VICTORY_PORT_CONSTANTS=1` | The title's 174 MB constants heap is read by the card where the title writes it: no uploads, no fills |
| `VICTORY_PORT_DESCRIPTORS=1` | The title's descriptor-table stores are patched (byte-checked) to report the slots they write, instead of a scan of ~600,000 slots a frame |
| `VICTORY_PORT_CBV=1` | Translated shaders read constant buffers in place as raw bytes (~22,000 fills a frame to none; -4.7 ms) |
| `VICTORY_PORT_GUARD=1` | The write guards' lock is shared by faults and refreshes, with a claim per 64 KiB block; the submission's planning runs beside it (title threads' waits 29 ms to ~2 ms a frame; -2 ms) |
| `VICTORY_PORT_JOURNAL=1` | The heaps' records are asked for at the title's submit, and the journal's notes are split into 64 parts with locks of their own (the submit thread's waits ~5 ms + 1 ms to ~0.3 ms; -1 to -3 ms) |

### Other switches

| Switch | What it does |
|---|---|
| `--console series-s` | The Series S profile: 1080p output, about 5 ms faster, a softer picture |
| `VICTORY_OUTPUT=1920x1080` | The title renders to that output size itself (Series X profile; about 16 ms less card time) |
| `VICTORY_FRAME_INTERVAL_MS=16.667` | Lets the title go above 30 fps (menus and videos reach 60; the game's clock stays right) |
| `VICTORY_AMBIENT_SETTLE` | How fast light settles after walking indoors (on by default, `250,3`; `0` turns it off) |
| `VICTORY_REFLECT_BOUND=<multiple>` | Reflections that missed (the sky where a ray would have found the room) hold no more than that multiple of the light around; ends the frosted, speckled floors indoors. `play.ps1` sets `1`; `0` turns it off. Water is left alone |
| `VICTORY_TEXTURE_STORE=1` | Converted textures kept between runs (about 8 ms a frame for now: off) |
| `VICTORY_TEXTURE_TURN_BACKOFF=1` | A guarded texture whose every-eighth-frame hash keeps finding nothing is hashed less often, up to every 64th frame; a change found brings it back (texture hashing 38 to 4 MB a frame in a street; off while it is measured) |
| `VICTORY_HEAP_PASS_AT_SUBMIT=1` | A list waits for the heap's view pass when it is submitted, not when a translating thread closes it (that wait was 10 ms a frame; the frame about 3 ms shorter). `play.ps1` sets `1` |
| `VICTORY_PIPELINE_PIECES=1` | The title's two pipeline-creation methods that take loose program pieces (device slots 0x1c8 graphics, 0x218 compute) make the pipeline instead of failing (from VIctory-NVIDIA-RT). With `VICTORY_PIPELINE_PIECES_ONLY=graphics` and `VICTORY_PIPELINE_PIECES_REFUSE_UNTRANSLATED=1`: 2,671 of 16,384 graphics pipelines made a run, no frame cost, no visible change in the street scene; off while it is measured |
| `VICTORY_KBM_GTA=1` | Optional GTA V PC-style keyboard and mouse mapping on the stand-in pad: `F2` or `` ` `` (`~`) toggles mouse-look and auto-hides the cursor while locked (`Esc`/`P`/focus loss unlocks and restores it), LMB/RMB = RT/LT, MMB/`C` = R3, `Shift` = A, `Space` = X, `R` = B, `F` = Y, `Tab` = LB, `Q` = RB, `Ctrl` = L3, `E` = D-pad Right, `V` = View. Tune with `VICTORY_MOUSE_SENS` (default `1.0`) and `VICTORY_MOUSE_INVERT_Y=1` (or pass via `scripts\play.ps1 -More @{VICTORY_KBM_GTA='1'}`) |

### What it changed from VIctory

The reasons and the measurements are in [docs/findings.md](docs/findings.md).

- **List calls are written down, not carried out, on the title's threads**
  (`VICTORY_THIN`): threads of ours translate them, a quarter of the logical
  cores (2 to 8; `VICTORY_THIN_THREADS`). Upstream ran 8, which on this 8-core
  CPU fought the title's own threads: 21-30 µs a list call instead of 5.8.
- **Submissions are prepared by a thread of ours and joined**
  (`VICTORY_ASYNC_SUBMIT`, `VICTORY_JOIN_PARTS`), with what can be spread over
  threads spread: refreshes, compares (AVX2), descriptor records, landings.
- **What shaders wrote comes back beside the write guards**, by the dwords that
  changed, on a thread of its own (`VICTORY_RELAY`, `VICTORY_LAND`).
- **Buffer fills that need bytes still on the card are done on the card**
  (`VICTORY_CARD_STEPS`) instead of waiting for it.
- **The title's writes are caught more cheaply**: one view opened at a fault,
  blocks that rest, places in the title's code that look before they write.
- **Textures go up on a copy queue of their own** and a slot shows one only after
  its upload ran, which also ended a device loss that came every 15 minutes or so.
- **Boats float**: textures that shaders write and the title's CPU reads go back
  to title memory (from fxceless and Geckomacabre, on by default here).
- **Frame record and replay** with per-pass times and shader variants compared
  turn by turn ([docs/replay.md](docs/replay.md)).

What it lists as wrong:

- Trees, signs and far objects flicker in the distance in some places.
- Lighting is too bright or too dark after walking indoors, until the title has
  run for a while; rooms with little direct light are too dark without `-RayTracing`.
- Textures of what just came into view appear late while the title streams.
- The first run of a new build stutters while the driver makes pipelines
  (`victory warm` makes them ahead).
- Recording a frame in the world makes the title's own watchdog end it.

## How to run it

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, Rust (the toolchain is pinned), the MSVC build
tools, and your own dump of the title.

```powershell
git clone https://github.com/kyokyotosh/VIctory-6700XT
cd VIctory-6700XT
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
scripts\play.ps1 -Exe <path of your copy's executable>
```

`scripts\play.ps1 -Safe` leaves out the newest, faster ways; try it first when
something is wrong. `-RayTracing` turns the title's ray-traced indirect light on.
`-Overlay` shows FPS, ms a frame, draws a frame and a line chart of the last
240 frames' rates (0-40 fps, red where the rate fell, green where it rose, a
dotted baseline at 30 fps) with their lowest and highest, in the upper left
corner; F3 hides and shows it. `-OverlayFull` adds the submitting thread's, the card's and the
memory's numbers. **`-Console series-s` is the way to play on this card**: the
Series S profile (1080p output, lighter content), 12-20 fps in the world, with
the title's own upscaler. The Series S profile picks AMD FidelityFX, which here
leaves a dithered ghost of whatever moves and stipples the walls; `play.ps1`
sets `VICTORY_UPSCALER=6` for it (`-More @{VICTORY_UPSCALER='7'}` brings
FidelityFX back). `-More @{VICTORY_OUTPUT='1920x1080'}` keeps the Series X
profile and renders at 1080p, 12-16 fps. Native output runs 9-12 (see
[Milestones](#milestones)).
`-TextureMax auto` caps textures' largest side at 1024 texels, 512 from 92 % of
the card's memory budget and 256 from 97 % (`-TextureMax 1024`: a fixed size):
less card memory for coarser textures. Off by default; upstream has it on
(beloooo's rule).
If the device is still lost on your PC, `-More @{VICTORY_RELAY='0'}` waits for
every submission (slower, but it was the one setting that ran before this fork's
changes).

On a hybrid-graphics PC, the runtime and offline pipeline checks prefer a
high-performance adapter with Shader Model 6.6. The runtime logs its adapter
name, PCI vendor and device IDs, dedicated VRAM, and the memory segments the
graphics kernel reports. Set `VICTORY_D3D12_ADAPTER` to a zero-based DXGI index
or part of the adapter's name to select it explicitly. With the launcher, pass
`-More @{VICTORY_D3D12_ADAPTER='NVIDIA'}`.

Controllers: Xbox pads (XInput), and PlayStation pads (DualSense, DualSense
Edge, DualShock 4) by USB or Bluetooth with no driver or DS4Windows, read from
their own HID reports. Cross is A, Circle B, Square X, Triangle Y, Create View,
Options Menu; rumble, the touchpad, the gyro and the adaptive triggers are not
passed on. With Steam running, turn its PlayStation desktop layout off (or quit
Steam): Steam turns the pad into mouse and keys for windows that are not its
games. `VICTORY_PS_PADS`: `auto` (the default) reads a PlayStation pad only
while no Xbox pad is connected, so DS4Windows' or Steam's Xbox pad and the pad
under it are not both read; `1` always; `0` never. A pad is logged when it
connects; `scripts\play.ps1 -PadLog` logs its reports each time what they say
changes, to check the mapping. With no pad connected the keyboard is one:
W/A/S/D and I/J/K/L the sticks, the arrows the D-pad, Enter/Backspace/X/Y the
A/B/X/Y buttons, Q/E the shoulders, Z/C the triggers, F/H the stick clicks,
Esc/Tab Menu/View. `scripts\play.ps1 -Kbm` (`VICTORY_KBM_GTA=1`) makes that pad
GTA V PC-style, beside a pad you hold too: F2 or `` ` `` locks mouse look and
hides the cursor (Esc, P or leaving the window lets go), the mouse buttons are
the triggers, W/A/S/D the left stick, Shift A, Space X, R B, F Y, Tab and Q the
shoulders, Ctrl and C the stick clicks, E the D-pad's right, V View
(`VICTORY_MOUSE_SENS`, `VICTORY_MOUSE_INVERT_Y=1`). After zhu1025960209-web's
pull request #295 to Belobug/VIctory-Belobug-AMD.

Before the first run, ahead of time (both can be stopped and go on later). The
shader archive here is the dump's `shaders.rpf` (25,694 shaders, about a
minute; 195 containers without a program do not translate):

```powershell
target\diag\victory.exe shaders <the shader archives>   # every shader translated, minutes
target\diag\victory.exe warm --used                     # pipelines earlier runs drew with, for a new driver or build
```

Commands, debugging switches and upstream's own description:
[docs/README-fxceless.md](docs/README-fxceless.md). Every `VICTORY_...` switch is
described in a comment where it is read.

## Credits

This is a tree of other people's work with more on top. The history is kept, and
commits taken from elsewhere name the commit and its author.

- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**
  by avatarmanz: the fork this one is cloned from, with its renderer patches,
  its threads and its findings. Nearly all of the speed here is its work.
- **@Zayfway**: [PR #225](https://github.com/avatarmanz/VIctory-7900XTX/pull/225),
  the card-buffer limits for 10-12 GB cards this fork's tier is built on.
- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end and
  the shader recompiler.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**: the fork
  the 7900 XTX fork started from, and the people whose pull requests it carries.
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  cube gradients, sparse textures.
- **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**:
  the ambient light idea, pipeline workers, and work by romainhedouin.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read for how the same
  title behaves under another layer; no code of it is here.
- This fork's changes, like the 7900 XTX fork's, were made with
  [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
