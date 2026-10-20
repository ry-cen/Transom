# Transom graphical app: implementation notes

This is the implementation companion to [`ui-spec.md`](ui-spec.md). It reproduces, verbatim, the
parts of the planner's design that an implementer needs and that the spec leaves out: the verified
library facts and traps, the frozen net and proto APIs, the dependency pins and frozen GUI types,
the risk list, and the round-2 resolutions.

Reading rules:

- Section 10 below (the resolutions) overrides earlier sections where they conflict.
- The text is copied unchanged, so its section references (for example "§4.2.9", "§5 WP-A",
  "`probe/`", "`docs/ui-spec.md` §13") point into the design, which is not stored in the repo.
  The probe files are not in the repo either.
- Copy ids in the design (W1, K3, E6 …) are the old ids. `ui-spec.md` section 5 maps them to the
  new ids (LIST, PAIR, END …).
- The sections are numbered as in the design, so the headings below keep the design's numbers.

---

### 3.4 Verified library facts (probe in `probe/`, run on this Mac, 2026-10-19)

**Versions on crates.io:**

| Crate | Version | Released | Requires |
|---|---|---|---|
| egui, egui-winit, egui-wgpu, egui_kittest | 0.37.0 | 2026-10-15 | wgpu `^30.0`, winit `^0.30.13`, MSRV 1.95 |
| (fallback) all four | 0.36.2 | 2026-09-08 | wgpu `^30.0`, winit `^0.30.13` |

**What the probe showed:**

- With the deps of §5 WP-A step 1 added to a copy of this workspace, `cargo check -p transom
  --all-targets` passes. The lock has exactly one `wgpu 30.0.1`, one `winit 0.30.13`, one
  `raw-window-handle 0.6.2` and one `image 0.25.10`, and no `links` conflict. 130 new crates.
  `cargo check -p transom --no-default-features` still passes.
- `probe/main.rs` composes like this, and runs on macOS arm64. Frames render and the process
  exits cleanly when killed:
  - one winit 0.30 window,
  - one wgpu 30 surface (non-sRGB format),
  - one render pass that clears, then draws egui via `egui_wgpu::Renderer::render(&mut
    pass.forget_lifetime(), …)`,
  - `egui_winit::State` with AccessKit.
- `egui_kittest::Harness::new_ui` / `new_ui_state` with `get_by_label(..).click()` runs headless
  (no GPU, no display) and passes (`probe/kit_test.rs`).
- `probe/` is reference code, not a template. Its `Cargo.toml` pins the same crates as WP-A
  step 1, minus the app's own crates.
- **Not verified:** Windows, X11 and Wayland runs, and Windows cross-compilation (no rustup
  targets on this machine). CI is the first check there.

**API facts the workers must follow (egui 0.37):**

- `egui` has **no default features**: enable `default_fonts` or no text renders.
- Top level is `ctx.run_ui(raw_input, |ui| …)`. Panels are `egui::Panel::top("id").show(ui, …)`
  and `egui::CentralPanel::default().show(ui, …)`. `show_inside` is deprecated.
- `FullOutput.textures_delta` must be **drained**: `textures_delta.set.drain()` before painting,
  with `update_texture(device, queue, id, &delta)` for each delta in the `SmallVec`;
  `textures_delta.free.drain()` after present. Drain them also on a frame whose surface could not
  be acquired. Dropping an undrained delta panics in debug builds.
- `egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default())`.
- `update_buffers(&device, &queue, &mut encoder, &jobs, &screen)` returns command buffers to
  submit **before** the frame's encoder.
- `render(&mut RenderPass<'static>, &jobs, &screen)`. Get the `'static` pass with
  `pass.forget_lifetime()`. egui sets its own viewport and scissor, and picks the right shader for
  a non-sRGB target.
- `egui_winit::State::new(ctx, ViewportId::ROOT, event_loop, Some(scale_factor as f32),
  window.theme(), Some(max_texture_side))`. Then `on_window_event(&window, &event) ->
  EventResponse { consumed, repaint }`, `take_egui_input(&window)` and
  `handle_platform_output_with_event_loop(&window, el, output)`.
- **AccessKit pitfall:** create the window with `.with_visible(false)`, call
  `state.init_accesskit(el, &window, proxy.clone())`, then `window.set_visible(true)`.
  Otherwise accesskit_winit panics "must be created before the window is shown". The user event
  type must implement `From<egui_winit::accesskit_winit::Event>`.
- `ctx.is_pointer_over_egui()`, `ctx.egui_wants_pointer_input()`,
  `ctx.egui_is_using_pointer()` and `ctx.egui_wants_keyboard_input()` exist with these names.
- `out.viewport_output[&ViewportId::ROOT].repaint_delay` is `Duration::MAX` when idle and
  `0` while animating. Use it for `ControlFlow::WaitUntil`. `ctx.set_request_repaint_callback`
  is for repaints requested from other threads.
- **Cursor pitfall:** `handle_platform_output` caches the last cursor icon and calls
  `window.set_cursor` only when egui's icon changes (`egui-winit-0.37.0/src/lib.rs:1396-1411`).
  So:
  - When the shell sets the Mac's custom cursor, egui will not override it.
  - When the pointer leaves the video for Transom UI, the shell must set
    `CursorIcon::Default` itself.
  - Rule: track `mac_cursor_shown: bool`. Set the Mac cursor when (pointer over video and not
    over egui) becomes true. Set `CursorIcon::Default` when it becomes false. Always do this
    **after** `handle_platform_output`.

#### 4.1.5 net and proto additions (frozen API; §5 WP-A writes the signatures, WP-B1 the behaviour)

`transom-proto::host_settings` (new module; JSON keys are the Swift property names):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamSettings {
    #[serde(rename = "maxFPS")] pub max_fps: i64,
    pub bitrate: i64,
    #[serde(rename = "captureScale")] pub capture_scale: f64,
    #[serde(rename = "prioritizeSpeed")] pub prioritize_speed: bool,
    #[serde(rename = "virtualDisplay")] pub virtual_display: bool,
    #[serde(rename = "directWireless", default, skip_serializing_if = "Option::is_none")]
    pub direct_wireless: Option<bool>,
}
pub struct RunningStream { width: i64, height: i64, fps: i64, mbps: i64,
    #[serde(rename = "onVirtualDisplay")] on_virtual_display: bool }
pub struct AwayQuality { homeBitrate: i64, homeCaptureScale: f64, awayBitrate: i64,
    awayCaptureScale: f64, thisConnectionAway: bool, awayRunning: bool }   // snake_case + renames
pub struct LinkReport { state: String, withheldPerSecond: i64, bitrate: i64,
    carriedKbps: Option<i64>, suggestedBitrate: Option<i64>, suggestedCaptureScale: Option<f64> }
pub struct HostSettingsState { settings: StreamSettings, persistent: bool,
    virtualDisplayAvailable: bool, virtualDisplayNote: Option<String>, softwareEncoder: bool,
    stream: Option<RunningStream>, answering: Option<i64>, away: Option<AwayQuality>,
    link: Option<LinkReport> }
pub struct HostSettingsChange { token: Option<i64>, maxFPS: Option<i64>, bitrate: Option<i64>,
    captureScale: Option<f64>, prioritizeSpeed: Option<bool>, virtualDisplay: Option<bool>,
    directWireless: Option<bool> }   // every None omitted
impl HostSettingsChange { pub fn is_empty(&self) -> bool; pub fn applied_to(&self, s: &StreamSettings) -> StreamSettings; }
pub const CHOICES_MAX_FPS: [i64; 2] = [60, 120];
pub const CHOICES_CAPTURE_SCALE: [f64; 2] = [2.0, 1.0];
pub const QUALITY_PRESETS: [(i64, &str); 7] = [(4_000_000,"Low"),(8_000_000,"Efficient"),
    (15_000_000,"Balanced"),(25_000_000,"High"),(40_000_000,"Pro"),(80_000_000,"Ultra"),(150_000_000,"Extreme")];
pub fn mbps_text(bps: i64) -> String;            // "15", or "8.5"
pub fn bitrate_title(bps: i64) -> String;        // "Balanced — 15 Mbps" | "Custom — 12 Mbps"
pub const LINK_BEHIND: &str = "behind";
pub struct SettingsLedger { … }                  // Phase 2 (WP-D1): port of HostSettingsLedger.swift
```

`transom-net::session` (changed):

```rust
pub enum SessionEvent {
    Status(String),
    Connected { mac_name: String, peer: SocketAddr, local_peer: bool },
    WindowList(WindowList),
    AppList(Vec<AppInfo>),
    /// Kind 3, decoded with blobs::decode_thumbnail. JPEG bytes, not yet decoded.
    Thumbnail { id: u32, jpeg: Vec<u8> },
    /// Kind 4, decoded with blobs::decode_icon. PNG bytes, not yet decoded.
    AppIcon { bundle_id: String, png: Vec<u8> },
    /// Kind 16.
    HostSettings(HostSettingsState),
    CursorShape(CursorShape),
    MacInfo { info: MacInfo, fingerprint: [u8; 32] },
    Stats { rtt_ms: Option<f64>, fps: u32 },
    Ended { text: String, retry_in: Option<Duration>, kind: EndKind },
    Finished { text: String, kind: EndKind },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndKind {
    /// Before admission: not found on the network, dial or TLS failed.
    ConnectFailed,
    /// An admitted session dropped without a goodbye.
    Lost,
    /// The Mac's goodbye (reason is the raw string: goodbye_reason::* or anything else).
    Goodbye { reason: String, message: Option<String>, minimum_version: Option<String> },
    NotPaired,
    PinMismatch,
}
pub enum SessionCommand {
    Select(StreamSource), Viewport(Viewport), Input(InputEvent), LaunchApp(String),
    WindowCommand(WindowCommand),
    /// Kind 17. Dropped (not queued) while no session runs.
    ChangeSettings(HostSettingsChange),
    /// Ends a reconnect backoff wait now; ignored at any other time.
    RetryNow,
    Close,
}
```

`transom-net::supervisor`: add `Endpoint::Hosts(Vec<(String, u16)>)`. It resolves each host in
order, concatenates the addresses and dials them with `dial()`, which already tries addresses in
order. `Ended`/`Finished` fill `kind`. `text` stays what the CLI prints today.

`transom-net::discovery` (new, live):

```rust
pub enum BrowseEvent { Found(FoundMac), Removed { fullname: String } }
pub struct Watch { /* ServiceDaemon + reader thread */ }
/// Browses _sill._tcp until stopped; every resolution is a Found (same fullname = update).
pub fn watch(out: crossbeam_channel::Sender<BrowseEvent>, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Watch, NetError>;
impl Watch { pub fn stop(self); }   // Drop also shuts the daemon down
```

`transom-net::store`: add `pub fn read_file(&self, name: &str) -> Result<Option<Vec<u8>>,
NetError>` and `pub fn write_file(&self, name: &str, data: &[u8]) -> Result<(), NetError>`. The
write is atomic with mode 0600, as `write_atomic`. `name` must match `^[a-z0-9][a-z0-9._-]*$`;
any other name returns `NetError::Store("bad file name")`. The GUI keeps `prefs.json` there.

### WP-A, step 1: Workspace deps

1. **Workspace deps.** Add these, exactly:

   ```toml
   image = { version = "0.25.10", default-features = false, features = ["png", "jpeg"] }   # was ["png"]
   egui = { version = "0.37.0", default-features = false, features = ["default_fonts"] }
   egui-winit = { version = "0.37.0", default-features = false, features = ["accesskit", "clipboard", "links", "wayland", "x11"] }
   egui-wgpu = { version = "0.37.0", default-features = false, features = ["winit", "wayland", "x11"] }
   egui_kittest = "0.37.0"
   ```

   - `crates/app/Cargo.toml`: lib and bins and features as §4.1.2. Optional deps `egui`,
     `egui-winit`, `egui-wgpu`, `image`, `serde`, `serde_json` (workspace). Dev-deps
     `egui_kittest`, `transom-net` with `features = ["mock"]`, `transom-video`.
   - `crates/net/Cargo.toml`: dev-dep `image.workspace = true`, for the mockhost example's
     synthetic images.
   - Run `cargo update -p egui` only if needed. Commit the resulting `Cargo.lock` changes.

### WP-A, step 7: `gui/model.rs`

7. **`gui/model.rs`.** Types only, each with `Default` or `new` where it makes sense. Copy
   these exact definitions into the file. B3 implements `update`, and B4 reads the fields.

   ```rust
   pub struct Model { pub screen: Screen, pub saved: Vec<SavedMac>, pub found: Vec<FoundMac>,
       /// Recomputed by `update` whenever saved, found or prefs change (macs::rows, WP-B3).
       pub rows_mine: Vec<MacRow>, pub rows_other: Vec<MacRow>,
       pub prefs: Prefs, pub identity: IdentityInfo, pub banner: Option<Banner>,
       pub dialog: Option<Dialog>, pub status: String, pub started_at: Instant, pub toasts: Vec<Toast> }
   pub enum RowWord { Home, Remote, NotFound, NotPaired, UpdateSill, Unencrypted }
   pub struct MacRow { pub key: RowKey, pub name: String, pub word: RowWord, pub detail: String,
       pub saved: Option<SavedMac>, pub found: Option<FoundMac> }
   pub enum Screen { Macs { card: Option<Card> }, Stream(StreamView) }
   pub enum Card { Pair(PairCard), Add(AddCard), OpenDoor { found: FoundMac } }
   pub struct PairCard { pub mac_name: String, pub addrs: Vec<SocketAddr>, pub state: PairState, pub gen: u64, pub remote_host: Option<String> }
   pub enum PairState { Asking, Code { pin: [u8; 32], shown: bool, problem: Option<String> },
       Working, Paired { mac_id: String }, Failed { text: String } }
   pub struct AddCard { pub link_mode: bool, pub problem: Option<(AddField, String)>, pub pending_link: Option<PairLink>, pub state: PairState, pub gen: u64 }
   pub enum AddField { Address, Code, Link, Card }
   pub struct StreamView { pub mac_id: Option<String>, pub route_desc: RouteDesc, pub session: SessionState,
       pub bar: BarState, pub panel: Option<StreamPanel>, pub toast_n1_shown: bool }
   pub enum RouteDesc { Home { addr: Option<String> }, Remote { host: String, via: Option<String> }, Unpinned, Plain }
   pub struct BarState { pub pinned: bool, pub hover_since: Option<Instant>, pub left_at: Option<Instant>, pub reveal_until: Option<Instant> }
   pub enum StreamPanel { MacSettings, Apps, ThumbMenu { id: u32 } }
   pub enum Dialog { Preferences { tab: PrefsTab }, MacDetails { mac_id: String }, ConfirmForget { mac_id: String, then_pair: bool },
       ConfirmPlain { found: FoundMac }, }
   pub enum PrefsTab { General, Input, Streaming, Advanced }
   pub struct Banner { pub text: String, pub actions: Vec<BannerAction>, pub mac_id: Option<String> }
   pub enum BannerAction { PairAgain, Pair, ConnectAtHome, ForgetAndPair, GetSill }
   pub struct Toast { pub text: String, pub until: Instant, pub action: Option<ToastAction> }
   pub enum ToastAction { LowerQuality }
   pub struct IdentityInfo { pub fingerprint_b64: String, pub fingerprint_hex: String, pub config_dir: String }
   pub enum Msg {
       Tick, Browse(BrowseEvent), Session(SessionEvent), FrameShown,
       ClickRow { key: RowKey }, RowMenu { key: RowKey, item: RowMenuItem },
       OpenAdd, CancelCard, SubmitCode(String), SubmitAddress { address: String, code: String },
       SubmitLink(String), ConfirmLink, ToggleLinkMode,
       PairAsked(u64, Result<(PairResult, [u8; 32]), String>), PairDone(u64, Result<Paired, PairFailure>),
       Connect(ConnectPlan), Disconnect, RetryNow, SelectDesktop, SelectWindow(u32), NextWindow,
       ToggleBar, BarHover(bool), OpenPanel(StreamPanel), ClosePanel, ToggleFullscreen,
       OpenDialog(Dialog), CloseDialog, ConfirmDialog, BannerAction(BannerAction), DismissBanner,
       SetPrefs(Prefs), SetRemoteHost { mac_id: String, host: Option<String> }, Copy(String),
       ToastAction(ToastAction),
       /// The store's Macs, re-read by the effects runner after every SaveMac or Forget, and
       /// after every SessionEvent::MacInfo (the supervisor rewrites macs.json then).
       SavedMacs(Vec<SavedMac>),
   }
   pub enum RowKey { Saved(String /* mac_id */), Found(String /* fullname */) }
   pub enum RowMenuItem { Connect, ConnectHome, ConnectRemote, Details, Forget }
   pub enum PairFailure { Refused { reason: String, tries_left: Option<i64>, retry_after: Option<f64>, message: Option<String> }, Net(String), BadProof }
   pub enum Effect {
       Ask { gen: u64, addrs: Vec<SocketAddr> }, PairCode { gen: u64, addrs: Vec<SocketAddr>, pin: Option<[u8; 32]>, code: String, remote: bool },
       /// The Add card: the runner resolves host:port (dial::resolve) itself; the model stays pure.
       PairCodeAt { gen: u64, host: String, port: u16, code: String },
       PairLink { gen: u64, link: PairLink }, CancelAsk { addrs: Vec<SocketAddr>, pin: [u8; 32] },
       SaveMac(SavedMac), Forget(String), SavePrefs(Prefs),
       StartStream { route: Route, mac_name: String }, StopStream, Session(SessionCommand),
       SetFullscreen(bool), ReleaseAll, CopyText(String), OpenUrl(String),
   }
   pub enum ConnectPlan { Home(SavedMac), Remote { mac: SavedMac, hosts: Vec<(String, u16)> }, Unpinned(FoundMac), Plain(FoundMac) }
   // Prefs lives in gui/prefs.rs (B3) but its struct is declared here so B4 compiles:
   #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
   pub struct Prefs { pub version: i64, pub device_name: Option<String>, pub keymap: Option<String> /* "pc"|"mac" */,
       pub allow_local_pointer: bool, pub allow_local_keyboard: bool, pub fps: i64, pub show_stats: bool,
       pub allow_plain: bool, pub hello_version: Option<String>, pub log_file: bool, pub log_debug: bool,
       pub ui_zoom: f32, pub macs: BTreeMap<String, MacPrefs> }
   #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
   pub struct MacPrefs { pub remote_host: Option<String> }
   impl Model { pub fn update(&mut self, msg: Msg, now: Instant) -> Vec<Effect> { Vec::new() } } // B3 fills in
   ```

   Every field of `Prefs` has `#[serde(default)]` and a `Default`: version 1, fps 60,
   show_stats true, ui_zoom 1.0.

   **Derives.** Every type above derives `Clone, Debug`. `Prefs`, `MacPrefs`, `RowWord`,
   `RowKey`, `RowMenuItem`, `PrefsTab`, `BannerAction`, `ToastAction` and `AddField` also derive
   `PartialEq` (`Eq` where possible). Types that hold net types (`Route`, `SessionEvent`,
   `SessionCommand`, `FoundMac`, `SavedMac` …) do **not** derive `PartialEq`, because those net
   types lack it. Tests compare them with `matches!`. Never add derives to net or proto types
   from a GUI package.

### WP-A, step 8: `gui/view/mod.rs`

8. **`gui/view/mod.rs`.**

   ```rust
   pub struct Drafts { pub code: String, pub address: String, pub link: String, pub search: String, pub remote_host: String, pub device_name: String, pub hello_version: String }
   pub struct Image { pub id: egui::TextureId, pub size: egui::Vec2 }
   pub trait ImageLookup { fn thumbnail(&self, window_id: u32) -> Option<Image>; fn icon(&self, bundle_id: &str) -> Option<Image>; }
   pub fn draw(ui: &mut egui::Ui, model: &Model, drafts: &mut Drafts, images: &dyn ImageLookup, now: Instant, out: &mut Vec<Msg>) {}
   ```

## 8. Risks and review notes

- **egui 0.37.0 is 4 days old.** Both 0.37.0 and 0.36.2 require wgpu ^30 and winit ^0.30.13,
  so 0.36.2 is the fallback. API differences between them are small but real (`run_ui`,
  `Panel`). Pin exactly, and do not mix them.
- **Two mDNS daemons.** The GUI's `watch` runs for the app's lifetime, and `Route::SavedHome`
  starts a second `ServiceDaemon` per connect attempt (supervisor.rs:114-131). mdns-sd binds
  5353 with reuse, so this is expected to work, but it is untested on Windows and Linux. If it
  fails, change `SavedHome` dialing to use the watch's latest addresses through a
  pinned `Route::Direct` with `Endpoint::Addrs`. Reconnect then needs the watch, because the home
  port changes at every Sill launch. Check this in Phase 1 acceptance 6 on both OSes.
- **Keyboard leaks.**
  - The design depends on never feeding key events to egui in Mac mode, and on `release_all` at
    every Mac→Transom transition. Review that `Ime` events are blocked too.
  - Review that the chord's modifiers (already sent to the Mac) are released when a panel
    opens.
  - Windows AltGr: the left-Ctrl rule.
- **Cursor.** egui-winit's cursor cache (§3.4). A mistake shows the Mac cursor over the bar, or
  the arrow over the video. The manual check covers both.
- **Repaint cost.** One egui pass per video frame (60-120 per second). Measure CPU with the bar
  hidden. If it is above about 3 % of a core, skip the egui pass when nothing is visible and no
  repaint was requested, and draw video only.
- **Untrusted images.**
  - Thumbnails, icons and cursor PNGs come from the Mac, which is trusted after pairing, but an
    open or plain door is not. Decode with `image::Limits` (§5 WP-C step 4), lazily and
    bounded per frame.
  - About 100 icons arrive at connect. Store bytes, and decode on demand.
- **Security UX.**
  - Plain doors are hidden behind a preference, with confirmation P1.
  - Open doors carry warning O1.
  - Links always confirm (L1), and the Phase 3 URL handler too.
  - Prefs files are 0600.
  - The fingerprint is shown in full in Preferences, so it can be compared with Sill's Devices
    pane.
  - Never log codes or link secrets. Review `log::` calls in `pairflow`.
- **Remote pairing** is untested against a real Mac. The CLI's remote paths are untested too
  (README). Phase 1 acceptance 6 includes a Tailscale pairing.
- **Screen readers.** egui has no live regions, so state changes rely on moving focus to the
  card. Verify with Orca or NVDA once.
- **Fonts.** Default egui fonts lack CJK, so Japanese or Chinese window titles show boxes until
  Phase 2's fallback.
- **Compatibility.** `SessionEvent` and `SessionCommand` gain variants, and `Ended`/`Finished`
  gain a field. Every exhaustive match in the workspace changes in WP-A (viewer.rs, the tests in
  `crates/net/tests/mock_e2e.rs`). Review that `mock_e2e.rs` still asserts the same behaviour.
- **Orchestrator double-checks:**
  - `Cargo.lock` has exactly one wgpu and one winit after WP-A.
  - The `--no-default-features` build still has no egui.
  - `transom-gui.exe` has no console window (CI artifact, run once on Windows).
  - CLI `transom pair` and `transom connect` behave as before (B2 refactor).

## 10. Resolutions (round 2)

Answers to the gaps in `docs/ui-spec.md` §13. Where an answer conflicts with an earlier section,
this section wins.

1. **No-argument `transom`.** "CLI unchanged" means every subcommand, flag, output and exit code
   stays the same. The one intended change: in a `gui` build, `transom` with no subcommand opens
   the GUI instead of failing with a usage error. On Windows, that window keeps its console and
   logs there. Without `gui`, it prints help and exits 2.
2. **ConnectFailed text.** `EndKind` stays payload-free. The signature becomes
   `texts::end_text(kind: &EndKind, mac: &str, supervisor_text: &str)` (WP-A step 9). Only
   `ConnectFailed` uses `supervisor_text`, passed through `safetext::label(_, 300)`. The
   supervisor's connect errors contain no commands.
3. **Strings in prose.** Yes, every one gets an id. Every user-visible string, including button
   labels, headers, placeholders and accessible labels, lives in `gui/texts.rs` under an id from
   the copy list. The spec's §5.4 strings join that list with their prose wording unchanged. A
   string with no id is a review failure.
4. **Bar order.** Follow Sill: [Apps] [thumbnail strip] [Desktop] | stats | [Settings]
   [Full screen] [Disconnect]. There is no reason to differ. In Phase 1, Apps is absent and the
   strip starts at the left edge.
5. **Handle and chord+B.** Drop the double-click on the handle; full screen is only the bar
   button and chord+F. Clicking the handle does the same as chord+B:
   - when the bar is hidden or shown by hover, pin it open;
   - when it is pinned, unpin it and hide it at once.
6. **Keymap labels.** Use one pair of labels everywhere, the Settings panel and Preferences ›
   Input alike: "PC layout: Ctrl acts as Command" and "Mac layout: Command is Command, Ctrl is
   Control".
7. **Copy ids.** Rename by area, with a prefix and a number in order of first appearance in the
   spec. Prefixes:
   - `LIST` (Macs screen; was W1, S1, H1; also row words and row menu),
   - `DET` (Mac details; was H2),
   - `PAIR` (was K0–K4, T0, T1, R1–R9),
   - `ADD` (was A0, A2, A3, L1, L2),
   - `DOOR` (was O1, P1, P2, U1),
   - `CONN` (was C1–C3),
   - `END` (was E1–E11, plus the banner buttons),
   - `BAR` (was N1, V1, plus the stats tooltip and the accessible labels),
   - `APPS` (was A1, plus the search, sections and counts),
   - `SET` (was S2–S10, plus the header lines and the streaming lines),
   - `PREF` (was G1, V2, plus the labels),
   - `DLG` (was F1, F2),
   - `BTN` (shared buttons: Cancel, Close, Done, Try Again, Try Now, Disconnect, Copy).

   The spec keeps an "old id" column for one revision. `texts.rs` uses descriptive function
   names, with the id in each doc comment.
8. **macOS modifier.** Ctrl+N becomes Cmd+N on macOS. The rule: outside streaming, every Ctrl
   shortcut (Ctrl+N, Ctrl+, and Ctrl+= / Ctrl+−) uses Cmd on macOS. The streaming chord stays
   Control+Option+Shift.
9. **Menus and Mac pointer have no flag.**
   - Menus: send one kind 27 without an id after the first window list. Show the Menus button
     only once a kind 24 top level with menus arrives (`MacMenu.swift` header). An older host
     skips kind 27 and never answers.
   - Mac pointer: draw it only when kind 26 arrives.
10. **"Keep the bar open".** It is only the Phase 2 preference: the bar starts pinned at every
    connect. Q3's side rail is a separate, unscheduled idea called "thumbnail rail". Strike
    "variant B".
11. **Remote pairing key check.** No extra step. The code's PBKDF2 key is bound to the Mac's
    TLS fingerprint, and `pair_with_code` verifies `proof_M` over both fingerprints, so a host
    that doesn't know the code (or a man in the middle) fails with R7 (`BadMacProof`). The key
    is then pinned. Mac details shows the full fingerprint for anyone who wants to compare it.
12. **"8 devices".** Keep it. `busy` comes only from the remote door at `maxRemoteSessions = 8`
    (`DoorPolicy.swift:16-17, 110`), and Sill's own client says the same
    (`GoodbyePolicy.swift:67`). The host doesn't send the limit. Cite both in the doc comment.
13. **Phase scope in the goal.** §1 and §2 describe the whole design, all phases. Reword
    criterion 3 so its Apps and Mac Settings items are marked "from Phase 2". In the Goal, add
    "by the end of Phase 2" after "change the app's and the Mac's settings".
14. **Phase 1 panel.** §4.2.8 is authoritative: sections A, D and E. Reword the bar table to
    "Settings (gear): in Phase 1 the header, This Computer and Disconnect (sections A, D, E)".
15. **About Transom.** A small modal with:
    - "Transom ‹version›" (`CARGO_PKG_VERSION`),
    - "An independent client for Sill. Not part of Sill and not endorsed by its author.",
    - "Apache License 2.0" and the Sill credit line from NOTICE,
    - links to github.com/ry-cen/sill_port and getsill.app,
    - [Close].

    "Get Sill" opens `https://getsill.app/download`.

**Owner decisions, still open:**
- **Binary layout.** Recommended: two binaries, `transom` (console CLI; no arguments opens the
  GUI) and `transom-gui` (no console on Windows).
- **App icon.** Recommended: a new Transom icon, not Sill's. This needs the owner's approval of
  a design, or permission to commission one.

**Implementation notes in the repo.** Yes. Add `docs/ui-implementation-notes.md`, copied
verbatim from this file's §3.4 (verified facts and traps), §4.1.5 (frozen net and proto APIs),
§5 WP-A steps 1, 7 and 8 (dependency pins and frozen GUI types), §8 (risks) and this section.
Point the spec's appendix at it instead of `~/.claude/briefs`. The technical writer does the
copy, and the planner's brief stays outside the repo. This file remains the working source until
wave A lands; after that the repo copy is authoritative.
