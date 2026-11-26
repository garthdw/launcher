//! Reads a section's Markdown into the blocks shells draw. Only what a detail can show is kept:
//! a heading reads as a bold paragraph, a quote as its text, raw HTML as text, and a rule not
//! at all.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::{Block, ListItem, Span, Style};

/// Backslash-escapes every ASCII punctuation mark, so text from outside, such as a web page's
/// title, reads as itself in Markdown. CommonMark allows escaping any of them.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn blocks(markdown: &str) -> Vec<Block> {
    let mut reader = Reader::default();
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES) {
        reader.read(event);
    }
    reader.end_paragraph();
    reader.blocks
}

#[derive(Default)]
struct Reader {
    blocks: Vec<Block>,
    /// The text of the paragraph, list item or table cell being read.
    spans: Vec<Span>,
    /// How many bold and italic runs the text is inside; a heading counts as bold.
    bold: usize,
    italic: usize,
    link: Option<String>,
    /// The text of the code block being read.
    code: Option<String>,
    /// Each list the text is inside, outermost first, with its next number if it's numbered.
    lists: Vec<Option<u64>>,
    /// The items read so far of the outermost list.
    items: Vec<ListItem>,
    /// The marker of the item being read, until its text is read.
    marker: String,
    /// Inside a table, and its header and rows so far.
    table: bool,
    header: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
}

impl Reader {
    fn read(&mut self, event: Event) {
        match event {
            Event::Start(Tag::Heading { .. } | Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::End(TagEnd::Heading(_)) => {
                self.bold = self.bold.saturating_sub(1);
                self.end_paragraph();
            }
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => self.link = web_address(&dest_url),
            Event::End(TagEnd::Link) => self.link = None,
            // A list item's second paragraph.
            Event::Start(Tag::Paragraph) if !self.spans.is_empty() => self.push("\n", false),
            Event::End(TagEnd::Paragraph | TagEnd::HtmlBlock) => self.end_paragraph(),
            Event::Start(Tag::CodeBlock(_)) => {
                self.end_paragraph();
                self.code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some(code) = self.code.take() else { return };
                let code = code.trim_end_matches('\n');
                if self.in_list_or_table() {
                    if !self.spans.is_empty() {
                        self.push("\n", false);
                    }
                    self.push(code, true);
                } else {
                    self.blocks.push(Block::Code(code.into()));
                }
            }
            Event::Start(Tag::List(first)) => {
                self.end_paragraph();
                // The text of the item this list is in, before it.
                self.end_item();
                self.lists.push(first);
            }
            Event::Start(Tag::Item) => {
                self.marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}.", *number - 1)
                    }
                    _ => "•".into(),
                };
            }
            Event::End(TagEnd::Item) => self.end_item(),
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
                if self.lists.is_empty() && !self.items.is_empty() {
                    self.blocks
                        .push(Block::List(std::mem::take(&mut self.items)));
                }
            }
            Event::Start(Tag::Table(_)) => {
                self.end_paragraph();
                self.table = true;
            }
            Event::End(TagEnd::TableCell) => {
                let cell = self.take_spans();
                self.row.push(cell);
            }
            Event::End(TagEnd::TableHead) => self.header = std::mem::take(&mut self.row),
            Event::End(TagEnd::TableRow) => {
                let row = std::mem::take(&mut self.row);
                self.rows.push(row);
            }
            Event::End(TagEnd::Table) => {
                self.table = false;
                self.blocks.push(Block::Table {
                    header: std::mem::take(&mut self.header),
                    rows: std::mem::take(&mut self.rows),
                });
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                match &mut self.code {
                    Some(code) => code.push_str(&text),
                    None => self.push(&text, false),
                }
            }
            Event::Code(text) => self.push(&text, true),
            Event::SoftBreak => self.push(" ", false),
            Event::HardBreak => self.push("\n", false),
            _ => {}
        }
    }

    /// Adds `text` in the current style, to the last span if it has the same style.
    fn push(&mut self, text: &str, code: bool) {
        let style = Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            code,
        };
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == self.link => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.into(),
                style,
                link: self.link.clone(),
            }),
        }
    }

    fn in_list_or_table(&self) -> bool {
        !self.lists.is_empty() || self.table
    }

    /// The text read so far, as a list item or table cell keeps it.
    fn take_spans(&mut self) -> Vec<Span> {
        // An HTML block's lines end in a line break.
        if let Some(last) = self.spans.last_mut() {
            last.text.truncate(last.text.trim_end().len());
        }
        self.spans.retain(|span| !span.text.is_empty());
        std::mem::take(&mut self.spans)
    }

    /// Ends a paragraph, unless it's part of a list item or table cell, which ends it itself.
    fn end_paragraph(&mut self) {
        if self.in_list_or_table() {
            return;
        }
        let spans = self.take_spans();
        if !spans.is_empty() {
            self.blocks.push(Block::Paragraph(spans));
        }
    }

    fn end_item(&mut self) {
        let spans = self.take_spans();
        if spans.is_empty() {
            return;
        }
        self.items.push(ListItem {
            depth: u8::try_from(self.lists.len().saturating_sub(1)).unwrap_or(u8::MAX),
            marker: std::mem::take(&mut self.marker),
            spans,
        });
    }
}

/// `url` if it's a web page or an email address; other schemes, such as `file:` or
/// `javascript:`, could run something when clicked.
fn web_address(url: &str) -> Option<String> {
    ["http://", "https://", "mailto:"]
        .into_iter()
        .any(|scheme| {
            url.get(..scheme.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
        })
        .then(|| url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span {
            text: text.into(),
            style: Style::default(),
            link: None,
        }
    }

    fn styled(text: &str, bold: bool, italic: bool, code: bool) -> Span {
        Span {
            style: Style { bold, italic, code },
            ..plain(text)
        }
    }

    fn linked(text: &str, url: &str) -> Span {
        Span {
            link: Some(url.into()),
            ..plain(text)
        }
    }

    #[test]
    fn inline_styles_become_spans() {
        assert_eq!(
            blocks("A **bold _and italic_** `code`\nwrapped  \nbroken"),
            [Block::Paragraph(vec![
                plain("A "),
                styled("bold ", true, false, false),
                styled("and italic", true, true, false),
                plain(" "),
                styled("code", false, false, true),
                plain(" wrapped\nbroken"),
            ])]
        );
    }

    #[test]
    fn headings_quotes_and_html_read_as_paragraphs() {
        assert_eq!(
            blocks("# Title\n\n> quoted\n\n---\n\n<div>raw</div>\n\nlast"),
            [
                Block::Paragraph(vec![styled("Title", true, false, false)]),
                Block::Paragraph(vec![plain("quoted")]),
                Block::Paragraph(vec![plain("<div>raw</div>")]),
                Block::Paragraph(vec![plain("last")]),
            ]
        );
    }

    #[test]
    fn code_blocks_keep_their_lines() {
        assert_eq!(
            blocks("Before\n```json\n{\n  \"a\": 1\n}\n```\n\n    indented"),
            [
                Block::Paragraph(vec![plain("Before")]),
                Block::Code("{\n  \"a\": 1\n}".into()),
                Block::Code("indented".into()),
            ]
        );
    }

    #[test]
    fn only_web_and_email_links_are_kept() {
        assert_eq!(
            blocks(
                "[web](https://example.com) [mail](MAILTO:a@example.com) \
                 [script](javascript:alert(1)) [file](file:///etc) <https://auto.example>"
            ),
            [Block::Paragraph(vec![
                linked("web", "https://example.com"),
                plain(" "),
                linked("mail", "MAILTO:a@example.com"),
                plain(" script file "),
                linked("https://auto.example", "https://auto.example"),
            ])]
        );
    }

    fn item(depth: u8, marker: &str, spans: Vec<Span>) -> ListItem {
        ListItem {
            depth,
            marker: marker.into(),
            spans,
        }
    }

    #[test]
    fn lists_keep_their_items_depth_and_numbers() {
        assert_eq!(
            blocks(
                "Before\n\n- one\n- **two**\n\n  3. three\n  4. four\n\n     second paragraph\n\n  \
                 after\n- five\n\n      code\n\nAfter"
            ),
            [
                Block::Paragraph(vec![plain("Before")]),
                Block::List(vec![
                    item(0, "•", vec![plain("one")]),
                    item(0, "•", vec![styled("two", true, false, false)]),
                    item(1, "3.", vec![plain("three")]),
                    item(1, "4.", vec![plain("four\nsecond paragraph")]),
                    item(0, "", vec![plain("after")]),
                    item(
                        0,
                        "•",
                        vec![plain("five\n"), styled("code", false, false, true)]
                    ),
                ]),
                Block::Paragraph(vec![plain("After")]),
            ]
        );
    }

    #[test]
    fn tables_keep_their_header_and_cells() {
        assert_eq!(
            blocks("| Claim | Time |\n|---|--:|\n| `exp` | in **5** min |\n| iat |\n\nAfter"),
            [
                Block::Table {
                    header: vec![vec![plain("Claim")], vec![plain("Time")]],
                    rows: vec![
                        vec![
                            vec![styled("exp", false, false, true)],
                            vec![plain("in "), styled("5", true, false, false), plain(" min")],
                        ],
                        vec![vec![plain("iat")], vec![]],
                    ],
                },
                Block::Paragraph(vec![plain("After")]),
            ]
        );
    }

    #[test]
    fn copying_a_section_drops_the_markdown() {
        let section = crate::Section::markdown("", "A **b** [c](https://c.example)\n\n```\nd\n```");
        assert_eq!(section.plain_text(), "A b c\nd");
        let list = crate::Section::markdown("", "1. one\n   - two\n\n   after");
        assert_eq!(list.plain_text(), "1. one\n  • two\nafter");
        let table = crate::Section::markdown("", "| a | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(table.plain_text(), "a\tb\n1\t2");
        let mono = crate::Section::mono("", "**kept**\n\n");
        assert_eq!(mono.plain_text(), "**kept**\n\n");
    }

    #[test]
    fn escaped_text_reads_as_itself() {
        let text = r"*not bold* _x_ `y` [z](w) <b> # 1. - | \ & !";
        assert_eq!(blocks(&escape(text)), [Block::Paragraph(vec![plain(text)])]);
    }
}
