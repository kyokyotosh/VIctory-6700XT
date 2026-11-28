//! Where the title's threads spend a frame (`VICTORY_TITLE_TIME=1`, off by
//! default; `2` also times the title's contended locks): one line with the
//! frame-rate lines, cheap enough for release builds.
//!
//! Why: moving the submissions' preparing off the title's thread
//! (`VICTORY_ASYNC_SUBMIT=1`) left the frame as long as it was (run
//! prisao-s-p67async: 38.9 ms against 39.0), so what bounds a frame is not
//! that thread's preparing. The line says, per frame and per kind of title
//! thread (the one that submits, those that record lists, the others), what
//! the thread did with its time: blocked in the system's waits (on an event a
//! fence of ours sets, on other objects, sleeping, on addresses or condition
//! variables, on its own locks), inside the layer (list calls, waiting for a
//! list's lock, submit calls, present calls, the queues' other calls, write
//! faults, settles), spinning on a fence's completed value, and the rest: the
//! title's own work (or the thread not running). With the async line it also
//! says what our thread that prepares did: working, idle with nothing in the
//! line (it waits for the title), idle with the line standing at a wait (it
//! waits for the card or a value), and what ended each idle stretch. And of
//! the frame: from its last submission to its present, the present call,
//! from the present to the next first list call and first submission.
//!
//! How: the title's imports of the waits are bound to wrappers here
//! ([`import`], only with the switch); the layer's entry points note their
//! time in a thread-local, which goes to a shared table every few
//! milliseconds, so nothing contended is touched per call.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

/// Whether the clocks run (`VICTORY_TITLE_TIME=1` or `2`; off by default).
pub(crate) fn on() -> bool {
    level() != 0
}

/// `VICTORY_TITLE_TIME`: 0 off, 1 the waits and the layer, 2 also the
/// title's contended locks (an extra try per lock taken).
fn level() -> u8 {
    static LEVEL: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *LEVEL.get_or_init(|| match std::env::var("VICTORY_TITLE_TIME").as_deref().map(str::trim) {
        Ok("1") => 1,
        Ok("2") => 2,
        _ => 0,
    })
}

/// What a stretch of a title thread's time was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub(crate) enum Kind {
    /// A system wait on an event a fence of ours sets (`SetEventOnCompletion`).
    WaitFence,
    /// A system wait on other objects (events, semaphores, threads, ...).
    WaitOther,
    /// `Sleep`, `SleepEx`.
    Sleep,
    /// `WaitOnAddress`, condition variables.
    WaitAddress,
    /// The title's own critical sections and SRW locks, when contended
    /// (`VICTORY_TITLE_TIME=2`).
    Lock,
    /// Inside our list calls.
    List,
    /// Of them, waiting for a list's own lock (another thread in it).
    ListLock,
    /// Inside the title's submit calls.
    Submit,
    /// Inside the title's present calls.
    Present,
    /// Inside the queues' other calls (signal, wait, page mappings).
    Queue,
    /// Opening a guarded block for a write fault of the title's.
    Fault,
    /// Calling `GetCompletedValue` of one fence again and again while it did
    /// not move (from the first such call to the one that saw it move).
    Poll,
    /// Settling flights outside a submit or present call (a fence waited
    /// for, a read of memory flights may write, ...).
    Settle,
}

const KINDS: usize = 13;
const NAMES: [&str; KINDS] = [
    "fence events",
    "other objects",
    "sleeps",
    "addresses",
    "locks",
    "list calls",
    "a list's lock",
    "submit calls",
    "present calls",
    "queue calls",
    "write faults",
    "fence polls",
    "settles",
];

/// What one thread noted since it last gave its notes away.
#[derive(Clone, Copy, Default)]
struct Notes {
    ns: [u64; KINDS],
    n: [u64; KINDS],
    /// Of the submit calls: how many came before the frame's first one.
    submits: u64,
}

impl Notes {
    fn add(&mut self, other: &Notes) {
        for k in 0..KINDS {
            self.ns[k] += other.ns[k];
            self.n[k] += other.n[k];
        }
        self.submits += other.submits;
    }
}

struct Local {
    notes: Notes,
    given: Option<Instant>,
    /// Calls of the layer running on this thread (a settle inside a submit
    /// call is the submit call's).
    depth: u32,
    /// The fence polled last, the value it gave, and when it first did.
    poll: (usize, u64, Option<Instant>),
}

thread_local! {
    static LOCAL: RefCell<Local> = const {
        RefCell::new(Local { notes: Notes { ns: [0; KINDS], n: [0; KINDS], submits: 0 }, given: None, depth: 0, poll: (0, 0, None) })
    };
}

/// By thread id: its notes since the last line, and whether it is one of ours.
static THREADS: Mutex<Vec<(u32, bool, Notes)>> = Mutex::new(Vec::new());

/// How often a thread gives its notes to [`THREADS`].
const GIVE_EVERY: std::time::Duration = std::time::Duration::from_millis(4);

fn thread_id() -> u32 {
    // SAFETY: a plain query.
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

/// Whether this thread is one of ours (they are named `victory-*`).
fn ours() -> bool {
    std::thread::current().name().is_some_and(|n| n.starts_with("victory"))
}

fn note(kind: Kind, ns: u64) {
    let _ = LOCAL.try_with(|l| {
        let Ok(mut l) = l.try_borrow_mut() else { return };
        l.notes.ns[kind as usize] += ns;
        l.notes.n[kind as usize] += 1;
        give(&mut l);
    });
}

fn give(l: &mut Local) {
    let now = Instant::now();
    if l.given.is_some_and(|at| now.duration_since(at) < GIVE_EVERY) {
        return;
    }
    l.given = Some(now);
    let notes = std::mem::take(&mut l.notes);
    let id = thread_id();
    let mut all = THREADS.lock().unwrap_or_else(|e| e.into_inner());
    match all.iter_mut().find(|t| t.0 == id) {
        Some(t) => t.2.add(&notes),
        None => all.push((id, ours(), notes)),
    }
}

/// A stretch of kind `kind` begun with [`begin`]; noted when dropped.
pub(crate) struct Timed(Option<(Kind, Instant)>);

impl Drop for Timed {
    fn drop(&mut self) {
        if let Some((kind, began)) = self.0.take() {
            let ns = began.elapsed().as_nanos() as u64;
            let _ = LOCAL.try_with(|l| {
                if let Ok(mut l) = l.try_borrow_mut() {
                    l.depth = l.depth.saturating_sub(1);
                }
            });
            note(kind, ns);
        }
    }
}

/// Begins a call of the layer of kind `kind` on this thread (nothing with
/// the switch off). A settle inside another call of the layer is that
/// call's, and not timed.
pub(crate) fn begin(kind: Kind) -> Timed {
    if !on() {
        return Timed(None);
    }
    let nested = LOCAL
        .try_with(|l| {
            let Ok(mut l) = l.try_borrow_mut() else { return true };
            let nested = l.depth > 0;
            if !(nested && kind == Kind::Settle) {
                l.depth += 1;
            }
            nested
        })
        .unwrap_or(true);
    if nested && kind == Kind::Settle {
        return Timed(None);
    }
    if kind == Kind::Submit {
        submit_begins();
    }
    Timed(Some((kind, Instant::now())))
}

/// `ns` of kind `kind` measured by the caller (a list's lock, a fault).
pub(crate) fn add(kind: Kind, ns: u64) {
    if on() {
        note(kind, ns);
    }
}

/// When a stretch the caller times begins: `None` with the switch off.
pub(crate) fn now() -> Option<Instant> {
    on().then(Instant::now)
}

/// The title's `GetCompletedValue` of `fence` gave `value`.
pub(crate) fn polled(fence: usize, value: u64) {
    if !on() {
        return;
    }
    let spun = LOCAL.try_with(|l| l.try_borrow_mut().ok().and_then(|mut l| poll_step(&mut l, fence, value)));
    if let Ok(Some(ns)) = spun {
        note(Kind::Poll, ns);
    }
}

/// [`polled`] on this thread's notes: the nanoseconds of a spin it ends.
fn poll_step(l: &mut Local, fence: usize, value: u64) -> Option<u64> {
    let (was, before, since) = l.poll;
    if was == fence && before == value {
        if since.is_none() {
            l.poll.2 = Some(Instant::now());
        }
        return None;
    }
    l.poll = (fence, value, None);
    // It moved (or another fence is asked): the spin, if there was one, is
    // over.
    since.filter(|_| was == fence).map(|s| s.elapsed().as_nanos() as u64)
}

// ---------------------------------------------------------------------------
// The frame's points.

static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn us() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// When the last submit call returned, the last present returned; whether a
/// list call and a submit call came since that present.
static LAST_SUBMIT: AtomicU64 = AtomicU64::new(0);
static LAST_PRESENT: AtomicU64 = AtomicU64::new(0);
static LISTED_SINCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static SUBMITTED_SINCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Per line: frames; microseconds from the last submission to the present,
/// from the present to the next first list call, to the next first submit
/// call; wall microseconds of the line.
static FRAME: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
const F_FRAMES: usize = 0;
const F_TAIL: usize = 1;
const F_TO_LIST: usize = 2;
const F_TO_SUBMIT: usize = 3;

fn submit_begins() {
    if !SUBMITTED_SINCE.load(Relaxed) && !SUBMITTED_SINCE.swap(true, Relaxed) {
        FRAME[F_TO_SUBMIT].fetch_add(us().saturating_sub(LAST_PRESENT.load(Relaxed)), Relaxed);
    }
}

/// A submit call of the title's returns.
fn submitted() {
    if on() {
        LAST_SUBMIT.store(us(), Relaxed);
    }
}

/// The title calls its present.
fn present_begins() {
    if on() {
        FRAME[F_TAIL].fetch_add(us().saturating_sub(LAST_SUBMIT.load(Relaxed)), Relaxed);
    }
}

/// The title's present returns.
fn presented() {
    if !on() {
        return;
    }
    FRAME[F_FRAMES].fetch_add(1, Relaxed);
    LAST_PRESENT.store(us(), Relaxed);
    LISTED_SINCE.store(false, Relaxed);
    SUBMITTED_SINCE.store(false, Relaxed);
}

/// A submit or present call of the title's, timed, and its point of the
/// frame noted when it returns.
pub(crate) struct Call(Timed, bool);

impl Drop for Call {
    fn drop(&mut self) {
        if self.0.0.is_none() {
            return;
        }
        // (Its time first: the present's frame point comes after it.)
        drop(std::mem::replace(&mut self.0, Timed(None)));
        if self.1 { presented() } else { submitted() }
    }
}

/// The title's submit call begins: [`Kind::Submit`] until the result is
/// dropped, then the frame's last submission so far.
pub(crate) fn submit_call() -> Call {
    Call(begin(Kind::Submit), false)
}

/// The title's present call begins: [`Kind::Present`] until the result is
/// dropped, then the frame's end.
pub(crate) fn present_call() -> Call {
    if on() {
        present_begins();
    }
    Call(begin(Kind::Present), true)
}

/// A list call begins: [`Kind::List`] until the result is dropped.
pub(crate) fn list_call() -> Timed {
    if !on() {
        return Timed(None);
    }
    if !LISTED_SINCE.load(Relaxed) && !LISTED_SINCE.swap(true, Relaxed) {
        FRAME[F_TO_LIST].fetch_add(us().saturating_sub(LAST_PRESENT.load(Relaxed)), Relaxed);
    }
    begin(Kind::List)
}

// ---------------------------------------------------------------------------
// The async line's thread (`crate::queue::behind`).

/// Per line: nanoseconds working, idle with the line empty, idle standing at
/// a wait; idle stretches ended by a submission or other entry put in, by a
/// wake (something a wait stands at moved), by a drain, by the millisecond's
/// look.
static LANE: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
const L_WORK: usize = 0;
const L_EMPTY: usize = 1;
const L_STANDING: usize = 2;
const L_BY: usize = 3;

/// What woke the line's thread: put in, wake, drain, its own look.
pub(crate) const WOKEN_PUT: u8 = 0;
pub(crate) const WOKEN_WAKE: u8 = 1;
pub(crate) const WOKEN_DRAIN: u8 = 2;
pub(crate) const WOKEN_LOOK: u8 = 3;

/// The line's thread was idle `ns` (`empty`: nothing in the line; else it
/// stood at a wait), and `why` ended it.
pub(crate) fn lane_idle(ns: u64, empty: bool, why: u8) {
    LANE[if empty { L_EMPTY } else { L_STANDING }].fetch_add(ns, Relaxed);
    LANE[L_BY + usize::from(why.min(3))].fetch_add(1, Relaxed);
}

/// The line's thread worked `ns`.
pub(crate) fn lane_worked(ns: u64) {
    LANE[L_WORK].fetch_add(ns, Relaxed);
}

// ---------------------------------------------------------------------------
// The line.

/// Since the last line: when it was.
static LINE_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// The title time line (with the frame-rate lines).
pub(crate) fn line(_frames: f64) {
    if !on() {
        return;
    }
    let wall = {
        let mut at = LINE_AT.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let wall = at.map_or(0, |a| now.duration_since(a).as_nanos() as u64);
        *at = Some(now);
        wall
    };
    // This thread's own notes first.
    let _ = LOCAL.try_with(|l| {
        if let Ok(mut l) = l.try_borrow_mut() {
            l.given = None;
            give(&mut l);
        }
    });
    let threads = std::mem::take(&mut *THREADS.lock().unwrap_or_else(|e| e.into_inner()));
    let frame: Vec<u64> = FRAME.iter().map(|f| f.swap(0, Relaxed)).collect();
    let lane: Vec<u64> = LANE.iter().map(|l| l.swap(0, Relaxed)).collect();
    let frames = frame[F_FRAMES].max(1) as f64;
    if wall == 0 {
        return;
    }
    let ms = |ns: u64| ns as f64 / 1e6 / frames;
    let title: Vec<&(u32, bool, Notes)> = threads.iter().filter(|t| !t.1).collect();
    // The thread that submits most; those with list calls; the rest.
    let submitter = title.iter().max_by_key(|t| t.2.n[Kind::Submit as usize]).filter(|t| t.2.n[Kind::Submit as usize] > 0).map(|t| t.0);
    let (mut recorders, mut others): (Vec<&(u32, bool, Notes)>, Vec<&(u32, bool, Notes)>) = title
        .iter()
        .filter(|t| Some(t.0) != submitter)
        .partition(|t| t.2.n[Kind::List as usize] > 0);
    // What a thread did not spend in anything noted: the title's own work
    // (or not running).
    let rest = |n: &Notes| wall.saturating_sub(n.ns.iter().sum::<u64>() - n.ns[Kind::ListLock as usize]);
    let kinds = |n: &Notes, which: &[Kind]| -> String {
        which
            .iter()
            .filter(|&&k| n.ns[k as usize] > 0)
            .map(|&k| format!("{} {:.2} ({:.0})", NAMES[k as usize], ms(n.ns[k as usize]), n.n[k as usize] as f64 / frames))
            .collect::<Vec<_>>()
            .join(", ")
    };
    const WAITS: [Kind; 5] = [Kind::WaitFence, Kind::WaitOther, Kind::Sleep, Kind::WaitAddress, Kind::Lock];
    const LAYER: [Kind; 8] =
        [Kind::Submit, Kind::Present, Kind::List, Kind::ListLock, Kind::Queue, Kind::Fault, Kind::Settle, Kind::Poll];
    let sum = |list: &[&(u32, bool, Notes)]| {
        let mut all = Notes::default();
        list.iter().for_each(|t| all.add(&t.2));
        all
    };
    let submitting = match submitter.and_then(|id| title.iter().find(|t| t.0 == id)) {
        Some(t) => format!(
            "submitting thread {}: in the layer {}; blocked {}; the rest (its own work) {:.2} ms",
            t.0,
            kinds(&t.2, &LAYER[..8]),
            kinds(&t.2, &WAITS),
            ms(rest(&t.2)),
        ),
        None => "no title thread submitted".into(),
    };
    recorders.sort_by_key(|t| std::cmp::Reverse(t.2.ns[Kind::List as usize]));
    let all = sum(&recorders);
    let busiest = recorders
        .iter()
        .take(6)
        .map(|t| {
            format!(
                "{} {:.1}/{:.1}/{:.1}",
                t.0,
                ms(t.2.ns[Kind::List as usize]),
                ms(WAITS.iter().map(|&k| t.2.ns[k as usize]).sum()),
                ms(rest(&t.2))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let recording = format!(
        "recording threads {}: in the layer {}; blocked {}; the rest {:.2} ms; by thread (list calls/blocked/rest ms) {}",
        recorders.len(),
        kinds(&all, &LAYER[2..8]),
        kinds(&all, &WAITS),
        recorders.iter().map(|t| rest(&t.2)).sum::<u64>() as f64 / 1e6 / frames,
        busiest,
    );
    others.sort_by_key(|t| std::cmp::Reverse(wall.saturating_sub(rest(&t.2))));
    let rest_all = sum(&others);
    let other = format!(
        "other title threads {}: in the layer {}; blocked {}",
        others.len(),
        kinds(&rest_all, &LAYER[..8]),
        kinds(&rest_all, &WAITS),
    );
    let lane = if lane.iter().any(|&v| v != 0) {
        format!(
            "; our thread that prepares (VICTORY_ASYNC_SUBMIT): working {:.2} ms, idle with the line empty {:.2} ms, standing at a wait \
             {:.2} ms; idle stretches ended by an entry put in {:.1}, a wake {:.1}, a drain {:.1}, its own look {:.1}",
            ms(lane[L_WORK]),
            ms(lane[L_EMPTY]),
            ms(lane[L_STANDING]),
            lane[L_BY] as f64 / frames,
            lane[L_BY + 1] as f64 / frames,
            lane[L_BY + 2] as f64 / frames,
            lane[L_BY + 3] as f64 / frames,
        )
    } else {
        String::new()
    };
    tracing::info!(
        "title time per frame (VICTORY_TITLE_TIME, ms (times)): {:.0} frames, {:.2} ms each; last submission to present {:.2}, \
         present to the first list call {:.2}, to the first submission {:.2}; {submitting} | {recording} | {other}{lane}",
        frames,
        wall as f64 / 1e6 / frames,
        frame[F_TAIL] as f64 / 1e3 / frames,
        frame[F_TO_LIST] as f64 / 1e3 / frames,
        frame[F_TO_SUBMIT] as f64 / 1e3 / frames,
    );
}

// ---------------------------------------------------------------------------
// The title's waits.

/// The events fences of ours were asked to set (`SetEventOnCompletion`).
static FENCE_EVENTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// A fence of ours will set `event`.
pub(crate) fn fence_event(event: usize) {
    if !on() || event == 0 {
        return;
    }
    let mut all = FENCE_EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    if !all.contains(&event) {
        if all.len() >= 4096 {
            all.clear();
        }
        all.push(event);
    }
}

fn is_fence_event(handles: &[usize]) -> bool {
    let all = FENCE_EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    handles.iter().any(|h| all.contains(h))
}

type Handle = *mut core::ffi::c_void;

/// The real functions behind the wrappers, by [`WRAPPED`]'s index.
static REAL: [AtomicUsize; 13] = [const { AtomicUsize::new(0) }; 13];

const WRAPPED: [&str; 13] = [
    "WaitForSingleObject",
    "WaitForSingleObjectEx",
    "WaitForMultipleObjects",
    "WaitForMultipleObjectsEx",
    "Sleep",
    "SleepEx",
    "WaitOnAddress",
    "SleepConditionVariableSRW",
    "SleepConditionVariableCS",
    "EnterCriticalSection",
    "AcquireSRWLockExclusive",
    "AcquireSRWLockShared",
    "SignalObjectAndWait",
];

fn real(i: usize) -> usize {
    REAL[i].load(Relaxed)
}

/// Runs `call`, a wait of kind `kind`; what the caller asks `GetLastError`
/// about stays.
fn waited<T>(kind: Kind, call: impl FnOnce() -> T) -> T {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    let began = Instant::now();
    let result = call();
    // SAFETY: plain queries of this thread's own.
    unsafe {
        let error = GetLastError();
        note(kind, began.elapsed().as_nanos() as u64);
        SetLastError(error);
    }
    result
}

fn object_kind(handles: &[usize]) -> Kind {
    if is_fence_event(handles) { Kind::WaitFence } else { Kind::WaitOther }
}

unsafe extern "system" fn wait_one(object: Handle, ms: u32) -> u32 {
    // SAFETY: the real function, with the caller's own arguments (so below).
    let f: unsafe extern "system" fn(Handle, u32) -> u32 = unsafe { std::mem::transmute(real(0)) };
    if ms == 0 {
        return unsafe { f(object, ms) };
    }
    waited(object_kind(&[object as usize]), || unsafe { f(object, ms) })
}

unsafe extern "system" fn wait_one_ex(object: Handle, ms: u32, alertable: i32) -> u32 {
    let f: unsafe extern "system" fn(Handle, u32, i32) -> u32 = unsafe { std::mem::transmute(real(1)) };
    if ms == 0 {
        return unsafe { f(object, ms, alertable) };
    }
    waited(object_kind(&[object as usize]), || unsafe { f(object, ms, alertable) })
}

fn handles(count: u32, objects: *const Handle) -> Vec<usize> {
    if objects.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller passes `count` handles.
    unsafe { std::slice::from_raw_parts(objects, count.min(64) as usize) }.iter().map(|&h| h as usize).collect()
}

unsafe extern "system" fn wait_many(count: u32, objects: *const Handle, all: i32, ms: u32) -> u32 {
    let f: unsafe extern "system" fn(u32, *const Handle, i32, u32) -> u32 = unsafe { std::mem::transmute(real(2)) };
    if ms == 0 {
        return unsafe { f(count, objects, all, ms) };
    }
    waited(object_kind(&handles(count, objects)), || unsafe { f(count, objects, all, ms) })
}

unsafe extern "system" fn wait_many_ex(count: u32, objects: *const Handle, all: i32, ms: u32, alertable: i32) -> u32 {
    let f: unsafe extern "system" fn(u32, *const Handle, i32, u32, i32) -> u32 = unsafe { std::mem::transmute(real(3)) };
    if ms == 0 {
        return unsafe { f(count, objects, all, ms, alertable) };
    }
    waited(object_kind(&handles(count, objects)), || unsafe { f(count, objects, all, ms, alertable) })
}

unsafe extern "system" fn sleep(ms: u32) {
    let f: unsafe extern "system" fn(u32) = unsafe { std::mem::transmute(real(4)) };
    waited(Kind::Sleep, || unsafe { f(ms) })
}

unsafe extern "system" fn sleep_ex(ms: u32, alertable: i32) -> u32 {
    let f: unsafe extern "system" fn(u32, i32) -> u32 = unsafe { std::mem::transmute(real(5)) };
    waited(Kind::Sleep, || unsafe { f(ms, alertable) })
}

unsafe extern "system" fn wait_on_address(at: Handle, compare: Handle, size: usize, ms: u32) -> i32 {
    let f: unsafe extern "system" fn(Handle, Handle, usize, u32) -> i32 = unsafe { std::mem::transmute(real(6)) };
    waited(Kind::WaitAddress, || unsafe { f(at, compare, size, ms) })
}

unsafe extern "system" fn sleep_cv_srw(cv: Handle, lock: Handle, ms: u32, flags: u32) -> i32 {
    let f: unsafe extern "system" fn(Handle, Handle, u32, u32) -> i32 = unsafe { std::mem::transmute(real(7)) };
    waited(Kind::WaitAddress, || unsafe { f(cv, lock, ms, flags) })
}

unsafe extern "system" fn sleep_cv_cs(cv: Handle, lock: Handle, ms: u32) -> i32 {
    let f: unsafe extern "system" fn(Handle, Handle, u32) -> i32 = unsafe { std::mem::transmute(real(8)) };
    waited(Kind::WaitAddress, || unsafe { f(cv, lock, ms) })
}

/// The try-functions of the locks, by the lock functions' index.
static TRY: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

/// Takes `lock` with `take` (index `i` of [`REAL`]); timed only when its
/// try does not get it.
fn locked(i: usize, lock: Handle) {
    let take: unsafe extern "system" fn(Handle) = unsafe { std::mem::transmute(real(i)) };
    let try_at = TRY[i - 9].load(Relaxed);
    if try_at != 0 {
        // SAFETY: `TryEnterCriticalSection` or a `TryAcquireSRWLock*`: one
        // pointer, a truth value in the low byte.
        let tried: unsafe extern "system" fn(Handle) -> u8 = unsafe { std::mem::transmute(try_at) };
        if unsafe { tried(lock) } != 0 {
            return;
        }
    }
    waited(Kind::Lock, || unsafe { take(lock) })
}

unsafe extern "system" fn enter_cs(lock: Handle) {
    locked(9, lock)
}

unsafe extern "system" fn srw_exclusive(lock: Handle) {
    locked(10, lock)
}

unsafe extern "system" fn srw_shared(lock: Handle) {
    locked(11, lock)
}

unsafe extern "system" fn signal_and_wait(signal: Handle, object: Handle, ms: u32, alertable: i32) -> u32 {
    let f: unsafe extern "system" fn(Handle, Handle, u32, i32) -> u32 = unsafe { std::mem::transmute(real(12)) };
    waited(object_kind(&[object as usize]), || unsafe { f(signal, object, ms, alertable) })
}

fn wrapper(i: usize) -> usize {
    match i {
        0 => wait_one as *const () as usize,
        1 => wait_one_ex as *const () as usize,
        2 => wait_many as *const () as usize,
        3 => wait_many_ex as *const () as usize,
        4 => sleep as *const () as usize,
        5 => sleep_ex as *const () as usize,
        6 => wait_on_address as *const () as usize,
        7 => sleep_cv_srw as *const () as usize,
        8 => sleep_cv_cs as *const () as usize,
        9 => enter_cs as *const () as usize,
        10 => srw_exclusive as *const () as usize,
        11 => srw_shared as *const () as usize,
        12 => signal_and_wait as *const () as usize,
        _ => 0,
    }
}

/// What the title's import `name`, found at `address`, is bound to: our
/// wrapper of it with the switch on, else the address itself.
pub fn import(name: &str, address: usize) -> usize {
    if !on() || address == 0 {
        return address;
    }
    let Some(i) = WRAPPED.iter().position(|&w| w == name) else {
        return address;
    };
    if (9..=11).contains(&i) {
        if level() < 2 {
            return address;
        }
        let try_name = match i {
            9 => c"TryEnterCriticalSection",
            10 => c"TryAcquireSRWLockExclusive",
            _ => c"TryAcquireSRWLockShared",
        };
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        for dll in [c"kernelbase.dll", c"kernel32.dll", c"ntdll.dll"] {
            // SAFETY: NUL-terminated names of modules every process has.
            let module = unsafe { GetModuleHandleA(dll.as_ptr().cast()) };
            if module.is_null() {
                continue;
            }
            // SAFETY: as above.
            if let Some(f) = unsafe { GetProcAddress(module, try_name.as_ptr().cast()) } {
                TRY[i - 9].store(f as *const () as usize, Relaxed);
                break;
            }
        }
        if TRY[i - 9].load(Relaxed) == 0 {
            return address;
        }
    }
    let _ = REAL[i].compare_exchange(0, address, Relaxed, Relaxed);
    if REAL[i].load(Relaxed) != address {
        // Bound to another address before: keep the title's own.
        return address;
    }
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| tracing::info!(level = level(), "the title's waits are timed (VICTORY_TITLE_TIME)"));
    wrapper(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls of a fence that does not move, then moves, make one spin; a
    /// poll of another fence ends nothing.
    #[test]
    fn a_spin_on_a_fence_is_from_the_first_unmoved_poll_to_the_move() {
        let mut l = Local { notes: Notes::default(), given: None, depth: 0, poll: (0, 0, None) };
        assert_eq!(poll_step(&mut l, 1, 5), None, "a first poll");
        assert_eq!(poll_step(&mut l, 1, 5), None, "the spin begins");
        assert_eq!(poll_step(&mut l, 1, 5), None);
        assert!(poll_step(&mut l, 1, 6).is_some(), "it moved: one spin");
        assert_eq!(poll_step(&mut l, 2, 6), None, "another fence");
        assert_eq!(poll_step(&mut l, 1, 6), None, "back, no spin standing");
        assert_eq!(poll_step(&mut l, 1, 7), None, "moved with no spin before it");
    }

    #[test]
    fn every_wrapped_name_has_a_wrapper() {
        for i in 0..WRAPPED.len() {
            assert_ne!(wrapper(i), 0, "{}", WRAPPED[i]);
        }
    }
}
