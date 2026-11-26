//! PlayStation pads (DualSense, DualSense Edge, DualShock 4), read from their
//! own HID input reports (USB or Bluetooth, no driver or DS4Windows) for the
//! host slots XInput leaves empty. The device is opened shared, so Steam
//! holding it too is no obstacle; a thread per pad reads its reports.
//!
//! (Windows.Gaming.Input's raw controllers listed no DualSense in a desktop
//! process on the RX 6700 XT's PC, the pad on USB: hence the reports.)
//!
//! `VICTORY_PS_PADS`: `auto` (the default) reads them only while no XInput
//! pad is connected, so that the Xbox pad DS4Windows or Steam makes of one
//! and the pad under it are not both read; `1` always; `0` never.
//! `VICTORY_PS_PADS_RAW=1` (`play.ps1 -PadLog`) logs a report's bytes each
//! time what it says changes (not its counter, which every report moves).
//!
//! Cross is A, Circle B, Square X, Triangle Y; L1 and R1 the shoulders; L2 and
//! R2 the triggers; Create (Share) View; Options Menu; L3 and R3 the sticks'
//! clicks; the hat the D-pad. The PS button, the touchpad, the gyro, rumble
//! and the adaptive triggers are not passed on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid,
    HidD_GetPreparsedData, HidP_GetCaps,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
};

use super::{
    A, B, DOWN, LEFT, LEFT_SHOULDER, LEFT_STICK, MENU, PadState, RIGHT, RIGHT_SHOULDER, RIGHT_STICK, UP, VIEW, X, Y,
};

/// Sony's USB vendor ID.
const SONY: u16 = 0x054C;

/// The pads taken, by product ID, and whether each has the DualSense's
/// report layout ([`Kind`]).
const PRODUCTS: [(u16, Kind); 5] = [
    (0x0CE6, Kind::DualSense),
    (0x0DF2, Kind::DualSense), // Edge
    (0x05C4, Kind::DualShock4),
    (0x09CC, Kind::DualShock4),
    (0x0BA0, Kind::DualShock4), // wireless adapter
];

/// How often the devices are looked for again.
const LOOK_AGAIN: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    DualSense,
    DualShock4,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Auto,
    Always,
    Never,
}

static MODE: LazyLock<Mode> = LazyLock::new(|| match std::env::var("VICTORY_PS_PADS").as_deref() {
    Ok("1") => Mode::Always,
    Ok("0") => Mode::Never,
    _ => Mode::Auto,
});

static RAW: LazyLock<bool> = LazyLock::new(|| std::env::var_os("VICTORY_PS_PADS_RAW").is_some_and(|v| v == "1"));

/// The D-pad buttons of a hat value (0 up, clockwise to 7 up-left; 8 none).
fn hat(value: u8) -> u32 {
    match value & 0x0F {
        0 => UP,
        1 => UP | RIGHT,
        2 => RIGHT,
        3 => DOWN | RIGHT,
        4 => DOWN,
        5 => DOWN | LEFT,
        6 => LEFT,
        7 => UP | LEFT,
        _ => 0,
    }
}

/// A console pad's state from the report's bytes: the sticks, the triggers
/// and two button bytes (hat and face; shoulders, triggers, Create, Options,
/// stick clicks). Raw values, no dead zone: titles apply their own.
fn state(sticks: [u8; 4], triggers: [u8; 2], buttons: [u8; 2]) -> PadState {
    let axis = |v: u8| ((f32::from(v) - 128.0) / 127.0).clamp(-1.0, 1.0);
    let bit = |byte: u8, n: u8, button: u32| if byte & (1 << n) != 0 { button } else { 0 };
    let [face, other] = buttons;
    PadState {
        buttons: hat(face)
            | bit(face, 4, X)
            | bit(face, 5, A)
            | bit(face, 6, B)
            | bit(face, 7, Y)
            | bit(other, 0, LEFT_SHOULDER)
            | bit(other, 1, RIGHT_SHOULDER)
            | bit(other, 4, VIEW)
            | bit(other, 5, MENU)
            | bit(other, 6, LEFT_STICK)
            | bit(other, 7, RIGHT_STICK),
        left_trigger: f32::from(triggers[0]) / 255.0,
        right_trigger: f32::from(triggers[1]) / 255.0,
        // HID's y grows downwards, the console's upwards.
        left_x: axis(sticks[0]),
        left_y: -axis(sticks[1]),
        right_x: axis(sticks[2]),
        right_y: -axis(sticks[3]),
    }
}

/// The state in an input report (`report[0]` its ID), if it is one we know:
/// the DualSense's full report over USB (0x01, 64 bytes: sticks 1-4,
/// triggers 5-6, buttons 8-9) and over Bluetooth (0x31, the same one byte
/// on); the short report a pad sends over Bluetooth before it is asked for
/// the full one, and the DualShock 4's over USB (0x01: sticks 1-4, buttons
/// 5-6, triggers 8-9), and the DualShock 4's over Bluetooth (0x11, two bytes
/// on).
pub(super) fn parse(report: &[u8], kind: Kind) -> Option<PadState> {
    let full = |o: usize| -> Option<PadState> {
        let b = report.get(o..o + 11)?;
        Some(state([b[1], b[2], b[3], b[4]], [b[5], b[6]], [b[8], b[9]]))
    };
    let short = |o: usize| -> Option<PadState> {
        let b = report.get(o..o + 10)?;
        Some(state([b[1], b[2], b[3], b[4]], [b[8], b[9]], [b[5], b[6]]))
    };
    match (*report.first()?, kind) {
        (0x01, Kind::DualSense) if report.len() >= 64 => full(0),
        (0x31, Kind::DualSense) => full(1),
        (0x01, _) => short(0),
        (0x11, Kind::DualShock4) => short(2),
        _ => None,
    }
}

/// A handle that crosses to the reading thread.
struct Handle(HANDLE);

// SAFETY: a file handle, used by one thread at a time.
unsafe impl Send for Handle {}

/// One pad: where its device is, its latest state, whether its thread reads.
struct Pad {
    path: Vec<u16>,
    state: Arc<Mutex<Option<PadState>>>,
    reading: Arc<AtomicBool>,
}

/// The PlayStation pads found.
#[derive(Default)]
pub(super) struct Pads {
    looked: Option<Instant>,
    pads: Vec<Pad>,
}

impl Pads {
    /// The pads' states, in the order they were found; none while `xinput`
    /// has a pad in `auto` mode ([`MODE`]).
    pub(super) fn read(&mut self, xinput: bool) -> Vec<PadState> {
        match *MODE {
            Mode::Never => return Vec::new(),
            Mode::Auto if xinput => return Vec::new(),
            _ => {}
        }
        if self.looked.is_none_or(|at| at.elapsed() >= LOOK_AGAIN) {
            self.looked = Some(Instant::now());
            self.pads.retain(|p| p.reading.load(Ordering::Relaxed));
            for path in hid_paths() {
                if !self.pads.iter().any(|p| p.path == path)
                    && let Some(pad) = open(path)
                {
                    self.pads.push(pad);
                }
            }
        }
        self.pads.iter().filter_map(|p| *p.state.lock().unwrap_or_else(|e| e.into_inner())).collect()
    }
}

/// The device paths of the HID devices present (null-terminated).
fn hid_paths() -> Vec<Vec<u16>> {
    let mut paths = Vec::new();
    // SAFETY: plain SetupAPI queries into buffers of the sizes they ask for;
    // the list is destroyed at the end.
    unsafe {
        let mut guid = std::mem::zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set as isize == -1 {
            return paths;
        }
        for index in 0.. {
            let mut interface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, index, &mut interface) == 0 {
                break;
            }
            let mut size = 0;
            SetupDiGetDeviceInterfaceDetailW(set, &interface, std::ptr::null_mut(), 0, &mut size, std::ptr::null_mut());
            if size == 0 {
                continue;
            }
            // u32s: the detail's alignment.
            let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
            let detail = buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &interface, detail, size, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                continue;
            }
            let start = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
            let length = (0..).take_while(|&i| *start.add(i) != 0).count();
            let mut path = std::slice::from_raw_parts(start, length).to_vec();
            path.push(0);
            paths.push(path);
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    paths
}

/// Opens the device at `path` if it is a PlayStation pad we know, and starts
/// its reading thread.
fn open(path: Vec<u16>) -> Option<Pad> {
    // Read and write (the Bluetooth feature report asks it of some stacks),
    // else read alone.
    let device = [GENERIC_READ | GENERIC_WRITE, GENERIC_READ]
        .into_iter()
        .map(|access| {
            // SAFETY: a null-terminated path; the handle is closed on every way out.
            unsafe {
                CreateFileW(
                    path.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            }
        })
        .find(|&handle| handle != INVALID_HANDLE_VALUE)?;
    let close = |why: Option<&str>| {
        // SAFETY: our handle, closed once.
        unsafe { CloseHandle(device) };
        if let Some(why) = why {
            tracing::warn!(why, "a PlayStation pad that could not be read");
        }
        None
    };
    // SAFETY: plain data the call fills in.
    let mut attributes: HIDD_ATTRIBUTES = unsafe { std::mem::zeroed() };
    attributes.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
    // SAFETY: room for the attributes.
    if !unsafe { HidD_GetAttributes(device, &mut attributes) } || attributes.VendorID != SONY {
        return close(None);
    }
    let Some(&(_, kind)) = PRODUCTS.iter().find(|(id, _)| *id == attributes.ProductID) else {
        return close(None);
    };
    // SAFETY: plain data the call fills in.
    let mut caps: HIDP_CAPS = unsafe { std::mem::zeroed() };
    let mut preparsed = 0;
    // SAFETY: the parsed descriptor is freed right after its capabilities are read.
    unsafe {
        if !HidD_GetPreparsedData(device, &mut preparsed) {
            return close(Some("no report descriptor"));
        }
        HidP_GetCaps(preparsed, &mut caps);
        HidD_FreePreparsedData(preparsed);
    }
    let length = usize::from(caps.InputReportByteLength);
    if length < 10 {
        // Another of the pad's interfaces.
        return close(None);
    }
    // Over Bluetooth a pad sends a short report until its calibration feature
    // report is read; then the full one (DualSense 0x31, DualShock 4 0x11).
    let bluetooth = length > 64;
    if bluetooth {
        let mut feature = [0u8; 64];
        feature[0] = if kind == Kind::DualSense { 0x05 } else { 0x02 };
        // SAFETY: a buffer of the length given.
        unsafe { HidD_GetFeature(device, feature.as_mut_ptr().cast(), feature.len() as u32) };
    }
    tracing::info!(
        product = format_args!("{:#06x}", attributes.ProductID),
        ?kind,
        bluetooth,
        report_bytes = length,
        "a PlayStation pad, read from its HID reports (VICTORY_PS_PADS)"
    );
    let state = Arc::new(Mutex::new(None));
    let reading = Arc::new(AtomicBool::new(true));
    let (into, flag, handle) = (state.clone(), reading.clone(), Handle(device));
    let spawned = std::thread::Builder::new().name("victory-ps-pad".into()).spawn(move || {
        let handle = handle;
        let mut report = vec![0u8; length];
        let mut last = None;
        loop {
            let mut read = 0;
            // SAFETY: our handle and a buffer of the report's length; a
            // blocking read, ended by an error when the pad goes.
            if unsafe { ReadFile(handle.0, report.as_mut_ptr(), length as u32, &mut read, std::ptr::null_mut()) } == 0 {
                break;
            }
            let report = &report[..read as usize];
            let pad = parse(report, kind);
            if *RAW && pad != last {
                tracing::info!(bytes = ?&report[..report.len().min(12)], ?pad, "a PlayStation pad's report");
            }
            last = pad;
            if let Some(pad) = pad {
                *into.lock().unwrap_or_else(|e| e.into_inner()) = Some(pad);
            }
        }
        tracing::info!("a PlayStation pad went");
        *into.lock().unwrap_or_else(|e| e.into_inner()) = None;
        flag.store(false, Ordering::Relaxed);
        // SAFETY: our handle, closed once.
        unsafe { CloseHandle(handle.0) };
    });
    if spawned.is_err() {
        return close(Some("no thread to read it"));
    }
    Some(Pad { path, state, reading })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DualSense's USB report: Cross and Options held, the hat up-left, the
    /// left stick fully right and up, R2 halfway, the right stick at rest.
    #[test]
    fn a_dualsense_usb_report_becomes_a_console_pad() {
        let mut report = [0u8; 64];
        report[..11].copy_from_slice(&[0x01, 255, 0, 128, 128, 0, 128, 0, 0x20 | 7, 0x20, 0]);
        let state = parse(&report, Kind::DualSense).unwrap();
        assert_eq!(state.buttons, A | MENU | UP | LEFT);
        assert_eq!((state.left_x, state.left_y, state.right_x, state.right_y), (1.0, 1.0, 0.0, 0.0));
        assert_eq!(state.left_trigger, 0.0);
        assert!((state.right_trigger - 128.0 / 255.0).abs() < 1e-6);
    }

    /// Over Bluetooth the full report is one byte on, and nothing on the hat
    /// (8) is no D-pad button.
    #[test]
    fn a_dualsense_bluetooth_report_is_one_byte_on() {
        let mut report = [0u8; 78];
        report[..12].copy_from_slice(&[0x31, 0, 128, 128, 128, 128, 255, 0, 0, 8 | 0x40, 0x01, 0]);
        let state = parse(&report, Kind::DualSense).unwrap();
        assert_eq!(state.buttons, B | LEFT_SHOULDER);
        assert_eq!(state.left_trigger, 1.0);
    }

    /// The short report (a DualShock 4's, or any pad's over Bluetooth before
    /// the full one): buttons before the triggers.
    #[test]
    fn the_short_report_has_the_buttons_first() {
        let report = [0x01, 128, 128, 128, 128, 8 | 0x10, 0x10, 0, 0, 255];
        let state = parse(&report, Kind::DualShock4).unwrap();
        assert_eq!(state.buttons, X | VIEW);
        assert_eq!((state.left_trigger, state.right_trigger), (0.0, 1.0));
        assert_eq!(parse(&[0x07; 20], Kind::DualSense), None, "another report");
    }
}
