//! Window layouts: where an action puts a window within a display's visible frame. Pure
//! geometry, so every shell lays windows out the same way and only moves them natively.

use launcher_core::{Hotkey, Key};

/// A frame in global coordinates with the origin at the top left and y growing down, as the
/// macOS Accessibility API and X11 both report windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowAction {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    FirstThird,
    CentreThird,
    LastThird,
    Maximise,
    /// Keeps the window's size, shrunk to fit if it must.
    Centre,
    /// Back to the frame from before the window was first laid out, which the shell remembers
    /// for each window.
    Restore,
}

impl WindowAction {
    /// Where this action puts `window` within `visible`, a display's frame less its menu bar and
    /// dock; `None` for `Restore`, which has no frame of its own.
    pub fn frame(self, window: Rect, visible: Rect) -> Option<Rect> {
        let whole = Span::new(0, 1, 1);
        let first_half = Span::new(0, 1, 2);
        let second_half = Span::new(1, 2, 2);
        let (columns, rows) = match self {
            Self::LeftHalf => (first_half, whole),
            Self::RightHalf => (second_half, whole),
            Self::TopHalf => (whole, first_half),
            Self::BottomHalf => (whole, second_half),
            Self::TopLeft => (first_half, first_half),
            Self::TopRight => (second_half, first_half),
            Self::BottomLeft => (first_half, second_half),
            Self::BottomRight => (second_half, second_half),
            Self::FirstThird => (Span::new(0, 1, 3), whole),
            Self::CentreThird => (Span::new(1, 2, 3), whole),
            Self::LastThird => (Span::new(2, 3, 3), whole),
            Self::Maximise => (whole, whole),
            Self::Centre => return Some(centre(window, visible)),
            Self::Restore => return None,
        };
        let (x, right) = columns.edges(visible.x, visible.width);
        let (y, bottom) = rows.edges(visible.y, visible.height);
        Some(Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

/// Cells `start..end` of a length split into `count` equal cells.
#[derive(Clone, Copy)]
struct Span {
    start: u8,
    end: u8,
    count: u8,
}

impl Span {
    const fn new(start: u8, end: u8, count: u8) -> Self {
        Self { start, end, count }
    }

    /// Rounded to whole points, so neighbouring cells meet without a gap or an overlap.
    fn edges(&self, origin: f64, length: f64) -> (f64, f64) {
        let edge = |index: u8| (origin + length * f64::from(index) / f64::from(self.count)).round();
        (edge(self.start), edge(self.end))
    }
}

/// Which of `displays`' frames holds most of `window`: the display its layouts use. The first
/// when it's off every display.
pub fn display_holding(window: Rect, displays: &[Rect]) -> Option<usize> {
    let overlap = |display: &Rect| {
        let width =
            (window.x + window.width).min(display.x + display.width) - window.x.max(display.x);
        let height =
            (window.y + window.height).min(display.y + display.height) - window.y.max(display.y);
        width.max(0.0) * height.max(0.0)
    };
    displays
        .iter()
        .enumerate()
        .rev()
        .max_by(|(_, first), (_, second)| overlap(first).total_cmp(&overlap(second)))
        .map(|(index, _)| index)
}

/// `frame` moved as little as it takes to lie within `visible`, keeping its size: for a window
/// whose minimum size is larger than its layout's frame, which would otherwise hang off the
/// display's edge. One larger than `visible` goes to its top left.
pub fn moved_within(frame: Rect, visible: Rect) -> Rect {
    Rect {
        x: frame
            .x
            .min(visible.x + visible.width - frame.width)
            .max(visible.x),
        y: frame
            .y
            .min(visible.y + visible.height - frame.height)
            .max(visible.y),
        ..frame
    }
}

fn centre(window: Rect, visible: Rect) -> Rect {
    let width = window.width.min(visible.width);
    let height = window.height.min(visible.height);
    Rect {
        x: (visible.x + (visible.width - width) / 2.0).round(),
        y: (visible.y + (visible.height - height) / 2.0).round(),
        width,
        height,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub hotkey: Hotkey,
    pub action: WindowAction,
}

/// Rectangle's defaults: ⌃⌥ with arrows for halves, U I J K for quarters, D F G for thirds.
pub const DEFAULT_BINDINGS: &[Binding] = &[
    control_option(Key::Left, WindowAction::LeftHalf),
    control_option(Key::Right, WindowAction::RightHalf),
    control_option(Key::Up, WindowAction::TopHalf),
    control_option(Key::Down, WindowAction::BottomHalf),
    control_option(Key::Character('u'), WindowAction::TopLeft),
    control_option(Key::Character('i'), WindowAction::TopRight),
    control_option(Key::Character('j'), WindowAction::BottomLeft),
    control_option(Key::Character('k'), WindowAction::BottomRight),
    control_option(Key::Character('d'), WindowAction::FirstThird),
    control_option(Key::Character('f'), WindowAction::CentreThird),
    control_option(Key::Character('g'), WindowAction::LastThird),
    control_option(Key::Return, WindowAction::Maximise),
    control_option(Key::Character('c'), WindowAction::Centre),
    control_option(Key::Delete, WindowAction::Restore),
];

const fn control_option(key: Key, action: WindowAction) -> Binding {
    Binding {
        hotkey: Hotkey {
            key,
            control: true,
            option: true,
            command: false,
            shift: false,
        },
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// A 1512 × 982 display less a 33-point menu bar.
    const VISIBLE: Rect = Rect {
        x: 0.0,
        y: 33.0,
        width: 1512.0,
        height: 949.0,
    };
    const WINDOW: Rect = Rect {
        x: 100.0,
        y: 200.0,
        width: 800.0,
        height: 600.0,
    };

    fn frame(action: WindowAction) -> Option<Rect> {
        action.frame(WINDOW, VISIBLE)
    }

    #[test]
    fn halves_and_quarters_split_the_visible_frame() {
        assert_eq!(
            frame(WindowAction::LeftHalf),
            Some(rect(0.0, 33.0, 756.0, 949.0))
        );
        assert_eq!(
            frame(WindowAction::RightHalf),
            Some(rect(756.0, 33.0, 756.0, 949.0))
        );
        // 949 is odd: the halves still meet, the first rounding up.
        assert_eq!(
            frame(WindowAction::TopHalf),
            Some(rect(0.0, 33.0, 1512.0, 475.0))
        );
        assert_eq!(
            frame(WindowAction::BottomHalf),
            Some(rect(0.0, 508.0, 1512.0, 474.0))
        );
        assert_eq!(
            frame(WindowAction::TopLeft),
            Some(rect(0.0, 33.0, 756.0, 475.0))
        );
        assert_eq!(
            frame(WindowAction::BottomRight),
            Some(rect(756.0, 508.0, 756.0, 474.0))
        );
        assert_eq!(frame(WindowAction::Maximise), Some(VISIBLE));
    }

    #[test]
    fn thirds_tile_without_gaps() {
        let thirds = |visible: Rect| {
            [
                WindowAction::FirstThird,
                WindowAction::CentreThird,
                WindowAction::LastThird,
            ]
            .map(|action| action.frame(WINDOW, visible))
        };
        assert_eq!(
            thirds(VISIBLE),
            [
                Some(rect(0.0, 33.0, 504.0, 949.0)),
                Some(rect(504.0, 33.0, 504.0, 949.0)),
                Some(rect(1008.0, 33.0, 504.0, 949.0)),
            ]
        );
        assert_eq!(
            thirds(rect(10.0, 0.0, 1000.0, 500.0)),
            [
                Some(rect(10.0, 0.0, 333.0, 500.0)),
                Some(rect(343.0, 0.0, 334.0, 500.0)),
                Some(rect(677.0, 0.0, 333.0, 500.0)),
            ]
        );
    }

    #[test]
    fn display_holding_picks_the_largest_overlap() {
        let displays = [
            rect(0.0, 0.0, 1512.0, 982.0),
            rect(1512.0, -200.0, 1920.0, 1080.0),
        ];
        let straddling = rect(1300.0, 100.0, 800.0, 600.0);
        assert_eq!(display_holding(straddling, &displays), Some(1));
        assert_eq!(display_holding(WINDOW, &displays), Some(0));
        let lost = rect(-5000.0, -5000.0, 100.0, 100.0);
        assert_eq!(display_holding(lost, &displays), Some(0));
        assert_eq!(display_holding(WINDOW, &[]), None);
    }

    #[test]
    fn layouts_follow_a_display_away_from_the_origin() {
        // Left of and above the primary display, so negative in global coordinates.
        let visible = rect(-1920.0, -1080.0, 1920.0, 1055.0);
        assert_eq!(
            WindowAction::BottomRight.frame(WINDOW, visible),
            Some(rect(-960.0, -553.0, 960.0, 528.0))
        );
    }

    #[test]
    fn centre_keeps_the_size_and_shrinks_only_to_fit() {
        assert_eq!(
            frame(WindowAction::Centre),
            Some(rect(356.0, 208.0, 800.0, 600.0))
        );
        let tall = rect(0.0, 0.0, 400.0, 2000.0);
        assert_eq!(
            WindowAction::Centre.frame(tall, VISIBLE),
            Some(rect(556.0, 33.0, 400.0, 949.0))
        );
    }

    #[test]
    fn moved_within_pulls_an_overhanging_window_back() {
        // A right half that kept a minimum width of 900.
        let overhanging = rect(756.0, 33.0, 900.0, 949.0);
        assert_eq!(
            moved_within(overhanging, VISIBLE),
            rect(612.0, 33.0, 900.0, 949.0)
        );
        assert_eq!(moved_within(WINDOW, VISIBLE), WINDOW);
        let huge = rect(-50.0, 500.0, 2000.0, 1200.0);
        assert_eq!(moved_within(huge, VISIBLE), rect(0.0, 33.0, 2000.0, 1200.0));
    }

    #[test]
    fn restore_has_no_frame_of_its_own() {
        assert_eq!(frame(WindowAction::Restore), None);
    }

    #[test]
    fn default_bindings_are_distinct() {
        for (index, binding) in DEFAULT_BINDINGS.iter().enumerate() {
            for other in &DEFAULT_BINDINGS[index + 1..] {
                assert_ne!(binding.hotkey, other.hotkey);
                assert_ne!(binding.action, other.action);
            }
        }
    }
}
