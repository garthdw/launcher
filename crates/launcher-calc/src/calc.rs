mod expression;
mod rates;
mod units;

use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use jiff::civil::Date;
use launcher_core::{Action, Icon, Item, Plugin, ResultsChanged, Section, log};

use expression::{Answer, Environment};
use units::{Error, Money, Rates};

pub const NAME: &str = "Calculator";
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
const SIGNIFICANT_DIGITS: i32 = 6;
const DATE_FORMAT: &str = "%A, %-d %B %Y";

pub struct Calc {
    rates: Arc<Mutex<ExchangeRates>>,
}

/// What the calculator has of exchange rates.
enum ExchangeRates {
    Loading,
    Unavailable,
    Loaded(Box<Rates>, Date),
}

/// Only the thread keeping rates current writes them, assigning whole values, so a panic while
/// they're locked can't leave them half-written: they're used even poisoned.
fn lock(rates: &Mutex<ExchangeRates>) -> MutexGuard<'_, ExchangeRates> {
    rates.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for Calc {
    fn default() -> Self {
        Self::new()
    }
}

impl Calc {
    /// A calculator without exchange rates: currencies say rates are loading.
    pub fn new() -> Self {
        Self {
            rates: Arc::new(Mutex::new(ExchangeRates::Loading)),
        }
    }

    /// Loads exchange rates in the background, from the cache in `directory` or the ECB, and
    /// keeps them a day old at most. Calls `changed` whenever the answers change.
    pub fn start(directory: PathBuf, changed: ResultsChanged) -> Self {
        let calc = Self::new();
        let rates = Arc::downgrade(&calc.rates);
        thread::spawn(move || keep_rates_current(&directory, &rates, &*changed));
        calc
    }
}

/// Assigns `update` and says the answers changed; false once the calculator is gone.
fn deliver(rates: &Weak<Mutex<ExchangeRates>>, changed: &dyn Fn(), update: ExchangeRates) -> bool {
    let Some(rates) = rates.upgrade() else {
        return false;
    };
    *lock(&rates) = update;
    changed();
    true
}

/// Runs until the calculator, which holds `rates`, is gone.
fn keep_rates_current(directory: &Path, rates: &Weak<Mutex<ExchangeRates>>, changed: &dyn Fn()) {
    let mut loaded = None;
    let mut told_unavailable = false;
    loop {
        // The calculator may have gone while fetching, as when settings turn it off.
        if rates.strong_count() == 0 {
            return;
        }
        let delivered = match rates::current(directory) {
            Some((current, date)) if loaded != Some(date) => {
                log!("exchange rates of {date} loaded");
                loaded = Some(date);
                deliver(
                    rates,
                    changed,
                    ExchangeRates::Loaded(Box::new(current), date),
                )
            }
            // Only before any loaded: older rates beat none.
            None if loaded.is_none() && !told_unavailable => {
                told_unavailable = true;
                deliver(rates, changed, ExchangeRates::Unavailable)
            }
            _ => true,
        };
        if !delivered {
            return;
        }
        thread::sleep(if loaded.is_some() {
            RATES_CHECK
        } else {
            RATES_RETRY
        });
    }
}

/// A muted row saying why a calculation has no answer.
fn notice(title: &str) -> Item {
    Item::notice(title, "", Icon::Calculator, CALCULATION)
}

impl Plugin for Calc {
    fn name(&self) -> &'static str {
        NAME
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        let has_number = query.chars().any(|character| character.is_ascii_digit());
        // A lone letter such as `e` is more likely the start of an app's name than a constant.
        if !has_number && query.chars().nth(1).is_none() {
            return Vec::new();
        }
        // Held for the query; the thread keeping rates current writes them once a day.
        let exchange_rates = lock(&self.rates);
        let rates = match &*exchange_rates {
            ExchangeRates::Loaded(rates, _) => Some(&**rates),
            ExchangeRates::Loading | ExchangeRates::Unavailable => None,
        };
        let evaluate = |reading: &expression::Reading, newton_metres| {
            reading.evaluate(&Environment {
                rates,
                newton_metres,
                now: OnceCell::new(),
            })
        };
        let answer = expression::read(query).and_then(|reading| {
            let answer = match evaluate(&reading, false) {
                // `nm` is nanometres, unless that leaves the query meaningless, as in
                // `100 nm to ftlb`.
                Err(Error::Invalid) if query.contains("nm") => evaluate(&reading, true),
                answer => answer,
            };
            Ok((reading, answer?))
        });
        let (reading, answer) = match answer {
            Ok(answer) => answer,
            Err(Error::RatesMissing) if has_number => {
                return vec![notice(match *exchange_rates {
                    ExchangeRates::Unavailable => "Exchange rates unavailable",
                    _ => "Loading exchange rates…",
                })];
            }
            Err(_) => return Vec::new(),
        };
        // Bare words such as an app name are often units ("m", "day"), but echoing them back as
        // "1 m" is noise in a launcher. Only a plain number, as `pi` is, says something.
        let relevance = match &answer {
            _ if has_number => CALCULATION,
            Answer::Quantity(quantity) if quantity.is_plain() => CONSTANT,
            _ => return Vec::new(),
        };
        let mut subtitle = String::new();
        let (title, copied, precise) = match answer {
            Answer::Text(text) => (text.clone(), text.clone(), text),
            Answer::Date(date) => {
                let iso = date.to_string();
                (date.strftime(DATE_FORMAT).to_string(), iso.clone(), iso)
            }
            Answer::Time(time) => (
                time.strftime(&format!("{DATE_FORMAT}, %H:%M")).to_string(),
                time.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
                time.strftime("%Y-%m-%d %H:%M:%S %Z (%:z)").to_string(),
            ),
            Answer::Quantity(quantity) => {
                if let ExchangeRates::Loaded(_, date) = &*exchange_rates
                    && quantity.is_money()
                {
                    subtitle = format!("ECB rates of {}", date.strftime("%-d %b"));
                }
                if let Some(money) = quantity.as_money() {
                    amounts(quantity.magnitude, &money)
                } else {
                    let unit = if quantity.is_plain() {
                        String::new()
                    } else {
                        format!(" {}", quantity.unit_name())
                    };
                    let shown = rounded(quantity.magnitude);
                    (
                        format!("{shown}{unit}"),
                        // Digit separators read well but break pasting into most other apps.
                        format!("{}{unit}", shown.replace('_', "")),
                        format!("{}{unit}", precise(quantity.magnitude)),
                    )
                }
            }
        };
        let mut detail = vec![Section {
            title: "Input".into(),
            text: reading.to_string(),
        }];
        if precise != copied {
            detail.push(Section {
                title: "Full precision".into(),
                text: precise,
            });
        }
        vec![Item {
            title,
            subtitle,
            icon: Icon::Calculator,
            action: Action::Copy(copied),
            detail,
            relevance,
        }]
    }
}

/// An amount as shown, copied and to full precision: to the currency's smallest unit, as
/// `€9.60`, `¥1600` or `0.95 CHF`.
fn amounts(value: f64, money: &Money) -> (String, String, String) {
    let scale = 10f64.powi(money.decimals as i32);
    let rounded = (value * scale).round() / scale;
    // So an amount that rounds to nothing isn't `-€0.00`.
    let sign = if rounded < 0.0 { "-" } else { "" };
    let written = |number: &str| {
        if money.symbol_first {
            format!("{sign}{}{number}", money.symbol)
        } else {
            format!("{sign}{number} {}", money.symbol)
        }
    };
    let fixed = format!("{:.*}", money.decimals, rounded.abs());
    let (integer, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let point = if fraction.is_empty() { "" } else { "." };
    let grouped = if integer.len() >= 5 {
        group_thousands(integer)
    } else {
        integer.to_owned()
    };
    (
        written(&format!("{grouped}{point}{fraction}")),
        written(&fixed),
        written(&precise(value.abs())),
    )
}

/// Integers below this show whole, as `2^40` does: f64 holds them exactly.
const WHOLE_BELOW: f64 = 1e15;

/// `value` to `SIGNIFICANT_DIGITS`, or whole if it's an integer, with `_` between groups of
/// thousands in numbers of five digits or more: `2_000_000`, `0.333333`, `9.46073e12`.
fn rounded(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let scientific = format!("{:.*e}", (SIGNIFICANT_DIGITS - 1) as usize, value);
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    let whole = value.fract() == 0.0 && value.abs() < WHOLE_BELOW;
    if !whole && !(-5..SIGNIFICANT_DIGITS).contains(&exponent) {
        return format!("{}e{exponent}", trim_fraction(mantissa));
    }
    let decimals = (SIGNIFICANT_DIGITS - 1 - exponent).max(0) as usize;
    let fixed = format!("{value:.decimals$}");
    let fixed = trim_fraction(&fixed);
    let (sign, unsigned) = match fixed.strip_prefix('-') {
        Some(unsigned) => ("-", unsigned),
        None => ("", fixed),
    };
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let integer = if integer.len() >= 5 {
        group_thousands(integer)
    } else {
        integer.to_owned()
    };
    let point = if fraction.is_empty() { "" } else { "." };
    format!("{sign}{integer}{point}{fraction}")
}

fn trim_fraction(number: &str) -> &str {
    if number.contains('.') {
        number.trim_end_matches('0').trim_end_matches('.')
    } else {
        number
    }
}

fn group_thousands(digits: &str) -> String {
    let mut grouped = String::with_capacity(digits.len() * 2);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push('_');
        }
        grouped.push(digit);
    }
    grouped
}

/// `value` as precisely as an f64 holds it: the shortest text that reads back as it, in
/// scientific notation when very large or small.
fn precise(value: f64) -> String {
    let exponent = value.abs().log10().floor();
    if value != 0.0 && exponent.is_finite() && !(-5.0..15.0).contains(&exponent) {
        format!("{value:e}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rates::RATES_FILE;
    use crate::rates::tests::ECB_XML;

    fn with_rates() -> Calc {
        let calc = Calc::new();
        if let Some((rates, date)) = rates::parse(ECB_XML) {
            *lock(&calc.rates) = ExchangeRates::Loaded(Box::new(rates), date);
        }
        calc
    }

    fn titles(query: &str) -> Vec<String> {
        with_rates()
            .query(query)
            .into_iter()
            .map(|item| item.title)
            .collect()
    }

    fn title(query: &str) -> String {
        titles(query).join(" / ")
    }

    fn answers(queries: &[(&str, &str)]) {
        for (query, expected) in queries {
            assert_eq!(title(query), *expected, "{query}");
        }
    }

    fn shown(calc: &mut Calc, query: &str) -> Vec<(String, String)> {
        calc.query(query)
            .into_iter()
            .map(|item| (item.title, item.subtitle))
            .collect()
    }

    #[test]
    fn arithmetic() {
        answers(&[
            ("2 + 3 * 4", "14"),
            ("(1 + 2) x (3 + 4)", "21"),
            ("2 x 2", "4"),
            ("3x4", "12"),
            ("10 X 1.5", "15"),
            ("2x1000000", "2_000_000"),
            ("-2^2", "-4"),
            ("2^-1", "0.5"),
            ("2^3^2", "512"),
            ("1/3", "0.333333"),
            ("10!", "3_628_800"),
            ("50%", "0.5"),
            ("15% of 80", "12"),
            ("2^1000", "1.07151e301"),
            ("1e100", "1e100"),
            ("0.1 + 0.2", "0.3"),
            ("1/0", ""),
            ("6 ÷ 4", "1.5"),
            ("2 ** 10", "1024"),
        ]);
    }

    #[test]
    fn bases() {
        answers(&[
            ("0x10", "16"),
            ("0x10 x 2", "32"),
            ("0xff + 0b101", "260"),
            ("0o17", "15"),
            ("255 to hex", "0xff"),
            ("2^70 to binary", &format!("0b1{}", "0".repeat(70))),
            ("2^70 -> bin", &format!("0b1{}", "0".repeat(70))),
            ("1.5 to hex", ""),
        ]);
    }

    #[test]
    fn functions_and_constants() {
        answers(&[
            ("sqrt(2)*pi", "4.44288"),
            ("sqrt(16 m^2)", "4 m"),
            ("ln(2)", "0.693147"),
            ("log10(1000)", "3"),
            ("sin(30 deg)", "0.5"),
            ("cos(pi)", "-1"),
            ("2 pi", "6.28319"),
            ("2e", "5.43656"),
            ("e*e", "7.38906"),
            ("1 c to km/s", "299_792 km/s"),
        ]);
    }

    #[test]
    fn units() {
        answers(&[
            ("5 km to m", "5000 m"),
            ("2 x 3 km", "6 km"),
            ("2 x 1000000 m", "2_000_000 m"),
            ("10 kg to lb", "22.0462 lb"),
            ("10 kg -> lb", "22.0462 lb"),
            ("100 km/h to mph", "62.1371 mph"),
            ("100 km / 2 h", "50 km/h"),
            ("3 m * 2 m", "6 m²"),
            ("1 km / 1 m", "1000"),
            ("1 lightyear to km", "9.46073e12 km"),
            ("5 feet 11 inches to cm", "180.34 cm"),
            ("3 hours 20 min to min", "200 min"),
            ("5 in to cm", "12.7 cm"),
            ("5 m in cm", "500 cm"),
            ("1 acre to m^2", "4046.86 m²"),
            ("1 m²", "1 m²"),
            ("3 m² to ft²", "32.2917 ft²"),
            ("6 m² / 2 m", "3 m"),
            ("(2 m)^0", "1"),
            ("(1 farad)^40", "1 farad⁴⁰"),
            ("5 in cm", "12.7 cm"),
            ("1 m in in", "39.3701 in"),
            ("1 gal to L", "3.78541 L"),
            ("1 atm to psi", "14.6959 psi"),
            ("1 kWh to J", "3_600_000 J"),
            ("2 lux", "2 lx"),
            ("30 deg to rad", "0.523599 rad"),
        ]);
    }

    #[test]
    fn units_keep_their_case_and_nm_stays_nanometres() {
        answers(&[
            ("1 nm to m", "1e-9 m"),
            ("1 mW to W", "0.001 W"),
            ("1 MW to W", "1_000_000 W"),
            ("1 KW to W", ""),
        ]);
    }

    #[test]
    fn force_power_and_torque() {
        answers(&[
            ("100 N to lbf", "22.4809 lbf"),
            ("1 lbf to N", "4.44822 N"),
            ("1 hp to kW", "0.7457 kW"),
            ("1 metric_hp to kW", "0.735499 kW"),
            ("1 hp to kw", "0.7457 kW"),
            ("5kw to W", "5000 W"),
            ("100 lbft to Nm", "135.582 N·m"),
            ("100 lb-ft to N m", "135.582 N·m"),
            ("1 Nm / 1 s to W", "1 W"),
        ]);
        for torque in [
            "100 Nm to ftlb",
            "100 nm to ftlb",
            "100 N m to ft-lb",
            "100Nm to ft lbf",
        ] {
            assert_eq!(title(torque), "73.7562 ft·lbf", "{torque}");
        }
    }

    #[test]
    fn detail_shows_the_reading_and_full_precision() {
        let detail = |query| {
            with_rates()
                .query(query)
                .into_iter()
                .flat_map(|item| item.detail)
                .map(|section| (section.title, section.text))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            detail("100 N to lbf"),
            [
                ("Input".into(), "100 N ➞ lbf".into()),
                ("Full precision".into(), "22.48089430997105 lbf".into()),
            ]
        );
        assert_eq!(
            detail("2x1000000"),
            [("Input".into(), "2 × 1000000".into())],
            "precision matching what ↵ copies adds nothing"
        );
    }

    #[test]
    fn temperature_and_data() {
        answers(&[
            ("20 °C to °F", "68 °F"),
            ("20 C to F", "68 °F"),
            ("-40 degF to degC", "-40 °C"),
            ("0 K to celsius", "-273.15 °C"),
            ("20 °C + 5 K", "25 °C"),
            ("1 GiB to MB", "1073.74 MB"),
            ("1 MB to KiB", "976.562 KiB"),
            ("8 bit to B", "1 B"),
            ("2 KB to B", "2000 B"),
            ("1 kbps * 1 min to KB", "7.5 kB"),
            ("100 Mbit/s * 1 h to GB", "45 GB"),
        ]);
    }

    #[test]
    fn dates() {
        let today = jiff::ZonedDateTime::now().date();
        let years = |years: i64| {
            today
                .checked_add(jiff::Span::new().years(years))
                .map(|date| date.strftime(DATE_FORMAT).to_string())
                .unwrap_or_default()
        };
        let day = |days: i64| {
            today
                .checked_add(jiff::Span::new().days(days))
                .map(|date| date.strftime(DATE_FORMAT).to_string())
                .unwrap_or_default()
        };
        answers(&[
            ("today + 3 days", &day(3)),
            ("today - 1 week", &day(-7)),
            ("2 days + today", &day(2)),
            ("tomorrow + 1 day", &day(2)),
            ("today + 1 decade", &years(10)),
            ("today - 2 centuries", &years(-200)),
            ("today + 36 hours", ""),
            ("today * 2", ""),
            ("today + 3 m", ""),
            ("today + 1 day to m", ""),
        ]);
        assert_eq!(
            with_rates()
                .query("today + 1 month")
                .into_iter()
                .map(|item| item.action)
                .collect::<Vec<_>>(),
            [Action::Copy(
                today
                    .checked_add(jiff::Span::new().months(1))
                    .map(|date| date.to_string())
                    .unwrap_or_default()
            )],
            "a calendar month, copied as ISO 8601"
        );
        for query in ["now + 3 hours", "now() + 2 days", "now - 1 year"] {
            assert_eq!(titles(query).len(), 1, "{query}");
        }
        for word in ["today", "now", "now()", "tomorrow"] {
            assert_eq!(title(word), "", "{word} is a word, not a calculation");
        }
    }

    #[test]
    fn currencies() {
        answers(&[
            ("100 USD to EUR", "€80.00"),
            ("12 usd to eur", "€9.60"),
            ("£20 to EUR", "€25.00"),
            ("$5 to EUR", "€4.00"),
            ("100USD to EUR", "€80.00"),
            ("100 EUR + 5 €", "€105.00"),
            ("-5 €", "-€5.00"),
            ("1000000 USD to EUR", "€800_000.00"),
            ("10 € to ¥", "¥1600"),
            ("0.01 EUR to JPY", "¥2"),
            ("1 GBP to KRW", "1875 KRW"),
            ("0.0333 EUR to ISK", "5 ISK"),
            ("5 €/kg", "5 €/kg"),
            ("1 EUR / 1 USD", "1.25"),
            ("100 CHF to EUR", ""),
        ]);
        assert_eq!(
            shown(&mut with_rates(), "100 USD to EUR"),
            [("€80.00".into(), "ECB rates of 21 Nov".into())]
        );
        let items = with_rates().query("1000000 USD / 3 to EUR");
        assert_eq!(
            items
                .iter()
                .map(|item| (&item.title, &item.action))
                .collect::<Vec<_>>(),
            [(
                &"€266_666.67".to_owned(),
                &Action::Copy("€266666.67".into())
            )],
            "copied without separators"
        );
        assert_eq!(
            items
                .into_iter()
                .flat_map(|item| item.detail)
                .find(|section| section.title == "Full precision")
                .map(|section| section.text),
            Some("€266666.6666666667".into())
        );
        assert_eq!(
            shown(&mut with_rates(), "2 m"),
            [("2 m".into(), String::new())],
            "only money is dated"
        );
    }

    #[test]
    fn currency_before_rates_load_says_so() {
        let mut calc = Calc::new();
        assert_eq!(
            calc.query("100 USD to EUR"),
            [notice("Loading exchange rates…")]
        );
        assert_eq!(
            calc.query("£20 to EUR"),
            [notice("Loading exchange rates…")]
        );
        assert_eq!(
            calc.query("100USD to EUR"),
            [notice("Loading exchange rates…")]
        );
        assert_eq!(
            shown(&mut calc, "100 EUR + 5 €"),
            [("€105.00".into(), String::new())]
        );
        assert_eq!(calc.query("100 parsecs + 2 usd2"), []);
        *lock(&calc.rates) = ExchangeRates::Unavailable;
        assert_eq!(
            calc.query("100 USD to EUR"),
            [notice("Exchange rates unavailable")]
        );
    }

    #[test]
    fn fresh_cached_rates_load_in_the_background() -> Result<(), Box<dyn std::error::Error>> {
        let cache = tempfile::tempdir()?;
        std::fs::write(cache.path().join(RATES_FILE), ECB_XML)?;
        let (loaded, wait) = std::sync::mpsc::channel();
        let mut calc = Calc::start(
            cache.path().to_path_buf(),
            Arc::new(move || {
                loaded.send(()).ok();
            }),
        );
        wait.recv_timeout(Duration::from_secs(10))?;
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("€80.00".into(), "ECB rates of 21 Nov".into())]
        );
        Ok(())
    }

    #[test]
    fn a_panic_holding_the_rates_loses_none() {
        let mut calc = with_rates();
        let held = std::panic::catch_unwind(|| {
            let _held = calc.rates.lock();
            panic!("panicking with the rates locked");
        });
        assert!(held.is_err() && calc.rates.is_poisoned());
        assert_eq!(
            shown(&mut calc, "100 USD to EUR"),
            [("€80.00".into(), "ECB rates of 21 Nov".into())]
        );
    }

    #[test]
    fn copies_without_separators() {
        let items = with_rates().query("2 x 1000000 m");
        assert_eq!(items[0].action, Action::Copy("2000000 m".into()));
    }

    #[test]
    fn half_typed_queries_answer_nothing() {
        for query in [
            "(((",
            "2^^3",
            "0x",
            "$",
            "°",
            "sqrt(",
            "sqrt()",
            "now(",
            "1 to",
            "1 m to",
            "5 m to 3 m",
            "171!",
            "(-1)!",
            "1.5!",
            "2 m ^ 0.5",
            "10^400",
            "0b2",
            "2 +",
            "3 m + 2 s",
            "m^64 * m^64",
            "192.168.1.1",
            "10.0.0.1",
            "1.2.3",
        ] {
            assert_eq!(title(query), "", "{query}");
        }
    }

    #[test]
    fn words_answer_only_with_a_plain_number() {
        let answer = |query| {
            with_rates()
                .query(query)
                .into_iter()
                .map(|item| (item.title, item.relevance))
                .collect::<Vec<_>>()
        };
        assert_eq!(answer("pi"), [("3.14159".into(), CONSTANT)]);
        assert_eq!(answer("e"), [], "a lone letter is the start of a word");
        assert_eq!(answer("e*e"), [("7.38906".into(), CONSTANT)]);
        assert_eq!(answer("e * 2"), [("5.43656".into(), CALCULATION)]);
        assert_eq!(answer("pi * e"), [("8.53973".into(), CONSTANT)]);
        for word in [
            "m",
            "day",
            "c",
            "kg to lb",
            "dollar",
            "true",
            "firefox",
            "safari",
            "terminal",
            "visual studio code",
            "1pass",
            "x + 1",
            "let x = 3",
            "use units::currencies; 1",
        ] {
            assert_eq!(answer(word), [], "{word}");
        }
    }
}
