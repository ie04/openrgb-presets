//! Keypress-driven radial ripple animation.

use std::{
    collections::HashMap,
    error::Error,
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};

use evdev::{Device, EventSummary, KeyCode};
use openrgb2::{Color, OpenRgbClient};

use crate::{CYAN, is_target_keyboard};

pub const DEFAULT_SPEED: f32 = 24.0;
pub const MIN_SPEED: f32 = 1.0;
pub const MAX_SPEED: f32 = 50.0;

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const RING_WIDTH: f32 = 1.0;
const SPATIAL_DECAY: f32 = 0.035;
const MATRIX_ROWS: usize = 7;
const MATRIX_COLUMNS: usize = 23;
const UNUSED_LED: i16 = -1;

type RippleResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Debug)]
struct ActiveRipple {
    origin: Point,
    started: Instant,
}

#[derive(Clone, Copy, Debug)]
struct RippleSample {
    origin: Point,
    age: f32,
}

// OpenRGB's matrix for the G810-family driver used by the G512/G513. Values
// are controller LED indexes; gaps preserve the physical spacing between key
// groups better than the controller's one-dimensional LED order.
const KEYBOARD_MATRIX: [[i16; MATRIX_COLUMNS]; MATRIX_ROWS] = [
    [
        111, UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED,
        UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED, 116, 114, 115, UNUSED_LED, 113, UNUSED_LED,
        112, 110, UNUSED_LED, UNUSED_LED, UNUSED_LED,
    ],
    [
        37, UNUSED_LED, 54, 55, 56, 57, UNUSED_LED, 58, 59, 60, 61, UNUSED_LED, 62, 63, 64, 65, 66,
        67, 68, 109, 108, 107, 106,
    ],
    [
        49, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, UNUSED_LED, 41, 42, 38, UNUSED_LED, 69, 70, 71,
        79, 80, 81, 82,
    ],
    [
        39, UNUSED_LED, 16, 22, 4, 17, UNUSED_LED, 19, 24, 20, 8, 14, 15, 43, 44, 45, 72, 73, 74,
        91, 92, 93, 83,
    ],
    [
        53, UNUSED_LED, 0, 18, 3, 5, UNUSED_LED, 6, 7, 9, 10, 11, 47, 48, 46, 36, UNUSED_LED,
        UNUSED_LED, UNUSED_LED, 88, 89, 90, UNUSED_LED,
    ],
    [
        99, 96, 25, 23, 2, 21, UNUSED_LED, 1, UNUSED_LED, 13, 12, 50, 51, 52, 103, UNUSED_LED,
        UNUSED_LED, 78, UNUSED_LED, 85, 86, 87, 84,
    ],
    [
        98, 101, 100, UNUSED_LED, UNUSED_LED, UNUSED_LED, UNUSED_LED, 40, UNUSED_LED, UNUSED_LED,
        UNUSED_LED, UNUSED_LED, 104, 105, 97, 102, 76, 77, 75, 94, UNUSED_LED, 95, UNUSED_LED,
    ],
];

/// Runs the ripple animation until `Ctrl+C`.
///
/// The evdev device is only observed, never grabbed. Key events are held only
/// as short-lived in-memory animation state and are never printed or persisted.
pub async fn apply(speed: f32) -> RippleResult<bool> {
    let mut client = OpenRgbClient::connect().await?;
    client.set_name("openrgb-presets ripple").await?;

    let controllers = client.get_all_controllers().await?;
    let Some(keyboard) = controllers.iter().find(|controller| {
        is_target_keyboard(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        )
    }) else {
        return Ok(false);
    };

    let input_path = find_keyboard_input()?;
    let device = Device::open(&input_path)?;
    let mut events = device.into_event_stream()?;

    let coordinates = keyboard_coordinates(keyboard.num_leds());
    let led_ids = keyboard
        .led_iter()
        .map(|led| (led.name().to_owned(), led.id()))
        .collect::<HashMap<_, _>>();

    keyboard.set_controllable_mode().await?;
    keyboard.set_all_leds(Color::default()).await?;

    let max_distance = ((MATRIX_COLUMNS as f32).powi(2) + (MATRIX_ROWS as f32).powi(2)).sqrt();
    let max_age = (max_distance + 3.0 * RING_WIDTH) / speed;
    let mut ripples = Vec::<ActiveRipple>::new();
    let mut frame_timer = tokio::time::interval(FRAME_INTERVAL);
    frame_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    println!(
        "running ripple at {speed:.1} cells/second from {}; press Ctrl+C to stop",
        input_path.display()
    );

    let animation_result: RippleResult<()> = async {
        loop {
            tokio::select! {
                event = events.next_event() => {
                    let event = event?;
                    if let EventSummary::Key(_, key, 1) = event.destructure()
                        && let Some(name) = led_name_for_key(key)
                        && let Some(&led_id) = led_ids.get(name)
                        && let Some(origin) = coordinates.get(led_id).copied().flatten()
                    {
                        ripples.push(ActiveRipple {
                            origin,
                            started: Instant::now(),
                        });
                    }
                }
                _ = frame_timer.tick() => {
                    let now = Instant::now();
                    ripples.retain(|ripple| now.duration_since(ripple.started).as_secs_f32() <= max_age);

                    let samples = ripples
                        .iter()
                        .map(|ripple| RippleSample {
                            origin: ripple.origin,
                            age: now.duration_since(ripple.started).as_secs_f32(),
                        })
                        .collect::<Vec<_>>();
                    let colors = render_frame(&coordinates, &samples, speed);
                    keyboard.set_leds(colors).await?;
                }
                result = &mut shutdown => {
                    result?;
                    break;
                }
                _ = terminate.recv() => break,
            }
        }

        Ok(())
    }
    .await;

    // Restore the normal static preset even when the animation loop fails.
    let restore_result = keyboard.set_all_leds(CYAN).await;
    animation_result?;
    restore_result?;

    Ok(true)
}

fn find_keyboard_input() -> io::Result<PathBuf> {
    fs::read_dir("/dev/input/by-id")?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.starts_with("usb-Logitech_G513_Carbon_")
                && name.ends_with("-event-kbd")
                && !name.contains("-if01-"))
            .then(|| entry.path())
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "G513 keyboard input device not found",
            )
        })
}

fn keyboard_coordinates(led_count: usize) -> Vec<Option<Point>> {
    let mut coordinates = vec![None; led_count];

    for (row, values) in KEYBOARD_MATRIX.iter().enumerate() {
        for (column, &led_id) in values.iter().enumerate() {
            if led_id != UNUSED_LED
                && let Some(coordinate) = coordinates.get_mut(led_id as usize)
            {
                *coordinate = Some(Point {
                    x: column as f32,
                    y: row as f32,
                });
            }
        }
    }

    coordinates
}

fn render_frame(coordinates: &[Option<Point>], ripples: &[RippleSample], speed: f32) -> Vec<Color> {
    coordinates
        .iter()
        .map(|coordinate| {
            let Some(point) = coordinate else {
                return Color::default();
            };

            let intensity = ripples
                .iter()
                .map(|ripple| ripple_intensity(*point, *ripple, speed))
                .sum::<f32>()
                .clamp(0.0, 1.0);
            let channel = (intensity * 255.0).round() as u8;

            Color::new(0, channel, channel)
        })
        .collect()
}

// This expanding Gaussian ring is an analytic approximation of a radial wave:
// radius grows linearly with real time, while the Gaussian envelope gives the
// front a smooth finite width. Independent waves are summed by `render_frame`,
// following the superposition principle.
fn ripple_intensity(point: Point, ripple: RippleSample, speed: f32) -> f32 {
    let dx = point.x - ripple.origin.x;
    let dy = point.y - ripple.origin.y;
    let distance = dx.hypot(dy);
    let radius = speed * ripple.age;
    let offset = distance - radius;
    let envelope = (-(offset * offset) / (2.0 * RING_WIDTH * RING_WIDTH)).exp();
    let attenuation = (-SPATIAL_DECAY * radius).exp();

    envelope * attenuation
}

fn led_name_for_key(key: KeyCode) -> Option<&'static str> {
    Some(match key.code() {
        1 => "Key: Escape",
        2 => "Key: 1",
        3 => "Key: 2",
        4 => "Key: 3",
        5 => "Key: 4",
        6 => "Key: 5",
        7 => "Key: 6",
        8 => "Key: 7",
        9 => "Key: 8",
        10 => "Key: 9",
        11 => "Key: 0",
        12 => "Key: -",
        13 => "Key: =",
        14 => "Key: Backspace",
        15 => "Key: Tab",
        16 => "Key: Q",
        17 => "Key: W",
        18 => "Key: E",
        19 => "Key: R",
        20 => "Key: T",
        21 => "Key: Y",
        22 => "Key: U",
        23 => "Key: I",
        24 => "Key: O",
        25 => "Key: P",
        26 => "Key: [",
        27 => "Key: ]",
        28 => "Key: Enter",
        29 => "Key: Left Control",
        30 => "Key: A",
        31 => "Key: S",
        32 => "Key: D",
        33 => "Key: F",
        34 => "Key: G",
        35 => "Key: H",
        36 => "Key: J",
        37 => "Key: K",
        38 => "Key: L",
        39 => "Key: ;",
        40 => "Key: '",
        41 => "Key: `",
        42 => "Key: Left Shift",
        43 => "Key: \\ (ANSI)",
        44 => "Key: Z",
        45 => "Key: X",
        46 => "Key: C",
        47 => "Key: V",
        48 => "Key: B",
        49 => "Key: N",
        50 => "Key: M",
        51 => "Key: ,",
        52 => "Key: .",
        53 => "Key: /",
        54 => "Key: Right Shift",
        55 => "Key: Number Pad *",
        56 => "Key: Left Alt",
        57 => "Key: Space",
        58 => "Key: Caps Lock",
        59..=68 => F_KEYS[(key.code() - 59) as usize],
        69 => "Key: Num Lock",
        70 => "Key: Scroll Lock",
        71 => "Key: Number Pad 7",
        72 => "Key: Number Pad 8",
        73 => "Key: Number Pad 9",
        74 => "Key: Number Pad -",
        75 => "Key: Number Pad 4",
        76 => "Key: Number Pad 5",
        77 => "Key: Number Pad 6",
        78 => "Key: Number Pad +",
        79 => "Key: Number Pad 1",
        80 => "Key: Number Pad 2",
        81 => "Key: Number Pad 3",
        82 => "Key: Number Pad 0",
        83 => "Key: Number Pad .",
        86 => "Key: \\ (ISO)",
        87 => "Key: F11",
        88 => "Key: F12",
        96 => "Key: Number Pad Enter",
        97 => "Key: Right Control",
        98 => "Key: Number Pad /",
        99 => "Key: Print Screen",
        100 => "Key: Right Alt",
        102 => "Key: Home",
        103 => "Key: Up Arrow",
        104 => "Key: Page Up",
        105 => "Key: Left Arrow",
        106 => "Key: Right Arrow",
        107 => "Key: End",
        108 => "Key: Down Arrow",
        109 => "Key: Page Down",
        110 => "Key: Insert",
        111 => "Key: Delete",
        113 => "Key: Media Mute",
        119 => "Key: Pause/Break",
        125 => "Key: Left Windows",
        126 => "Key: Right Windows",
        127 | 139 => "Key: Menu",
        163 => "Key: Media Next",
        164 => "Key: Media Play/Pause",
        165 => "Key: Media Previous",
        166 => "Key: Media Stop",
        _ => return None,
    })
}

const F_KEYS: [&str; 10] = [
    "Key: F1", "Key: F2", "Key: F3", "Key: F4", "Key: F5", "Key: F6", "Key: F7", "Key: F8",
    "Key: F9", "Key: F10",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_assigns_every_keyboard_led() {
        let coordinates = keyboard_coordinates(117);
        assert_eq!(
            coordinates.iter().filter(|point| point.is_some()).count(),
            117
        );
    }

    #[test]
    fn pressed_key_starts_at_full_intensity() {
        let origin = Point { x: 4.0, y: 3.0 };
        let ripple = RippleSample { origin, age: 0.0 };
        assert!((ripple_intensity(origin, ripple, DEFAULT_SPEED) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wavefront_moves_according_to_speed() {
        let ripple = RippleSample {
            origin: Point { x: 0.0, y: 0.0 },
            age: 0.5,
        };
        let point = Point { x: 6.0, y: 0.0 };
        let intensity = ripple_intensity(point, ripple, 12.0);
        let expected_attenuation = (-SPATIAL_DECAY * 6.0).exp();

        assert!((intensity - expected_attenuation).abs() < 0.0001);
    }

    #[test]
    fn overlapping_ripples_add_and_clamp() {
        let point = Point { x: 1.0, y: 1.0 };
        let coordinates = [Some(point)];
        let ripples = [
            RippleSample {
                origin: point,
                age: 0.0,
            },
            RippleSample {
                origin: point,
                age: 0.0,
            },
        ];

        assert_eq!(render_frame(&coordinates, &ripples, DEFAULT_SPEED), [CYAN]);
    }

    #[test]
    fn maps_keypresses_to_openrgb_names() {
        assert_eq!(led_name_for_key(KeyCode::KEY_A), Some("Key: A"));
        assert_eq!(
            led_name_for_key(KeyCode::KEY_KP3),
            Some("Key: Number Pad 3")
        );
        assert_eq!(led_name_for_key(KeyCode::KEY_F12), Some("Key: F12"));
    }
}
