//! Reads a section's Markdown into the blocks shells draw. Only what a detail can show is kept:
//! a heading reads as a bold paragraph, a quote as its text, raw HTML as text, and a rule not
//! at all.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::{Block, Span, Style};

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
    /// The text of the paragraph being read.
    spans: Vec<Span>,
    /// How many bold and italic runs the text is inside; a heading counts as bold.
    bold: usize,
    italic: usize,
    link: Option<String>,
    /// The text of the code block being read.
    code: Option<String>,
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
            Event::Start(Tag::CodeBlock(_)) => {
                self.end_paragraph();
                self.code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(code) = self.code.take() {
                    self.blocks
                        .push(Block::Code(code.trim_end_matches('\n').into()));
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::HtmlBlock) => self.end_paragraph(),
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

    fn end_paragraph(&mut self) {
        // An HTML block's lines end in a line break.
        if let Some(last) = self.spans.last_mut() {
            last.text.truncate(last.text.trim_end().len());
        }
        self.spans.retain(|span| !span.text.is_empty());
        if !self.spans.is_empty() {
            self.blocks
                .push(Block::Paragraph(std::mem::take(&mut self.spans)));
        }
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

    #[test]
    fn copying_a_section_drops_the_markdown() {
        let section = crate::Section::markdown("", "A **b** [c](https://c.example)\n\n```\nd\n```");
        assert_eq!(section.plain_text(), "A b c\nd");
        let mono = crate::Section::mono("", "**kept**\n\n");
        assert_eq!(mono.plain_text(), "**kept**\n\n");
    }

    #[test]
    fn escaped_text_reads_as_itself() {
        let text = r"*not bold* _x_ `y` [z](w) <b> # 1. - | \ & !";
        assert_eq!(blocks(&escape(text)), [Block::Paragraph(vec![plain(text)])]);
    }
}
