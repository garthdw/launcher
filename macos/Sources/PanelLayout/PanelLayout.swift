import CoreGraphics

/// How a result row shares its width between the title, from the left, and the subtitle with
/// its key hints, ending at the right. Each takes the room the other leaves; when both are long,
/// the subtitle keeps the row's last 220 points and loses its start, such as an app's folder,
/// rather than the keys. The subtitle never starts before the title, however narrow the row.
public struct RowLayout {
    public let titleWidth: CGFloat
    public let subtitleX: CGFloat
    public let subtitleWidth: CGFloat

    /// The title starts at `titleX` and the subtitle ends at `end`; `title` and `subtitle` are
    /// the widths each needs to show whole.
    public init(titleX: CGFloat, end: CGFloat, title: CGFloat, subtitle: CGFloat) {
        let gap: CGFloat = 4
        let wholeSubtitleX = end - subtitle
        let shared = min(max(titleX + title + gap, wholeSubtitleX), max(end - 220, wholeSubtitleX))
        subtitleX = max(shared, titleX + gap)
        subtitleWidth = end - subtitleX
        titleWidth = subtitleX - gap - titleX
    }
}

/// Moves a selection through `count` entries, wrapping at either end.
public func step(_ index: Int, by offset: Int, count: Int) -> Int {
    precondition(count > 0, "nothing to step through")
    return ((index + offset) % count + count) % count
}

/// How wide each column of a table is, given the width each needs to show its cells on one line.
/// As needed when they fit in `available`; otherwise columns that fit an even share keep their
/// width, and the others share what's left evenly, so their cells wrap.
public func columnWidths(needed: [CGFloat], available: CGFloat) -> [CGFloat] {
    guard needed.reduce(0, +) > available else { return needed }
    var widths = needed
    var wide = Array(needed.indices)
    var room = available
    while true {
        let share = room / CGFloat(wide.count)
        let fitting = wide.filter { needed[$0] <= share }
        // Some column is always too wide, as all of them together are.
        guard !fitting.isEmpty else {
            for column in wide { widths[column] = share }
            return widths
        }
        room -= fitting.reduce(0) { $0 + needed[$1] }
        wide.removeAll { fitting.contains($0) }
    }
}
