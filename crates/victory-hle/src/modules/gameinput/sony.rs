//! PlayStation pads (DualSense, DualSense Edge, DualShock 4), read through
//! Windows.Gaming.Input's `RawGameController` (USB and Bluetooth, no driver
//! to install) for the host slots XInput leaves empty.
//!
//! `VICTORY_PS_PADS`: `auto` (the default) reads them only while no XInput
//! pad is connected, so that the Xbox pad DS4Windows or Steam makes of one
//! and the pad under it are not both read; `1` always; `0` never.
//! `VICTORY_PS_PADS_RAW=1` logs each change of a pad's raw buttons, axes and
//! hat, to check the mapping on a pad not seen yet.
//!
//! Cross is A, Circle B, Square X, Triangle Y; L1 and R1 the shoulders; L2 and
//! R2 the triggers; Create View; Options Menu; L3 and R3 the sticks' clicks;
//! the hat the D-pad. The PS button, the touchpad, the gyro, rumble and the
//! adaptive triggers are not passed on.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use windows::Gaming::Input::{GameControllerSwitchPosition, RawGameController};

use super::{
    A, B, DOWN, LEFT, LEFT_SHOULDER, LEFT_STICK, MENU, PadState, RIGHT, RIGHT_SHOULDER, RIGHT_STICK, UP, VIEW, X, Y,
};

/// Sony's USB vendor ID.
const SONY: u16 = 0x054C;

/// How often the list of controllers is looked at.
const LOOK_AGAIN: Duration = Duration::from_secs(1);

/// Buttons by their place in the pad's HID report (Square, Cross, Circle,
/// Triangle, L1, R1, L2, R2, Create, Options, L3, R3, PS, touchpad).
const BUTTONS: [(usize, u32); 10] = [
    (0, X),
    (1, A),
    (2, B),
    (3, Y),
    (4, LEFT_SHOULDER),
    (5, RIGHT_SHOULDER),
    (8, VIEW),
    (9, MENU),
    (10, LEFT_STICK),
    (11, RIGHT_STICK),
];

/// L2 and R2 as buttons, for a pad whose triggers are no axes.
const DIGITAL_TRIGGERS: [usize; 2] = [6, 7];

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

/// Which axes are the sticks (left x, left y, right x, right y) and which the
/// triggers (left, right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Layout {
    sticks: [usize; 4],
    triggers: Option<[usize; 2]>,
}

impl Layout {
    /// From a reading at rest (0 to 1): the sticks rest in the middle, the
    /// triggers at an end, each in the report's order. A pad held at the
    /// first reading gets the usual order of a PlayStation pad's HID axes
    /// (X, Y, Z, Rx, Ry, Rz: left stick, right x, the triggers, right y).
    pub(super) fn at_rest(axes: &[f64]) -> Layout {
        let middle: Vec<usize> = (0..axes.len()).filter(|&i| (axes[i] - 0.5).abs() < 0.25).collect();
        let ends: Vec<usize> = (0..axes.len()).filter(|&i| (axes[i] - 0.5).abs() >= 0.25).collect();
        match (middle.as_slice(), ends.as_slice()) {
            (&[lx, ly, rx, ry], &[lt, rt, ..]) => Layout { sticks: [lx, ly, rx, ry], triggers: Some([lt, rt]) },
            (&[lx, ly, rx, ry], _) => Layout { sticks: [lx, ly, rx, ry], triggers: None },
            _ if axes.len() >= 6 => Layout { sticks: [0, 1, 2, 5], triggers: Some([3, 4]) },
            _ => Layout { sticks: [0, 1, 2, 3], triggers: None },
        }
    }
}

/// The D-pad buttons of a hat position.
fn hat(position: GameControllerSwitchPosition) -> u32 {
    match position {
        GameControllerSwitchPosition::Up => UP,
        GameControllerSwitchPosition::UpRight => UP | RIGHT,
        GameControllerSwitchPosition::Right => RIGHT,
        GameControllerSwitchPosition::DownRight => DOWN | RIGHT,
        GameControllerSwitchPosition::Down => DOWN,
        GameControllerSwitchPosition::DownLeft => DOWN | LEFT,
        GameControllerSwitchPosition::Left => LEFT,
        GameControllerSwitchPosition::UpLeft => UP | LEFT,
        _ => 0,
    }
}

/// A console pad's state from a raw reading: buttons by report place, the
/// hat, axes from 0 to 1 (HID's y grows downwards, the console's upwards).
/// Raw values, no dead zone: titles apply their own.
pub(super) fn convert(buttons: &[bool], position: GameControllerSwitchPosition, axes: &[f64], layout: &Layout) -> PadState {
    let pressed = |i: usize| buttons.get(i).copied().unwrap_or(false);
    let axis = |i: usize| axes.get(i).copied().unwrap_or(0.5);
    let stick = |v: f64| ((v * 2.0 - 1.0) as f32).clamp(-1.0, 1.0);
    let trigger = |n: usize| match layout.triggers {
        Some(t) => (axis(t[n]) as f32).clamp(0.0, 1.0),
        None => f32::from(u8::from(pressed(DIGITAL_TRIGGERS[n]))),
    };
    let [lx, ly, rx, ry] = layout.sticks;
    PadState {
        buttons: BUTTONS.iter().filter(|(i, _)| pressed(*i)).fold(hat(position), |all, (_, bit)| all | bit),
        left_trigger: trigger(0),
        right_trigger: trigger(1),
        left_x: stick(axis(lx)),
        left_y: -stick(axis(ly)),
        right_x: stick(axis(rx)),
        right_y: -stick(axis(ry)),
    }
}

/// WinRT on the calling thread, once (the poll thread and the first caller).
fn init_thread() {
    thread_local! {
        static INIT: () = {
            // SAFETY: plain initialisation; a thread already in another
            // apartment keeps it, and WinRT works there as well.
            let _ = unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) };
        };
    }
    INIT.with(|_| ());
}

/// One pad and its reading buffers.
struct Pad {
    controller: RawGameController,
    id: String,
    layout: Option<Layout>,
    buttons: Vec<bool>,
    switches: Vec<GameControllerSwitchPosition>,
    axes: Vec<f64>,
    /// The last raw reading logged (`VICTORY_PS_PADS_RAW`).
    logged: Option<(Vec<bool>, i32, Vec<i32>)>,
}

impl Pad {
    fn read(&mut self) -> Option<PadState> {
        self.controller.GetCurrentReading(&mut self.buttons, &mut self.switches, &mut self.axes).ok()?;
        let layout = *self.layout.get_or_insert_with(|| {
            let layout = Layout::at_rest(&self.axes);
            tracing::info!(id = %self.id, axes_at_rest = ?self.axes, ?layout, "a PlayStation pad's axes");
            layout
        });
        let position = self.switches.first().copied().unwrap_or(GameControllerSwitchPosition::Center);
        if *RAW {
            let now = (self.buttons.clone(), position.0, self.axes.iter().map(|v| (v * 100.0).round() as i32).collect());
            if self.logged.as_ref() != Some(&now) {
                let pressed: Vec<usize> = (0..self.buttons.len()).filter(|&i| self.buttons[i]).collect();
                tracing::info!(id = %self.id, ?pressed, hat = position.0, axes_percent = ?now.2, "a PlayStation pad's raw reading");
                self.logged = Some(now);
            }
        }
        Some(convert(&self.buttons, position, &self.axes, &layout))
    }
}

/// The PlayStation pads found.
#[derive(Default)]
pub(super) struct Pads {
    looked: Option<Instant>,
    pads: Vec<Pad>,
}

// SAFETY: Windows.Gaming.Input's controllers are agile (callable from any
// thread); the buffers are plain data, used under the source's lock.
unsafe impl Send for Pads {}

impl Pads {
    /// The pads' states, in the order they were found; none while `xinput`
    /// has a pad in `auto` mode ([`MODE`]).
    pub(super) fn read(&mut self, xinput: bool) -> Vec<PadState> {
        match *MODE {
            Mode::Never => return Vec::new(),
            Mode::Auto if xinput => return Vec::new(),
            _ => {}
        }
        init_thread();
        if self.looked.is_none_or(|at| at.elapsed() >= LOOK_AGAIN) {
            self.looked = Some(Instant::now());
            self.find();
        }
        self.pads.iter_mut().filter_map(Pad::read).collect()
    }

    /// Takes in the PlayStation pads now connected and lets go of the others.
    fn find(&mut self) {
        let Ok(list) = RawGameController::RawGameControllers() else {
            return;
        };
        let mut seen = Vec::new();
        for controller in list {
            let counts = (
                controller.ButtonCount().unwrap_or(0),
                controller.AxisCount().unwrap_or(0),
                controller.SwitchCount().unwrap_or(0),
            );
            if controller.HardwareVendorId().ok() != Some(SONY) || counts.0 < 12 || counts.1 < 4 {
                continue;
            }
            let id = controller.NonRoamableId().map(|s| s.to_string()).unwrap_or_default();
            if !self.pads.iter().any(|p| p.id == id) {
                tracing::info!(
                    name = %controller.DisplayName().map(|s| s.to_string()).unwrap_or_default(),
                    product = format_args!("{:#06x}", controller.HardwareProductId().unwrap_or(0)),
                    buttons = counts.0,
                    axes = counts.1,
                    switches = counts.2,
                    "a PlayStation pad, read through Windows.Gaming.Input (VICTORY_PS_PADS)"
                );
                self.pads.push(Pad {
                    controller,
                    id: id.clone(),
                    layout: None,
                    buttons: vec![false; counts.0 as usize],
                    switches: vec![GameControllerSwitchPosition::Center; counts.2 as usize],
                    axes: vec![0.5; counts.1 as usize],
                    logged: None,
                });
            }
            seen.push(id);
        }
        self.pads.retain(|p| {
            let kept = seen.contains(&p.id);
            if !kept {
                tracing::info!(id = %p.id, "a PlayStation pad went");
            }
            kept
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DualSense at rest in the HID order (sticks in the middle, triggers
    /// at 0, right y last), Cross and Options held, the hat up-left, the left
    /// stick fully up and right, R2 halfway.
    #[test]
    fn a_dualsense_reading_becomes_a_console_pad() {
        let layout = Layout::at_rest(&[0.5, 0.5, 0.5, 0.0, 0.0, 0.5]);
        assert_eq!(layout, Layout { sticks: [0, 1, 2, 5], triggers: Some([3, 4]) });
        let mut buttons = vec![false; 14];
        buttons[1] = true;
        buttons[9] = true;
        let state = convert(&buttons, GameControllerSwitchPosition::UpLeft, &[1.0, 0.0, 0.5, 0.0, 0.5, 0.5], &layout);
        assert_eq!(state.buttons, A | MENU | UP | LEFT);
        assert_eq!((state.left_x, state.left_y, state.right_x, state.right_y), (1.0, 1.0, 0.0, 0.0));
        assert_eq!((state.left_trigger, state.right_trigger), (0.0, 0.5));
    }

    /// Triggers placed elsewhere in the report are found by where they rest.
    #[test]
    fn the_triggers_are_found_where_they_rest() {
        assert_eq!(
            Layout::at_rest(&[0.0, 0.5, 0.5, 0.5, 0.5, 0.0]),
            Layout { sticks: [1, 2, 3, 4], triggers: Some([0, 5]) }
        );
        // Held at the first reading: the usual order.
        assert_eq!(Layout::at_rest(&[0.9, 0.5, 0.5, 0.0, 0.0, 0.5]), Layout { sticks: [0, 1, 2, 5], triggers: Some([3, 4]) });
    }

    /// Without trigger axes L2 and R2 count as buttons.
    #[test]
    fn digital_triggers_without_axes() {
        let layout = Layout::at_rest(&[0.5; 4]);
        assert_eq!(layout.triggers, None);
        let mut buttons = vec![false; 14];
        buttons[7] = true;
        let state = convert(&buttons, GameControllerSwitchPosition::Center, &[0.5; 4], &layout);
        assert_eq!((state.left_trigger, state.right_trigger, state.buttons), (0.0, 1.0, 0));
    }
}
