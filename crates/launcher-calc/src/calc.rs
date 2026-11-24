use launcher_core::{Action, Item, Plugin};
use numbat::markup::plain_text_format;
use numbat::module_importer::BuiltinModuleImporter;
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
        let Ok((_, InterpreterResult::Value(value))) =
            self.context.interpret(&query, CodeSource::Text)
        else {
            return Vec::new();
        };
        vec![Item {
            title: plain_text(&value, &FormatOptions::default()),
            subtitle: "Calculator · ↵ copies".into(),
            // Digit separators read well but break pasting into most other apps.
            action: Action::Copy(plain_text(
                &value,
                &FormatOptions {
                    digit_separator: String::new(),
                    ..FormatOptions::default()
                },
            )),
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

    fn titles(query: &str) -> Vec<String> {
        let mut calc = Calc::new().expect("prelude loads");
        calc.query(query)
            .into_iter()
            .map(|item| item.title)
            .collect()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(titles("2 + 3 * 4"), ["14"]);
    }

    #[test]
    fn copies_large_numbers_without_separators() {
        let mut calc = Calc::new().expect("prelude loads");
        let items = calc.query("2 x 1000000 m");
        assert_eq!(
            items
                .iter()
                .map(|item| (item.title.as_str(), &item.action))
                .collect::<Vec<_>>(),
            [("2_000_000 m", &Action::Copy("2000000 m".into()))]
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
