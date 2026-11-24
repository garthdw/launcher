//! ECB reference rates, cached on disk.

use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use jiff::civil::Date;

const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
pub(crate) const RATES_FILE: &str = "eurofxref-daily.xml";
/// The ECB publishes once per weekday, around 16:00 CET.
const RATES_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// The newest rates to be had, and their date: cached ones under a day old, else fetched ones,
/// else older cached ones.
pub(crate) fn current(directory: &Path) -> Option<(String, Date)> {
    let path = directory.join(RATES_FILE);
    let xml = match read_cache(&path) {
        Some((xml, true)) => xml,
        stale => match fetch() {
            Ok(xml) => {
                if let Err(error) = store(directory, &path, &xml) {
                    eprintln!("could not cache exchange rates: {error}");
                }
                xml
            }
            Err(error) => {
                eprintln!("could not fetch exchange rates: {error}");
                stale?.0
            }
        },
    };
    let date = rates_date(&xml)?;
    Some((xml, date))
}

/// Cached rates under a day old, and their date.
pub(crate) fn fresh(directory: &Path) -> Option<(String, Date)> {
    let (xml, true) = read_cache(&directory.join(RATES_FILE))? else {
        return None;
    };
    let date = rates_date(&xml)?;
    Some((xml, date))
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

fn fetch() -> Result<String, Box<dyn Error>> {
    let xml = attohttpc::get(ECB_URL)
        .timeout(Duration::from_secs(15))
        .send()?
        .error_for_status()?
        .text()?;
    if rates_date(&xml).is_none() {
        return Err(format!("no rates in the response from {ECB_URL}").into());
    }
    Ok(xml)
}

/// Caches `xml` at `path`, in `directory`. Renamed into place, so a write cut short never
/// leaves a cache that looks fresh.
fn store(directory: &Path, path: &Path, xml: &str) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    let partial = path.with_extension("partial");
    fs::write(&partial, xml)?;
    fs::rename(&partial, path)
}

/// The date of the rates in an ECB reference rates file, if it holds any rates.
fn rates_date(xml: &str) -> Option<Date> {
    if numbat_exchange_rates::parse_exchange_rates(xml).is_none_or(|rates| rates.is_empty()) {
        return None;
    }
    let (_, after) = xml.split_once("time=")?;
    after.get(1..11)?.parse().ok()
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
		</Cube>
	</Cube>
</gesmes:Envelope>";

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
    fn rejects_responses_without_rates() {
        assert_eq!(rates_date(ECB_XML), Some(jiff::civil::date(2026, 11, 21)));
        assert_eq!(rates_date("<html><body>Sign in</body></html>"), None);
        assert_eq!(rates_date("<Cube time='2026-11-21'></Cube>"), None);
    }
}
