use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use jiff::civil::Date;
use launcher_core::{Action, Item, Plugin, Section};
use numbat::markup::plain_text_format;
use numbat::module_importer::BuiltinModuleImporter;
use numbat::pretty_print::PrettyPrint;
use numbat::resolver::CodeSource;
use numbat::value::Value;
use numbat::{Context, FormatOptions, InterpreterResult, NumbatError};

const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
const RATES_FILE: &str = "eurofxref-daily.xml";
/// The ECB publishes once per weekday, around 16:00 CET.
const RATES_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// The date of the rates numbat holds. numbat keeps them in a process-wide `OnceLock` that
/// panics when set twice, so they load once per process, and only through `use_rates`.
static RATES_DATE: OnceLock<Date> = OnceLock::new();

pub struct Calc {
    context: Context,
}

impl Calc {
    pub fn new() -> Result<Self, Box<NumbatError>> {
        let mut context = Context::new(BuiltinModuleImporter::default());
        context
            .interpret("use prelude", CodeSource::Internal)
            .map(drop)?;
        Ok(Self { context })
    }

    /// Loads the ECB rates cached in `directory` when they are under a day old. Otherwise
    /// fetches fresh ones in the background, falling back to the old cache if that fails, since
    /// rates load only once per process.
    pub fn load_exchange_rates(directory: &Path) {
        let path = directory.join(RATES_FILE);
        match read_cache(&path) {
            Some((xml, true)) => {
                use_rates(&xml);
            }
            stale => {
                let directory = directory.to_path_buf();
                std::thread::spawn(move || {
                    if let Err(error) = refresh_exchange_rates(&directory, &path) {
                        eprintln!("could not fetch exchange rates: {error}");
                        if let Some((xml, _)) = stale {
                            use_rates(&xml);
                        }
                    }
                });
            }
        }
    }
}

/// The cached rates file, if it holds rates, and whether it is under a day old.
fn read_cache(path: &Path) -> Option<(String, bool)> {
    let xml = match fs::read_to_string(path) {
        Ok(xml) => xml,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("could not read {}: {error}", path.display());
            return None;
        }
    };
    if rates_date(&xml).is_none() {
        eprintln!("no exchange rates in {}", path.display());
        return None;
    }
    let fresh = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < RATES_LIFETIME);
    Some((xml, fresh))
}

fn refresh_exchange_rates(directory: &Path, path: &Path) -> Result<(), Box<dyn Error>> {
    let xml = attohttpc::get(ECB_URL)
        .timeout(Duration::from_secs(15))
        .send()?
        .error_for_status()?
        .text()?;
    if !use_rates(&xml) {
        return Err(format!("no rates in the response from {ECB_URL}").into());
    }
    fs::create_dir_all(directory)?;
    // Renamed into place, so a write cut short never leaves a cache that looks fresh.
    let partial = path.with_extension("partial");
    fs::write(&partial, &xml)?;
    fs::rename(&partial, path)?;
    Ok(())
}

/// Hands valid rates to numbat, unless it holds some already.
fn use_rates(xml: &str) -> bool {
    let Some(date) = rates_date(xml) else {
        return false;
    };
    RATES_DATE.get_or_init(|| {
        Context::set_exchange_rates(xml);
        date
    });
    true
}

/// The date of the rates in an ECB reference rates file, if it holds any rates.
fn rates_date(xml: &str) -> Option<Date> {
    if numbat_exchange_rates::parse_exchange_rates(xml).is_none_or(|rates| rates.is_empty()) {
        return None;
    }
    let (_, after) = xml.split_once("time=")?;
    after.get(1..11)?.parse().ok()
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
    fn query(&mut self, query: &str) -> Vec<Item> {
        // Bare words such as an app name are valid numbat identifiers ("m", "day"), but echoing
        // them back as "1 m" is noise in a launcher.
        if !query.chars().any(|c| c.is_ascii_digit()) {
            return Vec::new();
        }
        // The prelude has all a launcher needs, and importing the currency module before rates
        // load would leave numbat without them for good.
        if query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| word == "use")
        {
            return Vec::new();
        }
        let rates_date = RATES_DATE.get();
        // Meeting a currency makes numbat look its rates up. Before they load, that lookup would
        // leave numbat without rates for good.
        self.context
            .load_currency_module_on_demand(rates_date.is_some());
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
        let subtitle = match rates_date {
            Some(date) if is_money(&value) => format!("ECB rates of {}", date.strftime("%-d %b")),
            _ => "Calculator".into(),
        };
        vec![Item {
            title: plain_text(&value, &FormatOptions::default()),
            subtitle,
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

    const ECB_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<gesmes:Envelope xmlns:gesmes=\"http://www.gesmes.org/xml/2002-08-01\" xmlns=\"http://www.ecb.int/vocabulary/2002-08-01/eurofxref\">
	<gesmes:subject>Reference rates</gesmes:subject>
	<Cube>
		<Cube time='2026-11-21'>
			<Cube currency='USD' rate='1.25'/>
			<Cube currency='JPY' rate='160'/>
		</Cube>
	</Cube>
</gesmes:Envelope>";

    #[test]
    fn currency_uses_cached_rates() -> Result<(), Box<dyn std::error::Error>> {
        let cache = tempfile::tempdir()?;
        std::fs::write(cache.path().join(RATES_FILE), ECB_XML)?;
        Calc::load_exchange_rates(cache.path());
        assert_eq!(
            items("100 USD to EUR")
                .into_iter()
                .map(|item| (item.title, item.subtitle))
                .collect::<Vec<_>>(),
            [("80 €".into(), "ECB rates of 21 Nov".into())]
        );
        assert_eq!(
            items("2 m")
                .into_iter()
                .map(|item| item.subtitle)
                .collect::<Vec<_>>(),
            ["Calculator"],
            "only money is dated"
        );
        Ok(())
    }

    #[test]
    fn cache_is_used_only_when_valid_and_says_when_stale() -> Result<(), Box<dyn std::error::Error>>
    {
        let cache = tempfile::tempdir()?;
        let path = cache.path().join(RATES_FILE);
        assert_eq!(read_cache(&path), None, "missing");
        std::fs::write(&path, "<Cube time='2026-11")?;
        assert_eq!(
            read_cache(&path),
            None,
            "a cut-short write is fetched again"
        );
        std::fs::write(&path, ECB_XML)?;
        assert_eq!(read_cache(&path), Some((ECB_XML.into(), true)));
        std::fs::File::options()
            .write(true)
            .open(&path)?
            .set_modified(std::time::SystemTime::now() - RATES_LIFETIME)?;
        assert_eq!(read_cache(&path), Some((ECB_XML.into(), false)));
        Ok(())
    }

    #[test]
    fn ignores_imports() {
        // Importing the currency module before rates load would leave numbat without them.
        assert!(titles("use units::currencies; 1").is_empty());
        assert!(titles("use all; 2").is_empty());
    }

    #[test]
    fn rejects_responses_without_rates() {
        assert_eq!(rates_date(ECB_XML), Some(jiff::civil::date(2026, 11, 21)));
        assert_eq!(rates_date("<html><body>Sign in</body></html>"), None);
        assert_eq!(rates_date("<Cube time='2026-11-21'></Cube>"), None);
    }

    #[test]
    fn ignores_words_and_errors() {
        assert!(titles("firefox").is_empty());
        assert!(titles("2 +").is_empty());
        assert!(titles("3 m + 2 s").is_empty());
    }
}
