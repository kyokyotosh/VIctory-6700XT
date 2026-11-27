//! A small flush of refreshes tried on the submitting thread first
//! (`VICTORY_REFRESH_PROBE_US`, `VICTORY_REFRESH_PROBE_TEXTURES`), ported
//! from VIctory-NVIDIA-RT (Kemerd, `42c41ef` and `4749cd7`, host/lean.rs).
//!
//! Handing a flush to the kept threads (`crate::shadow::wide_each`) costs
//! the submitting thread a wake for each and a wait at the end until the
//! last of them is done, whether or not they had anything to do. A frame
//! here has about 44 flushes, most of them a few dozen copies after those
//! known to copy nothing are left out. With the probe a small flush goes
//! through its refreshes one after the other on this thread until the probe
//! has run out, and only then hands the rest over: a flush that is small is
//! done without any wake, and one that is not loses at most the probe and
//! the one refresh it was in.
//!
//! Where the bytes are does not change: title memory is read into the copy
//! by the same refresh, once; only which thread calls it does.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How long a small flush of refreshes is worked on this thread alone before
/// the rest goes to the kept threads (`VICTORY_REFRESH_PROBE_US=<microseconds>`,
/// 1 to 5000; unset, 0 or anything else: every flush goes to the threads at
/// once, as before).
fn probe() -> Option<Duration> {
    static PROBE: LazyLock<Option<Duration>> = LazyLock::new(|| probe_of(std::env::var("VICTORY_REFRESH_PROBE_US").ok().as_deref()));
    *PROBE
}

/// [`probe`] for a setting.
fn probe_of(setting: Option<&str>) -> Option<Duration> {
    let us: u64 = setting?.trim().parse().ok()?;
    // A probe past a few milliseconds is a flush done here whatever it is.
    (us > 0).then(|| Duration::from_micros(us.min(5_000)))
}

/// Bytes of title memory a due texture may read and still count as light for
/// [`textures_small`].
const SMALL_TEXTURE: usize = 256 << 10;

/// Whether a flush with only light textures due still counts as small for
/// [`spread`] (`VICTORY_REFRESH_PROBE_TEXTURES=1`, only with the probe; off: a
/// flush with any texture due goes to the threads at once, as before).
fn probe_textures() -> bool {
    static ON: LazyLock<bool> =
        LazyLock::new(|| probe().is_some() && textures_of(std::env::var("VICTORY_REFRESH_PROBE_TEXTURES").ok().as_deref()));
    *ON
}

/// [`probe_textures`] for a setting: only "1".
fn textures_of(setting: Option<&str>) -> bool {
    setting == Some("1")
}

/// Since the last frame-rate line: flushes done on this thread alone, those
/// that went on to the threads after the probe, the nanoseconds of both
/// calls, the refreshes done here, and flushes taken as small only because
/// their due textures were light.
static PROBED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Whether every texture in `due` is light enough ([`SMALL_TEXTURE`]) for the
/// flush to be tried on this thread first. False with the switch off, so the
/// caller's own test stands as it was. Counts a yes for a non-empty `due` when
/// with the `spans` the flush has two or more pieces: with fewer it never went
/// to the threads, so the textures did not change where it ran.
pub(super) fn textures_small(due: &[std::sync::Arc<crate::texture::Staging>], spans: usize) -> bool {
    if !probe_textures() || !due.iter().all(|t| t.cheap(SMALL_TEXTURE)) {
        return false;
    }
    if !due.is_empty() && spans + due.len() >= 2 {
        PROBED[4].fetch_add(1, Ordering::Relaxed);
    }
    true
}

/// What [`spread_with`] did.
#[derive(Debug, PartialEq, Eq)]
struct Spread {
    /// How many of the `n` were done on this thread before the rest left it.
    here: usize,
    /// Whether the rest went to the kept threads.
    went: bool,
}

/// Calls `work` with every number below `n`, on this thread while less than
/// `probe` has passed since the first, and with what is left on the kept
/// threads and this one (`crate::shadow::wide_each`). With no `probe`, or
/// `small` false, or fewer than two to do: all of it goes to `wide_each`.
fn spread_with(n: usize, small: bool, probe: Option<Duration>, work: impl Fn(usize) + Send + Sync + 'static) -> Spread {
    let Some(probe) = probe.filter(|_| small && n >= 2) else {
        crate::shadow::wide_each(n, work);
        return Spread { here: 0, went: true };
    };
    let started = std::time::Instant::now();
    let here = serial_first(n, probe, || started.elapsed(), &work);
    if here >= n {
        return Spread { here, went: false };
    }
    // What the thread did not get to: numbers from `here` on.
    crate::shadow::wide_each(n - here, move |i| work(here + i));
    Spread { here, went: true }
}

/// Calls `work` with 0, 1, ... while `elapsed` says the probe has not run out;
/// at least once. How many it called, `n` if it called all.
fn serial_first(n: usize, probe: Duration, mut elapsed: impl FnMut() -> Duration, work: &impl Fn(usize)) -> usize {
    let mut i = 0;
    while i < n {
        work(i);
        i += 1;
        if i < n && elapsed() >= probe {
            break;
        }
    }
    i
}

/// `crate::shadow::wide_each` under [`probe`], counted. `small`: the caller's
/// flush is one the probe is for (few copies, and no textures due, or with
/// `VICTORY_REFRESH_PROBE_TEXTURES=1` only light ones: [`textures_small`]).
/// With the probe off this is `wide_each`, and no clock is read.
pub(super) fn spread(n: usize, small: bool, work: impl Fn(usize) + Send + Sync + 'static) {
    let Some(probe) = probe().filter(|_| small && n >= 2) else {
        crate::shadow::wide_each(n, work);
        return;
    };
    let started = std::time::Instant::now();
    let done = spread_with(n, small, Some(probe), work);
    PROBED[usize::from(done.went)].fetch_add(1, Ordering::Relaxed);
    PROBED[2].fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    PROBED[3].fetch_add(done.here as u64, Ordering::Relaxed);
}

/// The counters, per frame, for the frame-rate line; nothing with the probe
/// off.
#[cfg(feature = "diagnostics")]
pub(super) fn line(frames: f64) {
    let Some(probe) = probe() else {
        return;
    };
    let frames = frames.max(1.0);
    let p: [f64; 5] = std::array::from_fn(|i| PROBED[i].swap(0, Ordering::Relaxed) as f64 / frames);
    tracing::info!(
        "small refresh flushes per frame (VICTORY_REFRESH_PROBE_US={}): {:.1} done on the submitting thread alone, {:.1} went on to \
         the threads after the probe, {:.2} ms in these calls, {:.0} refreshes made there; {:.1} taken as small only because their \
         due textures were light (VICTORY_REFRESH_PROBE_TEXTURES is {})",
        probe.as_micros(),
        p[0],
        p[1],
        p[2] / 1e6,
        p[3],
        p[4],
        if probe_textures() { "on" } else { "off" }
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    /// A table of how many times each number was called.
    fn tally(n: usize) -> Arc<Vec<AtomicUsize>> {
        Arc::new((0..n).map(|_| AtomicUsize::new(0)).collect())
    }

    #[test]
    fn the_texture_switch_is_one_and_nothing_else() {
        assert!(textures_of(Some("1")));
        assert!(!textures_of(None));
        assert!(!textures_of(Some("")));
        assert!(!textures_of(Some("0")));
        assert!(!textures_of(Some("true")));
    }

    #[test]
    fn the_probe_is_a_number_of_microseconds_or_nothing() {
        assert_eq!(probe_of(None), None);
        assert_eq!(probe_of(Some("")), None);
        assert_eq!(probe_of(Some("0")), None);
        assert_eq!(probe_of(Some("fast")), None);
        assert_eq!(probe_of(Some("-5")), None);
        assert_eq!(probe_of(Some(" 150 ")), Some(Duration::from_micros(150)));
        // A probe is bounded.
        assert_eq!(probe_of(Some("90000000")), Some(Duration::from_micros(5_000)));
    }

    #[test]
    fn the_thread_works_until_the_probe_has_run_out_and_at_least_once() {
        let called = tally(10);
        let work = {
            let called = called.clone();
            move |i: usize| {
                called[i].fetch_add(1, Ordering::Relaxed);
            }
        };
        // A clock that has run out at once: the first is still done.
        assert_eq!(serial_first(10, Duration::from_micros(1), || Duration::from_secs(1), &work), 1);
        // A clock that never does.
        assert_eq!(serial_first(10, Duration::from_secs(1), || Duration::ZERO, &work), 10);
        // One that runs out after three looks.
        let mut looks = 0;
        let here = serial_first(
            10,
            Duration::from_micros(30),
            || {
                looks += 1;
                Duration::from_micros(10 * looks)
            },
            &work,
        );
        assert_eq!(here, 3);
        assert_eq!(called[0].load(Ordering::Relaxed), 3);
        assert_eq!(called[3].load(Ordering::Relaxed), 1);
        assert_eq!(called[9].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn every_number_is_called_once_whoever_does_it() {
        for (n, small, probe) in [
            (0, true, Some(Duration::from_secs(1))),
            (1, true, Some(Duration::from_secs(1))),
            (2, true, None),
            (37, false, Some(Duration::from_secs(1))),
            // All here: the probe is long.
            (37, true, Some(Duration::from_secs(60))),
            // Next to none here: the probe has run out after the first.
            (37, true, Some(Duration::from_nanos(1))),
            (500, true, Some(Duration::from_nanos(1))),
        ] {
            let called = tally(n);
            let work = {
                let called = called.clone();
                move |i: usize| {
                    called[i].fetch_add(1, Ordering::Relaxed);
                }
            };
            let done = spread_with(n, small, probe, work);
            assert!(called.iter().all(|c| c.load(Ordering::Relaxed) == 1), "n {n}, small {small}, probe {probe:?}");
            match (n, small, probe) {
                (37, true, Some(p)) if p >= Duration::from_secs(60) => assert_eq!(done, Spread { here: 37, went: false }),
                (37 | 500, true, Some(_)) => assert!(done.went && done.here >= 1),
                _ => assert!(done.went && done.here == 0),
            }
        }
    }
}
