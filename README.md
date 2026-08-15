# openrgb-presets

Linux-focused Rust CLI for discovering and controlling a Logitech G513 keyboard
and G502 mouse through a local OpenRGB SDK server. It currently provides a
static cyan device-group preset and a keypress-driven spatial ripple animation.

## Features

- Identifies supported devices by type, vendor, and model instead of unstable
  OpenRGB controller indexes.
- Applies cyan to the complete keyboard-and-mouse group only after both devices
  have been discovered.
- Renders concurrent keypress ripples at 30 FPS using the keyboard's physical
  OpenRGB matrix.
- Supports configurable ripple propagation speed.
- Observes keyboard input without grabbing, printing, logging, or persisting
  key events.
- Restores static cyan when the ripple process receives `Ctrl+C`.

## Requirements

- Linux with access to the G513 evdev device through the active-seat ACL or an
  equivalent permission rule.
- OpenRGB 1.0rc3 or another server supporting SDK protocol version 5.
- Rust 2024 edition toolchain.
- Logitech G513 (`046d:c33c`, reported by OpenRGB as `Logitech G512 RGB`).
- Logitech G502 HERO mouse as reported by OpenRGB.

The current device matching and keyboard matrix are intentionally specific to
this hardware pair.

## Build

```sh
cargo build --release
cargo test
```

The release binary is written to `target/release/openrgb-presets`.

## Usage

Start OpenRGB in a separate terminal:

```sh
openrgb --server --server-host 127.0.0.1 --server-port 6742 --noautoconnect
```

Then run one of the available commands:

```sh
cargo run -- devices
cargo run -- apply cyan-static
cargo run -- apply ripple
cargo run -- apply ripple 18
```

Ripple speed is measured in OpenRGB keyboard-matrix cells per second. The
default is `24`; accepted values range from `1` through `50`. The ripple command
runs until `Ctrl+C`, then restores static cyan.

Only one lighting client should write to OpenRGB at a time. Concurrent clients
can interleave the G513's multi-packet updates and produce mixed colors.

## Ripple Algorithm

Each initial keypress creates an expanding radial wave. At destination key
`k`, ripple `i` contributes:

```text
exp(-((distance(i, k) - speed * age(i))^2) / (2 * width^2))
    * exp(-spatial_decay * speed * age(i))
```

This is an analytic expanding Gaussian ring. It follows the constant-speed
wavefront and superposition ideas from the linear wave equation without the
state and stability requirements of a finite-difference PDE solver. Active
ripples are summed and clamped, so overlapping fronts combine constructively.
Real elapsed time controls radius, keeping animation speed stable if a frame is
late.

References:

- [Wave equation](https://mathworld.wolfram.com/WaveEquation.html)
- [Superposition principle](https://mathworld.wolfram.com/SuperpositionPrinciple.html)
- [Gaussian function](https://mathworld.wolfram.com/GaussianFunction.html)
- [Distance fields and smooth radial contours](https://thebookofshaders.com/07/)

The renderer uses OpenRGB's 7x23 G810-family matrix for the G512/G513 rather
than treating controller LED indexes as physical positions.

## Input Privacy

Ripple observes the G513's main evdev endpoint without grabbing it, so normal
keyboard input continues to reach the desktop. Only initial press events are
converted into short-lived in-memory ripple origins. Key events are not printed,
logged, or persisted.

## Project Status

This is an early hardware-specific implementation. Presets are currently
compiled into the binary; TOML configuration, service integration, additional
device groups, and spatial mouse placement are planned but not implemented.
