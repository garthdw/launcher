use launcher_core::{Action, Item, Plugin, Section};
use numbat::markup::plain_text_format;
use numbat::module_importer::BuiltinModuleImporter;
use numbat::pretty_print::PrettyPrint;
use numbat::resolver::CodeSource;
use numbat::value::Value;
use numbat::{Context, FormatOptions, InterpreterResult, NumbatError};

pub struct Calc {
    context: Context,
}

impl Calc {
    pub fn new() -> Result<Self, Box<NumbatError>> {
        let mut context = Context::new(BuiltinModuleImporter::default());
        context.load_currency_module_on_demand(true);
        context
            .interpret("use prelude", CodeSource::Internal)
            .map(drop)?;
        Ok(Self { context })
    }

    /// Blocking ECB fetch. Without it, the first currency query fetches on the UI thread.
    pub fn prefetch_exchange_rates() {
        Context::prefetch_exchange_rates();
    }
}

impl Plugin for Calc {
    fn query(&mut self, query: &str) -> Vec<Item> {
        // Bare words such as an app name are valid numbat identifiers ("m", "day"), but echoing
        // them back as "1 m" is noise in a launcher.
        if !query.chars().any(|c| c.is_ascii_digit()) {
            return Vec::new();
        }
        let query = x_as_times(query);
        let Ok((statements, InterpreterResult::Value(value))) =
            self.context.interpret(&query, CodeSource::Text)
        else {
            return Vec::new();
        };
        // Digit separators read well but break pasting into most other apps.
        let unseparated = FormatOptions {
            digit_separator: String::new(),
            ..FormatOptions::default()
        };
        let copied = plain_text(&value, &unseparated);
        let precise = plain_text(
            &value,
            &FormatOptions {
                significant_digits: 17,
                ..unseparated
            },
        );
        let mut detail = vec![Section {
            title: "Input".into(),
            text: statements
                .iter()
                .map(|statement| plain_text_format(&statement.pretty_print(), false))
                .collect::<Vec<_>>()
                .join("\n"),
        }];
        if precise != copied {
            detail.push(Section {
                title: "Full precision".into(),
                text: precise,
            });
        }
        vec![Item {
            title: plain_text(&value, &FormatOptions::default()),
            subtitle: "Calculator".into(),
            action: Action::Copy(copied),
            detail,
        }]
    }
}

/// How numbat's REPL shows a value, unlike `Display`, which writes dates as `datetime("…")`.
fn plain_text(value: &Value, options: &FormatOptions) -> String {
    plain_text_format(&value.pretty_print_with(options), false)
        .trim()
        .to_owned()
}

/// People type `x` for multiplication ("2 x 3", "4x5"), but numbat reads only `*` and `×` and
/// takes `x` for a variable. Only an `x` between a number (or `)`) and a number (or `(`) is
/// rewritten, so names and units keep theirs, and `0x` stays a hex prefix.
fn x_as_times(query: &str) -> String {
    let characters: Vec<char> = query.chars().collect();
    let visible = |c: &&char| !c.is_whitespace();
    characters
        .iter()
        .enumerate()
        .map(|(index, &character)| {
            if !matches!(character, 'x' | 'X') {
                return character;
            }
            let (Some(preceding), Some(following)) =
                (characters.get(..index), characters.get(index + 1..))
            else {
                return character;
            };
            let hex_prefix = match preceding {
                ['0'] => true,
                [.., previous, '0'] => !previous.is_ascii_alphanumeric() && *previous != '.',
                _ => false,
            };
            let multiplies = preceding
                .iter()
                .rev()
                .find(visible)
                .is_some_and(|c| c.is_ascii_digit() || *c == ')')
                && following
                    .iter()
                    .find(visible)
                    .is_some_and(|c| c.is_ascii_digit() || matches!(c, '(' | '.'));
            if multiplies && !hex_prefix {
                '×'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(query: &str) -> Vec<Item> {
        Calc::new().expect("prelude loads").query(query)
    }

    fn titles(query: &str) -> Vec<String> {
        items(query).into_iter().map(|item| item.title).collect()
    }

    fn detail(query: &str) -> Vec<(String, String)> {
        items(query)
            .into_iter()
            .flat_map(|item| item.detail)
            .map(|section| (section.title, section.text))
            .collect()
    }

    #[test]
    fn detail_shows_the_reading_and_full_precision() {
        assert_eq!(
            detail("100 N to lbf"),
            [
                ("Input".into(), "100 newton ➞ pound_force".into()),
                ("Full precision".into(), "22.48089236553391 lbf".into()),
            ]
        );
        assert_eq!(
            detail("2x1000000"),
            [("Input".into(), "2 × 1_000_000".into())],
            "precision matching what ↵ copies adds nothing"
        );
    }

    #[test]
    fn arithmetic() {
        assert_eq!(titles("2 + 3 * 4"), ["14"]);
    }

    #[test]
    fn copies_large_numbers_without_separators() {
        assert_eq!(
            items("2 x 1000000 m")
                .into_iter()
                .map(|item| (item.title, item.action))
                .collect::<Vec<_>>(),
            [("2_000_000 m".into(), Action::Copy("2000000 m".into()))]
        );
    }

    #[test]
    fn dates_read_as_dates() {
        assert_eq!(
            titles("datetime(\"2026-01-02 03:04:05 UTC\")"),
            ["2026-01-02 03:04:05 UTC"]
        );
    }

    #[test]
    fn x_multiplies_between_numbers() {
        assert_eq!(titles("2 x 2"), ["4"]);
        assert_eq!(titles("3x4"), ["12"]);
        assert_eq!(titles("10 X 1.5"), ["15"]);
        assert_eq!(titles("(1 + 2) x (3 + 4)"), ["21"]);
        assert_eq!(titles("2 x 3 km"), ["6 km"]);
    }

    #[test]
    fn x_keeps_other_meanings() {
        assert_eq!(titles("0x10"), ["16"]);
        assert_eq!(titles("0x10 x 2"), ["32"]);
        assert_eq!(titles("let x = 3"), Vec::<String>::new());
        assert_eq!(titles("2 lux"), ["2 lx"]);
    }

    #[test]
    fn units() {
        assert_eq!(titles("5 km to m"), ["5000 m"]);
        assert_eq!(titles("10 kg -> lb")[0].split(' ').nth(1), Some("lb"));
        assert!(titles("100 N to lbf")[0].ends_with("lbf"));
    }

    #[test]
    fn currency() {
        Context::use_test_exchange_rates();
        assert!(
            titles("100 USD to EUR")[0].ends_with('€'),
            "{:?}",
            titles("100 USD to EUR")
        );
    }

    #[test]
    fn ignores_words_and_errors() {
        assert!(titles("firefox").is_empty());
        assert!(titles("2 +").is_empty());
        assert!(titles("3 m + 2 s").is_empty());
    }
}
