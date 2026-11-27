//! Reads a query into an expression and evaluates it.

use std::cell::{OnceCell, RefCell};
use std::f64::consts::{E, PI, TAU};
use std::fmt;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ZonedDateTime};

use crate::units::{self, Error, Factor, Quantity, Rates};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token<'query> {
    Number(f64),
    Name(&'query str),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Factorial,
    Percent,
    LeftParenthesis,
    RightParenthesis,
    /// `to`, `as`, `->` or `→`, before what to convert to.
    Arrow,
    /// `²` or `³`, as in `m²`.
    Superscript(i8),
}

fn tokens(query: &str) -> Result<Vec<Token<'_>>, Error> {
    let mut tokens = Vec::new();
    let mut rest = query;
    let mut spaced = true;
    while let Some(character) = rest.chars().next() {
        let after = rest.get(character.len_utf8()..).unwrap_or_default();
        if character.is_whitespace() {
            rest = after;
            spaced = true;
            continue;
        }
        let follows_number = !spaced && matches!(tokens.last(), Some(Token::Number(_)));
        spaced = false;
        if character.is_ascii_digit()
            || (character == '.' && after.starts_with(|next: char| next.is_ascii_digit()))
        {
            // Numbers that touch, as in `192.168.1.1` or `1.2.3`, are an address or a version.
            if follows_number {
                return Err(Error::Invalid);
            }
            let (value, remaining) = number(rest).ok_or(Error::Invalid)?;
            tokens.push(Token::Number(value));
            rest = remaining;
            continue;
        }
        if matches!(character, 'x' | 'X') && x_multiplies(tokens.last(), after) {
            tokens.push(Token::Times);
            rest = after;
            continue;
        }
        if let Some((name, remaining)) = name(rest) {
            tokens.push(match name {
                "to" | "as" => Token::Arrow,
                "of" => Token::Times,
                "per" => Token::Divide,
                _ => Token::Name(name),
            });
            rest = remaining;
            continue;
        }
        let (token, remaining) = match character {
            '+' => (Token::Plus, after),
            '-' | '−' => match after.strip_prefix('>') {
                Some(remaining) => (Token::Arrow, remaining),
                None => (Token::Minus, after),
            },
            '*' => match after.strip_prefix('*') {
                Some(remaining) => (Token::Power, remaining),
                None => (Token::Times, after),
            },
            '×' | '·' | '⋅' => (Token::Times, after),
            '/' | '÷' => (Token::Divide, after),
            '^' => (Token::Power, after),
            '!' => (Token::Factorial, after),
            '%' => (Token::Percent, after),
            '(' => (Token::LeftParenthesis, after),
            ')' => (Token::RightParenthesis, after),
            '→' | '➞' => (Token::Arrow, after),
            '²' => (Token::Superscript(2), after),
            '³' => (Token::Superscript(3), after),
            _ => return Err(Error::Invalid),
        };
        tokens.push(token);
        rest = remaining;
    }
    Ok(tokens)
}

/// People type `x` for multiplication ("2 x 3", "4x5"). Only an `x` between a number (or `)`)
/// and a number (or `(`) multiplies, so names keep theirs, and `0x` stays a hex prefix.
fn x_multiplies(previous: Option<&Token>, after: &str) -> bool {
    matches!(previous, Some(Token::Number(_) | Token::RightParenthesis))
        && after
            .trim_start()
            .starts_with(|next: char| next.is_ascii_digit() || matches!(next, '(' | '.'))
}

/// A number at the start of `text`, and what follows it: `12`, `1.5e3`, `.5`, `1_000`, `0xff`,
/// `0b101`, `0o17`. An `e` without digits after it is Euler's number, as in `2e`.
fn number(text: &str) -> Option<(f64, &str)> {
    for (prefix, radix) in [("0x", 16), ("0b", 2), ("0o", 8)] {
        let Some(digits) = text.strip_prefix(prefix) else {
            continue;
        };
        let end = digits
            .find(|digit: char| !digit.is_digit(radix) && digit != '_')
            .unwrap_or(digits.len());
        if end == 0 {
            // `0bit` is no bits.
            break;
        }
        let (digits, rest) = digits.split_at_checked(end)?;
        let value = digits
            .chars()
            .filter_map(|digit| digit.to_digit(radix))
            .fold(0.0, |value, digit| {
                value * f64::from(radix) + f64::from(digit)
            });
        return Some((value, rest));
    }
    let digits_end = |from: usize| {
        text.get(from..)
            .and_then(|rest| rest.find(|digit: char| !digit.is_ascii_digit() && digit != '_'))
            .map_or(text.len(), |offset| from + offset)
    };
    let mut end = digits_end(0);
    if text.get(end..)?.starts_with('.') {
        end = digits_end(end + 1);
    }
    let exponent = text
        .get(end..)?
        .strip_prefix(['e', 'E'])
        .map(|rest| rest.strip_prefix(['+', '-']).unwrap_or(rest));
    if let Some(exponent) = exponent
        && exponent.starts_with(|digit: char| digit.is_ascii_digit())
    {
        end = digits_end(text.len() - exponent.len());
    }
    let (literal, rest) = text.split_at_checked(end)?;
    let value = literal.replace('_', "").parse().ok()?;
    Some((value, rest))
}

/// A name at the start of `text`: a word, a unit such as `°C`, `ft-lb` or `µm`, or a currency
/// symbol.
fn name(text: &str) -> Option<(&str, &str)> {
    // Not `is_alphanumeric`, which takes in `²`, so `m²` would be one name.
    let is_word = |character: char| {
        character.is_alphabetic() || character.is_ascii_digit() || character == '_'
    };
    // Spellings such as `ft-lb`, which would otherwise read as a subtraction.
    for spelling in units::hyphenated() {
        if let Some(rest) = text.strip_prefix(spelling)
            && !rest.starts_with(is_word)
        {
            return Some((spelling, rest));
        }
    }
    let first = text.chars().next()?;
    if units::is_currency_symbol(first) {
        return Some(text.split_at(first.len_utf8()));
    }
    if !(first.is_alphabetic() || first == '_' || first == '°') {
        return None;
    }
    let end = text
        .char_indices()
        .skip(1)
        .find(|(_, character)| !is_word(*character))
        .map_or(text.len(), |(index, _)| index);
    text.split_at_checked(end)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
}

#[derive(Clone, Debug, PartialEq)]
enum Expression<'query> {
    Number(f64),
    Name(&'query str),
    Negate(Box<Self>),
    Binary(Operator, Box<Self>, Box<Self>),
    Factorial(Box<Self>),
    Percent(Box<Self>),
    Call(&'query str, Box<Self>),
    /// Terms side by side, as in `2 pi` or `5 ft 11 in`.
    Juxtaposed(Vec<Self>),
    /// The unit an ambiguous or miscased name was taken for.
    Unit(Vec<Factor>),
}

#[derive(Clone, Debug, PartialEq)]
enum Target<'query> {
    Unit(Expression<'query>),
    Radix(u32),
    /// Seconds, or milliseconds, since 1970 began in UTC.
    Unix {
        milliseconds: bool,
    },
}

/// How a query was read: an expression, perhaps converted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reading<'query> {
    expression: Expression<'query>,
    target: Option<Target<'query>>,
}

type Function = fn(Quantity) -> Result<Quantity, Error>;

const FUNCTIONS: &[(&str, Function)] = &[
    ("sqrt", |argument| argument.root(2, f64::sqrt)),
    ("cbrt", |argument| argument.root(3, f64::cbrt)),
    ("abs", |argument| Ok(keeping_unit(argument, f64::abs))),
    ("round", |argument| Ok(keeping_unit(argument, f64::round))),
    ("floor", |argument| Ok(keeping_unit(argument, f64::floor))),
    ("ceil", |argument| Ok(keeping_unit(argument, f64::ceil))),
    ("exp", |argument| of_number(&argument, f64::exp)),
    // `log` is the natural logarithm, as in most programming languages.
    ("ln", |argument| of_number(&argument, f64::ln)),
    ("log", |argument| of_number(&argument, f64::ln)),
    ("log10", |argument| of_number(&argument, f64::log10)),
    ("log2", |argument| of_number(&argument, f64::log2)),
    ("sin", |argument| of_number(&argument, f64::sin)),
    ("cos", |argument| of_number(&argument, f64::cos)),
    ("tan", |argument| of_number(&argument, f64::tan)),
    ("asin", |argument| of_number(&argument, f64::asin)),
    ("acos", |argument| of_number(&argument, f64::acos)),
    ("atan", |argument| of_number(&argument, f64::atan)),
];

fn function(name: &str) -> Option<Function> {
    FUNCTIONS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, function)| *function)
}

fn keeping_unit(mut quantity: Quantity, operation: fn(f64) -> f64) -> Quantity {
    quantity.magnitude = operation(quantity.magnitude);
    quantity
}

fn of_number(quantity: &Quantity, operation: fn(f64) -> f64) -> Result<Quantity, Error> {
    Ok(Quantity::number(operation(quantity.as_number()?)))
}

const RADIXES: &[(&str, u32)] = &[
    ("hex", 16),
    ("hexadecimal", 16),
    ("bin", 2),
    ("binary", 2),
    ("oct", 8),
    ("octal", 8),
];

struct Parser<'query> {
    tokens: Vec<Token<'query>>,
    position: usize,
}

impl<'query> Parser<'query> {
    fn peek(&self) -> Option<Token<'query>> {
        self.tokens.get(self.position).copied()
    }

    fn peek_after(&self) -> Option<Token<'query>> {
        self.tokens.get(self.position + 1).copied()
    }

    fn next(&mut self) -> Option<Token<'query>> {
        let token = self.peek();
        self.position += 1;
        token
    }

    fn expect(&mut self, expected: Token) -> Result<(), Error> {
        (self.next() == Some(expected))
            .then_some(())
            .ok_or(Error::Invalid)
    }

    /// `in` converts when a unit follows; otherwise it's inches, as in `5 in` or `5 in to cm`.
    /// Before a last `in`, as in `1 m in in`, it converts to inches.
    fn at_conversion(&self) -> bool {
        match self.peek() {
            Some(Token::Arrow) => true,
            Some(Token::Name("in")) => match self.peek_after() {
                Some(Token::Name("in")) => self.tokens.get(self.position + 2).is_none(),
                Some(Token::Name(_)) => true,
                _ => false,
            },
            _ => false,
        }
    }

    fn reading(&mut self) -> Result<Reading<'query>, Error> {
        let mut expression = self.sum()?;
        let target = if self.at_conversion() {
            // A number without a unit can't convert, so in `5 in cm` the `in` is inches.
            if self.next() == Some(Token::Name("in")) && !expression.has_name() {
                expression = Expression::Juxtaposed(vec![expression, Expression::Name("in")]);
            }
            let named = match self.tokens.get(self.position..).unwrap_or_default() {
                [Token::Name("unix")] => Some(Target::Unix {
                    milliseconds: false,
                }),
                [Token::Name("unix"), Token::Name("ms")] => {
                    Some(Target::Unix { milliseconds: true })
                }
                [Token::Name(name)] => RADIXES
                    .iter()
                    .find(|(radix_name, _)| radix_name == name)
                    .map(|(_, radix)| Target::Radix(*radix)),
                _ => None,
            };
            Some(match named {
                Some(target) => {
                    self.position = self.tokens.len();
                    target
                }
                None => Target::Unit(self.sum()?),
            })
        } else {
            None
        };
        if self.peek().is_some() {
            return Err(Error::Invalid);
        }
        Ok(Reading { expression, target })
    }

    fn sum(&mut self) -> Result<Expression<'query>, Error> {
        let mut left = self.product()?;
        loop {
            let operator = match self.peek() {
                Some(Token::Plus) => Operator::Add,
                Some(Token::Minus) => Operator::Subtract,
                _ => return Ok(left),
            };
            self.next();
            left = Expression::Binary(operator, Box::new(left), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Expression<'query>, Error> {
        let mut left = self.juxtaposed()?;
        loop {
            let operator = match self.peek() {
                Some(Token::Times) => Operator::Multiply,
                Some(Token::Divide) => Operator::Divide,
                _ => return Ok(left),
            };
            self.next();
            left = Expression::Binary(operator, Box::new(left), Box::new(self.juxtaposed()?));
        }
    }

    /// Side by side binds tighter than `/`, so `1 Nm / 1 s` is a watt.
    fn juxtaposed(&mut self) -> Result<Expression<'query>, Error> {
        let mut terms = vec![self.signed()?];
        while !self.at_conversion()
            && matches!(
                self.peek(),
                Some(Token::Name(_) | Token::Number(_) | Token::LeftParenthesis)
            )
        {
            terms.push(self.power()?);
        }
        Ok(match terms.len() {
            1 => terms.swap_remove(0),
            _ => Expression::Juxtaposed(terms),
        })
    }

    fn signed(&mut self) -> Result<Expression<'query>, Error> {
        match self.peek() {
            Some(Token::Minus) => {
                self.next();
                Ok(Expression::Negate(Box::new(self.signed()?)))
            }
            Some(Token::Plus) => {
                self.next();
                self.signed()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expression<'query>, Error> {
        let base = self.postfix()?;
        if self.peek() != Some(Token::Power) {
            return Ok(base);
        }
        self.next();
        let exponent = self.signed()?;
        Ok(Expression::Binary(
            Operator::Power,
            Box::new(base),
            Box::new(exponent),
        ))
    }

    fn postfix(&mut self) -> Result<Expression<'query>, Error> {
        let mut expression = self.primary()?;
        loop {
            expression = match self.peek() {
                Some(Token::Factorial) => Expression::Factorial(Box::new(expression)),
                Some(Token::Percent) => Expression::Percent(Box::new(expression)),
                Some(Token::Superscript(exponent)) => Expression::Binary(
                    Operator::Power,
                    Box::new(expression),
                    Box::new(Expression::Number(f64::from(exponent))),
                ),
                _ => return Ok(expression),
            };
            self.next();
        }
    }

    fn primary(&mut self) -> Result<Expression<'query>, Error> {
        match self.next() {
            Some(Token::Number(value)) => Ok(Expression::Number(value)),
            Some(Token::Name(name))
                if function(name).is_some() && self.peek() == Some(Token::LeftParenthesis) =>
            {
                self.next();
                let argument = self.sum()?;
                self.expect(Token::RightParenthesis)?;
                Ok(Expression::Call(name, Box::new(argument)))
            }
            // `now()` is `now`.
            Some(Token::Name("now")) if self.peek() == Some(Token::LeftParenthesis) => {
                self.next();
                self.expect(Token::RightParenthesis)?;
                Ok(Expression::Name("now"))
            }
            Some(Token::Name(name)) => Ok(Expression::Name(name)),
            Some(Token::LeftParenthesis) => {
                let inner = self.sum()?;
                self.expect(Token::RightParenthesis)?;
                Ok(inner)
            }
            _ => Err(Error::Invalid),
        }
    }
}

pub(crate) fn read(query: &str) -> Result<Reading<'_>, Error> {
    Parser {
        tokens: tokens(query)?,
        position: 0,
    }
    .reading()
}

/// What a calculation comes to.
#[derive(Debug, PartialEq)]
pub(crate) enum Answer {
    Quantity(Quantity),
    /// A number written in another base, such as `0xff`.
    Text(String),
    Date(Date),
    Time(ZonedDateTime),
}

/// What names mean while evaluating.
pub(crate) struct Environment<'rates> {
    rates: Option<&'rates Rates>,
    /// Read only when a query names it, as the first read loads the time zone.
    now: OnceCell<ZonedDateTime>,
    /// Names that meant nothing as typed and were read in another case, as `kb` is kilobytes,
    /// so the reading can show what they were taken for.
    recased: RefCell<Vec<(String, Vec<Factor>)>>,
}

impl<'rates> Environment<'rates> {
    pub(crate) fn new(rates: Option<&'rates Rates>) -> Self {
        Self {
            rates,
            now: OnceCell::new(),
            recased: RefCell::default(),
        }
    }

    fn now(&self) -> ZonedDateTime {
        self.now.get_or_init(ZonedDateTime::now).clone()
    }
}

/// A quantity, or a point in time, which only takes adding and subtracting durations.
enum Value {
    Quantity(Quantity),
    Date(Date),
    Time(ZonedDateTime),
}

/// At most this many combinations of meanings are tried, so a query full of ambiguous names
/// stays inside a keystroke.
const MOST_COMBINATIONS: usize = 64;

impl<'query> Reading<'query> {
    /// The answer, and the reading it comes from. When a conversion's two sides measure different
    /// things, names with other meanings try them, the likeliest first, and the first combination
    /// that converts wins: in `10 hp to ps`, `ps` is metric horsepower.
    pub(crate) fn resolve(mut self, environment: &Environment) -> Result<(Self, Answer), Error> {
        match self.evaluate(environment) {
            Ok(answer) => {
                self.show_recased(environment);
                return Ok((self, answer));
            }
            Err(Error::Mismatch) => {}
            Err(error) => return Err(error),
        }
        let mut names: Vec<(&str, Vec<Vec<Factor>>)> = Vec::new();
        let mut add = |name: &'query str| {
            if !names.iter().any(|(known, _)| *known == name) {
                let meanings = units::meanings(name);
                if !meanings.is_empty() {
                    names.push((name, meanings));
                }
            }
        };
        self.expression.for_each_name(&mut add);
        if let Some(Target::Unit(target)) = &self.target {
            target.for_each_name(&mut add);
        }
        // Each combination as the index of every name's meaning, fewest names changed from
        // their usual meaning first. Combination 0, the usual meanings, didn't convert.
        let combinations = names.iter().fold(1, |count: usize, (_, meanings)| {
            count.saturating_mul(meanings.len())
        });
        let choice = |combination: usize| {
            let mut rest = combination;
            names.iter().map(move |(_, meanings)| {
                let index = rest % meanings.len();
                rest /= meanings.len();
                index
            })
        };
        let mut order: Vec<usize> =
            (1..combinations.min(MOST_COMBINATIONS * MOST_COMBINATIONS)).collect();
        order.sort_by_key(|&combination| choice(combination).filter(|&index| index > 0).count());
        for combination in order.into_iter().take(MOST_COMBINATIONS) {
            let chosen: Vec<(&str, &[Factor])> = names
                .iter()
                .zip(choice(combination))
                .filter_map(|((name, meanings), index)| {
                    Some((*name, meanings.get(index)?.as_slice()))
                })
                .collect();
            let mut candidate = self.clone();
            candidate.respell(&chosen);
            if let Ok(answer) = candidate.evaluate(environment) {
                candidate.show_recased(environment);
                return Ok((candidate, answer));
            }
        }
        Err(Error::Mismatch)
    }

    /// Takes each name in `chosen` for its unit.
    fn respell(&mut self, chosen: &[(&str, &[Factor])]) {
        self.expression.respell(chosen);
        if let Some(Target::Unit(target)) = &mut self.target {
            target.respell(chosen);
        }
    }

    /// Shows the names read in another case as the units they were taken for: `1000 kB ➞ MB`.
    fn show_recased(&mut self, environment: &Environment) {
        let recased = environment.recased.borrow();
        let chosen: Vec<(&str, &[Factor])> = recased
            .iter()
            .map(|(name, unit)| (name.as_str(), unit.as_slice()))
            .collect();
        self.respell(&chosen);
    }

    fn evaluate(&self, environment: &Environment) -> Result<Answer, Error> {
        let value = value(&self.expression, environment)?;
        if let Some(Target::Unix { milliseconds }) = self.target {
            return in_unix_time(value, milliseconds);
        }
        let value = match value {
            Value::Quantity(quantity) => quantity,
            Value::Date(date) if self.target.is_none() => return Ok(Answer::Date(date)),
            Value::Time(time) if self.target.is_none() => return Ok(Answer::Time(time)),
            Value::Date(_) | Value::Time(_) => return Err(Error::Invalid),
        };
        match &self.target {
            None => Ok(Answer::Quantity(value)),
            Some(Target::Unit(target)) => {
                let target = evaluate(target, environment)?;
                Ok(Answer::Quantity(value.convert(&target)?))
            }
            Some(Target::Radix(radix)) => {
                if !value.is_plain() {
                    return Err(Error::Invalid);
                }
                in_radix(value.magnitude, *radix).map(Answer::Text)
            }
            Some(Target::Unix { .. }) => Err(Error::Invalid),
        }
    }
}

fn evaluate(expression: &Expression, environment: &Environment) -> Result<Quantity, Error> {
    match value(expression, environment)? {
        Value::Quantity(quantity) => Ok(quantity),
        Value::Date(_) | Value::Time(_) => Err(Error::Invalid),
    }
}

fn value(expression: &Expression, environment: &Environment) -> Result<Value, Error> {
    let quantity = match expression {
        Expression::Number(value) => Quantity::number(*value),
        Expression::Name("now") => return Ok(Value::Time(environment.now())),
        Expression::Name(name @ ("today" | "tomorrow" | "yesterday")) => {
            let today = environment.now().date();
            let date = match *name {
                "tomorrow" => today.tomorrow(),
                "yesterday" => today.yesterday(),
                _ => Ok(today),
            };
            return date.map(Value::Date).map_err(|_| Error::Invalid);
        }
        Expression::Name(name) => named(name, environment)?,
        Expression::Unit(unit) => Quantity {
            magnitude: 1.0,
            unit: unit.clone(),
        },
        Expression::Negate(inner) => {
            let mut quantity = evaluate(inner, environment)?;
            quantity.magnitude = -quantity.magnitude;
            quantity
        }
        Expression::Binary(operator @ (Operator::Add | Operator::Subtract), left, right) => {
            let sign = if *operator == Operator::Add { 1 } else { -1 };
            let mut left = value(left, environment)?;
            let mut right = value(right, environment)?;
            // A number beside a duration is a Unix time, as in `1795758014 + 1 day`.
            if let Some(time) = unix_time_beside(&left, &right) {
                left = Value::Time(time);
            } else if let Some(time) = unix_time_beside(&right, &left) {
                right = Value::Time(time);
            }
            let (point, duration) = match (left, right) {
                (Value::Quantity(left), Value::Quantity(right)) => {
                    return finite(left.add(right, f64::from(sign))?);
                }
                (point, Value::Quantity(duration)) => (point, duration),
                // A duration can come first only when added: `2 days + today`.
                (Value::Quantity(duration), point) if sign > 0 => (point, duration),
                _ => return Err(Error::Invalid),
            };
            return match point {
                Value::Date(date) => shift_date(date, &duration, sign).map(Value::Date),
                Value::Time(time) => shift_time(&time, &duration, sign).map(Value::Time),
                Value::Quantity(_) => Err(Error::Invalid),
            };
        }
        Expression::Binary(operator, left, right) => {
            let left = evaluate(left, environment)?;
            let right = evaluate(right, environment)?;
            match operator {
                Operator::Add | Operator::Subtract => return Err(Error::Invalid),
                Operator::Multiply => left.multiply(right, 1)?,
                Operator::Divide => left.multiply(right, -1)?,
                Operator::Power if right.is_plain() => left.power(right.magnitude)?,
                Operator::Power => return Err(Error::Invalid),
            }
        }
        Expression::Factorial(inner) => {
            let quantity = evaluate(inner, environment)?;
            let count = quantity.magnitude;
            if !quantity.is_plain() || count.fract() != 0.0 || !(0.0..=170.0).contains(&count) {
                return Err(Error::Invalid);
            }
            Quantity::number((2..=count as u32).map(f64::from).product())
        }
        Expression::Percent(inner) => {
            let mut quantity = evaluate(inner, environment)?;
            quantity.magnitude /= 100.0;
            quantity
        }
        Expression::Call(name, argument) => {
            function(name).ok_or(Error::Invalid)?(evaluate(argument, environment)?)?
        }
        Expression::Juxtaposed(terms) => juxtaposed(terms, environment)?,
    };
    finite(quantity)
}

fn finite(quantity: Quantity) -> Result<Value, Error> {
    if quantity.magnitude.is_finite() {
        Ok(Value::Quantity(quantity))
    } else {
        Err(Error::Invalid)
    }
}

/// A plain number as seconds since 1970, from 2001 until 2286, to the millisecond as in
/// `1795758014.123`, or a whole number as milliseconds over the same years, in the system's time
/// zone.
pub(crate) fn unix_time(quantity: &Quantity) -> Option<ZonedDateTime> {
    let count = quantity.magnitude;
    if !quantity.is_plain() {
        return None;
    }
    let milliseconds = if (1e9..1e10).contains(&count) {
        (count * 1000.0).round()
    } else if (1e12..1e13).contains(&count) && count.fract() == 0.0 {
        count
    } else {
        return None;
    };
    Some(
        Timestamp::from_millisecond(milliseconds as i64)
            .ok()?
            .to_zoned_datetime(TimeZone::system()),
    )
}

/// A time, the start of a date, or a number that reads as a Unix time, as a count of seconds or
/// milliseconds since 1970.
fn in_unix_time(value: Value, milliseconds: bool) -> Result<Answer, Error> {
    let time = match value {
        Value::Time(time) => time,
        Value::Date(date) => date
            .to_zoned_datetime(TimeZone::system())
            .map_err(|_| Error::Invalid)?,
        Value::Quantity(quantity) => unix_time(&quantity).ok_or(Error::Invalid)?,
    };
    let timestamp = time.timestamp();
    let count = if milliseconds {
        timestamp.as_millisecond()
    } else {
        timestamp.as_second()
    };
    Ok(Answer::Text(count.to_string()))
}

/// `value` as a Unix time when it's a number that reads as one and `other` is a duration.
fn unix_time_beside(value: &Value, other: &Value) -> Option<ZonedDateTime> {
    match (value, other) {
        (Value::Quantity(number), Value::Quantity(duration)) if duration.seconds().is_some() => {
            unix_time(number)
        }
        _ => None,
    }
}

/// `duration` as a span of the calendar: whole months and years as such, so a month from the
/// 31st of January is the end of February, and anything else in seconds.
fn span(duration: &Quantity, sign: i32) -> Result<Span, Error> {
    let whole = |count: f64| {
        (count.fract() == 0.0 && count.abs() < 100_000.0)
            .then_some(count as i64 * i64::from(sign))
            .ok_or(Error::Invalid)
    };
    let span = match duration.calendar_unit() {
        Some(("month", count)) => Span::new().try_months(whole(count)?),
        Some(("year", count)) => Span::new().try_years(whole(count)?),
        _ => {
            let seconds = duration.seconds().ok_or(Error::Invalid)?;
            // A span holds up to about 631 billion seconds, 20 thousand years.
            if seconds.abs() >= 6e11 {
                return Err(Error::Invalid);
            }
            let milliseconds = (seconds * 1000.0).round() as i64 * i64::from(sign);
            Span::new().try_milliseconds(milliseconds)
        }
    };
    span.map_err(|_| Error::Invalid)
}

/// A date moved by whole days, weeks, months or years; a part of a day has no date.
fn shift_date(date: Date, duration: &Quantity, sign: i32) -> Result<Date, Error> {
    let span = span(duration, sign)?;
    let span = if span.get_months() != 0 || span.get_years() != 0 {
        span
    } else {
        let milliseconds = span.get_milliseconds();
        if milliseconds % 86_400_000 != 0 {
            return Err(Error::Invalid);
        }
        Span::new()
            .try_days(milliseconds / 86_400_000)
            .map_err(|_| Error::Invalid)?
    };
    date.checked_add(span).map_err(|_| Error::Invalid)
}

/// A time moved by `duration`: by the calendar for months and years, else by the clock, so
/// across a change to summer time a day is 23 or 25 hours.
fn shift_time(
    time: &ZonedDateTime,
    duration: &Quantity,
    sign: i32,
) -> Result<ZonedDateTime, Error> {
    time.checked_add(span(duration, sign)?)
        .map_err(|_| Error::Invalid)
}

fn named(name: &str, environment: &Environment) -> Result<Quantity, Error> {
    let constant = match name {
        "pi" | "π" => PI,
        "tau" | "τ" => TAU,
        "e" => E,
        "c" => {
            let metres = Quantity {
                magnitude: 299_792_458.0,
                unit: units::lookup("m", None)?,
            };
            let second = Quantity {
                magnitude: 1.0,
                unit: units::lookup("s", None)?,
            };
            return metres.multiply(second, -1);
        }
        _ => {
            let unit = match units::lookup(name, environment.rates) {
                // A name that means nothing as typed may in another case, as `kb` does.
                Err(Error::Invalid) => {
                    let unit = units::in_any_case(name)
                        .into_iter()
                        .next()
                        .ok_or(Error::Invalid)?;
                    let mut recased = environment.recased.borrow_mut();
                    if !recased.iter().any(|(known, _)| known == name) {
                        recased.push((name.to_owned(), unit.clone()));
                    }
                    unit
                }
                unit => unit?,
            };
            return Ok(Quantity {
                magnitude: 1.0,
                unit,
            });
        }
    };
    Ok(Quantity::number(constant))
}

/// Groups of a number and its units that all measure the same thing add up, as in
/// `5 ft 11 in`; anything else multiplies, as in `2 pi` or `2 m`.
fn juxtaposed(terms: &[Expression], environment: &Environment) -> Result<Quantity, Error> {
    let mut groups: Vec<Quantity> = Vec::new();
    let mut group_has_name = false;
    for term in terms {
        let value = evaluate(term, environment)?;
        let starts_group = matches!(term, Expression::Number(_)) && group_has_name;
        match groups.last_mut() {
            Some(group) if !starts_group => {
                *group = std::mem::replace(group, Quantity::number(1.0)).multiply(value, 1)?;
            }
            _ => {
                groups.push(value);
                group_has_name = false;
            }
        }
        group_has_name |= !matches!(term, Expression::Number(_));
    }
    let [first, rest @ ..] = groups.as_slice() else {
        return Err(Error::Invalid);
    };
    if !rest.is_empty()
        && groups.iter().all(|group| !group.is_plain())
        && let Ok(sum) = rest
            .iter()
            .cloned()
            .try_fold(first.clone(), |sum, group| sum.add(group, 1.0))
    {
        return Ok(sum);
    }
    let mut groups = groups.into_iter();
    let first = groups.next().ok_or(Error::Invalid)?;
    groups.try_fold(first, |product, group| product.multiply(group, 1))
}

/// An integer in base 2, 8 or 16, with its prefix: `0xff`. Exact for any integer an f64 holds,
/// such as 2⁷⁰.
fn in_radix(value: f64, radix: u32) -> Result<String, Error> {
    if value.fract() != 0.0 {
        return Err(Error::Invalid);
    }
    let (prefix, bits_per_digit) = match radix {
        2 => ("0b", 1),
        8 => ("0o", 3),
        _ => ("0x", 4),
    };
    // An integral f64 is its mantissa shifted left; written in binary, that's the mantissa's
    // bits then zeros.
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1075;
    let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
    let binary = if value == 0.0 {
        "0".to_owned()
    } else if exponent < 0 {
        format!("{:b}", mantissa >> -exponent)
    } else {
        format!("{mantissa:b}{}", "0".repeat(exponent as usize))
    };
    let padding = (bits_per_digit - binary.len() % bits_per_digit) % bits_per_digit;
    let padded = format!("{}{binary}", "0".repeat(padding));
    let digits: String = padded
        .as_bytes()
        .chunks(bits_per_digit)
        .filter_map(|chunk| {
            let digit = chunk
                .iter()
                .fold(0, |digit, bit| digit * 2 + u32::from(bit - b'0'));
            char::from_digit(digit, radix)
        })
        .collect();
    let sign = if value < 0.0 { "-" } else { "" };
    Ok(format!("{sign}{prefix}{digits}"))
}

impl Operator {
    fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Subtract => 1,
            Self::Multiply | Self::Divide => 2,
            Self::Power => 5,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "×",
            Self::Divide => "/",
            Self::Power => "^",
        }
    }
}

impl<'query> Expression<'query> {
    fn for_each_name(&self, visit: &mut impl FnMut(&'query str)) {
        match self {
            Self::Number(_) | Self::Unit(_) => {}
            Self::Name(name) => visit(name),
            Self::Negate(inner)
            | Self::Factorial(inner)
            | Self::Percent(inner)
            | Self::Call(_, inner) => inner.for_each_name(visit),
            Self::Binary(_, left, right) => {
                left.for_each_name(visit);
                right.for_each_name(visit);
            }
            Self::Juxtaposed(terms) => {
                for term in terms {
                    term.for_each_name(visit);
                }
            }
        }
    }

    /// Takes each name in `chosen` for its unit.
    fn respell(&mut self, chosen: &[(&str, &[Factor])]) {
        match self {
            Self::Number(_) | Self::Unit(_) => {}
            Self::Name(name) => {
                if let Some((_, unit)) = chosen.iter().find(|(chosen, _)| chosen == name) {
                    *self = Self::Unit(unit.to_vec());
                }
            }
            Self::Negate(inner)
            | Self::Factorial(inner)
            | Self::Percent(inner)
            | Self::Call(_, inner) => inner.respell(chosen),
            Self::Binary(_, left, right) => {
                left.respell(chosen);
                right.respell(chosen);
            }
            Self::Juxtaposed(terms) => {
                for term in terms {
                    term.respell(chosen);
                }
            }
        }
    }

    /// Whether a name, such as a unit or a constant, appears anywhere in the expression.
    fn has_name(&self) -> bool {
        match self {
            Self::Number(_) => false,
            Self::Name(_) | Self::Call(..) | Self::Unit(_) => true,
            Self::Negate(inner) | Self::Factorial(inner) | Self::Percent(inner) => inner.has_name(),
            Self::Binary(_, left, right) => left.has_name() || right.has_name(),
            Self::Juxtaposed(terms) => terms.iter().any(Self::has_name),
        }
    }

    fn precedence(&self) -> u8 {
        match self {
            Self::Binary(operator, ..) => operator.precedence(),
            // Below `/`, though it binds tighter, so `1 / (2 m)` reads unambiguously.
            Self::Juxtaposed(_) => 2,
            // `N·m` binds as a term beside a number, but not under `^`.
            Self::Negate(_) => 4,
            Self::Unit(unit) if unit.len() > 1 => 4,
            _ => 6,
        }
    }

    /// Writes `inner` in parentheses when it binds looser than `bound`.
    fn write_bound(formatter: &mut fmt::Formatter, inner: &Self, bound: u8) -> fmt::Result {
        if inner.precedence() < bound {
            write!(formatter, "({inner})")
        } else {
            write!(formatter, "{inner}")
        }
    }
}

/// The expression as read, with its grouping explicit: `1 / (2 m)`, `100 N ➞ lbf`.
impl fmt::Display for Expression<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Number(value) => formatter.write_str(&crate::precise(*value)),
            Self::Name(name) => formatter.write_str(name),
            Self::Unit(unit) => formatter.write_str(&units::unit_name(unit)),
            Self::Negate(inner) => {
                formatter.write_str("-")?;
                Self::write_bound(formatter, inner, 6)
            }
            Self::Binary(operator, left, right) => {
                let precedence = operator.precedence();
                // Power is right-associative; the others left.
                let (left_bound, right_bound) = match operator {
                    Operator::Power => (precedence + 1, precedence),
                    _ => (precedence, precedence + 1),
                };
                Self::write_bound(formatter, left, left_bound)?;
                write!(formatter, " {} ", operator.symbol())?;
                Self::write_bound(formatter, right, right_bound)
            }
            Self::Factorial(inner) => {
                Self::write_bound(formatter, inner, 6)?;
                formatter.write_str("!")
            }
            Self::Percent(inner) => {
                Self::write_bound(formatter, inner, 6)?;
                formatter.write_str("%")
            }
            Self::Call(function, argument) => write!(formatter, "{function}({argument})"),
            Self::Juxtaposed(terms) => {
                for (index, term) in terms.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(" ")?;
                    }
                    Self::write_bound(formatter, term, 4)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Reading<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}", self.expression)?;
        match &self.target {
            None => Ok(()),
            Some(Target::Unit(target)) => write!(formatter, " ➞ {target}"),
            Some(Target::Radix(radix)) => {
                let name = RADIXES
                    .iter()
                    .find(|(_, candidate)| candidate == radix)
                    .map_or("", |(name, _)| name);
                write!(formatter, " ➞ {name}")
            }
            Some(Target::Unix { milliseconds }) => {
                let unit = if *milliseconds { " ms" } else { "" };
                write!(formatter, " ➞ unix{unit}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(query: &str) -> String {
        read(query).map_or_else(|error| format!("{error:?}"), |reading| reading.to_string())
    }

    #[test]
    fn reads_numbers() {
        assert_eq!(number("1.5e3 m"), Some((1500.0, " m")));
        assert_eq!(number("2e"), Some((2.0, "e")));
        assert_eq!(number("1e-9"), Some((1e-9, "")));
        assert_eq!(number("0xff"), Some((255.0, "")));
        assert_eq!(number("0bit"), Some((0.0, "bit")));
        assert_eq!(number("1_000"), Some((1000.0, "")));
        assert_eq!(number(".5"), Some((0.5, "")));
    }

    #[test]
    fn grouping() {
        assert_eq!(reading("1 + 2 * 3"), "1 + 2 × 3");
        assert_eq!(reading("(1 + 2) * 3"), "(1 + 2) × 3");
        assert_eq!(reading("1/2 m"), "1 / (2 m)");
        assert_eq!(reading("-2^2"), "-(2 ^ 2)");
        assert_eq!(reading("2^3^2"), "2 ^ 3 ^ 2");
        assert_eq!(reading("1 Nm / 1 s to W"), "1 Nm / (1 s) ➞ W");
        assert_eq!(reading("5 in"), "5 in");
        assert_eq!(reading("5 in to cm"), "5 in ➞ cm");
        assert_eq!(reading("5 in in cm"), "5 in ➞ cm");
        assert_eq!(reading("5 m in cm"), "5 m ➞ cm");
        assert_eq!(reading("5 in cm"), "5 in ➞ cm");
        assert_eq!(reading("1 m in in"), "1 m ➞ in");
        assert_eq!(reading("1.2.3"), "Invalid");
        assert_eq!(reading("2^70 -> bin"), "2 ^ 70 ➞ bin");
        assert_eq!(reading("3x4"), "3 × 4");
        assert_eq!(reading("lb-ft"), "lb-ft");
        assert_eq!(reading("2 +"), "Invalid");
        assert_eq!(reading("let x = 3"), "Invalid");
    }

    #[test]
    fn writes_integers_in_other_bases() {
        assert_eq!(in_radix(255.0, 16), Ok("0xff".into()));
        assert_eq!(in_radix(-5.0, 2), Ok("-0b101".into()));
        assert_eq!(in_radix(0.0, 8), Ok("0o0".into()));
        assert_eq!(in_radix(64.0, 8), Ok("0o100".into()));
        assert_eq!(
            in_radix(2f64.powi(70), 16),
            Ok(format!("0x4{}", "0".repeat(17)))
        );
        assert_eq!(in_radix(1.5, 16), Err(Error::Invalid));
    }
}
