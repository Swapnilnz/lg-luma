//! Game controllers (a Bluetooth DualSense, Xbox pad and the like) drive Luma as the remote
//! does: their buttons and left stick become the remote's keys, so every surface keeps one
//! input path. Directions repeat while held, as a remote's keys do.
use sdl2::{
    GameControllerSubsystem, JoystickSubsystem,
    controller::{Axis, Button, GameController},
    event::Event,
    joystick::{HatState, Joystick},
    keyboard::{Keycode, Mod},
};
use std::time::{Duration, Instant};

/// The first repeat of a held direction, then each one after.
const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_RATE: Duration = Duration::from_millis(110);
/// The left stick presses a direction past this deflection and lets go below the second.
const STICK_PRESS: i16 = 20_000;
const STICK_RELEASE: i16 = 12_000;

/// What a controller's button stands for, on the remote.
fn button_key(button: Button) -> Option<Keycode> {
    Some(match button {
        Button::DPadUp => Keycode::Up,
        Button::DPadDown => Keycode::Down,
        Button::DPadLeft => Keycode::Left,
        Button::DPadRight => Keycode::Right,
        // Cross on a PlayStation pad, A on Xbox's.
        Button::A => Keycode::Return,
        // Circle / B: Back (held, the recents).
        Button::B => Keycode::Escape,
        // Triangle / Y and Options / Start: the focused item's options.
        Button::Y | Button::Start => Keycode::Menu,
        // Square / X: the title's trailer.
        Button::X => Keycode::AudioPlay,
        // The PS / Guide button: back to the top of Home.
        Button::Guide => Keycode::Home,
        _ => return None,
    })
}

fn repeats(key: Keycode) -> bool {
    matches!(
        key,
        Keycode::Up | Keycode::Down | Keycode::Left | Keycode::Right
    )
}

fn key(down: bool, keycode: Keycode, repeat: bool) -> Event {
    if down {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(keycode),
            scancode: None,
            keymod: Mod::NOMOD,
            repeat,
        }
    } else {
        Event::KeyUp {
            timestamp: 0,
            window_id: 0,
            keycode: Some(keycode),
            scancode: None,
            keymod: Mod::NOMOD,
            repeat: false,
        }
    }
}

#[derive(Default)]
pub struct Gamepads {
    controllers: Option<GameControllerSubsystem>,
    joysticks: Option<JoystickSubsystem>,
    open: Vec<GameController>,
    /// Joysticks SDL has no controller mapping for: only their hat and first two buttons.
    raw: Vec<Joystick>,
    /// The direction held, and when it next repeats.
    held: Option<(Keycode, Instant)>,
    /// The direction each stick axis (X, Y) presses, if any.
    stick: [Option<Keycode>; 2],
    /// The direction an unmapped joystick's hat presses.
    hat: Option<Keycode>,
}

impl Gamepads {
    pub fn new(sdl: &sdl2::Sdl) -> Self {
        // webOS does not always tell SDL the window has focus; Luma itself only passes
        // controller input on while it is visible.
        sdl2::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
        let joysticks = sdl
            .joystick()
            .map_err(|error| eprintln!("Luma gamepad: no joystick support: {error}"))
            .ok();
        let controllers = sdl
            .game_controller()
            .map_err(|error| eprintln!("Luma gamepad: no controller support: {error}"))
            .ok();
        if let Some(joysticks) = &joysticks {
            eprintln!(
                "Luma gamepad: {} joystick(s) at start",
                joysticks.num_joysticks().unwrap_or(0)
            );
        }
        // Devices present at start arrive as added events too.
        Self {
            controllers,
            joysticks,
            ..Self::default()
        }
    }

    /// Turns a controller's event into the remote's key events (none for others' events,
    /// which come back unchanged). `visible`: whether Luma is in front to take input.
    pub fn translate(&mut self, event: Event, visible: bool) -> Vec<Event> {
        match event {
            Event::ControllerDeviceAdded { which, .. } => {
                if let Some(controllers) = &self.controllers {
                    match controllers.open(which) {
                        Ok(controller) => {
                            eprintln!("Luma gamepad: connected {}", controller.name());
                            let id = controller.instance_id();
                            self.raw.retain(|joystick| joystick.instance_id() != id);
                            self.open.push(controller);
                        }
                        Err(error) => eprintln!("Luma gamepad: couldn't open {which}: {error}"),
                    }
                }
                vec![]
            }
            Event::ControllerDeviceRemoved { which, .. } => {
                self.open
                    .retain(|controller| controller.instance_id() != which);
                eprintln!("Luma gamepad: disconnected {which}");
                self.release()
            }
            Event::JoyDeviceAdded { which, .. } => {
                let mapped = self
                    .controllers
                    .as_ref()
                    .is_some_and(|controllers| controllers.is_game_controller(which));
                if !mapped && let Some(joysticks) = &self.joysticks {
                    match joysticks.open(which) {
                        Ok(joystick) => {
                            eprintln!(
                                "Luma gamepad: {} has no controller mapping ({}); using its hat and first buttons",
                                joystick.name(),
                                joystick.guid()
                            );
                            self.raw.push(joystick);
                        }
                        Err(error) => {
                            eprintln!("Luma gamepad: couldn't open joystick {which}: {error}");
                        }
                    }
                }
                vec![]
            }
            Event::JoyDeviceRemoved { which, .. } => {
                let before = self.raw.len();
                self.raw.retain(|joystick| joystick.instance_id() != which);
                if self.raw.len() != before {
                    return self.release();
                }
                vec![]
            }
            Event::ControllerButtonDown { button, .. } if visible => {
                button_key(button).map_or_else(Vec::new, |keycode| self.press(keycode))
            }
            Event::ControllerButtonUp { button, .. } => {
                button_key(button).map_or_else(Vec::new, |keycode| self.lift(keycode))
            }
            Event::ControllerAxisMotion { axis, value, .. } => {
                let (index, negative, positive) = match axis {
                    Axis::LeftX => (0, Keycode::Left, Keycode::Right),
                    Axis::LeftY => (1, Keycode::Up, Keycode::Down),
                    _ => return vec![],
                };
                let wanted = match (self.stick[index], value) {
                    (_, value) if value <= -STICK_PRESS => Some(negative),
                    (_, value) if value >= STICK_PRESS => Some(positive),
                    (Some(current), value) if value.unsigned_abs() >= STICK_RELEASE as u16 => {
                        Some(current)
                    }
                    _ => None,
                };
                self.steer(index, wanted, visible)
            }
            Event::JoyHatMotion { which, state, .. } if self.is_raw(which) => {
                let wanted = match state {
                    HatState::Up => Some(Keycode::Up),
                    HatState::Down => Some(Keycode::Down),
                    HatState::Left => Some(Keycode::Left),
                    HatState::Right => Some(Keycode::Right),
                    _ => None,
                };
                let mut events = vec![];
                if let Some(previous) = self.hat.take() {
                    events.extend(self.lift(previous));
                }
                if let Some(keycode) = wanted
                    && visible
                {
                    self.hat = Some(keycode);
                    events.extend(self.press(keycode));
                }
                events
            }
            Event::JoyButtonDown {
                which, button_idx, ..
            } if visible && self.is_raw(which) => match button_idx {
                0 => self.press(Keycode::Return),
                1 => self.press(Keycode::Escape),
                _ => vec![],
            },
            Event::JoyButtonUp {
                which, button_idx, ..
            } if self.is_raw(which) => match button_idx {
                0 => self.lift(Keycode::Return),
                1 => self.lift(Keycode::Escape),
                _ => vec![],
            },
            Event::ControllerButtonDown { .. }
            | Event::JoyAxisMotion { .. }
            | Event::JoyBallMotion { .. }
            | Event::JoyHatMotion { .. }
            | Event::JoyButtonDown { .. }
            | Event::JoyButtonUp { .. }
            | Event::ControllerTouchpadDown { .. }
            | Event::ControllerTouchpadMotion { .. }
            | Event::ControllerTouchpadUp { .. }
            | Event::ControllerDeviceRemapped { .. } => vec![],
            other => vec![other],
        }
    }

    fn is_raw(&self, which: u32) -> bool {
        self.raw
            .iter()
            .any(|joystick| joystick.instance_id() == which)
    }

    /// Moves one stick axis's direction to `wanted`.
    fn steer(&mut self, index: usize, wanted: Option<Keycode>, visible: bool) -> Vec<Event> {
        if self.stick[index] == wanted {
            return vec![];
        }
        let mut events = vec![];
        if let Some(previous) = self.stick[index].take() {
            events.extend(self.lift(previous));
        }
        if let Some(keycode) = wanted
            && visible
        {
            self.stick[index] = Some(keycode);
            events.extend(self.press(keycode));
        }
        events
    }

    fn press(&mut self, keycode: Keycode) -> Vec<Event> {
        if repeats(keycode) {
            self.held = Some((keycode, Instant::now() + REPEAT_DELAY));
        }
        vec![key(true, keycode, false)]
    }

    fn lift(&mut self, keycode: Keycode) -> Vec<Event> {
        if self.held.is_some_and(|(held, _)| held == keycode) {
            self.held = None;
        }
        vec![key(false, keycode, false)]
    }

    /// Lets go of whatever a controller that went away was holding.
    fn release(&mut self) -> Vec<Event> {
        let mut events = vec![];
        if let Some((keycode, _)) = self.held.take() {
            events.push(key(false, keycode, false));
        }
        self.stick = [None; 2];
        self.hat = None;
        events
    }

    /// The held direction's repeat, once it is due. Hidden, the hold is let go.
    pub fn repeat(&mut self, now: Instant, visible: bool) -> Option<Event> {
        let (keycode, due) = self.held?;
        if !visible {
            self.held = None;
            return None;
        }
        if now < due {
            return None;
        }
        self.held = Some((keycode, now + REPEAT_RATE));
        Some(key(true, keycode, true))
    }

    /// How long the event loop may sleep before the next repeat is due.
    pub fn wait(&self, now: Instant) -> Option<u32> {
        self.held
            .map(|(_, due)| due.saturating_duration_since(now).as_millis() as u32)
    }
}
