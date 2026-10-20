# Transom

Transom shows a Mac's desktop or one of its windows on a Linux, Windows or macOS computer, and
sends your mouse, scroll wheel and keyboard back to the Mac. The Mac runs
[Sill](https://github.com/Saffsanity/sill), unchanged; Transom is a second client for Sill's
existing protocol, next to Sill's own iPhone and iPad app.

Transom is an independent project. It is not part of Sill and is not endorsed by its author.

## Status

Version 0.1. What works:

- Finding Macs that run Sill on your network (Bonjour, `_sill._tcp`).
- Pairing with a Mac by its 12-digit code or by its `sill://pair` link.
- Streaming the Mac's desktop or a single window, with the Mac's own cursor shape.
- Mouse (left and right buttons), scroll wheel and trackpad scrolling, and the keyboard.
- Connecting through Sill's Remote Access door by address (for example over Tailscale).

Not yet: the clipboard, the Mac's menus, trackpad gestures, Sill's settings, audio, pairing over
the USB cable, and Direct Wireless.

## Download

**Windows:** download `transom-…-windows-x64.zip` from the
[latest release](https://github.com/ry-cen/Transom/releases/latest), unzip it anywhere and
double-click `transom-gui.exe`. FFmpeg is included; keep its DLLs next to the program. The
program is not code-signed, so Windows SmartScreen may warn about an unknown publisher: choose
**More info**, then **Run anyway**.

Linux and macOS: build from source (below) for now.

## Build from source

Transom needs FFmpeg's shared libraries (libavcodec, libavformat, libavutil, libswscale) at
build time and at run time.

- **macOS:** `brew install ffmpeg pkg-config`
- **Debian and Ubuntu:** `sudo apt install pkg-config clang libclang-dev libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev`
- **Windows:** download a shared FFmpeg build (for example
  `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip` from
  [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases)), unpack it, set
  `FFMPEG_DIR` to the unpacked folder, set `LIBCLANG_PATH` to LLVM's `bin` folder, and put the
  FFmpeg `bin` folder on `PATH` so `transom.exe` finds the DLLs.

Then build with a current stable Rust:

```sh
cargo build --release
```

The program is `target/release/transom` (`transom.exe` on Windows).

## The app

- `transom-gui`, or `transom` with no arguments, opens the window. Pair a Mac from the list when it appears, or use **Add a Mac** with its address and the pairing code.
- While streaming, hold Ctrl+Alt+Shift (Control+Option+Shift on macOS) and press a key: B shows or hides the bar, D selects the desktop, W the next window, S opens the Mac's settings, F toggles full screen and Q disconnects.
- To leave full screen with no Mac connected, press Esc or F11, or click **Leave full screen** at the top of the Macs list.
- Preferences are saved as `prefs.json` in the config folder (see `transom identity` for its path).
- On Windows, `transom-gui.exe` opens no console window.

## Use

### Pair

Transom has to be paired with a Mac once. On the Mac, open Sill's menu and choose
**Pair iPhone or iPad…**. It shows a 12-digit code. Then run:

```sh
transom pair
```

Transom finds the Mac by Bonjour, asks for the code, and saves the Mac when the code is right.
Useful flags: `--mac NAME` to pick a Mac by name, `--code "4829 1355 7208"` to give the code
on the command line, `--url "sill://pair?..."` to pair with a pairing link, and
`--name LABEL` to change how this computer appears in Sill's device list (the default is
"Transom (Linux)", "Transom (Windows)" or "Transom (macOS)").

### Connect

```sh
transom connect
```

With one saved Mac this needs no arguments; with several, add `--mac NAME`. The window shows
the Mac's desktop or one window, the Mac's cursor shape, and the round-trip time in the title.
The first picture can take up to four seconds, because the Mac sends a full frame about every
four seconds. If the Mac quits or the network drops, Transom reconnects on its own.

- `--source auto|desktop|keep|window:ID` chooses what to show. `auto` (the default) keeps the
  Mac's current choice and shows the desktop if it has none. `keep` never changes it. Sill
  streams one source to all devices, so choosing a window here also changes what an iPad
  watching the same Mac sees.
- `--fps N` asks for a frame rate (default 60).
- Shortcuts in the window: **Ctrl+Alt+Shift+D** shows the desktop, **Ctrl+Alt+Shift+W** shows the
  next window, **Ctrl+Alt+Shift+F** toggles full screen, **Ctrl+Alt+Shift+Q** quits.
- `--keymap pc|mac` sets the modifier layout. The default is `mac` on macOS and `pc` elsewhere.

| Key you press | `--keymap pc` (Linux, Windows) | `--keymap mac` (macOS) |
|---|---|---|
| Ctrl | Command | Control |
| Win / Super | Control | Command |
| Alt | Option | Option |
| Shift | Shift | Shift |

### Other commands

- `transom discover [--timeout SECS]` lists Macs running Sill on this network, with their door
  ("pairing required", "open" or "plain") and the Mac ID if the Mac is saved.
- `transom list` shows the saved Macs.
- `transom windows` prints the windows the Mac is showing (`*` marks the active source).
- `transom forget MAC` removes a saved Mac (by Mac ID, name or Mac ID prefix).
- `transom identity` prints this computer's key fingerprint and the config folder.

Global flags: `--config-dir DIR` uses another folder for the key and saved Macs, and `-v` or
`-vv` turns on logging.

### Remote Access

If the Mac has Sill's Remote Access on, reach it by address (port 7455 unless you give
`--port`):

```sh
transom connect --host ADDRESS --remote
```

Pair first, at home on the same network as the Mac (`transom pair`): the remote door only
takes devices that are already paired. With more than one saved Mac, add `--mac NAME` (a
part of the name, or the start of its Mac ID, is enough).

Remote sessions have not been tested against a real Mac yet.

### Viewing the Mac you are sitting at

When the Mac is the computer Transom runs on, Transom would send its own input back to itself
and loop. So it never forwards the keyboard to a Mac at this computer's own address, and it
forwards the pointer only with `--allow-local-pointer`. `--allow-local-keyboard` turns the
keyboard on anyway; do not type into the window then. The title shows "(view only)" while input
is off.

### Troubleshooting

- **"This computer is not paired with that Mac."** The Mac does not know this computer's key
  (for example after you removed it in Sill's Devices list). Run `transom pair` again.
- **"the Mac presented a different key"** (pin mismatch). The Mac's key differs from the one
  saved. Only if you know why (for example Sill was reinstalled), run `transom forget MAC`
  and pair again.
- **Windows: the program does not start, or complains about a missing DLL.** Put FFmpeg's
  `bin` folder on `PATH` (see Build).
- **Linux/Wayland: the Mac's cursor looks too small or large.** Cursor sizes follow the window's
  scale factor, and some compositors ignore it. Try `WINIT_UNIX_BACKEND=x11`.

## Privacy and security

Transom keeps its key and the Macs it paired with in your user configuration folder
(`~/.config/transom` on Linux, `~/Library/Application Support/transom` on macOS,
`%APPDATA%\transom\config` on Windows). On Linux and macOS, a folder Transom creates is readable by you alone; for an existing `--config-dir` it only warns if others can read it.
Connections to a paired Mac are TLS 1.3 with both keys pinned, as Sill's own app connects. A Mac
connected with `--any-key` or through an open door is encrypted but its key is not pinned, and
`--plain` is not encrypted at all.

## Credits and license

Transom is licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE).

Sill is Copyright 2026 Noah Saffer and licensed under the Apache License 2.0. Transom's protocol
implementation, its test vectors and its descriptions of Sill's behaviour come from Sill's
sources.
