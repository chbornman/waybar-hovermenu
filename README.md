# waybar-hovermenu

A daemon that opens TUI/GUI menus when you hover over Waybar modules. Hover to peek, click to pin, leave to auto-close.

Built for Hyprland + Waybar.

## How it works

`waybar-hovermenu` runs as a daemon listening on a Unix socket. Waybar modules send hover/click/leave events via the `hovermenu-ctl` client. The daemon spawns the configured app (in a terminal for TUI apps, directly for GUI apps) and tracks cursor position to auto-close menus when the cursor leaves.

```
Waybar module  -->  hovermenu-ctl hover audio  -->  daemon  -->  ghostty -e wiremix
                    hovermenu-ctl leave         -->  daemon  -->  (auto-close after 500ms)
                    hovermenu-ctl click audio   -->  daemon  -->  (pin/unpin menu)
```

## Features

- **Hover to open** - menus appear when hovering over waybar modules
- **Click to pin** - pinned menus stay open (gold border indicator)
- **Auto-close** - cursor tracking closes menus when you move away
- **Slide-up animation** - menus animate out when closing
- **Right-click actions** - quick toggle actions (mute, wifi on/off, etc.)
- **Live status** - real-time waybar text via `follow` streaming (PulseAudio, dbus, inotify, polling)
- **TUI and GUI support** - terminal apps launched via configurable terminal command, GUI apps launched directly

## Building

```sh
cargo build --release
```

Produces two binaries:
- `waybar-hovermenu` - the daemon
- `hovermenu-ctl` - the client

Install them somewhere in your `$PATH` (e.g. `~/.local/bin/`).

## Configuration

Config lives at `~/.config/waybar-hovermenu/config.toml`:

```toml
[daemon]
terminal_cmd = "ghostty --title='{title}' --font-size=9 -e {command}"
waybar_height = 32
socket_path = "/tmp/waybar-hovermenu.sock"

[modules.audio]
kind = "tui"
command = "wiremix"
action = "pactl set-sink-mute @DEFAULT_SINK@ toggle"

[modules.bluetooth]
kind = "tui"
command = "bluetui"
action = "bluetoothctl power off || bluetoothctl power on"

[modules.network]
kind = "tui"
command = "impala"

[modules.cpu]
kind = "tui"
command = "/usr/bin/btop"
poll_interval = 3

[modules.battery]
kind = "tui"
command = "~/.local/bin/powertui"
poll_interval = 30

[modules.mail]
kind = "tui"
command = "mailtui"
action = "mbsync -a"
watch_dir = "~/.local/share/mail"

[modules.localsend]
kind = "gui"
command = "flatpak run org.localsend.localsend_app"
window_class = "localsend"
```

### Module options

| Field | Description |
|---|---|
| `kind` | `"tui"` (launched in terminal) or `"gui"` (launched directly) |
| `command` | Command to run. Tilde `~` is expanded. |
| `window_class` | Window class for GUI apps (used to detect the window) |
| `action` | Right-click quick action command |
| `poll_interval` | Status polling interval in seconds |
| `watch_dir` | Directory to watch with inotify for status updates |
| `enabled` | Set to `false` to disable a module |

### Daemon options

| Field | Default | Description |
|---|---|---|
| `terminal_cmd` | `foot -T {title} {command}` | Terminal launch template. `{title}` and `{command}` are substituted. |
| `waybar_height` | `32` | Height of waybar in pixels (for cursor tracking) |
| `socket_path` | `/tmp/waybar-hovermenu.sock` | IPC socket path |

## Waybar integration

Use `hovermenu-ctl` in your waybar config for hover/click events and streaming status:

```json
"custom/audio": {
    "exec": "hovermenu-ctl follow audio",
    "return-type": "json",
    "on-click": "hovermenu-ctl click audio",
    "on-click-right": "hovermenu-ctl action audio"
}
```

For hover/leave, use Waybar's `on-hover` and `on-hover-leave` if available, or set up `eventless` modules with cursor position tracking.

## IPC protocol

The daemon listens on a Unix socket and accepts newline-delimited commands:

| Command | Description |
|---|---|
| `hover <module>` | Open menu for module |
| `leave` | Close menu if not pinned (with debounce) |
| `click <module>` | Toggle pin state / open+pin |
| `action <module>` | Execute the module's quick action |
| `status <module>` | Get one-shot JSON status |
| `follow <module>` | Stream JSON status updates |

## Dependencies

- [Hyprland](https://hyprland.org/) - `hyprctl` for window management and cursor position
- [ydotool](https://github.com/ReimuNotMoe/ydotool) - mouse jiggle workaround for hover events
- A terminal emulator (default: [foot](https://codeberg.org/dnkl/foot), configurable)

## License

MIT
