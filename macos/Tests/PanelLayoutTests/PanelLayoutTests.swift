import CoreGraphics
import PanelLayout
import Testing

/// A row 570 points wide, about the panel's: the title from 46, the subtitle ending at 616.
private func layout(title: CGFloat, subtitle: CGFloat) -> RowLayout {
    RowLayout(titleX: 46, end: 616, title: title, subtitle: subtitle)
}

@Test func bothShowWholeWhenThereIsRoom() {
    let row = layout(title: 100, subtitle: 150)
    #expect(row.subtitleX == 466, "right-aligned at the end")
    #expect(row.subtitleWidth == 150)
    #expect(row.titleWidth == 416, "the title has the room up to the subtitle")
}

@Test func aLongTitleLeavesTheSubtitleWhole() {
    let row = layout(title: 900, subtitle: 150)
    #expect(row.subtitleX == 466)
    #expect(row.subtitleWidth == 150)
    #expect(row.titleWidth == 416, "the title loses its end")
}

@Test func aLongSubtitleTakesTheRoomAShortTitleLeaves() {
    let row = layout(title: 100, subtitle: 900)
    #expect(row.subtitleX == 150, "just after the title")
    #expect(row.subtitleWidth == 466)
    #expect(row.titleWidth == 100, "the title stays whole")
}

@Test func aLongSubtitleLeavesALongTitleUpTo396() {
    let row = layout(title: 900, subtitle: 900)
    #expect(row.subtitleX == 396)
    #expect(row.subtitleWidth == 220, "keeping its end, the keys")
    #expect(row.titleWidth == 346)
}

@Test func aNarrowRowKeepsTheSubtitleAfterTheTitle() {
    let row = RowLayout(titleX: 46, end: 200, title: 900, subtitle: 900)
    #expect(row.subtitleX == 50)
    #expect(row.titleWidth == 0)
}

@Test func stepsWrapAtEitherEnd() {
    #expect(step(0, by: 1, count: 3) == 1)
    #expect(step(2, by: 1, count: 3) == 0)
    #expect(step(0, by: -1, count: 3) == 2)
    #expect(step(0, by: -4, count: 3) == 2, "however far")
}
