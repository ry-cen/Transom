# Transom graphical app: specification

Status: accepted, revision 2. Nothing here is built yet. The owner decided the binary layout and
the app icon (section 12) on 2026-10-19.

This document specifies a graphical Transom, so that a user never needs a terminal. It is the
source for the implementation brief. It describes what the app does, how it is put together, and
how each phase is tested. It is written against the code as of repository commit `dcb3fd3` and
Sill commit `68e880e3f8ec83f5aefe2e617f9a4f246a2ce328`. Technical detail for the implementation
(frozen APIs, dependency pins, library traps, risks) is in
[`ui-implementation-notes.md`](ui-implementation-notes.md).

## 1. Goal, non-goals, audience

### 1.1 Goal

Ship a graphical Transom that does everything a user needs without a terminal: find and pair
Macs, connect at home or remotely, switch windows, and change the app's and the Mac's settings by
the end of Phase 2. It runs in one window, with the interface drawn over the video. The command
line stays and keeps working.

### 1.2 Who it is for

People on Linux or Windows who have a Mac running [Sill](https://github.com/Saffsanity/sill) and
want to see and control it from another computer. They should not need to know what a config
folder, a fingerprint or a flag is. macOS is a supported build target, mainly for testing.

### 1.3 Non-goals

- Changing the wire protocol, or Sill itself.
- Audio, pairing over USB, Direct Wireless (AWDL), and moving a live session between home and
  remote.
- Several streams or several windows at once.
- A light theme, or translation. Copy is English and lives in one file so translation can come
  later.
- Keyboard grabs on X11 or Wayland.
- Changing CLI behaviour. "CLI unchanged" means every subcommand, flag, output and exit code stays
  the same. The one intended change: in a build with the `gui` feature, `transom` with no
  subcommand opens the GUI instead of failing with a usage error (section 8.1).
- Installers beyond the notes in section 7.4.

### 1.4 Whole-design acceptance criteria

Each phase has its own testable list in section 10. These are the criteria for the design as a
whole, all phases.

1. `transom-gui` (on Windows `transom-gui.exe`, with no console window) opens a window listing
   saved Macs and Macs found by Bonjour. No terminal is needed from first launch to streaming.
2. A user can pair at home by typing the code the Mac shows, pair by address and code (the remote
   door), or pair by pasting a `sill://pair` link. Every refusal (`code`, `closed`, `expired`,
   `stopped`, `busy`, `locked`, `openOnMac`, unknown) shows the text in section 5.3, inline, with
   no dialog box.
3. While streaming, an auto-hiding top bar has Apps (from Phase 2), a strip of live window
   thumbnails with app-icon badges, Desktop, connection stats, settings (the Mac's settings from
   Phase 2), full screen and Disconnect. Choosing a thumbnail selects that window.
4. Keys go to the Mac only in "Mac has the keyboard" mode (section 6.1). Transom's own shortcuts
   never reach the Mac. Opening any Transom panel releases every key held on the Mac.
5. Connecting, reconnecting (with a countdown and "Try Now") and ended states are shown. Every
   ended state that needs the user (not paired, key changed, removed, pairing required, update)
   offers a button that fixes it. No text tells the user to run a command.
6. The CLI (`transom pair|connect|…`) behaves as before. `cargo test --workspace` and
   `cargo clippy --workspace --all-targets -- -D warnings` pass. `cargo build -p transom
   --no-default-features` still builds without FFmpeg.
7. Linux X11, Linux Wayland and Windows are first-class targets. macOS builds and runs for
   testing.

## 2. Decisions taken

| Decision | Reason | Alternatives considered |
|---|---|---|
| Toolkit: egui 0.37 (egui-winit and egui-wgpu), drawn in the window and wgpu surface Transom already owns | Overlaying UI on live video is what an immediate-mode GUI is good at. The window, event loop, renderer, pointer and key mappers and Mac cursor code are reused. It has screen-reader support (AccessKit) and a headless test harness (`egui_kittest`). | iced: its own wgpu/winit versions would gate Transom's, and video under widgets needs a custom shader widget. slint: owns the window and event loop, and its licence (GPL or royalty-free with attribution) sits badly with Apache-2.0. eframe: owns the event loop and device, so the renderer would become a paint callback and the viewer would not be reused. |
| Fallback: egui 0.36.2 | 0.37.0 is days old. Both need wgpu 30 and winit 0.30.13. | Do not mix the two. |
| Two binaries from one package: `transom` (console CLI) and `transom-gui` (no console on Windows) | A Windows-subsystem exe has no usable console. Re-attaching one is unreliable in PowerShell and cmd. | One Windows-subsystem exe that re-attaches a console. Decided by the owner, section 12. |
| `transom` with no subcommand opens the GUI (default build) | Double-clicking or typing `transom` should just work. | Print help (the behaviour of a build without the `gui` feature). |
| Refactor `viewer.rs`, do not duplicate it | One implementation of input, cursor and viewport logic serves both the CLI and GUI. | Copy the viewer into the GUI. |
| A pure `Model` / `Msg` / `Effect` core | Testable without a window or a network. | Views mutating state directly. |
| Dark theme only; upstream Sill's palette | Matches Sill's iPad app. | A light theme (out of scope). |
| Thumbnail strip in the top bar, in Sill's order (Apps, thumbnails, Desktop) | As Sill's iPad app does. | A "thumbnail rail" of larger thumbnails at the side: a separate, unscheduled idea. |
| The bar does not reveal on the top screen edge | The Mac's own menu bar lives there. | Edge reveal. |
| Plain (unencrypted) doors hidden behind a preference, off by default | As Sill's own release builds do. | Always show them. |
| Default frame rate 60 | Most Linux and Windows displays are 60 Hz, and the host runs at the highest rate any connected device asks for. | 120. |
| Phase 1 excludes Apps, window commands and Mac settings | Keeps the MVP to list, pair, connect, switch windows. | Pull them into the MVP (no design change needed). |
| Every user-visible string has an id in `gui/texts.rs` | One home for copy; translation later; a string with no id is a review failure. | Strings inline in views. |

## 3. Screens and flows

One window, 1100x720 logical points at first, minimum 640x420, titled "Transom". While
streaming the title is "‹Mac› — Transom", which also helps window switchers. There are two
screens: **Macs** and **Stream**. Everything else is a card inside Macs, a panel or popover
inside Stream, or a modal dialog. String ids (LIST1, PAIR3 …) refer to section 5.

### 3.1 Theme

Dark always. The palette is Sill's iPad app's.

| Role | Hex |
|---|---|
| bar and drawer | #16181C |
| control | #23262C |
| control open | #2C3550 |
| accent | #9BB7FF |
| text | #EEF0F3 |
| muted | #A4ABB5 |
| panel | #1F2126 |
| thumbnail placeholder | #26282D |
| icon fallback | #3A3D44 |
| error | #FF8A80 |

Body text 14 pt, secondary 12 pt, titles 17 pt semibold, codes monospace 20 pt. Corners are
10 pt on controls and 14 pt on cards.

### 3.2 First run

1. Start-up creates the device key silently and starts watching for Macs on the network.
2. With no saved Macs, the Macs screen shows the welcome line (LIST1), the status LIST2, any Macs
   found, and "Add a Mac…" (LIST7).
3. After 5 s with no Mac found, hint LIST3 appears under the status.
4. The device name defaults to "Transom (Linux)", "Transom (Windows)" or "Transom (macOS)". It
   is changed only in Preferences and is never prompted for.

### 3.3 Macs screen

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                     [⚙ Preferences]  │
│                                                                      │
│          Connect to a Mac                                            │
│          Looking for Macs on this network…            (status line)  │
│                                                                      │
│          MY MACS                                                     │
│          [▣] Studio                                Home        [⋯]   │
│              192.168.1.20                                            │
│          [▣] Ryan’s MacBook Pro                    Remote      [⋯]   │
│              100.101.102.103:7455 · Tailscale                        │
│          [▣] Mac mini                              Not found   [⋯]   │
│                                                                      │
│          OTHER MACS ON THIS NETWORK                                  │
│          [▣] Office iMac                           Not paired        │
│                                                                      │
│          Add a Mac…                                                  │
│                                                                      │
│   Needs the free Sill app on your Mac.  Get Sill · About Transom     │
└──────────────────────────────────────────────────────────────────────┘
```

- A column 440 pt wide, centred, top at 15 % of the window height. It scrolls when taller than
  the window. Rows are 52 pt tall.
- **MY MACS**: saved Macs, sorted by name. **OTHER MACS ON THIS NETWORK**: found Macs that no
  saved Mac matches, sorted by name. An empty section is not shown.
- Status line under the title: LIST2 until the first Mac is found, then empty, or a short result
  line (LIST24 "Forgot ‹Mac›.", PAIR8 "Paired with ‹Mac›.").
- **Footer.** "Get Sill" (LIST9) opens `https://getsill.app/download`. "About Transom" (LIST10)
  opens the About dialog (section 3.8).
- **Banner.** When a stream ends for good, the Macs screen returns with a banner above the title:
  the end text plus at most two buttons (section 5.8). It stays until dismissed (x) or another
  action starts.
- Clicking a row switches to the Stream screen's "Connecting" state at once.

**Row words.** A pure function of the row (`gui/macs.rs`).

| Row | Word | Second line | Click (primary action) |
|---|---|---|---|
| Saved, found at home | **Home** (LIST12) | first found IP | connect at home |
| Saved, not found, has a remote address (section 3.9) | **Remote** (LIST13) | `host:port · via` | connect remotely (pinned key) |
| Saved, not found, no remote address | **Not found** (LIST14) | none | open the Mac details dialog at "Remote address" with hint DET1 |
| Unsaved, door requires pairing | **Not paired** (LIST15) | IP | pairing card (section 3.4) |
| Unsaved, door open | **Not paired** (LIST15) | IP | open-door card: [Pair], or "Connect without pairing" (LIST23; any key; shows warning DOOR1) |
| Unsaved, plain door | **Update Sill** (LIST16; preference "Allow unencrypted connections" off) or **Unencrypted** (LIST17; on) | IP | off: tooltip DOOR2, no action. On: confirm DOOR3, then connect unencrypted |

**Row menu** ("⋯" on saved rows; also right-click or the Menu key): Connect (LIST18, the primary
action), Connect at Home (LIST19, only when found), Connect Remotely (LIST20, only when a remote
address exists), Details… (LIST21), Forget ‹Mac›… (LIST22).

**Forget.** Confirm dialog DLG1, then the saved Mac and its per-Mac preferences are removed.

**Keyboard.** Up/Down move between rows, Enter runs the primary action, Shift+F10 or Menu opens
the row menu.

### 3.4 Pairing at home (row "Not paired")

The card replaces the rows inside the column, titled "Pair with ‹Mac›" (PAIR1). Esc or Cancel
closes it.

```
  Pair with Office iMac
  Asking Office iMac for a code…            ◌             (state Asking)
  ─ then ─
  Office iMac is showing a code. Type it here.             (state Shown)
  ┌────────────────────────┐
  │ 4829 1355 7208         │  (monospace, groups of four as you type)
  └────────────────────────┘
  That code has a typo. Check it against your Mac.         (red, inline, when it applies)
  [  Pair  ]    Cancel
```

State machine (pure, `gui/pairing.rs`):

```
Asking ──ask ok, reason "shown"──────────▶ Code{pin: fingerprint from the ask, shown: true}
Asking ──ask ok, reason "openOnMac"──────▶ Code{pin, shown: false}   (PAIR4 instead of PAIR3)
Asking ──locked/closed/stopped/busy/other, or a network error──▶ Failed{text}   ([Try Again] [Close])
Code ──submit, wrong length──▶ Code + inline PAIR5
Code ──submit, typo──────────▶ Code + inline PAIR6   (also shown live once 12 digits are typed)
Code ──submit ok──▶ Working (PAIR7; pairs with the key pinned to the ask's fingerprint)
Working ──ok──▶ save the Mac; Paired (PAIR8) ──0.8 s──▶ connect at home
Working ──refused "code" with tries left──▶ Code + inline PAIR9 (field cleared)
Working ──any other refusal / network error──▶ Failed{text}
Cancel in Code{shown} ──▶ cancel the Mac's pairing window (best effort), close the card
Cancel elsewhere ──▶ close the card (a running task's result is ignored by generation number)
```

- **Field.** Accepts digits, spaces, hyphens and no-break spaces; shows groups of four; pasting
  "4829-1355-7208" works. Pair is enabled only with 12 digits and no typo. Enter submits.
- **While working** the field and button are disabled and a spinner with the status text shows.
  Key derivation takes about 1 s and never runs on the UI thread.

### 3.5 Add a Mac (address and code, or a pairing link)

```
  Add a Mac
  Pair once, and Transom reaches your Mac from anywhere, through your VPN or the internet.
  On your Mac, choose Pair iPhone or iPad… in the Sill menu. Type the address and code it shows.
  Address  [ 100.101.102.103 or mac.example.net        ]
  Code     [ 0000 0000 0000 ]
  [  Pair  ]    Use a Pairing Link Instead    Cancel
```

- **Address.** Parsed by the existing address parser. A missing port means 7455. Errors show
  inline under the field.
- **Code.** Validated as in section 3.4.
- **On success:** save the Mac, remember the typed address as that Mac's remote address, show
  PAIR8, and connect remotely after 0.8 s.
- **Refusals** are as in section 3.4. A remote door with no open pairing window answers
  "closed", which shows PAIR10.
- **Which Mac is it?** There is no earlier fingerprint to pin at the remote door, and none is
  needed. The code's derived key is bound to the Mac's TLS fingerprint, and Transom checks the
  Mac's proof over both fingerprints. A host that does not know the code, or a man in the middle,
  fails with PAIR15. After success the key is pinned. Mac details shows the full fingerprint for
  anyone who wants to compare it.
- **"Use a Pairing Link Instead"** swaps the fields for one field ("sill://pair?…"). On Pair
  the link is parsed, then a confirmation shows (ADD10, with [Pair] [Cancel]). Confirming tries
  Bonjour doors first, then the link's own addresses, exactly as `transom pair --url` does. A link
  that does not parse shows ADD11. "Use an Address and Code Instead" swaps back.
- After a link pairing, the Mac is connected at home if Bonjour finds it, otherwise remotely
  through the link's addresses (the first is saved as its remote address).

### 3.6 Stream screen

Layers, bottom to top: (1) video, letterboxed in the whole window as today; (2) the state card
(connecting, waiting, reconnecting), centred; (3) the top bar, an overlay that never shrinks the
video, so showing it never resizes the Mac's window; (4) popovers and panels; (5) modal dialogs.

```
 ┌────────────────────────────── top bar, 76 pt, #16181C at 94 % ─────────────────────────────┐
 │ [⌕ Apps] │ [thumb][thumb][thumb][thumb][thumb] …▸ │ [▭ Desktop] │ 18 ms · 60 fps │ [⚙] [⤢] [✕] │
 └────────────────────────────────────────────────────────────────────────────────────────────┘
                                   ▔▔▔▔▔▔▔  (handle: 120×5 pt pill, top centre, 40 % white,
                                             shown while the bar is hidden; hover area 160×24 pt)
```

**Bar contents, left to right** (Sill's order). In Phase 1 Apps is absent and the strip starts at
the left edge.

| Item | Behaviour |
|---|---|
| Apps (Phase 2) | Toggles the Apps popover |
| Thumbnail strip | Scrolls horizontally (wheel or Shift+wheel, drag, arrow buttons at the ends) |
| Desktop | Shows the desktop. Lit (accent) while the active source is the desktop |
| Stats pill | "‹rtt› ms · ‹fps› fps", on by default (preference). Click: tooltip BAR5 |
| Settings (gear) | The settings panel (section 3.8). In Phase 1: the header, This Computer and Disconnect (sections A, D, E) |
| Full screen | Toggle |
| Disconnect | Disconnects at once; no confirmation |

**Thumbnail tiles:**

- 96x60 pt, rounded 8 pt, in the order the Mac sends.
- The window's picture, aspect-fit, on #26282D. With no picture yet, a placeholder with the app
  icon centred.
- Badge: the app icon, 20x20 pt, bottom-left, overlapping 4 pt. With no icon, a two-letter badge
  on #3A3D44.
- The active window has a 2 pt accent border.
- Tooltip: "‹App› — ‹Title›" (BAR10).
- Click shows that window. The first click in a session also shows toast BAR12 once, because Sill
  streams one source to every device.
- Right-click, or "⋯" on hover (Phase 2): Show, Minimize on the Mac, Full Screen on the Mac,
  Close on the Mac (BAR13-BAR16).

**Auto-hide.** The bar is shown:

- while the phase is not Connected,
- for 3 s after the first frame,
- while the pointer hovers the handle area for 250 ms,
- while the bar is pinned (see below),
- while any popover or panel is open.

It hides 1.0 s after the pointer leaves it, unless one of those still holds. The top screen edge
alone does not reveal it.

**Pinning.** Clicking the handle does the same as chord+B. When the bar is hidden or shown only by
hover, it pins the bar open. When the bar is pinned, it unpins and hides it at once. There is no
double-click on the handle; full screen is only the bar button and chord+F. In Phase 2 the
preference "Keep the bar open" makes the bar start pinned at every connect.

**State cards** (centred, 380 pt):

- **Connecting.** Spinner, the latest status text (CONN1 at first), [Cancel].
- **Waiting.** Connected, no frame yet. After 1.5 s, CONN2 ("Waiting for the first picture…").
- **Reconnecting.** The video is cleared. The card shows the text for the end reason
  (section 5.8), then CONN3 counting down, with [Try Now] and [Disconnect].
- **View only.** A "View only" chip (BAR17) in the bar while the Mac is this computer and input
  gating turns a direction off. Tooltip BAR18, with a link to Preferences > Input.

**Toasts.** Bottom centre, 4 s, never modal. BAR12 is described above. The Phase 2 link notice
(the Mac reports a link "behind") is a toast, BAR20, with [Lower Quality] (BAR19), which applies the Mac's
suggested bitrate and resolution.

**Link chip.** While the Mac reports the link "behind", the bar shows a chip, BAR25, in the error
colour, whatever the stats preference. Its tooltip, BAR26, gives the withheld frames, the bitrate
the link could not carry, what it carried, and the Mac's suggestion. Each change of the report is
logged at info.

### 3.7 Apps popover (Phase 2)

Anchored under Apps, 360 pt wide, at most 520 pt tall.

- Search field focused on open, placeholder APPS1.
- **Open Now** (APPS2) lists apps with windows, grouped by app, with the trailing word APPS4 or
  APPS5. A click shows that app's first window. Clicking again while it is showing cycles to its
  next window.
- **All Apps** (APPS3) lists installed apps not open now. A click launches the app and shows toast
  APPS6.
- Filtering is a case-insensitive substring match on the name.
- Up/Down move, Enter activates, Esc closes.

### 3.8 Panels and dialogs

**Settings panel** (the gear): a right-side panel, 360 pt, under the bar, full height, scrolling.
In Phase 1 it holds sections A, D and E only.

- **A. Header.** "‹Mac›", then SET1 ("Connected at home · 192.168.1.20"), SET2 ("Connected
  remotely · 100.101.102.103 (Tailscale)") or SET3 ("Connected without a pinned key").
- **B. Streaming** (Phase 2; changes are sent to the Mac and apply to every device streaming from
  it):
  - Quality (SET4): a combo of seven presets (Low 4, Efficient 8, Balanced 15, High 25, Pro 40,
    Ultra 80, Extreme 150 Mbps), titled SET5 ("Balanced — 15 Mbps"). A value the Mac holds that
    is not a preset shows read-only as SET6 ("Custom — N Mbps") until a preset is chosen.
  - Resolution (SET7): Retina (SET8) or Standard (SET9).
  - Frame Rate (SET10): 60 fps (SET11) or 120 fps (SET12).
  - Prioritize Encoding Speed (SET13, switch), with footnote SET14.
  - Virtual Display (SET15, switch), disabled when the Mac says it is unavailable. Footnote: the
    Mac's own note, else SET16.
  - Footnote SET17 under the group. A pending pick shows a small spinner on its row. A refused
    pick snaps back and shows toast SET18.
  - Read-only SET19 ("Now streaming 2880×1800 · 60 fps · 15 Mbps"), or SET20 ("Not streaming").
  - Notes SET21 (software encoder), SET22 (settings not persistent) and SET23 (this connection
    counts as away) when they apply.
  - If no settings arrive within 2 s of connecting, SET25. Before that, SET24 ("Loading
    settings…").
- **C. Mac connection** (Phase 2): Direct Wireless Connection (SET26, switch), shown only when the
  Mac reports the field, with footnote SET27.
- **D. This Computer:** Keymap (SET32), with the same two labels as Preferences: PREF9 and
  PREF10. When the Mac is this computer: Send Pointer (SET28) and Send Keyboard (SET29) switches,
  with warning PREF14 next to Keyboard. In Phase 2: Text Size (SET30; 0.5, 0.75, 1, 1.25, 1.5,
  Off (SET31)), which scales the Mac window. Changes save to preferences and apply at once.
- **E. Footer** (pinned): [Disconnect from ‹Mac›] (BAR9).

**Preferences** (modal, 560 pt; tabs General, Input, Streaming, Advanced; PREF5-PREF8). Changes
save at once; close with [Done] or Esc.

- **General:** "Name on your Macs" (PREF1; default as section 3.2) with note PREF2. "This
  computer's key" (PREF3): a monospace fingerprint with [Copy] and a hex disclosure. "Config
  folder" (PREF4): the path with [Copy].
- **Input:** Modifier layout, as two radio buttons (PREF9 "PC layout: Ctrl acts as Command" and
  PREF10 "Mac layout: Command is Command, Ctrl is Control") with the README's mapping table
  beneath. The PC layout is the default on Linux and Windows, the Mac layout on macOS. "When the
  Mac is this computer" (PREF11): "Send the pointer" (PREF12, off) and "Send the keyboard"
  (PREF13, off, warning PREF14).
- **Streaming:** "Frame rate to ask for" (PREF15) 60 or 120 (default 60). "Show round trip and
  frame rate in the bar" (PREF16, on). "Keep the bar open" (PREF17; Phase 2; off): the bar starts
  pinned at every connect.
- **Advanced:** "Allow unencrypted connections" (PREF18, off) with warning DOOR4. "Sill version
  to report" (PREF19; default the compatibility version Transom reports today, with [Reset]).
  "Write a log file" (PREF20, off), with level Info or Debug (PREF21, PREF22) and the path;
  applies at next launch.

**Mac details dialog** (modal, 520 pt; from the row menu "Details…" or a "Not found" row):

- Name (DET5), Mac ID (DET6), and the full key fingerprint (DET7) with [Copy].
- "Addresses the Mac reported" (DET2): the saved list as `host:port · via · kind`, or DET3.
- "Remote address" (DET4): a text field (default port 7455) with DET1 as the placeholder line,
  [Save] and [Clear].
- [Forget ‹Mac›…] (LIST22). Esc closes.

**About dialog** (modal), opened from "About Transom":

- "Transom ‹version›" (DLG5; the package version).
- DLG6: "An independent client for Sill. Not part of Sill and not endorsed by its author."
- The licence and the Sill credit line, as in the NOTICE file (DLG7).
- Links to github.com/ry-cen/sill_port and getsill.app (DLG8).
- [Close].

**Confirm dialogs:** DLG1 (forget), DLG2 (forget after a key change), DOOR3 (unencrypted), DOOR1
(open door). Buttons are the action verb plus Cancel. The destructive button is red, and Cancel
has the default focus.

### 3.9 Remote address order for a saved Mac

1. The Mac's manually entered remote address, if set. Its port, else the Mac's saved remote port,
   else 7455.
2. Every address the Mac reported whose kind is not "lan", in saved order, each with its own port,
   else the saved remote port, else 7455.
3. None: the row word is "Not found".

All candidates are tried in order. The row's second line shows the first as `host:port · via`
(no "via" for a manual address).

## 4. States and error handling, in brief

| Situation | What the user sees |
|---|---|
| No Macs found | LIST2, then LIST3 after 5 s |
| Pairing refused | Inline text PAIR9-PAIR17 in the card; never a dialog |
| Bad code typed | PAIR5 or PAIR6 inline, Pair disabled |
| Connecting | Connecting card with Cancel |
| Connected, no picture | Waiting card after 1.5 s |
| Network drop or Mac quit | Reconnecting card with countdown and Try Now |
| Mac says goodbye for good, or key problems | Back to Macs with a banner and a fixing button |
| This Mac is the computer in use | "View only" chip |
| Settings unsupported by an older Sill | SET25 |

Every string from the Mac (names, window titles, app names, messages) is passed through the
existing `safetext::label` before it is shown. Codes, link secrets and keys are never logged.

## 5. Copy

Strings below are the exact user-facing text. "‹Mac›" stands for the Mac's name after safe-text
filtering; "N" is a number. Ellipses are U+2026 (…) and apostrophes are curly (’); copy them as
written. All GUI copy lives in `gui/texts.rs`, one id per string, with descriptive function names
and the id in each doc comment; `text.rs` stays the CLI's. Ids are by area (prefix and number, in
order of first appearance). The "Old id" column maps to the ids of revision 1 of this spec and the
design; it will be dropped in the next revision. "—" means the string was prose-only before and
had no id.

### 5.1 Macs screen (LIST) and Mac details (DET)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| LIST1 | W1 | Transom shows a Mac’s screen here and sends your mouse and keyboard to it. Your Mac needs the free Sill app. | Welcome line |
| LIST2 | S1 | Looking for Macs on this network… | Status, until the first Mac is found |
| LIST3 | H1 | No Mac yet? Open Sill on your Mac, on the same network as this computer. Away from home, use Add a Mac… with its address and a code. | 5 s with no Mac found |
| LIST4 | — | Connect to a Mac | Screen title |
| LIST5 | — | MY MACS | Section heading |
| LIST6 | — | OTHER MACS ON THIS NETWORK | Section heading |
| LIST7 | — | Add a Mac… | Last row |
| LIST8 | — | Needs the free Sill app on your Mac. | Footer |
| LIST9 | — | Get Sill | Footer link and the update banner button; opens https://getsill.app/download |
| LIST10 | — | About Transom | Footer link |
| LIST11 | — | Preferences | Top-right button |
| LIST12 | — | Home | Row word |
| LIST13 | — | Remote | Row word |
| LIST14 | — | Not found | Row word |
| LIST15 | — | Not paired | Row word |
| LIST16 | — | Update Sill | Row word (plain door, unencrypted off) |
| LIST17 | — | Unencrypted | Row word (plain door, unencrypted on) |
| LIST18 | — | Connect | Row menu |
| LIST19 | — | Connect at Home | Row menu |
| LIST20 | — | Connect Remotely | Row menu |
| LIST21 | — | Details… | Row menu |
| LIST22 | — | Forget ‹Mac›… | Row menu and details dialog |
| LIST23 | — | Connect without pairing | Open-door card |
| LIST24 | — | Forgot ‹Mac›. | Status line after forgetting |
| DET1 | H2 | Away from home, Transom reaches ‹Mac› at this address (for example its Tailscale address). Leave it empty to use the addresses ‹Mac› reports. | Details dialog, remote address field |
| DET2 | — | Addresses the Mac reported | Details dialog |
| DET3 | — | None yet. They arrive while connected, if Remote Access is on. | No reported addresses |
| DET4 | — | Remote address | Details dialog |
| DET5 | — | Name | Details dialog (label) |
| DET6 | — | Mac ID | Details dialog (label) |
| DET7 | — | Key fingerprint | Details dialog (label; the design says "fingerprint" without exact wording) |

### 5.2 Pairing at home (PAIR)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| PAIR1 | — | Pair with ‹Mac› | Pairing card title |
| PAIR2 | K0 | Asking ‹Mac› for a code… | State Asking |
| PAIR3 | K1 | ‹Mac› is showing a code. Type it here. | State Code (shown) |
| PAIR4 | K2 | ‹Mac› didn’t show a code by itself. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then type the code here. | Ask answered "openOnMac" |
| PAIR5 | T0 | A code has 12 digits. | Code of the wrong length submitted |
| PAIR6 | T1 | That code has a typo. Check it against your Mac. | Check digit fails (live at 12 digits, and on submit) |
| PAIR7 | K3 | Pairing with ‹Mac›… | State Working |
| PAIR8 | K4 | Paired with ‹Mac›. | Pairing succeeded |
| PAIR9 | R1 | Wrong code. N tries left. | Refusal "code" with tries left (same text as the CLI) |
| PAIR10 | R2 | ‹Mac›’s pairing window is closed. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then try again. | Refusal "closed" (including the remote door with no open pairing window) |
| PAIR11 | R3 | That code expired. Pair again to get a new one. | Refusal "expired" |
| PAIR12 | R4 | ‹Mac› stopped pairing after too many wrong codes. On the Mac, choose Pair iPhone or iPad… in the Sill menu, then try again. | Refusal "stopped" |
| PAIR13 | R5 | ‹Mac› is busy. Try again in N s. | Refusal "busy" |
| PAIR14 | R6 | Unlock ‹Mac›, then try again. | Refusal "locked" |
| PAIR15 | R7 | Pairing didn’t finish: ‹Mac› couldn’t show it knows the code. | The Mac’s proof of the code was bad |
| PAIR16 | R8 | ‹Mac› didn’t answer. Check that Sill is open on it, then try again. | Timeout, connect failure or closed connection while pairing |
| PAIR17 | R9 | ‹Mac› said: ‹message› | Unknown refusal reason with a message; otherwise "‹Mac› refused to pair (‹reason›)." |

### 5.3 Add a Mac (ADD)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| ADD1 | A0 | Add a Mac | Card title |
| ADD2 | A2 | Pair once, and Transom reaches your Mac from anywhere, through your VPN or the internet. | Card |
| ADD3 | A3 | On your Mac, choose Pair iPhone or iPad… in the Sill menu. Type the address and code it shows. | Card |
| ADD4 | — | Address | Field label |
| ADD5 | — | Code | Field label (also on the pairing card) |
| ADD6 | — | 100.101.102.103 or mac.example.net | Address placeholder |
| ADD7 | — | Use a Pairing Link Instead | Card |
| ADD8 | — | Use an Address and Code Instead | Link mode |
| ADD9 | — | sill://pair?… | Link field placeholder |
| ADD10 | L1 | Pair with ‹link name› at ‹first address›? Only pair with a link your own Mac shows. | Link confirmation |
| ADD11 | L2 | That isn’t a Sill pairing link. It starts with sill://pair. | Link does not parse |

### 5.4 Doors and confirmations (DOOR, DLG)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| DOOR1 | O1 | ‹Mac› lets any device connect without pairing. Transom can’t check that it is really ‹Mac›. Pair it to be sure. | Open-door warning |
| DOOR2 | U1 | ‹Mac›’s Sill is too old for Transom. Update Sill on the Mac. | Tooltip on a plain-door row when unencrypted is off |
| DOOR3 | P1 | ‹Mac› uses an unencrypted connection. Anyone on this network can see the stream and your typing. Connect anyway? | Confirm for a plain door |
| DOOR4 | P2 | Only for test builds of Sill. Unencrypted connections can be watched by anyone on the network. | Preferences > Advanced |
| DLG1 | F1 | Forget ‹Mac›? Transom removes its key. To use it again, pair again. ‹Mac› still lists this computer until you remove it in Sill’s Devices. | Forget confirm |
| DLG2 | F2 | Forget ‹Mac› and pair again? Only do this if you know why its key changed, for example Sill was reinstalled. | Confirm after a key change |
| DLG3 | — | Forget | Destructive button of DLG1 and DLG2 |
| DLG4 | — | Connect | Confirm button of DOOR3 |
| DLG5 | — | Transom ‹version› | About dialog |
| DLG6 | — | An independent client for Sill. Not part of Sill and not endorsed by its author. | About dialog |
| DLG7 | — | Apache License 2.0. A client for Sill (github.com/Saffsanity/sill), © 2026 Noah Saffer, licensed under the Apache License 2.0. | About dialog |
| DLG8 | — | github.com/ry-cen/sill_port · getsill.app | About dialog (links) |

### 5.5 Connection states (CONN)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| CONN1 | C1 | Connecting to ‹Mac›… | Connecting card, before the first status |
| CONN2 | C2 | Waiting for the first picture… The Mac sends a full picture about every 4 seconds. | Waiting card, after 1.5 s |
| CONN3 | C3 | Trying again in N s | Reconnecting card |

### 5.6 Bar (BAR) and Apps (APPS)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| BAR1 | — | Apps | Bar button and its accessible label (Phase 2) |
| BAR2 | — | Desktop | Bar button |
| BAR3 | — | Show the desktop | Desktop button, accessible label |
| BAR4 | — | ‹rtt› ms · ‹fps› fps | Stats pill |
| BAR5 | — | Round trip 18 ms, 60 frames per second, connected at home / remotely / without a pinned key | Stats tooltip, with the real values and the one connection phrase that applies |
| BAR6 | — | Mac settings | Gear, accessible label |
| BAR7 | — | Full screen | Button and accessible label |
| BAR8 | — | Leave full screen | Same button in full screen |
| BAR9 | — | Disconnect from ‹Mac› | Disconnect button accessible label; settings panel footer |
| BAR10 | — | ‹App› — ‹Title› | Tile tooltip and accessible label |
| BAR11 | — | (showing) | Added to the active tile’s accessible label |
| BAR12 | N1 | Everyone watching ‹Mac› now sees this window. | First window pick in a session (toast) |
| BAR13 | — | Show | Thumbnail menu (Phase 2) |
| BAR14 | — | Minimize on the Mac | Thumbnail menu (Phase 2) |
| BAR15 | — | Full Screen on the Mac | Thumbnail menu (Phase 2) |
| BAR16 | — | Close on the Mac | Thumbnail menu (Phase 2) |
| BAR17 | — | View only | Chip in the bar |
| BAR18 | V1 | View only: this Mac is the computer you are using. Sending its own input back to it would loop. | Chip tooltip |
| BAR19 | — | Lower Quality | Link-notice toast button (Phase 2) |
| BAR20 | — | The connection can’t keep up, so ‹Mac› is skipping frames. | Link-notice toast (Phase 2) |
| BAR25 | — | Network behind · ‹n› dropped/s | Link chip in the bar |
| BAR26 | — | The Mac is dropping ‹n› frames a second: the network to this computer cannot carry ‹rate› per 60 fps. It carried ‹rate›. Sill suggests ‹rate› at ‹Standard/Retina› resolution. Change it in Mac settings, or use a wired connection. | Link chip tooltip; the carried and suggestion sentences only when the Mac sends them |
| APPS1 | — | Search apps on ‹Mac› | Search placeholder (Phase 2) |
| APPS2 | — | Open Now | Section heading |
| APPS3 | — | All Apps | Section heading |
| APPS4 | — | 1 window | Row trailing word |
| APPS5 | — | N windows | Row trailing word |
| APPS6 | A1 | Opening ‹App› on ‹Mac›… | App launched (toast) |

### 5.7 Settings panel (SET) and Preferences (PREF)

| Id | Old id | Text | Shown when |
|---|---|---|---|
| SET1 | — | Connected at home · ‹address› | Panel header |
| SET2 | — | Connected remotely · ‹address› (‹via›) | Panel header |
| SET3 | — | Connected without a pinned key | Panel header |
| SET4 | — | Quality | Phase 2 |
| SET5 | — | ‹Preset› — ‹N› Mbps (for example Balanced — 15 Mbps) | Quality combo title; presets Low, Efficient, Balanced, High, Pro, Ultra, Extreme |
| SET6 | — | Custom — N Mbps | The Mac holds a non-preset bitrate (read-only) |
| SET7 | — | Resolution | Phase 2 |
| SET8 | — | Retina | Resolution choice |
| SET9 | — | Standard | Resolution choice |
| SET10 | — | Frame Rate | Phase 2 |
| SET11 | — | 60 fps | Choice |
| SET12 | — | 120 fps | Choice |
| SET13 | — | Prioritize Encoding Speed | Switch |
| SET14 | S2 | Lower latency when ‹Mac› is busy, at a softer picture. | Under SET13 |
| SET15 | — | Virtual Display | Switch |
| SET16 | S3 | The window you pick moves onto an invisible display on ‹Mac› while it streams, so it keeps updating when covered. | Under SET15, when the Mac gives no note |
| SET17 | S4 | Applies to every device streaming from ‹Mac›. The stream restarts for a moment. | Under the Streaming group |
| SET18 | S5 | ‹Mac› kept its setting. | Toast after a refused pick |
| SET19 | — | Now streaming ‹w›×‹h› · ‹n› fps · ‹n› Mbps | Read-only line |
| SET20 | — | Not streaming | Read-only line |
| SET21 | S6 | ‹Mac›’s hardware encoder isn’t answering, so it streams with the software encoder, up to 60 fps at Standard. | Mac reports the software encoder |
| SET22 | S7 | SillHost keeps these until it quits. | Mac reports settings are not persistent |
| SET23 | S8 | This connection counts as away from home: Quality and Resolution here set ‹Mac›’s away quality. | This connection is away |
| SET24 | — | Loading settings… | Before settings arrive |
| SET25 | S9 | Update Sill on ‹Mac› to change these from here. | No settings within 2 s of connecting |
| SET26 | — | Direct Wireless Connection | Switch, shown only when the Mac reports it |
| SET27 | S10 | Lets iPhones and iPads reach ‹Mac› without a shared Wi‑Fi network. Transom doesn’t use it. | Under SET26 |
| SET28 | — | Send Pointer | Panel switch |
| SET29 | — | Send Keyboard | Panel switch |
| SET30 | — | Text Size | Phase 2 |
| SET31 | — | Off | Text Size choice |
| SET32 | — | Keymap | Panel row label |
| SET34 | — | Stream | Mac settings panel heading for what the Mac runs |
| SET35 | — | The Mac has not reported a running stream yet. | No `stream` in the host settings |
| SET36 | — | The Mac encodes ‹w›×‹h› at up to ‹fps› fps, ‹n› Mbps[, on the virtual display]. | The host settings' `stream`; also logged at info on every change |
| SET37 | — | Software encoder: the Mac's hardware encoder did not answer, so streams run at up to 60 fps at Standard until it recovers (the Mac checks every 30 s). | `softwareEncoder` true, in the error colour |
| PREF1 | — | Name on your Macs | General |
| PREF2 | G1 | Macs you already paired keep showing the old name. | Under PREF1 |
| PREF3 | — | This computer's key | General |
| PREF4 | — | Config folder | General |
| PREF5 | — | General | Tab |
| PREF6 | — | Input | Tab |
| PREF7 | — | Streaming | Tab |
| PREF8 | — | Advanced | Tab |
| PREF9 | — | PC layout: Ctrl acts as Command | Radio; also the Settings panel |
| PREF10 | — | Mac layout: Command is Command, Ctrl is Control | Radio; also the Settings panel |
| PREF11 | — | When the Mac is this computer | Input |
| PREF12 | — | Send the pointer | Input |
| PREF13 | — | Send the keyboard | Input |
| PREF14 | V2 | Keys you type would reach this Mac twice. Don’t type into the Transom window while this is on. | Next to Send the keyboard / Send Keyboard |
| PREF15 | — | Frame rate to ask for | Streaming |
| PREF16 | — | Show round trip and frame rate in the bar | Streaming |
| PREF17 | — | Keep the bar open | Streaming (Phase 2) |
| PREF18 | — | Allow unencrypted connections | Advanced |
| PREF19 | — | Sill version to report | Advanced |
| PREF20 | — | Write a log file | Advanced |
| PREF21 | — | Info | Log level |
| PREF22 | — | Debug | Log level |

Shared buttons (BTN): BTN1 Cancel, BTN2 Close, BTN3 Done, BTN4 Try Again, BTN5 Try Now, BTN6
Disconnect, BTN7 Copy, BTN8 Pair, BTN9 Save, BTN10 Clear, BTN11 Reset. All are new (no old id).

### 5.8 Ended states and goodbye reasons (END)

"Reconnects" means the supervisor backs off and retries; the video is cleared and the
Reconnecting card shows. "Ends" means Transom returns to Macs with a banner.

| Reason | Outcome | Text | Banner buttons |
|---|---|---|---|
| Connection lost | reconnects | END1 | none |
| Could not connect | reconnects | the connection error text (for example "‹Mac› was not found on this network"); passed through safe-text with a 300-character limit | none |
| Goodbye `quit` | reconnects | END2 | none |
| Goodbye `busy` | reconnects | END3 | none |
| Goodbye `remoteOff` | ends if connected remotely, else reconnects | END4 | [Connect at Home] (END12) when the Mac is found |
| Goodbye `internetOff` | ends if connected remotely, else reconnects | END5 | none |
| Goodbye `removed` | ends | END6 | [Pair Again] (END13): pairing card for that Mac (home path when found, else the Add card with the remote address filled in) |
| Goodbye `pairingRequired` | ends | END7 | [Pair] (END14; as above) |
| Goodbye `update` | ends unless the Mac asks to reconnect | the Mac’s message, or END8 | [Get Sill] (LIST9) |
| Goodbye, any other | follows the Mac’s reconnect flag | the Mac’s message, or END9 | none |
| Not paired | ends | END10 | [Pair Again] (END13) |
| Key changed (pin mismatch) | ends | END11 | [Forget and Pair Again…] (END15): confirm DLG2, forget, then pairing |

| Id | Old id | Text |
|---|---|---|
| END1 | E1 | ‹Mac› disconnected. Transom reconnects when it can reach it. |
| END2 | E2 | ‹Mac› quit Sill. Transom reconnects when it is back. |
| END3 | E3 | ‹Mac› is already serving 8 devices. |
| END4 | E4 | ‹Mac› turned off Remote Access. |
| END5 | E5 | ‹Mac› stopped accepting connections from the internet. Connect through your VPN. |
| END6 | E6 | ‹Mac› removed this computer. Pair again to use it. |
| END7 | E7 | ‹Mac› now asks devices to pair. Pair this computer to use it. |
| END8 | E8 | Update Transom to keep using ‹Mac›. (followed by " It needs version ‹v› or later." when the Mac gives one) |
| END9 | E9 | ‹Mac› closed the connection. |
| END10 | E10 | ‹Mac› doesn’t know this computer. Pair it again. |
| END11 | E11 | ‹Mac› presented a different key from the one saved. If Sill was reinstalled or the Mac was reset, forget it and pair again. Otherwise, don’t connect. |
| END12 | — | Connect at Home |
| END13 | — | Pair Again |
| END14 | — | Pair |
| END15 | — | Forget and Pair Again… |

END3 keeps "8 devices". The Mac sends no limit; "busy" comes only from the remote door, which
caps remote sessions at 8, and Sill’s own client says the same.

## 6. Keyboard, pointer and accessibility

### 6.1 Keyboard modes (the core rule)

There are exactly two modes, recomputed before every key event.

- **Mac has the keyboard** when all of these hold: the phase is Connected; keyboard forwarding is
  on (it is off when the Mac is this computer, unless enabled in Preferences); the window is
  focused; no popover, panel, menu or dialog is open; and no Transom text field wants the
  keyboard. Keys and input-method events are **not** given to the UI. Transom's chord shortcuts
  are handled and swallowed; every other key is sent to the Mac. The bar being visible does not
  change the mode.
- **Transom has the keyboard** otherwise. Keys go to the UI. Nothing goes to the Mac, except that
  chords still work.

On every change from Mac to Transom, every held key and button is released on the Mac. Modifier
changes always reach both the UI and the chord detector.

**The chord** is Ctrl+Alt+Shift (Control+Option+Shift on macOS) plus one key, detected on the
physical key.

| Chord + | GUI | CLI viewer (unchanged) |
|---|---|---|
| B | pin the bar open; if it is already pinned, unpin and hide it at once (same as clicking the handle) | none |
| D | show the desktop | same |
| W | show the next window | same |
| A | open Apps and focus search (Phase 2) | none |
| S | open the settings panel | none |
| F | toggle full screen | same |
| Q | disconnect (back to Macs) | quit |

**Shortcuts outside streaming.** Every Ctrl shortcut here uses Cmd on macOS. The streaming chord
stays Control+Option+Shift.

| Shortcut | Action |
|---|---|
| Ctrl+, (Cmd+, on macOS) | Preferences |
| Ctrl+N (Cmd+N on macOS) | Add a Mac |
| Esc | close the top card, panel or dialog; with none open, leave full screen |
| F11 | toggle full screen (also Ctrl+Alt+Shift+F when no Mac is connected) |
| Ctrl+= / Ctrl+− (Cmd+= / Cmd+− on macOS) | zoom the UI between 0.75 and 2.0; remembered |

These are Transom-mode shortcuts. In Mac mode the same keys go to the Mac as ordinary keys and the
UI never sees them, so UI zoom does not fire. **Esc while streaming** goes to the Mac in Mac mode
and closes the top panel in Transom mode.

**Keys the OS keeps** (Alt+Tab, the Win key, Ctrl+Alt+Del, Super on GNOME and KDE) never reach the
window, so never reach the Mac. With the PC layout, Ctrl acts as Command, so Ctrl+Tab reaches the
Mac as Cmd+Tab. Phase 3 adds an opt-in Windows keyboard hook.

### 6.2 Pointer

- Pointer events go to the UI first. They are forwarded to the Mac only when the phase is
  Connected, pointer forwarding is on, no modal dialog is open, and the pointer is not over (or
  dragging) Transom UI, judged from the last frame.
- **Exception:** a drag that started on the video keeps going to the Mac until the button is
  released, even over the bar.
- Wheel events over the bar scroll the bar, not the Mac.
- When the window loses focus, everything held is released.
- **Cursor.** The Mac's cursor shows only while the pointer is over the video and not over
  Transom UI. Otherwise the normal arrow and the UI's own cursors are used.

### 6.3 Full screen, scaling and window

- Full screen is borderless, from the bar button or chord+F. The bar auto-hides as usual. Esc does
  **not** leave full screen in Mac mode, because Esc belongs to the Mac.
- UI scale is the display scale times the user's zoom. Display-scale changes rebuild the layout,
  the pointer mapping and the Mac cursor. The viewport sent to the Mac is in logical points as
  today. Thumbnails are sent at 2x and drawn at 96x60 pt.
- Default size 1100x720. In Phase 2 the app remembers the last size and full-screen state. On
  Linux the window class and app id are `transom`, so the desktop entry matches.

### 6.4 Accessibility

- Screen readers: Windows UI Automation, macOS AX and Linux AT-SPI, through AccessKit.
- Every control has a text label. Icon-only bar buttons have hover text and an accessible label
  (BAR1, BAR3, BAR6, BAR7/BAR8, BAR9). Thumbnails are labelled BAR10, with BAR11 added to the
  active one.
- Every screen except the video can be used by keyboard alone: Tab follows reading order, Enter
  activates, Esc closes, focus is visible (accent).
- Body text #EEF0F3 and muted text #A4ABB5 on #16181C, and error #FF8A80 on #16181C, all exceed
  4.5:1. Minimum target 28x28 pt.
- The toolkit has no live regions, so on a state change (connecting, ended) keyboard focus moves
  to the state card's first button, so screen readers read the card. This is a known limit,
  to be checked once with Orca or NVDA.

## 7. Platform notes

### 7.1 Linux (X11 and Wayland)

- Desktop entry, `packaging/linux/transom.desktop`:

  ```
  Name=Transom
  GenericName=Mac Viewer
  Comment=View and control a Mac running Sill
  Exec=transom-gui %u
  Icon=transom
  Terminal=false
  Type=Application
  Categories=Network;RemoteAccess;
  StartupWMClass=transom
  ```

  Phase 3 adds `MimeType=x-scheme-handler/sill;`.
- Icons: an SVG plus PNGs at 16, 32, 48, 64, 128, 256 and 512 px.
- Wayland: there is no keyboard grab, so the compositor keeps its shortcuts (Super, Alt+Tab). The
  Mac's cursor size follows the window scale and some compositors misreport it; the README's
  `WINIT_UNIX_BACKEND=x11` workaround stays. GNOME draws client-side decorations. Fractional
  scaling works. The app id must be `transom` for the dock icon.
- X11 works as today. A full-screen keyboard grab is out of scope.
- No new system packages. FFmpeg is still required.

### 7.2 Windows

- `transom-gui.exe` has no console window. `transom.exe` stays a console CLI; started with no
  subcommand in a `gui` build, it opens the GUI and keeps its console and logs there. Logging in
  `transom-gui` goes to a file when enabled in Preferences, because standard error is gone. A
  panic writes `crash.log` into the config folder.
- High-DPI works without a manifest.
- Alt+Tab, Win, Win+L and Ctrl+Alt+Del are system keys. AltGr is reported as Ctrl+Alt, so the
  chord requires the **left** Ctrl on Windows.
- Phase 3: an opt-in "Capture system keys in full screen" preference forwards Win, Alt+Tab and
  Alt+Esc to the Mac while the window is focused and full screen.
- The FFmpeg DLLs must sit next to `transom-gui.exe` or be on `PATH`; Windows shows its own
  missing-DLL dialog otherwise.

### 7.3 macOS (testing target)

- The default application menu is turned off so Cmd+Q and Cmd+H reach the Mac. The window's
  close button quits.
- Viewing this Mac from itself is gated as in the CLI.
- No `.app` bundle before Phase 3.

### 7.4 Packaging (Phase 3, notes only)

Linux: a tarball with both binaries, the desktop file and icons (a Flatpak later). Windows: a zip
with both exes and the FFmpeg DLLs, then an installer that registers `sill:` and a Start-menu
entry. macOS: a `.app`. CI uploads `transom-gui[.exe]` beside `transom[.exe]`.

## 8. Architecture

### 8.1 Binaries and features

The `transom` package becomes a library (`transom_app`, holding every module now in `main.rs`)
plus two binaries.

- `transom` (`src/main.rs`): the console CLI. With a subcommand it does what it does today. With
  no subcommand, it opens the GUI when built with the `gui` feature (on Windows that window keeps
  the console and logs there), otherwise prints help and exits 2. `--config-dir` and `-v` work for
  both.
- `transom-gui` (`src/bin/transom-gui.rs`): requires the `gui` feature; on Windows it uses the
  windowless subsystem. It parses only `--config-dir`, `-v` and (Phase 3) one `sill://` link.
- Features: `default = ["video", "gui"]`; `gui` implies `video`. `cargo build -p transom
  --no-default-features` keeps building the CLI without FFmpeg or the UI toolkit.

### 8.2 Model, Msg, Effect

The GUI state is a pure `Model` (no UI toolkit, no window, no I/O) changed only by
`Model::update(msg, now) -> Vec<Effect>`.

- Views read `&Model` and push `Msg`s. The shell feeds them to `update` after each frame.
- `Effect`s are carried out by an effects runner: it spawns pairing tasks, starts and stops the
  stream, saves the store and preferences, and copies text. Results return as `Msg`s.
- Session events become `Msg::Session(..)`; the model's `SessionState` absorbs them.
- Each pairing card carries a generation number; results with another number are ignored.
- The model exposes the next time it needs a `Tick` (bar timing, countdown, toasts).
- End texts come from `texts::end_text(kind, mac, supervisor_text)`. `EndKind` carries no text;
  only `ConnectFailed` uses `supervisor_text` (the supervisor's connect errors contain no
  commands).

### 8.3 Threads, channels, wake-ups

```
main thread (winit event loop, UI)
  └─ GuiShell: owns window, video renderer, UI context, Model, drafts, image cache,
               and the current stream (if any)

tokio runtime (2 workers)                         std threads
  ├─ supervisor task (one per stream) ──events──▶  ├─ video decoder (one per stream)
  └─ pairing tasks (ask / code / link / cancel)    ├─ discovery watch (one, app lifetime)
                                                   └─ the mDNS library's own thread

All workers → UI: crossbeam channels + a wake closure that sends a user event to the event loop.
```

User events: `Net` (session events waiting), `Frame` (decoded frame waiting), `Msg` (effect
results waiting), `Repaint` (the UI asked for a repaint from another thread), and the screen
reader's events.

**Redraw policy.** Redraw on each decoded frame, each Msg or Net batch, a UI-requested repaint,
and when the UI's repaint delay or the viewport debounce expires. Each redraw runs one UI pass,
then draws video and UI in one render pass.

**Closing the window:** release everything held, close the session, wait up to 300 ms for the
supervisor, stop the discovery watch, exit 0 (as the CLI viewer does).

### 8.4 Refactors

- **`stream.rs` (`StreamCore`)**: everything per-connection in today's `viewer.rs` that is not
  window chrome: supervisor handle, event receiver, frame slot, decoder, key and pointer mappers,
  modifiers, forwarding flags and local gating, Mac cursor, viewport debounce, `release_all`,
  chord detection. It holds no session state; it handles its own concerns and returns every event
  to its shell. The CLI viewer and the GUI both use it.
- **`session_state.rs` (`SessionState`)**: a pure fold of session events (Mac name, windows,
  active source, apps, thumbnail and icon bytes, host settings, Mac info, rtt/fps, phase). One
  per stream. The CLI viewer takes its title and window cycling from it; the GUI model holds one.
  Phases: Connecting, Connected, Reconnecting (with retry time) and Finished.
- **`pairflow.rs`**: the async pairing steps taken out of `pair_cmd.rs` (candidate addresses for a
  link, ask then code, cancel). `pair_cmd.rs` keeps its stdin prompts and calls it; the GUI calls
  the same functions from tokio tasks.
- **Renderer overlay hook**: `transom-render` gains an `Overlay` trait and `render_with`. The CLI
  keeps calling `render()`.
- **Discovery watch**: a live, app-lifetime browse of `_sill._tcp` producing Found/Removed events,
  next to today's one-shot, blocking `browse`.
- **Store**: `read_file` and `write_file` for small named files (atomic, mode 0600, restricted
  file names). The GUI keeps `prefs.json` there.

### 8.5 Session event additions

Today the session forwards only some message kinds, and thumbnails (kind 3), app icons (kind 4)
and host settings (kind 16) are dropped. The design adds:

- `SessionEvent::Thumbnail { id, jpeg }`, `AppIcon { bundle_id, png }` and
  `HostSettings(HostSettingsState)`; bytes stay undecoded until first needed.
- `Ended` and `Finished` gain `kind: EndKind` = ConnectFailed, Lost, Goodbye { reason, message,
  minimum_version }, NotPaired or PinMismatch. The existing `text` is unchanged for the CLI.
- `SessionCommand::ChangeSettings(HostSettingsChange)` (message 17; dropped when no session runs)
  and `RetryNow` (ends a backoff wait; ignored otherwise).
- `Endpoint::Hosts(Vec<(String, u16)>)`: resolve each host in order and dial the combined list.
- `transom-proto::host_settings`: the types for kinds 16/17 and their choices (frame rate 60 or
  120; capture scale 2 or 1; the seven bitrate presets), plus, in Phase 2, a port of Sill's
  unanswered-pick ledger.

How the Mac behaves, as relied on: thumbnails (208x132 JPEG) and app icons (64 px PNG) are pushed
unasked, thumbnails refreshing about every 2-6 s; a settings state arrives after the window list
and is the answer to each change (with the change's token); a device may pick only the listed
values; and the remote door answers "ask" and "cancel" with "closed" and accepts a code only while
the Mac's own pairing window is open (which is why remote pairing is "type the address and code").

### 8.6 Preferences storage

`prefs.json` in the config folder, versioned, every field defaulted: device name, keymap, local
pointer and keyboard, frame rate (60), show stats (true), allow plain, hello version, log file and
level, UI zoom (1.0), and per-Mac preferences (the manual remote address). Missing or bad files
give defaults and a log warning.

## 9. Testing approach

- **Pure core**: unit tests for rows, pairing transitions, preferences, keyboard mode and model
  updates.
- **Views**: headless tests with `egui_kittest` that find widgets by their accessible labels.
- **Protocol**: mock-host tests in `crates/net/tests/gui_protocol.rs`. The mock gains thumbnails,
  icons, apps, host settings, launch and window commands, an optional Bonjour registration, and
  a configurable listen address.
- **Flow**: `crates/app/tests/gui_flow.rs` drives model and effects against the mock with no
  window.
- **Manual**: `mockhost --bonjour` for a fake Mac on the same machine; a real Sill.app for the
  rest.

## 10. Phases and acceptance criteria

Markers: **mock** = automated against the mock host; **headless** = unit or `egui_kittest`, no
display or GPU; **real Mac** = manual against Sill.app; **manual (mock)** = manual, with the mock.

### Phase 1: MVP (Mac list, pairing, connect, window switcher)

Includes: the Macs screen with live discovery, row words and row menu; Mac details with the remote
address; pairing at home; Add a Mac (address and code, and link); connecting, waiting,
reconnecting and ended states with banners and buttons; the About dialog; the Stream screen (bar
with thumbnails with badges, Desktop, stats, full screen, Disconnect, and a gear panel with the
header, This Computer and Disconnect); Preferences (General, Input, Streaming fps and stats,
Advanced); keyboard modes, chords, pointer and cursor rules; the `transom-gui` binary; `transom`
with no subcommand; screen-reader support; the Linux app id; the runtime window icon.

| # | Criterion | Kind |
|---|---|---|
| 1 | In `gui_protocol.rs`: a mock with thumbnails, icons, apps and settings yields `Thumbnail`, `AppIcon`, `AppList` and `HostSettings` after `Connected`; `ChangeSettings` reaches the mock and is answered with `answering` equal to the token; after goodbye `quit`, `RetryNow` brings the next `Connected` within 1.5 s (backoff alone waits 2 s); `Endpoint::Hosts` dials the second host when the first refuses; goodbye `removed` gives `Finished` with Goodbye reason `removed`; an unpaired key gives `Finished` with `NotPaired`. | mock |
| 2 | In `gui_flow.rs`: Add a Mac with `127.0.0.1:<port>` and the mock's code leaves the Mac in the store, sets its remote address, and reaches Connected; selecting window 102 makes the mock see `Select(Window 102)`; a wrong code shows PAIR9 with "2 tries left". | mock |
| 3 | Views: the Macs screen shows the right word for each of the six row kinds in section 3.3. The pairing card with 12 digits and a bad check digit shows PAIR6 and disables Pair; Cancel emits the cancel message. The bar shows one tile per window labelled "‹App› — ‹Title›", and clicking one emits the select-window message. The Reconnecting card shows "Trying again in 4 s" and [Try Now] emits the retry message. | headless |
| 4 | A pure `keyboard_mode` function is tested for each condition in section 6.1; a test asserts the Mac-to-Transom transition requires releasing everything held. | headless |
| 5 | Run `mockhost --bonjour`, then `transom-gui` on the same machine: "Mock Mac" shows as "Not paired"; pairing with the printed code works and the row becomes "Home"; the bar shows two thumbnails with icon badges and clicking them switches; Disconnect returns to Macs. The mock runs on this computer, so the session is "View only", and window selection works. Pointer and keys need the Preferences > Input switches on. | manual (mock) |
| 6 | Against Sill.app on the LAN: pair from the row with the code Sill shows; quit and reopen Sill and see END2 then reconnection; thumbnails update about every 2-6 s and Desktop works; keys typed with the bar hidden reach the Mac and chord+D does not type "d"; a Remote row over Tailscale connects; removing the device in Sill’s Devices gives banner END6 with [Pair Again]. Also confirm the second Bonjour browse used when connecting at home works while the app-lifetime watch runs, on Linux and Windows. | real Mac |
| 7 | CI: formatting, the clippy and test jobs on all three OSes, and the core `--no-default-features` build pass. Release artifacts include `transom-gui[.exe]`. | CI |

### Phase 2: control the Mac

Includes: the Apps popover (search, open now, all apps, launch); window commands in the thumbnail
menu; Mac Settings sections B and C with the unanswered-pick ledger (timeout the larger of 4 s and
4x the worst round trip); the link-quality toast with [Lower Quality]; the away note; Text Size;
"Keep the bar open"; remembered window size; fallback fonts for CJK (a fixed list of system font
files per OS, loaded at start-up).

| # | Criterion | Kind |
|---|---|---|
| 1 | Ledger unit tests: a 60, 120, 60 double pick never shows 120; a refused field snaps back; a timeout reverts; a nil Direct Wireless is never sent; with no state, nothing is sent. | headless |
| 2 | The mock answers kind 17; the panel shows the answered value; a value outside the choices is refused and toast SET18 shows. | mock |
| 3 | `LaunchApp` and `WindowCommand(close)` reach the mock; close removes the tile. | mock |
| 4 | Apps search filters, "N windows" counts, and Enter launches. | headless |
| 5 | Against Sill.app: Quality changes the Mac’s menu value; Virtual Display toggles; a Japanese window title renders without missing-glyph boxes (on a machine with a CJK font). | real Mac |

### Phase 3: platform polish and packaging

Includes: Linux desktop file and icons; Windows `.ico` resource and zip; macOS `.app`; the
`sill://` URL handler (`transom-gui sill://pair?…` opens Add a Mac with the link and its
confirmation ADD10, and never pairs without that confirmation); the opt-in Windows system-key
hook; `crash.log`. All are manual, on each OS.

- The launcher shows the icon; the dock or taskbar groups the window under Transom. (manual, per
  OS)
- Clicking a `sill://pair` link opens the confirmation. (manual, per OS)
- With the hook on, in full screen, Alt+Tab switches Mac apps. (real Mac, Windows)

### Phase 4: protocol extras

Each feature needs real Sill.app testing. The design gives no acceptance criteria for this phase;
the ones below restate its feature lines and were derived when writing this spec. Detection is by
what the Mac sends, not by one flag.

| Feature | Criterion | Kind |
|---|---|---|
| Clipboard sync (message 32; text only) | A switch in Mac Settings, off by default; shown only when the Mac advertises `clipboard ≥ 1`. | real Mac |
| The Mac’s menus (messages 24, 25, 27) | A "Menus" button in the bar. Transom sends one message 27 without an id after the first window list, and shows the button only once a message 24 top level with menus arrives. An older host skips 27 and never answers, so no button shows. | real Mac |
| The Mac pointer (message 26) | Drawn only when message 26 arrives, i.e. when another device moves it. | real Mac |
| Gestures (message 28) | Menu items Mission Control, App Exposé, Spaces and Show Desktop, shown only when the Mac advertises `gestures ≥ 1`. | real Mac |

## 11. Risks

- **egui 0.37.0 is new.** Pin it exactly; fall back to 0.36.2 (small API differences).
- **Two mDNS browsers.** The app-lifetime watch runs next to the per-attempt browse used by
  "connect at home". Expected to work (the library shares port 5353), but untested on Windows and
  Linux. If not, home connects should dial the watch's latest addresses; reconnect then needs the
  watch, because the home port changes at every Sill launch.
- **Keyboard leaks.** The design depends on never giving keys to the UI in Mac mode, on releasing
  everything at each Mac-to-Transom change, on blocking input-method events too, and on the
  Windows left-Ctrl rule.
- **Cursor.** A mistake shows the Mac cursor over the bar or the arrow over the video.
- **Repaint cost.** One UI pass per video frame. If hidden-bar CPU is above about 3 % of a core,
  skip the UI pass when nothing is visible.
- **Untrusted images.** Open and plain doors are not authenticated. Decode with size and
  allocation limits, lazily and bounded per frame; about 100 icons arrive at connect.
- **Security UX.** Plain doors hide behind a preference (DOOR3); open doors warn (DOOR1); links
  always confirm (ADD10); preference files are 0600; the full fingerprint is in Preferences and
  Mac details for comparison with Sill’s Devices pane.
- **Remote pairing** and remote sessions are untested against a real Mac, as the README says.
- **Screen readers** get no live announcements (section 6.4).
- **Fonts.** CJK titles show boxes until Phase 2's fallback.
- **Compatibility.** `SessionEvent`/`SessionCommand` gain variants and `Ended`/`Finished` a field;
  every exhaustive match in the workspace changes.

The fuller risk list is in `ui-implementation-notes.md`.

## 12. Open questions for the owner

The owner decided questions 1 and 7 on 2026-10-19, taking the recommendations. The rest have a
recommended answer that this spec assumes.

| # | Question | Recommendation | Needs |
|---|---|---|---|
| 1 | **Binary layout.** Two binaries, or one? | Two: `transom` (console CLI; with no arguments it opens the GUI) and `transom-gui` (no console on Windows), the windowless GUI for the desktop entry and Start menu. The alternative is a single Windows-subsystem exe that re-attaches a console, whose output is unreliable in PowerShell and cmd. | **Decided 2026-10-19:** two binaries |
| 7 | **App icon.** | A new Transom icon (a window with a transom bar above it), not Sill’s `design/AppIcon.svg`, to avoid implying endorsement. The icon itself is still to be drawn. | **Decided 2026-10-19:** a new Transom icon |
| 2 | Is egui’s look (flat, dark, not native widgets) acceptable? | Yes. A native look would mean dropping the shared window and renderer, or a second window for the UI. | Confirmation |
| 3 | Thumbnails in the top bar, or a side rail? | The top bar, as Sill’s iPad app does. A "thumbnail rail" of larger thumbnails is a separate, unscheduled idea. | Confirmation |
| 4 | Should the bar reveal when the pointer touches the top edge? | No; only the centre handle and chord+B, because the Mac’s menu bar is at the top. | Confirmation |
| 5 | Show unencrypted (plain) doors? | Hidden behind Preferences > Advanced (off), as Sill’s own release builds do. | Confirmation |
| 6 | Register the `sill://` URL scheme on Linux and Windows (Phase 3)? | Yes, always with confirmation. Sill’s iOS app owns the scheme on iOS only. | Confirmation |
| 8 | Phase 1 scope. | As written: Apps search and launch, window commands and Mac settings in Phase 2. If wanted in the MVP, the Phase 2 work moves ahead of release with no design change. | Confirmation |
| 9 | Default frame rate. | 60, with 120 in Preferences. | Confirmation |

(The numbers are those of the design’s list of open questions.)

## 13. Resolved in revision 2

The 15 gaps found in revision 1 were resolved by the planner (design section 10). Summary of what
changed here:

- "CLI unchanged" now means every subcommand, flag, output and exit code; the no-argument GUI is
  the one intended change (1.3, 8.1).
- `ConnectFailed` text comes from the supervisor, filtered (5.8, 8.2).
- Every user-visible string has an id, renumbered by area (section 5).
- The bar follows Sill’s order, Apps, thumbnails, Desktop (3.6).
- Clicking the handle equals chord+B; the double-click is dropped (3.6, 6.1, 6.3).
- One pair of keymap labels, PREF9 and PREF10 (3.8).
- Outside streaming, Cmd replaces Ctrl on macOS for every Ctrl shortcut (6.1).
- Menus and the Mac pointer are detected by what the Mac sends (Phase 4).
- "Keep the bar open" is only the Phase 2 preference; the side rail is the "thumbnail rail".
- Remote pairing needs no extra key check; the code’s proof binds the key (3.5).
- END3 keeps "8 devices", the remote door’s limit.
- Goal and criterion 3 mark what comes from Phase 2 (1.1, 1.4).
- Phase 1 panel is sections A, D, E (3.6, 3.8).
- "About Transom" opens the About dialog (3.8); "Get Sill" opens https://getsill.app/download.

Nothing is open: the owner decided questions 1 and 7 (section 12) on 2026-10-19.

## Appendix: implementation notes

[`ui-implementation-notes.md`](ui-implementation-notes.md) is the implementation companion to this
spec. It copies, from the planner’s design, the verified library facts and traps, the frozen net
and proto APIs, the dependency pins and frozen GUI types, the risk list, and the round-2
resolutions. Where it conflicts with this spec, its section 10 (resolutions) wins over earlier
sections of the design; report any other difference rather than choosing silently. The design’s
file-by-file work packages stay outside the repository.
