//! ECB reference rates, cached on disk.

use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use jiff::civil::Date;
use launcher_core::log;

use crate::units::{CURRENCIES, Rates, currency_index};

const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
pub(crate) const RATES_FILE: &str = "eurofxref-daily.xml";
/// The ECB publishes once per weekday, around 16:00 CET.
const RATES_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// The newest rates to be had, and their date: cached ones under a day old, else fetched ones,
/// else older cached ones.
pub(crate) fn current(directory: &Path) -> Option<(Rates, Date)> {
    let path = directory.join(RATES_FILE);
    match read_cache(&path) {
        Some((rates, true)) => Some(rates),
        stale => match fetch() {
            Ok((xml, rates)) => {
                if let Err(error) = store(&path, &xml) {
                    log!("could not cache exchange rates: {error}");
                }
                Some(rates)
            }
            Err(error) => {
                log!("could not fetch exchange rates: {error}");
                Some(stale?.0)
            }
        },
    }
}

/// The cached rates, if the file holds any, and whether they are under a day old.
fn read_cache(path: &Path) -> Option<((Rates, Date), bool)> {
    let xml = match fs::read_to_string(path) {
        Ok(xml) => xml,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            log!("could not read {}: {error}", path.display());
            return None;
        }
    };
    let Some(rates) = parse(&xml) else {
        log!("no exchange rates in {}", path.display());
        return None;
    };
    let fresh = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < RATES_LIFETIME);
    Some((rates, fresh))
}

fn fetch() -> Result<(String, (Rates, Date)), Box<dyn Error>> {
    let xml = attohttpc::get(ECB_URL)
        .timeout(Duration::from_secs(15))
        .send()?
        .error_for_status()?
        .text()?;
    let rates = parse(&xml).ok_or_else(|| format!("no rates in the response from {ECB_URL}"))?;
    Ok((xml, rates))
}

/// Caches `xml` at `path`. Renamed into place, so a write cut short never leaves a cache that
/// looks fresh.
fn store(path: &Path, xml: &str) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let partial = path.with_extension("partial");
    fs::write(&partial, xml)?;
    fs::rename(&partial, path)
}

/// The rates in the ECB's daily reference rates, `<Cube time='2026-11-21'>` then
/// `<Cube currency='USD' rate='1.25'/>` for each currency, and their date. `None` if it has no
/// rates or no date.
pub(crate) fn parse(xml: &str) -> Option<(Rates, Date)> {
    let mut rates: Rates = [None; CURRENCIES.len()];
    for cube in xml.split("<Cube").skip(1) {
        let (Some(code), Some(rate)) = (attribute(cube, "currency"), attribute(cube, "rate"))
        else {
            continue;
        };
        if let (Some(index), Ok(rate)) = (currency_index(code), rate.parse::<f64>())
            && let Some(slot) = rates.get_mut(index)
            && rate > 0.0
        {
            *slot = Some(rate);
        }
    }
    if rates.iter().all(Option::is_none) {
        return None;
    }
    let date = attribute(xml, "time")?.parse().ok()?;
    Some((rates, date))
}

/// The value of the first attribute `name` in `text`, in either kind of quotes. Other mentions
/// of the name, as in a comment, are passed over.
fn attribute<'text>(text: &'text str, name: &str) -> Option<&'text str> {
    text.match_indices(name).find_map(|(start, _)| {
        let after = text.get(start + name.len()..)?.strip_prefix('=')?;
        let quote = after
            .chars()
            .next()
            .filter(|quote| matches!(quote, '\'' | '"'))?;
        let value = after.get(quote.len_utf8()..)?;
        Some(value.split_once(quote)?.0)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const ECB_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<gesmes:Envelope xmlns:gesmes=\"http://www.gesmes.org/xml/2002-08-01\" xmlns=\"http://www.ecb.int/vocabulary/2002-08-01/eurofxref\">
	<gesmes:subject>Reference rates</gesmes:subject>
	<Cube>
		<Cube time='2026-11-21'>
			<Cube currency='USD' rate='1.25'/>
			<Cube currency='JPY' rate='160'/>
			<Cube currency='GBP' rate='0.8'/>
			<Cube currency='KRW' rate='1500'/>
			<Cube currency='ISK' rate='150'/>
		</Cube>
	</Cube>
</gesmes:Envelope>";

    fn date(rates: Option<((Rates, Date), bool)>) -> Option<(Date, bool)> {
        rates.map(|((_, date), fresh)| (date, fresh))
    }

    #[test]
    fn cache_is_used_only_when_valid_and_says_when_stale() -> Result<(), Box<dyn std::error::Error>>
    {
        let cache = tempfile::tempdir()?;
        let path = cache.path().join(RATES_FILE);
        let rates_date = jiff::civil::date(2026, 11, 21);
        assert_eq!(date(read_cache(&path)), None, "missing");
        std::fs::write(&path, "<Cube time='2026-11")?;
        assert_eq!(
            date(read_cache(&path)),
            None,
            "a cut-short write is fetched again"
        );
        std::fs::write(&path, ECB_XML)?;
        assert_eq!(date(read_cache(&path)), Some((rates_date, true)));
        std::fs::File::options()
            .write(true)
            .open(&path)?
            .set_modified(std::time::SystemTime::now() - RATES_LIFETIME)?;
        assert_eq!(date(read_cache(&path)), Some((rates_date, false)));
        Ok(())
    }

    #[test]
    fn reads_the_rates_the_ecb_gives() {
        let Some((rates, date)) = parse(ECB_XML) else {
            panic!("no rates in {ECB_XML}");
        };
        assert_eq!(date, jiff::civil::date(2026, 11, 21));
        let rate =
            |code| currency_index(code).and_then(|index| rates.get(index).copied().flatten());
        assert_eq!(rate("USD"), Some(1.25));
        assert_eq!(rate("GBP"), Some(0.8));
        assert_eq!(rate("CHF"), None, "not in the file");
    }

    #[test]
    fn finds_the_date_after_other_mentions_of_time() {
        let commented = ECB_XML.replace("<Cube>", "<!-- published daily, time CET --><Cube>");
        assert_eq!(
            parse(&commented).map(|(_, date)| date),
            Some(jiff::civil::date(2026, 11, 21))
        );
    }

    #[test]
    fn rejects_responses_without_rates() {
        assert_eq!(parse("<html><body>Sign in</body></html>"), None);
        assert_eq!(parse("<Cube time='2026-11-21'></Cube>"), None);
        assert_eq!(
            parse("<Cube currency='USD' rate='1.25'/>"),
            None,
            "without a date"
        );
    }
}
