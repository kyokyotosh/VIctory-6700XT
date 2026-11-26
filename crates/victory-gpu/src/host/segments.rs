//! The adapter's memory segments as the graphics kernel counts them
//! (`D3DKMTQueryStatistics`): the size and use of the system-memory
//! (aperture) segments, which DXGI's budgets leave out.
//!
//! On an RX 6700 XT in a 32 GB PC one of them is 2 GB, and it holds the
//! driver's command buffers (1 MB and more for each command list), upload
//! heaps and CPU-cached custom heaps. Sending submissions ahead, the host
//! made lists faster than flights gave them back during the first world
//! load; the segment ran full, the video memory manager could not make a
//! new command list resident, and the device was removed
//! (`FINDINGS-6700XT.md`). [`tight`] says when to let the flights land
//! first.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_QUERYSTATISTICS, D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT,
    D3DKMT_QUERYSTATISTICS_RESULT, D3DKMT_QUERYSTATISTICS_SEGMENT, D3DKMT_QUERYSTATISTICS_TYPE, D3DKMTQueryStatistics,
};
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::IDXGIAdapter3;

/// A segment: its id, whether it is an aperture (system memory the card
/// maps), its limit and the bytes resident in it (all processes').
#[derive(Clone, Copy, Debug)]
pub(crate) struct Segment {
    pub(crate) id: u32,
    pub(crate) aperture: bool,
    pub(crate) limit: u64,
    pub(crate) resident: u64,
}

fn query(luid: LUID, kind: D3DKMT_QUERYSTATISTICS_TYPE, segment: u32) -> Option<D3DKMT_QUERYSTATISTICS_RESULT> {
    let mut q = D3DKMT_QUERYSTATISTICS { Type: kind, AdapterLuid: luid, ..Default::default() };
    q.Anonymous.QuerySegment = D3DKMT_QUERYSTATISTICS_QUERY_SEGMENT { SegmentId: segment };
    // SAFETY: a plain query that fills `q`.
    unsafe { D3DKMTQueryStatistics(&q) }.is_ok().then_some(q.QueryResult)
}

fn luid(adapter: &IDXGIAdapter3) -> Option<LUID> {
    // SAFETY: a plain query.
    unsafe { adapter.GetDesc1() }.ok().map(|d| d.AdapterLuid)
}

/// The adapter's segments.
pub(crate) fn segments(adapter: &IDXGIAdapter3) -> Vec<Segment> {
    let Some(luid) = luid(adapter) else {
        return Vec::new();
    };
    let Some(info) = query(luid, D3DKMT_QUERYSTATISTICS_ADAPTER, 0) else {
        return Vec::new();
    };
    // SAFETY: the member an adapter query fills.
    let count = unsafe { info.AdapterInformation.NbSegments };
    (0..count)
        .filter_map(|id| {
            let result = query(luid, D3DKMT_QUERYSTATISTICS_SEGMENT, id)?;
            // SAFETY: the member a segment query fills.
            let s = unsafe { result.SegmentInformation };
            Some(Segment {
                id,
                aperture: s.Aperture != 0,
                limit: s.CommitLimit,
                resident: s.BytesResident,
            })
        })
        .collect()
}

/// Says the segments once, at the start.
pub(crate) fn log(adapter: &IDXGIAdapter3) {
    for s in segments(adapter) {
        tracing::info!(
            id = s.id,
            aperture = s.aperture,
            limit_mb = s.limit >> 20,
            resident_mb = s.resident >> 20,
            "the card's memory segment as the graphics kernel counts it"
        );
    }
}

/// The share of an aperture segment's limit from which it counts as full
/// (`VICTORY_SEGMENT_TIGHT`, in percent; 85 by default, 0: never).
static TIGHT_PERCENT: LazyLock<u64> =
    LazyLock::new(|| std::env::var("VICTORY_SEGMENT_TIGHT").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(85));

/// How often the segments are looked at, at most.
const LOOK_MS: u64 = 100;

/// Whether an aperture segment holds more than [`TIGHT_PERCENT`] of its
/// limit, as of the last look (one every [`LOOK_MS`], by whichever thread
/// asks first). Said in the log when it starts and stops.
pub(crate) fn tight(adapter: &IDXGIAdapter3) -> bool {
    static TIGHT: AtomicBool = AtomicBool::new(false);
    static LOOKED: AtomicU64 = AtomicU64::new(0);
    static LOOKING: Mutex<()> = Mutex::new(());
    let percent = *TIGHT_PERCENT;
    if percent == 0 {
        return false;
    }
    let now = super::now_ms();
    if now >= LOOKED.load(Ordering::Relaxed) + LOOK_MS
        && let Ok(_looking) = LOOKING.try_lock()
    {
        LOOKED.store(now, Ordering::Relaxed);
        let fullest = segments(adapter)
            .into_iter()
            .filter(|s| s.aperture && s.limit > 0)
            .max_by_key(|s| s.resident * 1000 / s.limit);
        let tight = fullest.is_some_and(|s| s.resident * 100 > s.limit * percent);
        if TIGHT.swap(tight, Ordering::Relaxed) != tight
            && let Some(s) = fullest
        {
            tracing::info!(
                tight,
                segment = s.id,
                resident_mb = s.resident >> 20,
                limit_mb = s.limit >> 20,
                "an aperture segment of the card's memory {} {percent}% of its limit: flights sent ahead {}",
                if tight { "passed" } else { "fell below" },
                if tight { "land before host lists are made" } else { "go on as before" }
            );
        }
    }
    TIGHT.load(Ordering::Relaxed)
}
