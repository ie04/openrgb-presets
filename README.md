# openrgb-presets

`openrgb-presets` is a Linux-focused Rust CLI for controlling a Logitech G513
keyboard and G502 HERO mouse through a local
[OpenRGB](https://openrgb.org/) SDK server. It provides a static cyan preset for
both devices and a keypress-driven cyan ripple animation for the keyboard.

The project identifies hardware from OpenRGB metadata instead of relying on
controller indexes, which may change when devices are re-enumerated.

## Features

- Discovers the supported keyboard and mouse by device type, vendor, and model.
- Applies a static `#00FFFF` cyan preset only when the complete device group is
  available.
- Renders concurrent keypress ripples at approximately 30 FPS.
- Uses the keyboard's physical 7x23 OpenRGB matrix rather than its linear LED
  order.
- Supports ripple propagation speeds from 1 through 50 matrix cells per second.
- Observes keyboard input without grabbing the device, so normal input continues
  to reach the desktop.
- Keeps key events in memory only for the lifetime of each ripple; events are
  never printed, logged, or persisted.
- Restores static cyan after `Ctrl+C` or an animation-loop failure.

## Supported Hardware

The current device matching, input-device lookup, LED names, and keyboard matrix
are intentionally specific to this hardware:

| Device | OpenRGB identity | Additional identity |
| --- | --- | --- |
| Logitech G513 Carbon | `Logitech` / `Logitech G512 RGB` | USB ID `046d:c33c`; `/dev/input/by-id` name beginning with `usb-Logitech_G513_Carbon_` |
| Logitech G502 HERO | `Logitech` / `G502 HERO Gaming Mouse` | OpenRGB device type `Mouse` |

Other models are not currently supported, even if their physical layout is
similar.

## Requirements

- Linux
- OpenRGB 1.0rc3 or another release supporting SDK protocol version 5
- Rust with support for the 2024 edition
- Read access to the G513's evdev keyboard endpoint
- The supported G513 and G502 HERO devices for the complete static preset

On a typical desktop session, logind grants the active user access to the
keyboard's evdev endpoint through an ACL. Check the resolved device and its
permissions if the ripple command reports `Permission denied`:

```sh
readlink -f /dev/input/by-id/usb-Logitech_G513_Carbon_*-event-kbd
getfacl /dev/input/eventX
```

Replace `eventX` with the event device returned by `readlink`. Prefer an
active-seat ACL or a narrowly scoped udev rule over running the application as
root.

## Quick Start

Start an OpenRGB SDK server without automatically connecting to another server:

```sh
openrgb --server --server-host 127.0.0.1 --server-port 6742 --noautoconnect
```

In another terminal, clone, build, and run the device check:

```sh
git clone https://github.com/ie04/openrgb-presets.git
cd openrgb-presets
cargo build --release
./target/release/openrgb-presets devices
```

Apply static cyan to both supported devices:

```sh
./target/release/openrgb-presets apply cyan-static
```

Or start the keyboard ripple at its default speed:

```sh
./target/release/openrgb-presets apply ripple
```

Press `Ctrl+C` to stop the ripple and restore static cyan.

## Installation

### Build in Place

```sh
cargo build --release
cargo test
```

The optimized executable is written to
`target/release/openrgb-presets`.

### Install with Cargo

From the repository root:

```sh
cargo install --path .
```

This installs `openrgb-presets` into Cargo's binary directory, usually
`~/.cargo/bin`. Ensure that directory is present in `PATH`.

## Command Reference

```text
openrgb-presets devices
openrgb-presets apply cyan-static
openrgb-presets apply ripple [speed]
```

### `devices`

Connects to `127.0.0.1:6742`, prints the negotiated SDK protocol version, and
reports each supported controller found. It reads metadata only and does not
change lighting modes or colors.

```sh
openrgb-presets devices
```

### `apply cyan-static`

Locates both supported devices, switches them to an OpenRGB-controllable mode,
and sets every LED to cyan. Neither device is modified if one member of the
device group is missing.

```sh
openrgb-presets apply cyan-static
```

### `apply ripple [speed]`

Clears the keyboard, observes initial keypress events, and renders expanding
cyan rings from the physical location of each pressed key. `speed` is optional
and is measured in keyboard-matrix cells per second.

```sh
openrgb-presets apply ripple       # default: 24 cells/second
openrgb-presets apply ripple 18.5  # slower propagation
openrgb-presets apply ripple 40    # faster propagation
```

Accepted speeds range from `1` through `50`, including decimal values. The
ripple command controls only the keyboard while running. On shutdown it changes
the keyboard to static cyan; it does not modify the mouse.

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Command completed successfully |
| `1` | OpenRGB, evdev, signal, or animation error |
| `2` | Invalid command or speed, or required hardware was not found |

Diagnostics are written to standard error. Successful discovery and status
messages are written to standard output.

## How the Ripple Works

Each initial keypress creates an expanding radial wave. At destination key `k`,
ripple `i` contributes:

```text
exp(-((distance(i, k) - speed * age(i))^2) / (2 * width^2))
    * exp(-spatial_decay * speed * age(i))
```

This is an analytic expanding Gaussian ring. Its radius advances according to
real elapsed time, so propagation speed remains stable when a frame is late.
The renderer samples the ring at every key in OpenRGB's G810-family 7x23 matrix,
sums overlapping ripple intensities, clamps the result, and writes cyan channel
values to the keyboard.

The approach captures constant-speed wavefront propagation and superposition
without the state or numerical stability requirements of a finite-difference
wave-equation solver.

References:

- [Wave equation](https://mathworld.wolfram.com/WaveEquation.html)
- [Superposition principle](https://mathworld.wolfram.com/SuperpositionPrinciple.html)
- [Gaussian function](https://mathworld.wolfram.com/GaussianFunction.html)
- [Distance fields and smooth radial contours](https://thebookofshaders.com/07/)

## Input Privacy

The ripple process opens the G513's primary evdev keyboard endpoint for reading
but does not grab it. Normal keyboard input therefore continues to reach the
desktop and other applications. Only initial press events are mapped to
short-lived in-memory ripple origins. The program does not print, log, transmit,
or persist key events.

Because evdev access exposes raw keyboard input to a process, inspect the source
and use normal user permissions rather than elevated privileges.

## Architecture

- `src/main.rs` implements argument parsing, OpenRGB device discovery, static
  preset application, and hardware identity matching.
- `src/ripple.rs` implements evdev discovery, key-to-LED mapping, physical matrix
  coordinates, ripple simulation, rendering, and shutdown restoration.
- `openrgb2` handles the OpenRGB SDK protocol.
- `evdev` provides asynchronous Linux input events.
- `tokio` drives OpenRGB I/O, keyboard events, frame timing, and `Ctrl+C`
  concurrently.

## Troubleshooting

### OpenRGB connection fails

Confirm that the SDK server is running locally on its default endpoint,
`127.0.0.1:6742`, and that another OpenRGB server is not already using the port.
The server address is currently compiled into the OpenRGB client default and is
not configurable from the CLI.

### A device is not found

Run `openrgb-presets devices` and compare the reported hardware in OpenRGB with
the identities in [Supported Hardware](#supported-hardware). OpenRGB must detect
the device using the exact model identity expected by this application.

### The ripple cannot open the keyboard

Check that `/dev/input/by-id` contains a matching G513 `event-kbd` symlink and
that the current user can read its target. The code deliberately ignores the
G513 `if01` keyboard endpoint and uses the primary endpoint.

### Colors flicker or appear mixed

Only one lighting client should write to OpenRGB at a time. Concurrent clients
can interleave the G513's multi-packet updates and produce mixed colors. Stop
other effects, plugins, and SDK clients before running a preset.

## Limitations and Roadmap

This is an early, hardware-specific implementation. Current limitations include:

- Presets and colors are compiled into the binary.
- The OpenRGB host and port are not configurable.
- There is no background service or desktop-session integration.
- The ripple is keyboard-only and does not model the mouse's physical position.
- Hardware matching and evdev discovery support only the listed Logitech models.

Potential future work includes TOML configuration, additional device groups,
custom colors, configurable server endpoints, service integration, and spatial
effects spanning multiple devices.

## Development

Before submitting changes, run:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Unit tests cover device matching, speed validation, matrix completeness,
keypress-to-LED mapping, wavefront motion, and overlapping-ripple behavior.

## License

No license has been selected yet. Until one is added, the source remains under
the copyright holder's default rights.
