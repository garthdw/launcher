//! The units and currencies the calculator knows, and quantities made of them.

use std::f64::consts::PI;

/// Exponents of length, mass, time, current, temperature, amount, luminous intensity,
/// information and money. Angles are dimensionless, as in SI.
pub(crate) type Dimension = [i32; 9];

const SCALAR: Dimension = [0; 9];
const LENGTH: Dimension = [1, 0, 0, 0, 0, 0, 0, 0, 0];
const MASS: Dimension = [0, 1, 0, 0, 0, 0, 0, 0, 0];
const TIME: Dimension = [0, 0, 1, 0, 0, 0, 0, 0, 0];
const CURRENT: Dimension = [0, 0, 0, 1, 0, 0, 0, 0, 0];
const TEMPERATURE: Dimension = [0, 0, 0, 0, 1, 0, 0, 0, 0];
const AMOUNT: Dimension = [0, 0, 0, 0, 0, 1, 0, 0, 0];
const LUMINOUS: Dimension = [0, 0, 0, 0, 0, 0, 1, 0, 0];
const INFORMATION: Dimension = [0, 0, 0, 0, 0, 0, 0, 1, 0];
const MONEY: Dimension = [0, 0, 0, 0, 0, 0, 0, 0, 1];
const SPEED: Dimension = [1, 0, -1, 0, 0, 0, 0, 0, 0];
const AREA: Dimension = [2, 0, 0, 0, 0, 0, 0, 0, 0];
const VOLUME: Dimension = [3, 0, 0, 0, 0, 0, 0, 0, 0];
const FREQUENCY: Dimension = [0, 0, -1, 0, 0, 0, 0, 0, 0];
const FORCE: Dimension = [1, 1, -2, 0, 0, 0, 0, 0, 0];
const ENERGY: Dimension = [2, 1, -2, 0, 0, 0, 0, 0, 0];
const POWER: Dimension = [2, 1, -3, 0, 0, 0, 0, 0, 0];
const PRESSURE: Dimension = [-1, 1, -2, 0, 0, 0, 0, 0, 0];
const CHARGE: Dimension = [0, 0, 1, 1, 0, 0, 0, 0, 0];
const VOLTAGE: Dimension = [2, 1, -3, -1, 0, 0, 0, 0, 0];
const RESISTANCE: Dimension = [2, 1, -3, -2, 0, 0, 0, 0, 0];
const ILLUMINANCE: Dimension = [-2, 0, 0, 0, 0, 0, 1, 0, 0];
const CAPACITANCE: Dimension = [-2, -1, 4, 2, 0, 0, 0, 0, 0];
const CONDUCTANCE: Dimension = [-2, -1, 3, 2, 0, 0, 0, 0, 0];
const MAGNETIC_FLUX: Dimension = [2, 1, -2, -1, 0, 0, 0, 0, 0];
const MAGNETIC_FIELD: Dimension = [0, 1, -2, -1, 0, 0, 0, 0, 0];
const INDUCTANCE: Dimension = [2, 1, -2, -2, 0, 0, 0, 0, 0];
/// Absorbed and equivalent dose, J/kg.
const DOSE: Dimension = [2, 0, -2, 0, 0, 0, 0, 0, 0];
const CATALYTIC_ACTIVITY: Dimension = [0, 0, -1, 0, 0, 1, 0, 0, 0];
const DATA_RATE: Dimension = [0, 0, -1, 0, 0, 0, 0, 1, 0];

const POUND: f64 = 0.45359237;
const STANDARD_GRAVITY: f64 = 9.80665;
const INCH: f64 = 0.0254;
/// The US gallon, 231 in³.
const GALLON: f64 = 3.785_411_784e-3;
const MILE: f64 = 1609.344;
const NAUTICAL_MILE: f64 = 1852.0;
const DAY: f64 = 86400.0;
/// The Gregorian calendar's average year.
const YEAR: f64 = 365.2425 * DAY;

#[derive(Clone, Copy, PartialEq)]
enum Prefixes {
    Bare,
    Metric,
    /// Metric and binary, as for bytes.
    Binary,
}

struct UnitEntry {
    /// The first is how the unit shows. Short prefixes (`k`) apply to these.
    symbols: &'static [&'static str],
    /// Long prefixes (`kilo`) apply to these.
    names: &'static [&'static str],
    dimension: Dimension,
    /// In SI base units.
    scale: f64,
    /// Added after scaling, for temperatures.
    offset: f64,
    prefixes: Prefixes,
}

const fn unit(
    symbols: &'static [&'static str],
    names: &'static [&'static str],
    dimension: Dimension,
    scale: f64,
    prefixes: Prefixes,
) -> UnitEntry {
    UnitEntry {
        symbols,
        names,
        dimension,
        scale,
        offset: 0.0,
        prefixes,
    }
}

use Prefixes::{Bare, Binary, Metric};

const UNITS: &[UnitEntry] = &[
    unit(
        &["m"],
        &["meter", "meters", "metre", "metres"],
        LENGTH,
        1.0,
        Metric,
    ),
    unit(&["ft", "foot", "feet"], &[], LENGTH, 0.3048, Bare),
    unit(&["in", "inch", "inches"], &[], LENGTH, INCH, Bare),
    unit(&["yd", "yard", "yards"], &[], LENGTH, 0.9144, Bare),
    unit(&["mi", "mile", "miles"], &[], LENGTH, MILE, Bare),
    unit(
        &["nmi", "nautical_mile", "nautical_miles"],
        &[],
        LENGTH,
        NAUTICAL_MILE,
        Bare,
    ),
    unit(
        &["au", "AU", "astronomical_unit", "astronomical_units"],
        &[],
        LENGTH,
        149_597_870_700.0,
        Bare,
    ),
    unit(
        &["ly", "lightyear", "lightyears", "light_year", "light_years"],
        &[],
        LENGTH,
        9_460_730_472_580_800.0,
        Bare,
    ),
    unit(
        &["pc"],
        &["parsec", "parsecs"],
        LENGTH,
        3.085_677_581_491_367e16,
        Metric,
    ),
    unit(&["Å", "angstrom", "angstroms"], &[], LENGTH, 1e-10, Bare),
    unit(&["mil", "mils", "thou"], &[], LENGTH, INCH / 1000.0, Bare),
    unit(&["hand", "hands"], &[], LENGTH, 4.0 * INCH, Bare),
    unit(&["fathom", "fathoms"], &[], LENGTH, 1.8288, Bare),
    unit(&["furlong", "furlongs"], &[], LENGTH, 201.168, Bare),
    unit(&["league", "leagues"], &[], LENGTH, 3.0 * MILE, Bare),
    unit(
        &["g"],
        &["gram", "grams", "gramme", "grammes"],
        MASS,
        1e-3,
        Metric,
    ),
    // A ton is metric; the US and UK tons go by name.
    unit(
        &[
            "t",
            "tonne",
            "tonnes",
            "ton",
            "tons",
            "metric_ton",
            "metric_tons",
        ],
        &[],
        MASS,
        1000.0,
        Bare,
    ),
    unit(
        &["short_ton", "short_tons"],
        &[],
        MASS,
        2000.0 * POUND,
        Bare,
    ),
    unit(&["long_ton", "long_tons"], &[], MASS, 2240.0 * POUND, Bare),
    unit(&["grain", "grains"], &[], MASS, POUND / 7000.0, Bare),
    unit(&["ct", "carat", "carats"], &[], MASS, 2e-4, Bare),
    unit(&["lb", "lbs", "pound", "pounds"], &[], MASS, POUND, Bare),
    unit(&["oz", "ounce", "ounces"], &[], MASS, POUND / 16.0, Bare),
    unit(&["st", "stone", "stones"], &[], MASS, 14.0 * POUND, Bare),
    unit(&["s", "sec"], &["second", "seconds"], TIME, 1.0, Metric),
    unit(&["min", "minute", "minutes", "mins"], &[], TIME, 60.0, Bare),
    unit(
        &["h", "hr", "hour", "hours", "hrs"],
        &[],
        TIME,
        3600.0,
        Bare,
    ),
    unit(&["day", "days", "d"], &[], TIME, DAY, Bare),
    unit(&["week", "weeks", "wk"], &[], TIME, 7.0 * DAY, Bare),
    unit(&["fortnight", "fortnights"], &[], TIME, 14.0 * DAY, Bare),
    unit(&["month", "months"], &[], TIME, YEAR / 12.0, Bare),
    unit(&["year", "years", "yr"], &[], TIME, YEAR, Bare),
    unit(&["decade", "decades"], &[], TIME, 10.0 * YEAR, Bare),
    unit(&["century", "centuries"], &[], TIME, 100.0 * YEAR, Bare),
    unit(&["millennium", "millennia"], &[], TIME, 1000.0 * YEAR, Bare),
    unit(&["mph"], &[], SPEED, MILE / 3600.0, Bare),
    unit(&["kph", "kmh"], &[], SPEED, 1.0 / 3.6, Bare),
    unit(
        &["kn", "kt", "knot", "knots"],
        &[],
        SPEED,
        NAUTICAL_MILE / 3600.0,
        Bare,
    ),
    unit(&["ha", "hectare", "hectares"], &[], AREA, 1e4, Bare),
    unit(&["acre", "acres"], &[], AREA, 4_046.856_422_4, Bare),
    unit(&["are", "ares"], &[], AREA, 100.0, Bare),
    unit(
        &["L", "l"],
        &["liter", "liters", "litre", "litres"],
        VOLUME,
        1e-3,
        Metric,
    ),
    unit(&["gal", "gallon", "gallons"], &[], VOLUME, GALLON, Bare),
    unit(
        &["imperial_gallon", "imperial_gallons"],
        &[],
        VOLUME,
        4.546_09e-3,
        Bare,
    ),
    unit(&["cc"], &[], VOLUME, 1e-6, Bare),
    // The oil barrel, 42 US gallons.
    unit(
        &["bbl", "barrel", "barrels"],
        &[],
        VOLUME,
        42.0 * GALLON,
        Bare,
    ),
    unit(&["qt", "quart", "quarts"], &[], VOLUME, GALLON / 4.0, Bare),
    unit(&["pint", "pints"], &[], VOLUME, GALLON / 8.0, Bare),
    unit(&["cup", "cups"], &[], VOLUME, GALLON / 16.0, Bare),
    unit(
        &["floz", "fl_oz", "fluid_ounce", "fluid_ounces"],
        &[],
        VOLUME,
        GALLON / 128.0,
        Bare,
    ),
    unit(
        &["tbsp", "tablespoon", "tablespoons"],
        &[],
        VOLUME,
        GALLON / 256.0,
        Bare,
    ),
    unit(
        &["tsp", "teaspoon", "teaspoons"],
        &[],
        VOLUME,
        GALLON / 768.0,
        Bare,
    ),
    unit(&["Hz"], &["hertz"], FREQUENCY, 1.0, Metric),
    unit(&["rpm"], &[], FREQUENCY, 1.0 / 60.0, Bare),
    unit(
        &["Bq"],
        &["becquerel", "becquerels"],
        FREQUENCY,
        1.0,
        Metric,
    ),
    unit(&["N"], &["newton", "newtons"], FORCE, 1.0, Metric),
    unit(
        &["lbf", "pound_force"],
        &[],
        FORCE,
        POUND * STANDARD_GRAVITY,
        Bare,
    ),
    unit(
        &["kgf", "kilogram_force"],
        &[],
        FORCE,
        STANDARD_GRAVITY,
        Bare,
    ),
    unit(&["J"], &["joule", "joules"], ENERGY, 1.0, Metric),
    unit(&["cal"], &["calorie", "calories"], ENERGY, 4.184, Metric),
    unit(
        &["Wh"],
        &["watt_hour", "watt_hours"],
        ENERGY,
        3600.0,
        Metric,
    ),
    unit(
        &["eV"],
        &["electronvolt", "electronvolts"],
        ENERGY,
        1.602_176_634e-19,
        Metric,
    ),
    unit(&["BTU", "btu"], &[], ENERGY, 1_055.055_852_62, Bare),
    unit(&["therm", "therms"], &[], ENERGY, 105_505_585.262, Bare),
    unit(&["erg", "ergs"], &[], ENERGY, 1e-7, Bare),
    unit(&["W"], &["watt", "watts"], POWER, 1.0, Metric),
    // Apparent and reactive power, watts in all but name.
    unit(&["VA"], &[], POWER, 1.0, Metric),
    unit(&["var"], &[], POWER, 1.0, Metric),
    // Mechanical horsepower: 550 ft·lbf/s.
    unit(
        &["hp", "horsepower"],
        &[],
        POWER,
        745.699_871_582_270_2,
        Bare,
    ),
    unit(&["PS", "metric_hp"], &[], POWER, 735.498_75, Bare),
    unit(&["Pa"], &["pascal", "pascals"], PRESSURE, 1.0, Metric),
    unit(&["bar"], &["bars"], PRESSURE, 1e5, Metric),
    unit(
        &["atm", "atmosphere", "atmospheres"],
        &[],
        PRESSURE,
        101_325.0,
        Bare,
    ),
    unit(
        &["psi"],
        &[],
        PRESSURE,
        POUND * STANDARD_GRAVITY / (INCH * INCH),
        Bare,
    ),
    unit(&["mmHg"], &[], PRESSURE, 133.322_387_415, Bare),
    unit(&["torr", "Torr"], &[], PRESSURE, 101_325.0 / 760.0, Bare),
    unit(&["inHg"], &[], PRESSURE, 3_386.388_640_341, Bare),
    unit(
        &["A"],
        &["ampere", "amperes", "amp", "amps"],
        CURRENT,
        1.0,
        Metric,
    ),
    unit(&["Ah"], &[], CHARGE, 3600.0, Metric),
    unit(&["V"], &["volt", "volts"], VOLTAGE, 1.0, Metric),
    unit(&["Ω"], &["ohm", "ohms"], RESISTANCE, 1.0, Metric),
    // `C` and `F` are degrees, so coulombs and farads go by name.
    unit(&["coulomb"], &["coulomb", "coulombs"], CHARGE, 1.0, Metric),
    unit(&["farad"], &["farad", "farads"], CAPACITANCE, 1.0, Metric),
    unit(&["S"], &["siemens"], CONDUCTANCE, 1.0, Metric),
    unit(&["Wb"], &["weber", "webers"], MAGNETIC_FLUX, 1.0, Metric),
    unit(&["T"], &["tesla", "teslas"], MAGNETIC_FIELD, 1.0, Metric),
    unit(&["H"], &["henry", "henries"], INDUCTANCE, 1.0, Metric),
    unit(&["Gy"], &["gray", "grays"], DOSE, 1.0, Metric),
    unit(&["Sv"], &["sievert", "sieverts"], DOSE, 1.0, Metric),
    unit(
        &["kat"],
        &["katal", "katals"],
        CATALYTIC_ACTIVITY,
        1.0,
        Metric,
    ),
    unit(&["mol"], &["mole", "moles"], AMOUNT, 1.0, Metric),
    unit(&["cd"], &["candela"], LUMINOUS, 1.0, Metric),
    unit(&["lx"], &["lux"], ILLUMINANCE, 1.0, Metric),
    // Candela steradians; steradians are dimensionless, as radians are.
    unit(&["lm"], &["lumen", "lumens"], LUMINOUS, 1.0, Metric),
    unit(&["sr"], &["steradian", "steradians"], SCALAR, 1.0, Bare),
    unit(&["B"], &["byte", "bytes"], INFORMATION, 8.0, Binary),
    unit(&["bit"], &["bits"], INFORMATION, 1.0, Binary),
    unit(&["bps"], &[], DATA_RATE, 1.0, Metric),
    unit(&["rad"], &["radian", "radians"], SCALAR, 1.0, Metric),
    unit(
        &["°", "deg", "degree", "degrees"],
        &[],
        SCALAR,
        PI / 180.0,
        Bare,
    ),
    unit(
        &["arcmin", "arcminute", "arcminutes"],
        &[],
        SCALAR,
        PI / 180.0 / 60.0,
        Bare,
    ),
    unit(
        &["arcsec", "arcsecond", "arcseconds"],
        &[],
        SCALAR,
        PI / 180.0 / 3600.0,
        Bare,
    ),
    unit(
        &["gon", "gradian", "gradians"],
        &[],
        SCALAR,
        PI / 200.0,
        Bare,
    ),
    unit(
        &["turn", "turns", "rev", "revolution", "revolutions"],
        &[],
        SCALAR,
        2.0 * PI,
        Bare,
    ),
    unit(&["percent"], &[], SCALAR, 0.01, Bare),
    unit(&["ppm"], &[], SCALAR, 1e-6, Bare),
    unit(&["dozen"], &[], SCALAR, 12.0, Bare),
    unit(&["K"], &["kelvin"], TEMPERATURE, 1.0, Metric),
    UnitEntry {
        symbols: &["°C", "degC", "celsius", "C"],
        names: &[],
        dimension: TEMPERATURE,
        scale: 1.0,
        offset: 273.15,
        prefixes: Bare,
    },
    unit(
        &["°R", "degR", "rankine"],
        &[],
        TEMPERATURE,
        5.0 / 9.0,
        Bare,
    ),
    UnitEntry {
        symbols: &["°F", "degF", "fahrenheit", "F"],
        names: &[],
        dimension: TEMPERATURE,
        scale: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
        prefixes: Bare,
    },
];

/// Short symbol, long name, factor.
const METRIC_PREFIXES: &[(&str, &str, f64)] = &[
    ("Q", "quetta", 1e30),
    ("R", "ronna", 1e27),
    ("Y", "yotta", 1e24),
    ("Z", "zetta", 1e21),
    ("E", "exa", 1e18),
    ("P", "peta", 1e15),
    ("T", "tera", 1e12),
    ("G", "giga", 1e9),
    ("M", "mega", 1e6),
    ("k", "kilo", 1e3),
    ("h", "hecto", 1e2),
    ("da", "deca", 1e1),
    ("d", "deci", 1e-1),
    ("c", "centi", 1e-2),
    ("m", "milli", 1e-3),
    ("µ", "micro", 1e-6),
    ("μ", "micro", 1e-6),
    ("u", "micro", 1e-6),
    ("n", "nano", 1e-9),
    ("p", "pico", 1e-12),
    ("f", "femto", 1e-15),
    ("a", "atto", 1e-18),
    ("z", "zepto", 1e-21),
    ("y", "yocto", 1e-24),
];

const BINARY_PREFIXES: &[(&str, &str, f64)] = &[
    ("Ki", "kibi", 1024.0),
    ("Mi", "mebi", 1_048_576.0),
    ("Gi", "gibi", 1_073_741_824.0),
    ("Ti", "tebi", 1_099_511_627_776.0),
    ("Pi", "pebi", 1_125_899_906_842_624.0),
    ("Ei", "exbi", 1_152_921_504_606_846_976.0),
];

/// Units as people type them, which the table spells otherwise. Case counts: `kw` is plainly
/// kilowatts, but `mW` and `MW` differ by 10⁹.
const SPELLINGS: &[(&str, &[&str])] = &[
    // Kilobytes, though `K` is no prefix.
    ("KB", &["kB"]),
    ("Nm", &["N", "m"]),
    ("kw", &["kW"]),
    ("ftlb", &["ft", "lbf"]),
    ("ft-lb", &["ft", "lbf"]),
    ("lbft", &["lbf", "ft"]),
    ("lb-ft", &["lbf", "ft"]),
];

/// Spellings with a hyphen, which the lexer must read as one word rather than a subtraction.
pub(crate) fn hyphenated() -> impl Iterator<Item = &'static str> {
    SPELLINGS
        .iter()
        .map(|(typed, _)| *typed)
        .filter(|typed| typed.contains('-'))
}

pub(crate) struct Currency {
    pub(crate) code: &'static str,
    /// How it shows, if not as its code.
    symbol: Option<&'static str>,
    aliases: &'static [&'static str],
}

const fn currency_entry(
    code: &'static str,
    symbol: Option<&'static str>,
    aliases: &'static [&'static str],
) -> Currency {
    Currency {
        code,
        symbol,
        aliases,
    }
}

/// ECB currencies, which it quotes against the euro. Symbols such as ₹ that people may not
/// recognise read as aliases, so the code shows.
pub(crate) const CURRENCIES: &[Currency] = &[
    currency_entry("EUR", Some("€"), &["euro", "euros"]),
    currency_entry("USD", Some("$"), &["dollar", "dollars"]),
    currency_entry("GBP", Some("£"), &[]),
    currency_entry("JPY", Some("¥"), &["yen"]),
    currency_entry("INR", None, &["₹", "rupee", "rupees"]),
    currency_entry("KRW", None, &["₩", "won"]),
    currency_entry("TRY", None, &["₺", "lira"]),
    currency_entry("ILS", None, &["₪", "NIS"]),
    currency_entry("AUD", None, &[]),
    currency_entry("BGN", None, &[]),
    currency_entry("BRL", None, &[]),
    currency_entry("CAD", None, &[]),
    currency_entry("CHF", None, &[]),
    currency_entry("CNY", None, &[]),
    currency_entry("CZK", None, &[]),
    currency_entry("DKK", None, &[]),
    currency_entry("HKD", None, &[]),
    currency_entry("HUF", None, &[]),
    currency_entry("IDR", None, &[]),
    currency_entry("ISK", None, &[]),
    currency_entry("MXN", None, &[]),
    currency_entry("MYR", None, &[]),
    currency_entry("NOK", None, &[]),
    currency_entry("NZD", None, &[]),
    currency_entry("PHP", None, &[]),
    currency_entry("PLN", None, &[]),
    currency_entry("RON", None, &[]),
    currency_entry("SEK", None, &[]),
    currency_entry("SGD", None, &[]),
    currency_entry("THB", None, &[]),
    currency_entry("ZAR", None, &[]),
];

/// An amount of one currency, and how it's written.
pub(crate) struct Money {
    pub(crate) symbol: &'static str,
    /// Whether the symbol goes before the number, as in `$5.00`, rather than a code after it.
    pub(crate) symbol_first: bool,
    /// Digits after the point, as ISO 4217 gives them: none for the yen, won and króna.
    pub(crate) decimals: usize,
}

/// Where `code` sits in `CURRENCIES`, and so in `Rates`.
pub(crate) fn currency_index(code: &str) -> Option<usize> {
    CURRENCIES.iter().position(|currency| currency.code == code)
}

/// Whether `character` stands for a currency on its own, as `£` does in `£20`.
pub(crate) fn is_currency_symbol(character: char) -> bool {
    let mut buffer = [0; 4];
    let text: &str = character.encode_utf8(&mut buffer);
    !character.is_alphanumeric()
        && CURRENCIES
            .iter()
            .any(|currency| currency.symbol == Some(text) || currency.aliases.contains(&text))
}

/// Units of each of `CURRENCIES` per euro, as the ECB quotes them.
pub(crate) type Rates = [Option<f64>; CURRENCIES.len()];

/// One unit raised to a power, such as the `h⁻¹` of `km/h`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Factor {
    prefix: &'static str,
    symbol: &'static str,
    dimension: Dimension,
    /// In SI base units, prefix included.
    scale: f64,
    offset: f64,
    exponent: i8,
}

impl Factor {
    fn same_unit(&self, other: &Self) -> bool {
        self.prefix == other.prefix && self.symbol == other.symbol
    }

    fn name(&self) -> String {
        let superscript = match self.exponent.unsigned_abs() {
            1 => String::new(),
            exponent => exponent
                .to_string()
                .chars()
                .map(superscript_digit)
                .collect(),
        };
        format!("{}{}{superscript}", self.prefix, self.symbol)
    }
}

fn superscript_digit(digit: char) -> char {
    match digit {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        _ => '⁹',
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Error {
    /// Not something the calculator reads, or a calculation without an answer.
    Invalid,
    /// A currency other than the euro, before rates have loaded.
    RatesMissing,
}

/// The unit `name` stands for, as factors: `Nm` is two. `newton_metres` reads `nm` as N·m
/// rather than nanometres.
pub(crate) fn lookup(
    name: &str,
    rates: Option<&Rates>,
    newton_metres: bool,
) -> Result<Vec<Factor>, Error> {
    let name = if newton_metres && name == "nm" {
        "Nm"
    } else {
        name
    };
    if let Some((_, symbols)) = SPELLINGS.iter().find(|(typed, _)| *typed == name) {
        return symbols
            .iter()
            .map(|symbol| single_unit(symbol).ok_or(Error::Invalid))
            .collect();
    }
    if let Some(factor) = single_unit(name) {
        return Ok(vec![factor]);
    }
    currency(name, rates).map(|factor| vec![factor])
}

fn single_unit(name: &str) -> Option<Factor> {
    let factor = |entry: &UnitEntry, prefix: &'static str, scale: f64| {
        Some(Factor {
            prefix,
            symbol: entry.symbols.first()?,
            dimension: entry.dimension,
            scale: entry.scale * scale,
            offset: entry.offset,
            exponent: 1,
        })
    };
    for entry in UNITS {
        if entry.symbols.contains(&name) || entry.names.contains(&name) {
            return factor(entry, "", 1.0);
        }
    }
    for entry in UNITS {
        let prefixes: &[&[(&str, &str, f64)]] = match entry.prefixes {
            Bare => continue,
            Metric => &[METRIC_PREFIXES],
            Binary => &[METRIC_PREFIXES, BINARY_PREFIXES],
        };
        for &(symbol, long, scale) in prefixes.iter().copied().flatten() {
            let short = name
                .strip_prefix(symbol)
                .is_some_and(|rest| entry.symbols.contains(&rest));
            let spelled = name
                .strip_prefix(long)
                .is_some_and(|rest| entry.names.contains(&rest));
            if short || spelled {
                return factor(entry, symbol, scale);
            }
        }
    }
    None
}

fn currency(name: &str, rates: Option<&Rates>) -> Result<Factor, Error> {
    // A code in capitals or, as people type it, lower case: `usd` but not `Usd`.
    let code_matches = |code: &str| {
        name == code
            || (name.eq_ignore_ascii_case(code)
                && name.chars().all(|character| character.is_ascii_lowercase()))
    };
    let (index, currency) = CURRENCIES
        .iter()
        .enumerate()
        .find(|(_, currency)| {
            code_matches(currency.code)
                || currency.symbol == Some(name)
                || currency.aliases.contains(&name)
        })
        .ok_or(Error::Invalid)?;
    let per_euro = if currency.code == "EUR" {
        1.0
    } else {
        let rates = rates.ok_or(Error::RatesMissing)?;
        rates.get(index).copied().flatten().ok_or(Error::Invalid)?
    };
    Ok(Factor {
        prefix: "",
        symbol: currency.symbol.unwrap_or(currency.code),
        dimension: MONEY,
        scale: 1.0 / per_euro,
        offset: 0.0,
        exponent: 1,
    })
}

/// A number in some unit, such as `3 km/h`; a plain number when the unit has no factors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Quantity {
    pub(crate) magnitude: f64,
    pub(crate) unit: Vec<Factor>,
}

impl Quantity {
    pub(crate) fn number(magnitude: f64) -> Self {
        Self {
            magnitude,
            unit: Vec::new(),
        }
    }

    pub(crate) fn is_plain(&self) -> bool {
        self.unit.is_empty()
    }

    /// The amount's currency, if it's a plain amount of one, rather than money per kilogram.
    pub(crate) fn as_money(&self) -> Option<Money> {
        let [factor] = self.unit.as_slice() else {
            return None;
        };
        if factor.exponent != 1 || factor.dimension != MONEY {
            return None;
        }
        let currency = CURRENCIES.iter().find(|currency| {
            currency.symbol == Some(factor.symbol) || currency.code == factor.symbol
        })?;
        Some(Money {
            symbol: factor.symbol,
            symbol_first: currency.symbol.is_some(),
            decimals: if matches!(currency.code, "JPY" | "KRW" | "ISK") {
                0
            } else {
                2
            },
        })
    }

    pub(crate) fn is_money(&self) -> bool {
        self.unit.iter().any(|factor| factor.dimension == MONEY)
    }

    fn dimension(&self) -> Dimension {
        let mut dimension = SCALAR;
        for factor in &self.unit {
            for (total, exponent) in dimension.iter_mut().zip(factor.dimension) {
                *total += exponent * i32::from(factor.exponent);
            }
        }
        dimension
    }

    fn scale(&self) -> f64 {
        self.unit
            .iter()
            .map(|factor| factor.scale.powi(factor.exponent.into()))
            .product()
    }

    /// The offset of a lone unit such as °C, which only applies when it isn't multiplied.
    fn offset(&self) -> f64 {
        match self.unit.as_slice() {
            [factor] if factor.exponent == 1 => factor.offset,
            _ => 0.0,
        }
    }

    fn base_magnitude(&self) -> f64 {
        self.magnitude * self.scale() + self.offset()
    }

    /// The plain number a dimensionless quantity stands for: `30°` is π/6.
    pub(crate) fn as_number(&self) -> Result<f64, Error> {
        if self.dimension() != SCALAR {
            return Err(Error::Invalid);
        }
        Ok(self.base_magnitude())
    }

    /// The quantity in seconds, if it's a duration.
    pub(crate) fn seconds(&self) -> Option<f64> {
        (self.dimension() == TIME).then(|| self.base_magnitude())
    }

    /// A count of months or of years, which the calendar gives different lengths: decades,
    /// centuries and millennia count in years.
    pub(crate) fn calendar_unit(&self) -> Option<(&'static str, f64)> {
        let [factor] = self.unit.as_slice() else {
            return None;
        };
        let years = match factor.symbol {
            "month" => return (factor.exponent == 1).then_some(("month", self.magnitude)),
            "year" => 1.0,
            "decade" => 10.0,
            "century" => 100.0,
            "millennium" => 1000.0,
            _ => return None,
        };
        (factor.exponent == 1).then_some(("year", self.magnitude * years))
    }

    pub(crate) fn add(self, other: Self, sign: f64) -> Result<Self, Error> {
        if self.dimension() != other.dimension() {
            return Err(Error::Invalid);
        }
        // A difference in another unit, so `20 °C + 5 K` is 25 °C.
        let other_magnitude = if self.unit == other.unit {
            other.magnitude
        } else {
            other.magnitude * other.scale() / self.scale()
        };
        Ok(Self {
            magnitude: self.magnitude + sign * other_magnitude,
            unit: self.unit,
        })
    }

    /// The product, or with a negative `sign` the quotient; an error if a unit's power grows
    /// past what `i8` holds, as in `m^64 * m^64`.
    pub(crate) fn multiply(mut self, other: Self, sign: i8) -> Result<Self, Error> {
        if sign > 0 {
            self.magnitude *= other.magnitude;
        } else {
            self.magnitude /= other.magnitude;
        }
        for mut factor in other.unit {
            factor.exponent = factor.exponent.checked_mul(sign).ok_or(Error::Invalid)?;
            match self.unit.iter_mut().find(|own| own.same_unit(&factor)) {
                Some(own) => {
                    own.exponent = own
                        .exponent
                        .checked_add(factor.exponent)
                        .ok_or(Error::Invalid)?;
                }
                None => self.unit.push(factor),
            }
        }
        self.unit.retain(|factor| factor.exponent != 0);
        // Units that cancel, as in `km/m` or `EUR/USD`, leave a plain number; an angle alone
        // stays one.
        if self.dimension() == SCALAR && self.unit.iter().any(|factor| factor.dimension != SCALAR) {
            return Ok(Self::number(self.magnitude * self.scale()));
        }
        Ok(self)
    }

    pub(crate) fn power(self, exponent: f64) -> Result<Self, Error> {
        if self.is_plain() {
            return Ok(Self::number(self.magnitude.powf(exponent)));
        }
        if exponent.fract() != 0.0 || exponent.abs() > 64.0 {
            return Err(Error::Invalid);
        }
        let mut unit = self.unit;
        for factor in &mut unit {
            factor.exponent = factor
                .exponent
                .checked_mul(exponent as i8)
                .ok_or(Error::Invalid)?;
        }
        // To the power of 0, a unit is gone.
        unit.retain(|factor| factor.exponent != 0);
        Ok(Self {
            magnitude: self.magnitude.powf(exponent),
            unit,
        })
    }

    /// A root of `degree`, which must divide each unit's exponent: `sqrt(4 m²)` is 2 m.
    pub(crate) fn root(self, degree: i8, root: fn(f64) -> f64) -> Result<Self, Error> {
        let mut unit = self.unit;
        for factor in &mut unit {
            if factor.exponent % degree != 0 {
                return Err(Error::Invalid);
            }
            factor.exponent /= degree;
        }
        Ok(Self {
            magnitude: root(self.magnitude),
            unit,
        })
    }

    /// This quantity in `target`'s unit, which must measure the same thing.
    pub(crate) fn convert(&self, target: &Self) -> Result<Self, Error> {
        if self.dimension() != target.dimension() || target.magnitude != 1.0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            magnitude: (self.base_magnitude() - target.offset()) / target.scale(),
            unit: target.unit.clone(),
        })
    }

    /// `km/h`, `ft·lbf`, `m²`, `kg/(m·s²)`.
    pub(crate) fn unit_name(&self) -> String {
        let join = |positive: bool| {
            self.unit
                .iter()
                .filter(|factor| (factor.exponent > 0) == positive)
                .map(Factor::name)
                .collect::<Vec<_>>()
        };
        let numerator = join(true);
        let denominator = join(false);
        let numerator = if numerator.is_empty() && !denominator.is_empty() {
            "1".to_owned()
        } else {
            numerator.join("·")
        };
        match denominator.as_slice() {
            [] => numerator,
            [single] => format!("{numerator}/{single}"),
            several => format!("{numerator}/({})", several.join("·")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_has_a_symbol() {
        assert!(UNITS.iter().all(|entry| !entry.symbols.is_empty()));
    }

    #[test]
    fn units_match_case_and_prefixes() {
        let scale = |name| single_unit(name).map(|factor| factor.scale);
        assert_eq!(scale("km"), Some(1000.0));
        assert_eq!(scale("kilometres"), Some(1000.0));
        assert_eq!(scale("mW"), Some(1e-3));
        assert_eq!(scale("MW"), Some(1e6));
        assert_eq!(scale("KW"), None);
        assert_eq!(scale("GiB"), Some(8.0 * 1_073_741_824.0));
        assert_eq!(scale("ft"), Some(0.3048), "not femto-tonnes");
        assert_eq!(scale("min"), Some(60.0));
        assert_eq!(scale("Kim"), None, "binary prefixes are for data");
    }

    #[test]
    fn currencies_need_rates_but_the_euro() {
        assert_eq!(currency("USD", None), Err(Error::RatesMissing));
        assert!(currency("€", None).is_ok());
        assert!(currency("eur", None).is_ok());
        assert_eq!(currency("Eur", None), Err(Error::Invalid));
    }
}
