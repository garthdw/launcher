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
