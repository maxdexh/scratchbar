# Scratchbar

A highly configurable status bar panel using the [Kitty terminal's panel feature](https://sw.kovidgoyal.net/kitty/kittens/panel/).

## What?

By utilizing the [performance and efficiency of Kitty](https://sw.kovidgoyal.net/kitty/performance/), Scratchbar provides an efficient, low-resource way to
display system information and interact with your environment. Unlike traditional status bars (like Waybar or Polybar) that render using GUI toolkits,
Scratchbar renders entirely as a Terminal User Interface (TUI).

Modern terminal protocols, in particular those [innovated by Kitty](https://sw.kovidgoyal.net/kitty/protocol-extensions/), result in a rich yet simple basis with support for:
- True Colors
- Rich Text
- Text Sizing
- Mouse Interaction
- Images/Icons

(TODO: Link to library docs)

To make this as hackable as possible, this project is structured in an unorthodox way, consisting of two components:

1. **The Host:** The main program, which manages Kitty terminal instances, including event piping, monitor detection and the panels' lifecycles.
2. **The Controller:**
   A user-defined program that is responsible for *what* the bar displays, including any logic to listen to system events in order to acquire the necessary information.
   The Controller runs as a subprocess of the Host and communicates through the `scratchbar` Rust library.

## Dependencies

Running the Host program requires:
- A recent Kitty version. Versions prior to 0.46 result in a degraded experience due to recently fixed bugs.
- A desktop environment which works with Kitty panels.
  See [the compatibility list](https://sw.kovidgoyal.net/kitty/kittens/panel/#compatibility-with-various-platforms).
  Note that the panels used by this project are run as dock panels snapped to the screens' sides, so restrictions with desktop and background panels can be ignored.
- Currently, the Host program polls `wlr-randr` to check for monitor changes.
  Therefore, you must have it installed (and available in your $PATH) and be on a desktop environment supported by it.
  This is going to change in the future.

## Quick Start: Running the Example Controller

To see Scratchbar in action, you can run the provided example controller, implemented in `example-controller/`.

It displays the following parts of the environment, if available:
- Hyprland Workspaces (PRs for other desktops welcome)
- Energy information using the `UPower` dbus interface (only on battery)
- Power profiles using the `UPower.PowerProfiles` dbus interface
- System tray icons `StatusNotifierWatcher` dbus interface for the system tray
- Audio Information using `libpulseaudio`. Changes are applied through the `pactl` command

### Running the bar

> Before running, make sure you installed a recent rust toolchain (1.93+), as well as the Host program's dependencies.

```bash
# 1. Clone the repository:
git clone https://github.com/maxdexh/scratchbar.git
cd scratchbar

# 2. Run the host with the example controller
example-controller/run --release
```

## Customization: Writing Your Own Controller

You can customize the bar by changing the implementation of the controller.
Create a standalone repository based on `example-controller/` as follows:
- Clone this repository (`git clone https://github.com/maxdexh/scratchbar.git`)
- Install the `scratchbar` program (`cargo install --path scratchbar/scratchbar-bin`)
- Copy the example controller somewhere else (`cp -r scratchbar/example-controller scratchbar-controller`)
- Adjust the `scratchbar` dependency in the controller repo (see `example-controller/Cargo.toml`)

You can now run your bar using `scratchbar cargo run` (for development) or `scratchbar scratchbar-controller` (after `cargo install`ing it).
