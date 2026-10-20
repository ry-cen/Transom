//! Window positions, buttons and wheel deltas to Sill pointer and scroll events.

use transom_proto::geometry::{is_inside, Rect};
use transom_proto::messages::{InputEvent, PointerAction, ScrollPhase};
use winit::event::{MouseButton, MouseScrollDelta, TouchPhase};

/// Pixels per wheel line, before the window's scale factor.
const LINE_PIXELS: f64 = 40.0;

/// Maps physical-pixel window events to fractions of the video frame (`Input.swift:6-38`).
pub struct PointerMapper {
    rect: Option<Rect>,
    cursor: Option<(f64, f64)>,
    left: bool,
    right: bool,
    scale_factor: f64,
    gesture: bool,
    /// Last clamped fraction sent; used for ups when the cursor or layout is gone.
    last: (f64, f64),
}

impl Default for PointerMapper {
    fn default() -> Self {
        Self::new()
    }
}

fn clamp(f: (f64, f64)) -> (f64, f64) {
    (f.0.clamp(0.0, 1.0), f.1.clamp(0.0, 1.0))
}

impl PointerMapper {
    pub fn new() -> Self {
        PointerMapper {
            rect: None,
            cursor: None,
            left: false,
            right: false,
            scale_factor: 1.0,
            gesture: false,
            last: (0.0, 0.0),
        }
    }

    /// Where the video sits in the window now (None before the first frame).
    pub fn set_layout(&mut self, rect: Option<Rect>, scale_factor: f64) {
        self.rect = rect;
        self.scale_factor = scale_factor;
    }

    fn raw(&self) -> Option<(f64, f64)> {
        let r = self.rect?;
        let (x, y) = self.cursor?;
        Some(r.fraction(x, y))
    }

    pub fn cursor_inside(&self) -> bool {
        self.raw().is_some_and(is_inside)
    }

    /// A button went down on the Mac and has not come up yet.
    pub fn buttons_held(&self) -> bool {
        self.left || self.right
    }

    /// CursorMoved, physical px. Some(Pointer Move) when inside, or when a button is held
    /// (fraction clamped to 0..=1).
    pub fn moved(&mut self, x: f64, y: f64) -> Option<InputEvent> {
        self.cursor = Some((x, y));
        let raw = self.raw()?;
        if is_inside(raw) || self.left || self.right {
            let (fx, fy) = clamp(raw);
            self.last = (fx, fy);
            Some(InputEvent::Pointer {
                action: PointerAction::Move,
                x: fx,
                y: fy,
            })
        } else {
            None
        }
    }

    /// The cursor left the window.
    pub fn left_window(&mut self) {
        self.cursor = None;
    }

    /// Left/Right only (others None). Down: only when inside (sets the flag). Up: only when that
    /// button is down, at the clamped last position.
    pub fn button(&mut self, button: MouseButton, pressed: bool) -> Option<InputEvent> {
        let (down_action, up_action) = match button {
            MouseButton::Left => (PointerAction::LeftDown, PointerAction::LeftUp),
            MouseButton::Right => (PointerAction::RightDown, PointerAction::RightUp),
            _ => return None,
        };
        let flag = if button == MouseButton::Left {
            &mut self.left
        } else {
            &mut self.right
        };
        if pressed {
            let raw = self.raw()?;
            if !is_inside(raw) {
                return None;
            }
            let (x, y) = clamp(raw);
            if button == MouseButton::Left {
                self.left = true;
            } else {
                self.right = true;
            }
            self.last = (x, y);
            Some(InputEvent::Pointer {
                action: down_action,
                x,
                y,
            })
        } else if *flag {
            *flag = false;
            let (x, y) = self.position();
            Some(InputEvent::Pointer {
                action: up_action,
                x,
                y,
            })
        } else {
            None
        }
    }

    /// The clamped current position, or the last one sent when the cursor or layout is gone.
    fn position(&mut self) -> (f64, f64) {
        if let Some(raw) = self.raw() {
            self.last = clamp(raw);
        }
        self.last
    }

    /// Wheel or trackpad scroll. Needs a layout and a cursor; outside an open gesture the cursor
    /// must also be inside the frame. Signs pass through unchanged (`Input.swift:17-19`).
    pub fn scroll(&mut self, delta: MouseScrollDelta, phase: TouchPhase) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let (Some(rect), Some(raw)) = (self.rect, self.raw()) else {
            return out;
        };
        let inside = is_inside(raw);
        let (x, y) = clamp(raw);
        self.last = (x, y);
        match delta {
            MouseScrollDelta::LineDelta(lx, ly) => {
                if !inside && !self.gesture {
                    return out;
                }
                let k = LINE_PIXELS * self.scale_factor;
                let dx = f64::from(lx) * k / rect.width;
                let dy = f64::from(ly) * k / rect.height;
                out.push(InputEvent::Scroll { x, y, dx, dy });
            }
            MouseScrollDelta::PixelDelta(p) => {
                let dx = p.x / rect.width;
                let dy = p.y / rect.height;
                let nonzero = dx != 0.0 || dy != 0.0;
                let scroll = InputEvent::Scroll { x, y, dx, dy };
                match phase {
                    TouchPhase::Started => {
                        if inside {
                            out.push(InputEvent::ScrollGesture {
                                phase: ScrollPhase::Began,
                                x,
                                y,
                            });
                            self.gesture = true;
                            if nonzero {
                                out.push(scroll);
                            }
                        }
                    }
                    TouchPhase::Moved => {
                        if nonzero && (inside || self.gesture) {
                            out.push(scroll);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if nonzero && (inside || self.gesture) {
                            out.push(scroll);
                        }
                        if self.gesture {
                            out.push(InputEvent::ScrollGesture {
                                phase: ScrollPhase::Ended,
                                x,
                                y,
                            });
                            self.gesture = false;
                        }
                    }
                }
            }
        }
        out
    }

    /// Ups for held buttons (focus lost), and a scrollGesture ended if a gesture is open. Always
    /// emitted, at the last known position if the cursor or layout is gone, so the host never
    /// keeps a stuck button.
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let (x, y) = self.position();
        if self.left {
            self.left = false;
            out.push(InputEvent::Pointer {
                action: PointerAction::LeftUp,
                x,
                y,
            });
        }
        if self.right {
            self.right = false;
            out.push(InputEvent::Pointer {
                action: PointerAction::RightUp,
                x,
                y,
            });
        }
        if self.gesture {
            self.gesture = false;
            out.push(InputEvent::ScrollGesture {
                phase: ScrollPhase::Ended,
                x,
                y,
            });
        }
        out
    }
}
