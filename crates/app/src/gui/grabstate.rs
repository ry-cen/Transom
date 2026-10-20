//! Pure decisions for the Linux full-screen key capture (docs/ui-spec.md section 6.1): when to
//! (re)try the X11 grab, when the Wayland inhibitor must exist, and the names of X grab
//! statuses. It is compiled everywhere so the tests run everywhere.

use std::time::{Duration, Instant};

/// How long to wait before trying a failed X11 grab again.
pub const GRAB_RETRY: Duration = Duration::from_millis(250);

/// What the X11 grab must do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabStep {
    Grab,
    Ungrab,
    Wait,
}

/// The X11 grab's bookkeeping: what is wanted, what is held, and when to retry.
#[derive(Debug, Default)]
pub struct GrabState {
    wanted: bool,
    held: bool,
    retry_at: Option<Instant>,
    failures: u32,
}

impl GrabState {
    pub fn new() -> Self {
        Self::default()
    }

    /// What to do now; `on` is whether the keyboard should be grabbed.
    pub fn want(&mut self, on: bool, now: Instant) -> GrabStep {
        self.wanted = on;
        if on {
            if self.held || self.retry_at.is_some_and(|t| now < t) {
                GrabStep::Wait
            } else {
                GrabStep::Grab
            }
        } else {
            self.retry_at = None;
            self.failures = 0;
            if std::mem::take(&mut self.held) {
                GrabStep::Ungrab
            } else {
                GrabStep::Wait
            }
        }
    }

    /// `want` with the last wish, for timer wake-ups.
    pub fn again(&mut self, now: Instant) -> GrabStep {
        self.want(self.wanted, now)
    }

    /// The result of the `Grab` just attempted.
    pub fn grabbed(&mut self, ok: bool, now: Instant) {
        self.held = ok;
        if ok {
            self.retry_at = None;
            self.failures = 0;
        } else {
            self.retry_at = Some(now + GRAB_RETRY);
            self.failures = self.failures.saturating_add(1);
        }
    }

    pub fn held(&self) -> bool {
        self.held
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// When `again` should run: only while a grab is wanted, not held, and a retry is pending.
    pub fn retry_at(&self) -> Option<Instant> {
        if self.wanted && !self.held {
            self.retry_at
        } else {
            None
        }
    }
}

/// Whether the Wayland inhibitor must exist. It is created only when capture turns on, but a
/// focus loss keeps it: GNOME's prompt can take the keyboard focus, and destroying the inhibitor
/// then would cancel the prompt and loop. An unfocused inhibitor is inert.
pub fn keep_inhibitor(capturing: bool, inhibiting: bool, focused: bool) -> bool {
    capturing || (inhibiting && !focused)
}

/// The name of an `XGrabKeyboard` status (fixed X protocol values).
pub fn x_grab_status(code: i32) -> &'static str {
    match code {
        0 => "GrabSuccess",
        1 => "AlreadyGrabbed (another program holds the keyboard)",
        2 => "GrabInvalidTime",
        3 => "GrabNotViewable (the window is not shown)",
        4 => "GrabFrozen",
        _ => "unknown status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn grabs_once_while_wanted() {
        let t0 = Instant::now();
        let mut g = GrabState::new();
        assert_eq!(g.want(true, t0), GrabStep::Grab);
        g.grabbed(true, t0);
        assert!(g.held());
        assert_eq!(g.want(true, t0 + ms(1)), GrabStep::Wait);
        assert_eq!(g.retry_at(), None);
    }

    #[test]
    fn failed_grab_waits_then_retries() {
        let t0 = Instant::now();
        let mut g = GrabState::new();
        assert_eq!(g.want(true, t0), GrabStep::Grab);
        g.grabbed(false, t0);
        assert_eq!(g.failures(), 1);
        assert_eq!(g.retry_at(), Some(t0 + GRAB_RETRY));
        assert_eq!(g.want(true, t0 + ms(100)), GrabStep::Wait);
        assert_eq!(g.again(t0 + ms(100)), GrabStep::Wait);
        assert_eq!(g.again(t0 + GRAB_RETRY), GrabStep::Grab);
        g.grabbed(false, t0 + GRAB_RETRY);
        assert_eq!(g.failures(), 2);
        assert_eq!(g.again(t0 + 2 * GRAB_RETRY), GrabStep::Grab);
        g.grabbed(true, t0 + 2 * GRAB_RETRY);
        assert_eq!(g.failures(), 0);
        assert_eq!(g.retry_at(), None);
        assert!(g.held());
    }

    #[test]
    fn ungrabs_only_what_it_holds() {
        let t0 = Instant::now();
        let mut g = GrabState::new();
        assert_eq!(g.want(false, t0), GrabStep::Wait);
        assert_eq!(g.want(true, t0), GrabStep::Grab);
        g.grabbed(true, t0);
        assert_eq!(g.want(false, t0), GrabStep::Ungrab);
        assert!(!g.held());
        assert_eq!(g.want(false, t0), GrabStep::Wait);
    }

    #[test]
    fn stopping_forgets_a_pending_retry() {
        let t0 = Instant::now();
        let mut g = GrabState::new();
        assert_eq!(g.want(true, t0), GrabStep::Grab);
        g.grabbed(false, t0);
        assert_eq!(g.want(false, t0 + ms(1)), GrabStep::Wait);
        assert_eq!(g.retry_at(), None);
        assert_eq!(g.failures(), 0);
        assert_eq!(g.want(true, t0 + ms(2)), GrabStep::Grab);
    }

    #[test]
    fn retry_at_only_while_wanted_and_not_held() {
        let t0 = Instant::now();
        let mut g = GrabState::new();
        assert_eq!(g.retry_at(), None);
        assert_eq!(g.want(true, t0), GrabStep::Grab);
        g.grabbed(true, t0);
        assert_eq!(g.retry_at(), None);
        let mut f = GrabState::new();
        assert_eq!(f.want(true, t0), GrabStep::Grab);
        f.grabbed(false, t0);
        assert!(f.retry_at().is_some());
        f.want(false, t0);
        assert_eq!(f.retry_at(), None);
    }

    #[test]
    fn inhibitor_kept_while_unfocused_only_if_it_exists() {
        for capturing in [false, true] {
            for inhibiting in [false, true] {
                for focused in [false, true] {
                    assert_eq!(
                        keep_inhibitor(capturing, inhibiting, focused),
                        capturing || (inhibiting && !focused),
                        "{capturing} {inhibiting} {focused}"
                    );
                }
            }
        }
    }

    #[test]
    fn grab_status_names() {
        assert_eq!(x_grab_status(0), "GrabSuccess");
        assert_eq!(
            x_grab_status(1),
            "AlreadyGrabbed (another program holds the keyboard)"
        );
        assert_eq!(x_grab_status(2), "GrabInvalidTime");
        assert_eq!(
            x_grab_status(3),
            "GrabNotViewable (the window is not shown)"
        );
        assert_eq!(x_grab_status(4), "GrabFrozen");
        assert_eq!(x_grab_status(-1), "unknown status");
        assert_eq!(x_grab_status(5), "unknown status");
    }
}
