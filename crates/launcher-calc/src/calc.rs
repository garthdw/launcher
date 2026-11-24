mod rates;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use jiff::civil::Date;
use launcher_core::{Action, Item, Plugin, ResultsChanged, Section, log};
use numbat::markup::plain_text_format;
use numbat::module_importer::{BuiltinModuleImporter, ModuleImporter};
use numbat::pretty_print::PrettyPrint;
use numbat::resolver::{CodeSource, ModulePath};
use numbat::value::Value;
use numbat::{Context, FormatOptions, InterpreterResult, NumbatError};

/// How often to check whether the cached rates are a day old. Hourly rather than sleeping a
/// day, since a sleeping Mac doesn't advance a thread's sleep.
const RATES_CHECK: Duration = Duration::from_secs(60 * 60);
/// How soon to try again while no rates have loaded, as at login before the network is up.
const RATES_RETRY: Duration = Duration::from_secs(60);
/// A query with a number is almost surely a calculation. Only an app whose name starts with it,
/// as "1Password" does with "1pass", ranks higher.
const CALCULATION: u8 = 90;
/// A word such as `pi` may as well be the start of an app's name: below an app with a word
/// starting with it (Pixelmator), above looser matches (Preview).
const CONSTANT: u8 = 80;

pub struct Calc {
    context: Context,
    rates: Rates,
    /// The newest update not yet taken. One slot rather than a queue, so contexts don't pile up
    /// while no query comes to take them.
    pending: Arc<Mutex<Option<Update>>>,
}

/// What the calculator has of exchange rates.
enum Rates {
    Loading,
    Unavailable,
    Loaded(Date),
}

/// From the thread that keeps exchange rates current.
enum Update {
    /// A context with currencies at the rates of the date.
    Loaded(Box<Context>, Date),
    /// No rates could be fetched or found in the cache.
    Unavailable,
}

impl Calc {
    /// A calculator without exchange rates: currencies say rates are loading.
    pub fn new() -> Result<Self, Box<NumbatError>> {
        Ok(Self {
            context: context(false)?,
            rates: Rates::Loading,
            pending: Arc::default(),
        })
    }

    /// Loads exchange rates in the background, from the cache in `directory` or the ECB, and
    /// keeps them a day old at most, calling `changed` whenever currencies change.
    pub fn start(directory: PathBuf, changed: ResultsChanged) -> Result<Self, Box<NumbatError>> {
        // Rates under a day old load now, so launch builds one context rather than two.
        let calc = match rates::fresh(&directory) {
            Some((xml, date)) => {
                Context::set_exchange_rates(&xml);
                Self {
                    context: context(true)?,
                    rates: Rates::Loaded(date),
                    pending: Arc::default(),
                }
            }
            None => Self::new()?,
        };
        let loaded = match calc.rates {
            Rates::Loaded(date) => Some(date),
            Rates::Loading | Rates::Unavailable => None,
        };
        let pending = Arc::downgrade(&calc.pending);
        thread::spawn(move || keep_rates_current(&directory, loaded, &pending, &*changed));
        Ok(calc)
    }

    fn take_update(&mut self) {
        let update = match self.pending.lock() {
            Ok(mut pending) => pending.take(),
            Err(_) => {
                log!("the exchange rates thread panicked; keeping the rates there are");
                None
            }
        };
        match update {
            Some(Update::Loaded(context, date)) => {
                self.context = *context;
                self.rates = Rates::Loaded(date);
            }
            Some(Update::Unavailable) => self.rates = Rates::Unavailable,
            None => {}
        }
    }

    /// Says why a query that failed for want of rates has no result.
    fn rates_missing(&self, query: &str) -> Option<Item> {
        let title = match self.rates {
            Rates::Loading => "Loading exchange rates…",
            Rates::Unavailable => "Exchange rates unavailable",
            Rates::Loaded(_) => return None,
        };
        mentions_currency(query).then(|| Item {
            title: title.into(),
            subtitle: String::new(),
            action: Action::Nothing,
            detail: Vec::new(),
            relevance: CALCULATION,
        })
    }
}

/// A prelude context, with currencies at the rates numbat holds when `currencies` is set.
/// numbat's on-demand currency loading stays off: before rates load it would find none, and
/// after, the module loads here, off the UI thread.
fn context(currencies: bool) -> Result<Context, Box<NumbatError>> {
    let mut context = Context::new(BuiltinModuleImporter::default());
    context
        .interpret("use prelude", CodeSource::Internal)
        .map(drop)?;
    if currencies {
        context
            .interpret("use units::currencies", CodeSource::Internal)
            .map(drop)?;
    }
    Ok(context)
}

/// Leaves `update` for the calculator to take, replacing any it hasn't.
fn publish(pending: &Mutex<Option<Update>>, update: Update) {
    match pending.lock() {
        Ok(mut pending) => *pending = Some(update),
        Err(_) => log!("the calculator panicked; not updating exchange rates"),
    }
}

/// Runs until the calculator, which holds `pending`, is gone. `loaded` is the date of the rates
/// it started with, if any.
fn keep_rates_current(
    directory: &Path,
    mut loaded: Option<Date>,
    pending: &std::sync::Weak<Mutex<Option<Update>>>,
    changed: &dyn Fn(),
) {
    let mut told_unavailable = false;
    loop {
        // Before fetching, and again before building a context, as fetching takes a while: the
        // calculator may have gone, as when settings turn it off.
        if pending.strong_count() == 0 {
            return;
        }
        let current = rates::current(directory);
        if pending.strong_count() == 0 {
            return;
        }
        let update = match current {
            Some((xml, date)) if loaded != Some(date) => {
                let started = Instant::now();
                Context::set_exchange_rates(&xml);
                match context(true) {
                    Ok(context) => {
                        log!(
                            "exchange rates of {date} loaded in {} ms",
                            started.elapsed().as_millis()
                        );
                        loaded = Some(date);
                        Some(Update::Loaded(Box::new(context), date))
                    }
                    Err(error) => {
                        log!("could not load exchange rates: {error}");
                        None
                    }
                }
            }
            // Only before any loaded: older rates beat none.
            None if loaded.is_none() && !told_unavailable => {
                told_unavailable = true;
                Some(Update::Unavailable)
            }
            _ => None,
        };
        if let Some(update) = update {
            let Some(pending) = pending.upgrade() else {
                return;
            };
            publish(&pending, update);
            changed();
        }
        thread::sleep(if loaded.is_some() {
            RATES_CHECK
        } else {
            RATES_RETRY
        });
    }
}

/// The names numbat's currency module gives currencies: units, their aliases and symbols.
/// Read once, when a query first needs them.
static CURRENCY_NAMES: LazyLock<Vec<String>> = LazyLock::new(currency_names);

fn currency_names() -> Vec<String> {
    let path = ModulePath(vec!["units".into(), "currencies".into()]);
    let Some((module, _)) = BuiltinModuleImporter::default().import(&path) else {
        log!("numbat has no currency module");
        return Vec::new();
    };
    module
        .lines()
        .map(str::trim)
        .flat_map(|line| {
            let names = if let Some(aliases) = line.strip_prefix("@aliases(") {
                aliases.trim_end_matches(')')
            } else if let Some(unit) = line.strip_prefix("unit ") {
                unit.split(':').next().unwrap_or_default()
            } else {
                ""
            };
            names
                .split(',')
                .filter_map(|name| name.split(':').next())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether `query` names a currency other than the euro, whose rates must load first.
fn mentions_currency(query: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    CURRENCY_NAMES.iter().any(|name| {
        if name.chars().all(is_word) {
            // "100USD" is a number and a currency to numbat.
            query
                .split(|c| !is_word(c))
                .any(|word| word.trim_start_matches(|c: char| c.is_ascii_digit()) == name)
        } else {
            query.contains(name.as_str())
        }
    })
}

/// Whether `value` is in money, whose base unit is the euro.
fn is_money(value: &Value) -> bool {
    let Value::Quantity(quantity) = value else {
        return false;
    };
    quantity
        .unit()
        .to_base_unit_representation()
        .0
        .iter()
        .any(|factor| factor.unit_id.name == "euro")
}

impl Plugin for Calc {
    fn name(&self) -> &'static str {
        "Calculator"
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        self.take_update();
        let has_number = query.chars().any(|c| c.is_ascii_digit());
        // A lone letter such as `e` is more likely the start of an app's name than a constant.
        if !has_number && query.chars().nth(1).is_none() {
            return Vec::new();
        }
        // Imports and definitions stay in the context for good: `let x = 3` would leave `x` in
        // later answers, before rates load the currency module would keep NaN rates, and
        // `use all` would hold every module in memory. `ans` and `_` name the last result,
        // which every keystroke replaces, so they'd answer with a half-typed query's.
        if query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| {
                matches!(
                    word,
                    "use" | "let" | "fn" | "unit" | "dimension" | "struct" | "ans" | "_"
                )
            })
        {
            return Vec::new();
        }
        let rewritten = x_as_times(query);
        let Ok((statements, InterpreterResult::Value(value))) =
            self.context.interpret(&rewritten, CodeSource::Text)
        else {
            if !has_number {
                return Vec::new();
            }
            return self.rates_missing(query).into_iter().collect();
        };
        // Bare words such as an app name are often valid numbat ("m", "day"), but echoing them
        // back as "1 m" is noise in a launcher. Only a plain number, as `pi` is, says something.
        let relevance = match &value {
            _ if has_number => CALCULATION,
            Value::Quantity(quantity) if quantity.unit().is_scalar() => CONSTANT,
            _ => return Vec::new(),
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
        let subtitle = match self.rates {
            Rates::Loaded(date) if is_money(&value) => {
                format!("ECB rates of {}", date.strftime("%-d %b"))
            }
            _ => String::new(),
        };
        vec![Item {
            title: plain_text(&value, &FormatOptions::default()),
            subtitle,
            action: Action::Copy(copied),
            detail,
            relevance,
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
    use crate::rates::RATES_FILE;
    use crate::rates::tests::ECB_XML;

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

    /// numbat holds exchange rates process-wide, so tests that set them take turns.
    static RATES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn shown(calc: &mut Calc, query: &str) -> Vec<(String, String)> {
        calc.query(query)
            .into_iter()
            .map(|item| (item.title, item.subtitle))
            .collect()
    }

    #[test]
    fn currency_before_rates_load_says_so() -> Result<(), Box<NumbatError>> {
        let mut calc = Calc::new()?;
        assert_eq!(
            calc.query("100 USD to EUR"),
            [Item {
                title: "Loading exchange rates…".into(),
                subtitle: String::new(),
                action: Action::Nothing,
                detail: Vec::new(),
                relevance: CALCULATION,
            }]
        );
        assert_eq!(
            calc.query("£20 to EUR").len(),
            1,
            "symbols are currencies too"
        );
        assert_eq!(
            calc.query("100USD to EUR").len(),
            1,
            "a code may follow the number"
        );
        assert_eq!(
            titles("100 EUR + 5 €"),
            ["105 €"],
            "the euro needs no rates"
        );
        assert!(
            titles("100 parsecs + 2 usd2").is_empty(),
            "nor is every error one"
        );
        publish(&calc.pending, Update::Unavailable);
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("Exchange rates unavailable".into(), String::new())]
        );
        Ok(())
    }

    #[test]
    fn fresh_cached_rates_load_at_start() -> Result<(), Box<dyn std::error::Error>> {
        let _turn = RATES.lock();
        let cache = tempfile::tempdir()?;
        std::fs::write(cache.path().join(RATES_FILE), ECB_XML)?;
        let mut calc = Calc::start(cache.path().to_path_buf(), Arc::new(|| {}))?;
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("80 €".into(), "ECB rates of 21 Nov".into())]
        );
        assert_eq!(
            shown(&mut calc, "2 m"),
            [("2 m".into(), String::new())],
            "only money is dated"
        );
        Ok(())
    }

    #[test]
    fn replaced_rates_reach_a_new_context() -> Result<(), Box<dyn std::error::Error>> {
        let _turn = RATES.lock();
        let date = jiff::civil::date(2026, 11, 21);
        Context::set_exchange_rates(ECB_XML);
        let mut calc = Calc::new()?;
        publish(
            &calc.pending,
            Update::Loaded(Box::new(context(true)?), date),
        );
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("80 €".into(), "ECB rates of 21 Nov".into())]
        );
        Context::set_exchange_rates(&ECB_XML.replace("1.25", "2"));
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("80 €".into(), "ECB rates of 21 Nov".into())],
            "a context keeps the rates it loaded"
        );
        let outdated = context(false)?;
        publish(&calc.pending, Update::Loaded(Box::new(outdated), date));
        publish(
            &calc.pending,
            Update::Loaded(Box::new(context(true)?), date),
        );
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("50 €".into(), "ECB rates of 21 Nov".into())],
            "the newest update replaces one not yet taken"
        );
        Ok(())
    }

    #[test]
    fn currency_names_follow_numbats_module() {
        let mut codes: Vec<String> = currency_names()
            .into_iter()
            .filter(|name| name.len() == 3 && name.chars().all(|c| c.is_ascii_uppercase()))
            .collect();
        codes.sort();
        assert_eq!(
            codes,
            [
                "AUD", "BGN", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "GBP", "HKD", "HUF", "IDR",
                "ILS", "INR", "ISK", "JPY", "KRW", "MYR", "NIS", "NOK", "NZD", "PHP", "PLN", "RON",
                "SEK", "SGD", "THB", "TRY", "USD", "ZAR",
            ],
            "numbat's currencies changed: check `mentions_currency` still finds them"
        );
        assert!(currency_names().contains(&"$".to_owned()));
        assert!(currency_names().contains(&"dollar".to_owned()));
    }

    #[test]
    fn ignores_imports() {
        assert!(titles("use units::currencies; 1").is_empty());
        assert!(titles("2; use all").is_empty());
    }

    #[test]
    fn words_answer_only_with_a_plain_number() {
        let answer = |query| {
            items(query)
                .into_iter()
                .map(|item| (item.title, item.relevance))
                .collect::<Vec<_>>()
        };
        assert_eq!(answer("pi"), [("3.14159".into(), CONSTANT)]);
        assert_eq!(answer("e"), [], "a lone letter is the start of a word");
        assert_eq!(answer("e*e"), [("7.38906".into(), CONSTANT)]);
        assert_eq!(answer("e * 2"), [("5.43656".into(), CALCULATION)]);
        assert_eq!(answer("2e"), [("5.43656".into(), CALCULATION)]);
        assert_eq!(answer("pi * e"), [("8.53973".into(), CONSTANT)]);
        assert_eq!(answer("2 pi"), [("6.28319".into(), CALCULATION)]);
        for word in [
            "m", "day", "c", "kg to lb", "dollar", "true", "now()", "firefox",
        ] {
            assert_eq!(answer(word), [], "{word}");
        }
    }

    #[test]
    fn nothing_outlives_its_query() -> Result<(), Box<NumbatError>> {
        let mut calc = Calc::new()?;
        for definition in [
            "let x = 3",
            "fn double(y) = 2 y",
            "unit widget = 3 m",
            "dimension Foo",
            "struct Point2 { x: Scalar }",
        ] {
            calc.query(definition);
        }
        for uses in ["x + 1", "double(2)", "2 widget", "1 x", "x"] {
            assert_eq!(shown(&mut calc, uses), [], "{uses}");
        }
        calc.query("2 + 3");
        for earlier in ["ans", "_", "ans + 1", "_ * 2"] {
            assert_eq!(shown(&mut calc, earlier), [], "{earlier}");
        }
        Ok(())
    }

    #[test]
    fn ignores_words_and_errors() {
        assert!(titles("firefox").is_empty());
        assert!(titles("2 +").is_empty());
        assert!(titles("3 m + 2 s").is_empty());
    }
}
