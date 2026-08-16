//! Command-line entry point for `openrgb-presets`.
//!
//! The program can discover the supported Logitech keyboard and mouse or apply
//! a cyan preset to them through the local OpenRGB SDK server.

mod ripple;

// `env` provides access to command-line arguments. `ExitCode` lets `main`
// explicitly tell the shell whether the command succeeded or failed.
use std::{
    env,
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    process::{Child, Command, ExitCode, Stdio},
    time::{Duration, Instant},
};

// `Color` stores red, green, and blue intensity values.
// `DeviceType` describes categories such as keyboards and mice.
// `OpenRgbClient` communicates with the OpenRGB SDK server.
// `OpenRgbResult` is the result type returned by OpenRGB operations.
use openrgb2::{Color, DeviceType, OpenRgbClient, OpenRgbResult};

// Full-intensity RGB cyan (#00FFFF).
const CYAN: Color = Color {
    r: 0,
    g: 255,
    b: 255,
};

const OPENRGB_START_TIMEOUT: Duration = Duration::from_secs(10);
const OPENRGB_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const OPENRGB_ADDRESS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6742);

enum Action {
    Devices,
    CyanStatic,
    Ripple(f32),
}

struct ManagedServer(Child);

impl Drop for ManagedServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// OpenRGB communication is asynchronous. This attribute creates a Tokio
// runtime so `main` can be async and use `.await` for network operations.
#[tokio::main]
async fn main() -> ExitCode {
    // Argument zero is the program path, so `skip(1)` collects only the
    // user-supplied arguments.
    let args: Vec<String> = env::args().skip(1).collect();

    let action = match parse_action(&args) {
        Ok(action) => action,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("{error}");
            }
            print_usage();
            return ExitCode::from(2);
        }
    };

    let _managed_server = match ensure_openrgb_server().await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("could not start OpenRGB server: {error}");
            return ExitCode::FAILURE;
        }
    };

    match action {
        Action::Devices => {
            // Keep OpenRGB errors inside `find_devices` as `OpenRgbResult`, then
            // map each possible outcome to a process exit status.
            match find_devices().await {
                // The tuple is ordered as `(keyboard_found, mouse_found)`.
                Ok((true, true)) => ExitCode::SUCCESS,

                // Discovery succeeded, but at least one required device was
                // not present. Report each missing device separately.
                Ok((keyboard_found, mouse_found)) => {
                    if !keyboard_found {
                        eprintln!("keyboard not found");
                    }

                    if !mouse_found {
                        eprintln!("mouse not found");
                    }

                    ExitCode::from(2)
                }

                // Network and protocol failures arrive here through the `?`
                // operators used by `find_devices`.
                Err(error) => {
                    eprintln!("OpenRGB error: {error}");
                    ExitCode::FAILURE
                }
            }
        }

        Action::CyanStatic => match apply_cyan_static().await {
            Ok((true, true)) => {
                println!("applied cyan-static");
                ExitCode::SUCCESS
            }
            Ok((keyboard_found, mouse_found)) => {
                if !keyboard_found {
                    eprintln!("keyboard not found");
                }

                if !mouse_found {
                    eprintln!("mouse not found");
                }

                ExitCode::from(2)
            }
            Err(error) => {
                eprintln!("OpenRGB error: {error}");
                ExitCode::FAILURE
            }
        },
        Action::Ripple(speed) => run_ripple(speed).await,
    }
}

fn parse_action(args: &[String]) -> Result<Action, String> {
    match args {
        [] => Ok(Action::Ripple(ripple::DEFAULT_SPEED)),
        [preset] if preset == "ripple" => Ok(Action::Ripple(ripple::DEFAULT_SPEED)),
        [preset, speed] if preset == "ripple" => parse_ripple_speed(speed).map(Action::Ripple),
        [preset] if preset == "cyan-static" => Ok(Action::CyanStatic),
        [command] if command == "devices" => Ok(Action::Devices),
        [command, preset] if command == "apply" && preset == "cyan-static" => {
            Ok(Action::CyanStatic)
        }
        [command, preset] if command == "apply" && preset == "ripple" => {
            Ok(Action::Ripple(ripple::DEFAULT_SPEED))
        }
        [command, preset, speed] if command == "apply" && preset == "ripple" => {
            parse_ripple_speed(speed).map(Action::Ripple)
        }
        _ => Err(String::new()),
    }
}

async fn ensure_openrgb_server() -> Result<Option<ManagedServer>, Box<dyn Error>> {
    if openrgb_server_is_reachable() {
        return Ok(None);
    }

    let service_started = Command::new("systemctl")
        .args(["--no-ask-password", "start", "openrgb.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());

    if service_started && wait_for_openrgb(OPENRGB_START_TIMEOUT).await {
        println!("started OpenRGB system service");
        return Ok(None);
    }

    let child = Command::new("openrgb")
        .args([
            "--server",
            "--server-host",
            "127.0.0.1",
            "--server-port",
            "6742",
            "--noautoconnect",
        ])
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run openrgb: {error}")))?;
    let mut server = ManagedServer(child);

    if wait_for_openrgb(OPENRGB_START_TIMEOUT).await {
        println!("started user OpenRGB server");
        return Ok(Some(server));
    }

    if let Some(status) = server.0.try_wait()? {
        return Err(io::Error::other(format!(
            "openrgb exited before its SDK server was ready: {status}"
        ))
        .into());
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "OpenRGB SDK server did not become ready on 127.0.0.1:6742",
    )
    .into())
}

async fn wait_for_openrgb(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if openrgb_has_target_keyboard().await {
            return true;
        }

        if Instant::now() >= deadline {
            return openrgb_server_is_reachable();
        }
        tokio::time::sleep(OPENRGB_RETRY_INTERVAL).await;
    }
}

async fn openrgb_has_target_keyboard() -> bool {
    let Ok(mut client) = OpenRgbClient::connect().await else {
        return false;
    };
    if client.set_name("openrgb-presets startup").await.is_err() {
        return false;
    }
    let Ok(controllers) = client.get_all_controllers().await else {
        return false;
    };

    controllers.iter().any(|controller| {
        is_target_keyboard(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        )
    })
}

fn openrgb_server_is_reachable() -> bool {
    TcpStream::connect_timeout(&OPENRGB_ADDRESS.into(), OPENRGB_RETRY_INTERVAL).is_ok()
}

async fn run_ripple(speed: f32) -> ExitCode {
    match ripple::apply(speed).await {
        Ok(true) => {
            println!("stopped ripple");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("keyboard not found");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("ripple error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_ripple_speed(value: &str) -> Result<f32, String> {
    let speed = value.parse::<f32>().map_err(|_| {
        format!(
            "ripple speed must be a number from {} to {}",
            ripple::MIN_SPEED,
            ripple::MAX_SPEED
        )
    })?;

    if !speed.is_finite() || !(ripple::MIN_SPEED..=ripple::MAX_SPEED).contains(&speed) {
        return Err(format!(
            "ripple speed must be from {} to {} cells per second",
            ripple::MIN_SPEED,
            ripple::MAX_SPEED
        ));
    }

    Ok(speed)
}

fn print_usage() {
    eprintln!("usage: openrgb-presets [ripple [speed] | cyan-static | devices]");
    eprintln!("       openrgb-presets");
    eprintln!("       openrgb-presets devices");
    eprintln!("       openrgb-presets apply cyan-static");
    eprintln!("       openrgb-presets apply ripple [speed]");
}

/// Connects to OpenRGB and searches for all required controllers.
///
/// The returned tuple is `(keyboard_found, mouse_found)`. This function only
/// reads controller metadata; it does not initialize controllers, select modes,
/// or write LED colors.
///
/// # Errors
///
/// Returns an OpenRGB error if connecting to the SDK server, naming the client,
/// or retrieving the controller list fails.
async fn find_devices() -> OpenRgbResult<(bool, bool)> {
    // `connect()` uses the SDK's default address, 127.0.0.1:6742. The `?`
    // operator immediately returns an error if the connection cannot be made.
    let mut client = OpenRgbClient::connect().await?;

    // Give this connection a recognizable name in OpenRGB's SDK server view.
    client.set_name("openrgb-presets").await?;

    // Print the negotiated SDK protocol version to make compatibility issues
    // visible during development and troubleshooting.
    println!("protocol: {}", client.get_protocol_version());

    // Fetch metadata for every controller detected by the OpenRGB server.
    let controllers = client.get_all_controllers().await?;

    // Start with neither target found and update these flags while scanning.
    let mut keyboard_found = false;
    let mut mouse_found = false;

    // `.iter()` borrows each controller instead of consuming the collection.
    for controller in controllers.iter() {
        // Match stable descriptive fields rather than the controller's numeric
        // ID, which can change when OpenRGB enumerates devices in a new order.
        if is_target_keyboard(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        ) {
            keyboard_found = true;
            println!("keyboard: {}", controller.name());
        }

        // The mouse is checked independently so this loop can discover both
        // targets regardless of their order in the controller list.
        if is_target_mouse(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        ) {
            mouse_found = true;
            println!("mouse: {}", controller.name());
        }
    }

    // Wrap the two discovery flags in `Ok` to indicate that all SDK operations
    // completed successfully, even if one of the devices was absent.
    Ok((keyboard_found, mouse_found))
}

/// Applies a static cyan color to the supported keyboard and mouse.
///
/// Both devices are located before either one is changed. The returned tuple is
/// `(keyboard_found, mouse_found)` so the caller can report missing hardware.
///
/// # Errors
///
/// Returns an OpenRGB error if connecting to the server, naming the client,
/// retrieving controllers, selecting controllable mode, or writing colors
/// fails.
async fn apply_cyan_static() -> OpenRgbResult<(bool, bool)> {
    let mut client = OpenRgbClient::connect().await?;
    client.set_name("openrgb-presets").await?;

    let controllers = client.get_all_controllers().await?;

    let keyboard = controllers.iter().find(|controller| {
        is_target_keyboard(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        )
    });

    let mouse = controllers.iter().find(|controller| {
        is_target_mouse(
            controller.device_type(),
            controller.vendor(),
            controller.name(),
        )
    });

    let keyboard_found = keyboard.is_some();
    let mouse_found = mouse.is_some();

    // Do not change either device unless the complete target group is present.
    let (Some(keyboard), Some(mouse)) = (keyboard, mouse) else {
        return Ok((keyboard_found, mouse_found));
    };

    // Direct mode permits LED writes without the temporary rainbow produced by
    // `Controller::init()`.
    keyboard.set_controllable_mode().await?;
    mouse.set_controllable_mode().await?;

    keyboard.set_all_leds(CYAN).await?;
    mouse.set_all_leds(CYAN).await?;

    Ok((true, true))
}

/// Returns `true` when controller metadata identifies the supported keyboard.
///
/// Text comparisons are ASCII case-insensitive because capitalization is not a
/// meaningful part of a device identity. The keyboard serial is deliberately
/// excluded so another unit of the same model can use these presets.
fn is_target_keyboard(device_type: DeviceType, vendor: &str, name: &str) -> bool {
    device_type == DeviceType::Keyboard
        && vendor.eq_ignore_ascii_case("Logitech")
        && name.eq_ignore_ascii_case("Logitech G512 RGB")
}

/// Returns `true` when controller metadata identifies the supported mouse.
///
/// The OpenRGB-reported location and `/dev/hidraw` path are deliberately
/// excluded because Linux device paths may change between boots.
fn is_target_mouse(device_type: DeviceType, vendor: &str, name: &str) -> bool {
    device_type == DeviceType::Mouse
        && vendor.eq_ignore_ascii_case("Logitech")
        && name.eq_ignore_ascii_case("G502 HERO Gaming Mouse")
}

// Test code is compiled only by `cargo test`, not into the normal executable.
#[cfg(test)]
mod tests {
    // Import private functions and imported types from the parent module so the
    // tests exercise the same matching code used by the application.
    use super::*;

    #[test]
    fn matches_target_keyboard_case_insensitively() {
        // Lowercase metadata must still identify the supported keyboard.
        assert!(is_target_keyboard(
            DeviceType::Keyboard,
            "logitech",
            "logitech g512 rgb",
        ));
    }

    #[test]
    fn rejects_unrelated_keyboard() {
        // Matching the vendor and device category is insufficient when the
        // model name identifies a different keyboard.
        assert!(!is_target_keyboard(
            DeviceType::Keyboard,
            "Logitech",
            "Different Keyboard",
        ));
    }

    #[test]
    fn rejects_target_name_with_wrong_device_type() {
        // A matching vendor and model name must not override a contradictory
        // controller category.
        assert!(!is_target_mouse(
            DeviceType::Keyboard,
            "Logitech",
            "G502 HERO Gaming Mouse",
        ));
    }

    #[test]
    fn parses_valid_ripple_speed() {
        assert_eq!(parse_ripple_speed("18.5"), Ok(18.5));
    }

    #[test]
    fn defaults_to_ripple() {
        assert!(matches!(
            parse_action(&[]),
            Ok(Action::Ripple(speed)) if speed == ripple::DEFAULT_SPEED
        ));
    }

    #[test]
    fn accepts_short_preset_arguments() {
        assert!(matches!(
            parse_action(&["cyan-static".to_owned()]),
            Ok(Action::CyanStatic)
        ));
        assert!(matches!(
            parse_action(&["ripple".to_owned(), "18".to_owned()]),
            Ok(Action::Ripple(18.0))
        ));
    }

    #[test]
    fn rejects_invalid_ripple_speeds() {
        assert!(parse_ripple_speed("fast").is_err());
        assert!(parse_ripple_speed("0").is_err());
        assert!(parse_ripple_speed("51").is_err());
        assert!(parse_ripple_speed("NaN").is_err());
    }
}
