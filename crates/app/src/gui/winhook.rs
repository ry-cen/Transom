//! Windows only (declared `#[cfg(windows)]` in gui/mod.rs): the low-level keyboard hook
//! (`WH_KEYBOARD_LL`) that takes the OS-reserved keys from Windows while the stream is full
//! screen and the Mac has the keyboard (docs/ui-spec.md section 6.1). The decisions are in
//! `capture.rs`; this file is the Win32 glue.
//!
//! The hook lives on its own thread, which owns all of the hook's state (a thread-local) and
//! runs a message loop: Windows calls a low-level hook on the thread that installed it, from
//! that thread's `GetMessageW`, and silently removes a hook that is slow to answer. A thread of
//! its own keeps the callback fast whatever the UI thread is doing. The shell talks to the
//! thread with thread messages only; swallowed keys go back through `Sink` (the event loop
//! proxy). The hook is installed on activation and removed once capture is off and every
//! swallowed key has been released, so a key's release never reaches Windows without its press.

use crate::gui::capture::{Capture, RawKey, SysKey, Verdict, VK_ESCAPE};
use std::cell::RefCell;
use std::sync::mpsc;
use std::thread::JoinHandle;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN,
    LLKHF_UP, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_APP, WM_QUIT, WM_USER,
};

/// Start capturing; `wparam` is Transom's HWND.
const CMD_ACTIVATE: u32 = WM_APP + 1;
/// Stop capturing; the hook goes once every swallowed key is released.
const CMD_DEACTIVATE: u32 = WM_APP + 2;
/// Posted by the callback to its own thread: the last swallowed key was released.
const CMD_IDLE: u32 = WM_APP + 3;

/// Where swallowed keys go (the shell sends them to itself as `UserEvent::SysKey`). Called on the
/// hook thread, inside the hook callback: it must not block.
pub type Sink = Box<dyn Fn(SysKey) + Send + 'static>;

/// The hook thread's state. Touched only on the hook thread.
struct HookState {
    capture: Capture,
    /// 0 when no hook is installed.
    hook: HHOOK,
    /// Transom's window; 0 before the first activation.
    hwnd: HWND,
    sink: Sink,
    idle_posted: bool,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// The HWND of `window`, or None when it is not a Win32 window.
pub fn hwnd_of(window: &winit::window::Window) -> Option<HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// The shell's handle on the hook thread. The thread starts at the first `activate`.
pub struct Hook {
    sink: Option<Sink>,
    /// The hook thread's id and join handle, while it runs.
    thread: Option<(u32, JoinHandle<()>)>,
    /// The thread could not be started: do not try again.
    failed: bool,
}

impl Hook {
    pub fn new(sink: Sink) -> Hook {
        Hook {
            sink: Some(sink),
            thread: None,
            failed: false,
        }
    }

    /// Capture the reserved keys for `hwnd` from now on.
    pub fn activate(&mut self, hwnd: HWND) {
        if self.thread.is_none() && !self.failed {
            self.spawn();
        }
        if let Some((tid, _)) = &self.thread {
            post(*tid, CMD_ACTIVATE, hwnd as WPARAM);
        }
    }

    /// Stop capturing. Keys already swallowed keep being swallowed until released.
    pub fn deactivate(&mut self) {
        if let Some((tid, _)) = &self.thread {
            post(*tid, CMD_DEACTIVATE, 0);
        }
    }

    /// Removes the hook and ends the thread. Safe to call more than once.
    pub fn stop(&mut self) {
        if let Some((tid, join)) = self.thread.take() {
            if post(tid, WM_QUIT, 0) {
                let _ = join.join();
            }
        }
    }

    fn spawn(&mut self) {
        let Some(sink) = self.sink.take() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("transom-keyhook".into())
            .spawn(move || thread_main(sink, tx));
        match spawned {
            Ok(join) => match rx.recv() {
                Ok(tid) => self.thread = Some((tid, join)),
                Err(_) => {
                    log::warn!("the keyboard hook thread ended at start");
                    self.failed = true;
                    let _ = join.join();
                }
            },
            Err(e) => {
                log::warn!("could not start the keyboard hook thread: {e}");
                self.failed = true;
            }
        }
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        self.stop();
    }
}

fn post(tid: u32, msg: u32, wparam: WPARAM) -> bool {
    // SAFETY: plain FFI call with no pointers; a failure only means the thread has gone.
    let ok = unsafe { PostThreadMessageW(tid, msg, wparam, 0) };
    if ok == 0 {
        log::debug!("keyboard hook thread message {msg:#x} not posted");
    }
    ok != 0
}

fn thread_main(sink: Sink, ready: mpsc::Sender<u32>) {
    // SAFETY: MSG is plain data; all zeroes is a valid value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid out-pointer. This creates the thread's message queue before the
    // shell posts to it.
    unsafe { PeekMessageW(&mut msg, 0, WM_USER, WM_USER, PM_NOREMOVE) };
    STATE.with(|s| {
        *s.borrow_mut() = Some(HookState {
            capture: Capture::new(),
            hook: 0,
            hwnd: 0,
            sink,
            idle_posted: false,
        });
    });
    // SAFETY: plain FFI call.
    let _ = ready.send(unsafe { GetCurrentThreadId() });
    loop {
        // SAFETY: `msg` is a valid out-pointer. Hook callbacks run inside this call.
        let r = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
        if r == 0 || r == -1 {
            break; // WM_QUIT, or an error
        }
        if msg.hwnd != 0 {
            continue;
        }
        let (message, wparam) = (msg.message, msg.wParam);
        STATE.with(|s| {
            if let Ok(mut guard) = s.try_borrow_mut() {
                if let Some(st) = guard.as_mut() {
                    command(st, message, wparam);
                }
            }
        });
    }
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().take() {
            if st.hook != 0 {
                // SAFETY: `st.hook` was returned by SetWindowsHookExW on this thread.
                unsafe { UnhookWindowsHookEx(st.hook) };
            }
        }
    });
}

fn command(st: &mut HookState, message: u32, wparam: WPARAM) {
    match message {
        CMD_ACTIVATE => {
            st.hwnd = wparam as HWND;
            if st.hook == 0 {
                st.capture.reset();
                // SAFETY: `hook_proc` has the HOOKPROC signature and lives for the whole
                // program; a null module name gives this executable's handle.
                st.hook = unsafe {
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(hook_proc),
                        GetModuleHandleW(std::ptr::null()),
                        0,
                    )
                };
                if st.hook == 0 {
                    log::warn!(
                        "could not install the keyboard hook: {}",
                        std::io::Error::last_os_error()
                    );
                    return;
                }
            }
            // Keys Windows already has down keep going to Windows.
            // SAFETY: plain FFI call; a negative value means the key is down.
            st.capture
                .activate(|vk| unsafe { GetAsyncKeyState(vk as i32) } < 0);
        }
        CMD_DEACTIVATE => {
            st.capture.deactivate();
            unhook_if_idle(st);
        }
        CMD_IDLE => {
            st.idle_posted = false;
            unhook_if_idle(st);
        }
        _ => {}
    }
}

fn unhook_if_idle(st: &mut HookState) {
    if st.hook != 0 && st.capture.idle() {
        // SAFETY: `st.hook` was returned by SetWindowsHookExW on this thread.
        unsafe { UnhookWindowsHookEx(st.hook) };
        st.hook = 0;
        st.capture.reset();
    }
}

/// The hook callback. Must stay fast: no logging, no blocking.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for WH_KEYBOARD_LL and HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT that is
        // valid for the duration of this call.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if swallow(info) {
            return 1;
        }
    }
    // SAFETY: passes the event on unchanged, as the hook contract requires.
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

/// True when the event must not reach Windows.
fn swallow(info: &KBDLLHOOKSTRUCT) -> bool {
    STATE.with(|s| {
        let Ok(mut guard) = s.try_borrow_mut() else {
            return false;
        };
        let Some(st) = guard.as_mut() else {
            return false;
        };
        // Injected events (LLKHF_INJECTED: AutoHotkey, PowerToys remaps, on-screen keyboards) are
        // captured on purpose.
        let key = RawKey {
            vk: info.vkCode,
            up: info.flags & LLKHF_UP != 0,
            alt: info.flags & LLKHF_ALTDOWN != 0,
            // SAFETY: plain FFI call; a negative value means the key is down.
            ctrl: info.vkCode == VK_ESCAPE
                && unsafe { GetAsyncKeyState(i32::from(VK_CONTROL)) } < 0,
            time: info.time,
        };
        // SAFETY: plain FFI call.
        let foreground = st.hwnd != 0 && unsafe { GetForegroundWindow() } == st.hwnd;
        match st.capture.on_key(key, foreground) {
            Verdict::Pass => false,
            Verdict::Swallow(k) => {
                (st.sink)(k);
                if st.hook != 0 && st.capture.idle() && !st.idle_posted {
                    st.idle_posted = true;
                    // SAFETY: plain FFI calls; posts to this thread's own queue.
                    unsafe { PostThreadMessageW(GetCurrentThreadId(), CMD_IDLE, 0, 0) };
                }
                true
            }
        }
    })
}
