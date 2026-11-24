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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub frame: Rect,
    /// The frame less the menu bar and dock.
    pub visible: Rect,
}

/// Where an action puts a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub frame: Rect,
    /// The visible frame of the display the window lands on, to keep it within.
    pub visible: Rect,
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
    /// Back to the frame from before the window was first laid out or moved to another display,
    /// which the shell remembers for each window; see `restore_placement`.
    Restore,
    /// To the display on the right, keeping the window's place and size relative to the display.
    NextDisplay,
    PreviousDisplay,
}

impl WindowAction {
    /// Where this action puts `window`, laid out on the display holding it; `None` for `Restore`,
    /// which has no frame of its own, for a display move with only one display, and without
    /// displays.
    pub fn place(self, window: Rect, displays: &[Display]) -> Option<Placement> {
        let holding = display_holding(window, displays)?;
        match self {
            Self::NextDisplay => moved_along(window, displays, holding, 1),
            Self::PreviousDisplay => moved_along(window, displays, holding, -1),
            layout => {
                let visible = displays.get(holding)?.visible;
                layout
                    .frame(window, visible)
                    .map(|frame| Placement { frame, visible })
            }
        }
    }

    /// Where this action puts `window` within `visible`; `None` for the actions without a layout
    /// of their own.
    fn frame(self, window: Rect, visible: Rect) -> Option<Rect> {
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
            Self::Restore | Self::NextDisplay | Self::PreviousDisplay => return None,
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
fn display_holding(window: Rect, displays: &[Display]) -> Option<usize> {
    let overlap = |display: &Display| {
        let display = display.frame;
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

/// Where `Restore` puts `window` back to `original`, a frame saved among the displays `saved`:
/// `None` to restore it exactly while the display that held it is still where it was. Otherwise,
/// as after that display was unplugged or the displays rearranged (which also shifts global
/// coordinates when the main display changes), at the same place relative to the display holding
/// `window` as it had to its own. Displays are told apart by frame alone, so one swapped for
/// another of the same size in the same place counts as still there. Also `None` without
/// displays.
pub fn restore_placement(
    original: Rect,
    saved: &[Display],
    window: Rect,
    displays: &[Display],
) -> Option<Placement> {
    let held = saved.get(display_holding(original, saved)?)?;
    if displays.iter().any(|display| display.frame == held.frame) {
        return None;
    }
    let visible = displays.get(display_holding(window, displays)?)?.visible;
    Some(Placement {
        frame: scaled(original, held.visible, visible),
        visible,
    })
}

/// `window` on the display `offset` places along from `holding` in left-to-right order, wrapping
/// around; `None` with only one display.
fn moved_along(
    window: Rect,
    displays: &[Display],
    holding: usize,
    offset: isize,
) -> Option<Placement> {
    let order = left_to_right(displays);
    let position = isize::try_from(order.iter().position(|&index| index == holding)?).ok()?;
    let count = isize::try_from(order.len()).ok()?;
    let target = *order.get(usize::try_from((position + offset).rem_euclid(count)).ok()?)?;
    if target == holding {
        return None;
    }
    let visible = displays.get(target)?.visible;
    Some(Placement {
        frame: scaled(window, displays.get(holding)?.visible, visible),
        visible,
    })
}

/// `displays`' indices ordered left to right by their frames' left edges. A display whose centre
/// lies over a column's leftmost display, such as one stacked above or below it, joins that
/// column, and a column goes top to bottom.
fn left_to_right(displays: &[Display]) -> Vec<usize> {
    struct Keyed {
        column: usize,
        top: f64,
        index: usize,
    }
    let mut frames: Vec<(usize, Rect)> = displays
        .iter()
        .map(|display| display.frame)
        .enumerate()
        .collect();
    // Of displays sharing a left edge, the widest starts the column, so the order doesn't
    // depend on the order the system lists displays in.
    frames.sort_by(|(_, first), (_, second)| {
        first
            .x
            .total_cmp(&second.x)
            .then(second.width.total_cmp(&first.width))
            .then(first.y.total_cmp(&second.y))
    });
    let mut keyed = Vec::with_capacity(frames.len());
    let mut column = 0;
    let mut column_right = f64::NEG_INFINITY;
    for (index, frame) in frames {
        if frame.x + frame.width / 2.0 >= column_right {
            column += 1;
            column_right = frame.x + frame.width;
        }
        keyed.push(Keyed {
            column,
            top: frame.y,
            index,
        });
    }
    keyed.sort_by(|first, second| {
        first
            .column
            .cmp(&second.column)
            .then(first.top.total_cmp(&second.top))
    });
    keyed.into_iter().map(|keyed| keyed.index).collect()
}

/// `window` with the same place and size relative to `to` as it has to `from`, so a left half
/// stays a left half, and kept within `to`.
fn scaled(window: Rect, from: Rect, to: Rect) -> Rect {
    // Edges rounded to whole points, as layouts are, so a scaled layout matches the target's.
    let point = |x: f64, y: f64| {
        (
            (to.x + (x - from.x) * to.width / from.width).round(),
            (to.y + (y - from.y) * to.height / from.height).round(),
        )
    };
    let (x, y) = point(window.x, window.y);
    let (right, bottom) = point(window.x + window.width, window.y + window.height);
    let fitted = Rect {
        x,
        y,
        width: (right - x).min(to.width),
        height: (bottom - y).min(to.height),
    };
    moved_within(fitted, to)
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

/// Rectangle's defaults: ⌃⌥ with arrows for halves, U I J K for quarters, D F G for thirds, and
/// ⌃⌥⌘ with arrows for the next and previous display.
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
    control_option_command(Key::Right, WindowAction::NextDisplay),
    control_option_command(Key::Left, WindowAction::PreviousDisplay),
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

const fn control_option_command(key: Key, action: WindowAction) -> Binding {
    let mut binding = control_option(key, action);
    binding.hotkey.command = true;
    binding
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

    /// A display with no menu bar or dock.
    fn display(frame: Rect) -> Display {
        Display {
            frame,
            visible: frame,
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
    const LAPTOP: Display = Display {
        frame: Rect {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: 982.0,
        },
        visible: VISIBLE,
    };
    /// Larger, to the right of the laptop and reaching above it.
    const EXTERNAL: Display = Display {
        frame: Rect {
            x: 1512.0,
            y: -458.0,
            width: 2560.0,
            height: 1440.0,
        },
        visible: Rect {
            x: 1512.0,
            y: -433.0,
            width: 2560.0,
            height: 1415.0,
        },
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
        let displays = [LAPTOP, EXTERNAL];
        let straddling = rect(1300.0, 100.0, 800.0, 600.0);
        assert_eq!(display_holding(straddling, &displays), Some(1));
        assert_eq!(display_holding(WINDOW, &displays), Some(0));
        let lost = rect(-5000.0, -5000.0, 100.0, 100.0);
        assert_eq!(display_holding(lost, &displays), Some(0));
        assert_eq!(display_holding(WINDOW, &[]), None);
    }

    #[test]
    fn place_lays_out_on_the_display_holding_the_window() {
        let displays = [LAPTOP, EXTERNAL];
        let window = rect(2000.0, 0.0, 800.0, 600.0);
        assert_eq!(
            WindowAction::LeftHalf.place(window, &displays),
            Some(Placement {
                frame: rect(1512.0, -433.0, 1280.0, 1415.0),
                visible: EXTERNAL.visible,
            })
        );
        assert_eq!(WindowAction::Restore.place(window, &displays), None);
        assert_eq!(WindowAction::LeftHalf.place(window, &[]), None);
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
    fn restore_goes_back_exactly_while_the_display_is_there() {
        let both = [LAPTOP, EXTERNAL];
        let on_laptop = rect(0.0, 33.0, 756.0, 949.0);
        let on_external = rect(2000.0, -300.0, 1000.0, 800.0);
        assert_eq!(
            restore_placement(on_external, &both, on_laptop, &both),
            None
        );
        // Left mostly below the laptop's bottom edge on purpose.
        let low = rect(100.0, 700.0, 800.0, 700.0);
        assert_eq!(
            restore_placement(low, &[LAPTOP], on_laptop, &[LAPTOP]),
            None
        );
        // Mostly on the laptop: the external display going leaves it overhanging, as it was.
        let straddling = rect(1000.0, 100.0, 1000.0, 600.0);
        assert_eq!(
            restore_placement(straddling, &both, on_laptop, &[LAPTOP]),
            None
        );
        assert_eq!(restore_placement(on_laptop, &both, on_laptop, &[]), None);
    }

    #[test]
    fn restore_moves_to_a_remaining_display_after_its_own_is_unplugged() {
        let on_laptop = rect(0.0, 33.0, 756.0, 949.0);
        let restored = |original: Rect| {
            restore_placement(original, &[LAPTOP, EXTERNAL], on_laptop, &[LAPTOP])
                .map(|placement| placement.frame)
        };
        // Scaled from the external display to the laptop, as a display move would.
        assert_eq!(
            restored(rect(2000.0, -300.0, 1000.0, 800.0)),
            Some(rect(288.0, 122.0, 591.0, 537.0))
        );
        // Mostly on the external display, hanging off its left edge onto the laptop.
        assert_eq!(
            restored(rect(1400.0, 100.0, 1000.0, 600.0)),
            Some(rect(0.0, 390.0, 590.0, 403.0))
        );
    }

    #[test]
    fn restore_follows_its_display_when_the_main_display_changes() {
        // The external display was main, with the laptop left of it and lower down. Unplugged,
        // the laptop becomes main and the origin moves to its top left.
        let main_external = display(rect(0.0, 0.0, 2560.0, 1440.0));
        let laptop_beside = Display {
            frame: rect(-1512.0, 458.0, 1512.0, 982.0),
            visible: rect(-1512.0, 491.0, 1512.0, 949.0),
        };
        let on_laptop_before = rect(-1400.0, 500.0, 800.0, 600.0);
        assert_eq!(
            restore_placement(
                on_laptop_before,
                &[main_external, laptop_beside],
                rect(0.0, 33.0, 756.0, 949.0),
                &[LAPTOP],
            ),
            Some(Placement {
                frame: rect(112.0, 42.0, 800.0, 600.0),
                visible: VISIBLE,
            })
        );
    }

    fn placed(action: WindowAction, window: Rect, displays: &[Display]) -> Option<Rect> {
        action
            .place(window, displays)
            .map(|placement| placement.frame)
    }

    #[test]
    fn display_moves_keep_a_layout() {
        let displays = [LAPTOP, EXTERNAL];
        for layout in [
            WindowAction::LeftHalf,
            WindowAction::TopRight,
            WindowAction::LastThird,
        ] {
            let on_laptop = layout.frame(WINDOW, LAPTOP.visible);
            let on_external = layout.frame(WINDOW, EXTERNAL.visible);
            assert_eq!(
                on_laptop.and_then(|frame| placed(WindowAction::NextDisplay, frame, &displays)),
                on_external
            );
            assert_eq!(
                on_external.and_then(|frame| placed(
                    WindowAction::PreviousDisplay,
                    frame,
                    &displays
                )),
                on_laptop
            );
        }
    }

    #[test]
    fn display_moves_wrap_around() {
        let displays = [LAPTOP, EXTERNAL];
        for action in [WindowAction::NextDisplay, WindowAction::PreviousDisplay] {
            let moved = action.place(WINDOW, &displays);
            assert_eq!(
                moved.map(|placement| placement.visible),
                Some(EXTERNAL.visible)
            );
            let back = moved.and_then(|placement| action.place(placement.frame, &displays));
            assert_eq!(back.map(|placement| placement.frame), Some(WINDOW));
        }
    }

    #[test]
    fn display_moves_need_another_display() {
        assert_eq!(WindowAction::NextDisplay.place(WINDOW, &[LAPTOP]), None);
        assert_eq!(WindowAction::PreviousDisplay.place(WINDOW, &[LAPTOP]), None);
        assert_eq!(WindowAction::NextDisplay.place(WINDOW, &[]), None);
    }

    #[test]
    fn display_moves_keep_the_window_on_the_target() {
        // Hanging off the laptop's right edge.
        let overhanging = rect(1000.0, 100.0, 800.0, 600.0);
        assert_eq!(
            placed(WindowAction::NextDisplay, overhanging, &[LAPTOP, EXTERNAL]),
            Some(rect(2717.0, -333.0, 1355.0, 895.0))
        );
    }

    #[test]
    fn displays_go_left_to_right_and_columns_top_to_bottom() {
        // Left of the laptop, so at a negative x.
        let left = display(rect(-1920.0, 0.0, 1920.0, 1080.0));
        // Above the laptop, roughly centred, overlapping `left` horizontally.
        let above = display(rect(-204.0, -1080.0, 1920.0, 1080.0));
        let displays = [LAPTOP, EXTERNAL, left, above];
        assert_eq!(left_to_right(&displays), [2, 3, 0, 1]);
        let shuffled = [above, EXTERNAL, LAPTOP, left];
        assert_eq!(left_to_right(&shuffled), [3, 0, 2, 1]);

        let visible_after = |action: WindowAction, display: Display| {
            let window = rect(display.frame.x + 10.0, display.frame.y + 10.0, 400.0, 300.0);
            action
                .place(window, &displays)
                .map(|placement| placement.visible)
        };
        assert_eq!(
            visible_after(WindowAction::NextDisplay, above),
            Some(LAPTOP.visible)
        );
        assert_eq!(
            visible_after(WindowAction::PreviousDisplay, LAPTOP),
            Some(above.visible)
        );
        assert_eq!(
            visible_after(WindowAction::NextDisplay, EXTERNAL),
            Some(left.visible)
        );
        assert_eq!(
            visible_after(WindowAction::PreviousDisplay, left),
            Some(EXTERNAL.visible)
        );
    }

    #[test]
    fn a_display_below_another_joins_its_column() {
        // Starts left of the display above it but lies mostly under it.
        let displays = [
            display(rect(0.0, -1440.0, 2560.0, 1440.0)),
            display(rect(-100.0, 0.0, 1512.0, 982.0)),
        ];
        assert_eq!(left_to_right(&displays), [0, 1]);
        // Side by side, the left edges decide.
        let beside = [
            display(rect(1512.0, 0.0, 1920.0, 1080.0)),
            display(rect(0.0, -500.0, 1512.0, 982.0)),
        ];
        assert_eq!(left_to_right(&beside), [1, 0]);
    }

    #[test]
    fn the_order_doesnt_depend_on_how_displays_are_listed() {
        // A wide display above a laptop sharing its left edge, with another display beside the
        // laptop and under the wide one.
        let wide = display(rect(0.0, -1440.0, 3000.0, 1440.0));
        let laptop = display(rect(0.0, 0.0, 1200.0, 982.0));
        let beside = display(rect(1200.0, 0.0, 1800.0, 982.0));
        assert_eq!(left_to_right(&[wide, laptop, beside]), [0, 1, 2]);
        assert_eq!(left_to_right(&[laptop, wide, beside]), [1, 0, 2]);
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
