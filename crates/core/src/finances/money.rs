// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Currencies, amounts in the currency's smallest unit and exchange rates.

use std::collections::HashMap;

use super::FinanceError;

#[derive(Debug, PartialEq)]
pub struct Currency {
    pub code: &'static str,
    /// Digits after the decimal separator (ISO 4217 minor units).
    pub decimals: u32,
    pub symbol: &'static str,
}

const fn cur(code: &'static str, decimals: u32, symbol: &'static str) -> Currency {
    Currency {
        code,
        decimals,
        symbol,
    }
}

pub const CURRENCIES: &[Currency] = &[cur("CLP", 0, "$"), cur("USD", 2, "$")];

pub fn currency(code: &str) -> Option<&'static Currency> {
    CURRENCIES.iter().find(|c| c.code == code)
}

/// 10^13 in the smallest unit.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AmountStyle {
    Spanish,
    English,
}

impl AmountStyle {
    /// (thousands, decimal)
    fn separators(self) -> (char, char) {
        match self {
            AmountStyle::Spanish => ('.', ','),
            AmountStyle::English => (',', '.'),
        }
    }
}

/// The digits with grouping and decimals, no sign nor symbol.
fn format_digits(abs: u64, c: &Currency, style: AmountStyle) -> String {
    let (thousands, decimal) = style.separators();
    let scale = 10u64.pow(c.decimals);
    let int = (abs / scale).to_string();
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(thousands);
        }
        out.push(ch);
    }
    if c.decimals > 0 {
        out.push(decimal);
        out.push_str(&format!("{:0w$}", abs % scale, w = c.decimals as usize));
    }
    out
}

/// Symbol first; Spanish: `$ 1.234,50`, English: `$1,234.50`. Negatives get a U+2212 sign.
pub fn format_amount(amount: i64, c: &Currency, style: AmountStyle) -> String {
    let mut out = String::new();
    if amount < 0 {
        out.push('\u{2212}');
    }
    out.push_str(c.symbol);
    if style == AmountStyle::Spanish {
        out.push(' ');
    }
    out.push_str(&format_digits(amount.unsigned_abs(), c, style));
    out
}

/// The main currency keeps its symbol (`$ 17.062`); any other shows its code after the amount
/// (`17.062,00 USD`), so it is never mistaken for the main one.
pub fn format_in(amount: i64, c: &Currency, main: &Currency, style: AmountStyle) -> String {
    if c.code == main.code {
        return format_amount(amount, c, style);
    }
    let sign = if amount < 0 { "\u{2212}" } else { "" };
    let digits = format_digits(amount.unsigned_abs(), c, style);
    format!("{sign}{digits} {}", c.code)
}

/// The amount (>= 0) as a user would type it: no symbol, no grouping. `parse_amount` reads it back.
pub fn format_input(amount: i64, c: &Currency, style: AmountStyle) -> String {
    let (_, decimal) = style.separators();
    let scale = 10i64.pow(c.decimals);
    let frac = amount % scale;
    let mut out = (amount / scale).to_string();
    if frac != 0 {
        out.push(decimal);
        out.push_str(&format!("{:0w$}", frac, w = c.decimals as usize));
    }
    out
}

/// Reads what the user typed (either separator style) into the smallest unit.
pub fn parse_amount(text: &str, c: &Currency) -> Option<i64> {
    let text = text.trim();
    if !text.starts_with(|ch: char| ch.is_ascii_digit())
        || !text
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == ',')
    {
        return None;
    }
    let (int_part, frac) = match text.rfind(['.', ',']) {
        Some(i) => {
            let tail = &text[i + 1..];
            if tail.len() == 3 {
                (text, "")
            } else if c.decimals > 0 && (1..=2).contains(&tail.len()) {
                let int_part = &text[..i];
                // The decimal separator cannot also group thousands.
                if int_part.contains(&text[i..=i]) {
                    return None;
                }
                (int_part, tail)
            } else {
                return None;
            }
        }
        None => (text, ""),
    };
    let int_digits = match int_part.find(['.', ',']) {
        None => int_part.to_string(),
        Some(i) => {
            let sep = &int_part[i..=i];
            let groups: Vec<&str> = int_part.split(sep).collect();
            if groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3) {
                return None;
            }
            groups.concat()
        }
    };
    if int_digits.is_empty() || int_digits.contains(['.', ',']) || int_digits.len() > 18 {
        return None;
    }
    let scale = 10i64.pow(c.decimals);
    let frac_val: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse::<i64>().ok()? * 10i64.pow(c.decimals - frac.len() as u32)
    };
    let total = int_digits.parse::<i64>().ok()?.checked_mul(scale)? + frac_val;
    (total <= MAX_AMOUNT).then_some(total)
}

/// An exchange rate is stored with this many decimals.
const RATE_DECIMALS: u32 = 6;
/// A rate of exactly 1: the main currency's.
pub(super) const RATE_ONE: i64 = 1_000_000;
/// 10^13 with 6 decimals, like `MAX_AMOUNT`: ten million main units per unit.
pub const MAX_RATE: i64 = 10_000_000_000_000;

/// Reads a rate typed by the user (either separator style) with up to 6 decimals, > 0.
/// As in amounts, a lone separator followed by exactly 3 digits groups thousands (`1.050` is
/// 1050); otherwise the last separator starts the decimals. `0,001` is a decimal: 0 is never
/// grouped.
pub fn parse_rate(text: &str) -> Option<i64> {
    let text = text.trim();
    if !text.starts_with(|ch: char| ch.is_ascii_digit())
        || text.ends_with(['.', ','])
        || !text
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == ',')
    {
        return None;
    }
    let (int_part, frac) = match text.rfind(['.', ',']) {
        Some(i)
            if text.len() - i - 1 == 3 && !text[..i].contains(['.', ',']) && &text[..i] != "0" =>
        {
            (text, "")
        }
        Some(i) if !text[..i].contains(&text[i..=i]) => (&text[..i], &text[i + 1..]),
        _ => (text, ""),
    };
    if frac.len() > RATE_DECIMALS as usize {
        return None;
    }
    let int_digits = match int_part.find(['.', ',']) {
        None => int_part.to_string(),
        Some(i) => {
            let groups: Vec<&str> = int_part.split(&int_part[i..=i]).collect();
            if groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3) {
                return None;
            }
            groups.concat()
        }
    };
    if int_digits.is_empty() || int_digits.contains(['.', ',']) || int_digits.len() > 18 {
        return None;
    }
    let frac_val = match frac {
        "" => 0,
        f => f.parse::<i64>().ok()? * 10i64.pow(RATE_DECIMALS - f.len() as u32),
    };
    let total = int_digits
        .parse::<i64>()
        .ok()?
        .checked_mul(RATE_ONE)?
        .checked_add(frac_val)?;
    (0 < total && total <= MAX_RATE).then_some(total)
}

/// The rate as a user would type it, without trailing zeros; `parse_rate` reads it back.
pub fn format_rate(rate: i64, style: AmountStyle) -> String {
    let (_, decimal) = style.separators();
    let mut out = (rate / RATE_ONE).to_string();
    let frac = format!("{:06}", rate % RATE_ONE);
    let frac = frac.trim_end_matches('0');
    if !frac.is_empty() {
        out.push(decimal);
        out.push_str(frac);
        // Exactly 3 decimals would read back as thousands.
        if frac.len() == 3 && rate >= RATE_ONE {
            out.push('0');
        }
    }
    out
}

/// The exchange rates of an instance: every amount converts through the main currency.
#[derive(Debug)]
pub struct Rates {
    main: &'static Currency,
    stored: HashMap<&'static str, i64>,
}

impl Rates {
    pub fn new(
        main: &'static Currency,
        stored: impl IntoIterator<Item = (&'static str, i64)>,
    ) -> Self {
        Rates {
            main,
            stored: stored.into_iter().collect(),
        }
    }

    pub fn main(&self) -> &'static Currency {
        self.main
    }

    /// The stored rate of a non-main currency.
    pub fn stored(&self, c: &Currency) -> Option<i64> {
        self.stored.get(c.code).copied()
    }

    /// A non-main currency whose rate is still unknown.
    pub fn needs_rate(&self, c: &Currency) -> bool {
        c.code != self.main.code && self.stored(c).is_none()
    }

    fn rate(&self, c: &Currency) -> Result<i64, FinanceError> {
        if c.code == self.main.code {
            return Ok(RATE_ONE);
        }
        self.stored(c)
            .ok_or_else(|| FinanceError::MissingRate(c.code.to_string()))
    }

    /// `amount` of `from` in `to`, through the main currency. Rounds to nearest (halves away
    /// from zero) and saturates; a missing rate is an error, never 1.
    pub fn convert(
        &self,
        amount: i64,
        from: &Currency,
        to: &Currency,
    ) -> Result<i64, FinanceError> {
        if from.code == to.code {
            return Ok(amount);
        }
        let (rf, rt) = (self.rate(from)? as i128, self.rate(to)? as i128);
        let num = (amount.unsigned_abs() as i128)
            .checked_mul(rf)
            .and_then(|n| n.checked_mul(10i128.pow(to.decimals)));
        let den = 10i128.pow(from.decimals) * rt;
        // Overflowing i128 is far beyond i64, so it saturates too.
        let q = num
            .and_then(|n| n.checked_mul(2)?.checked_add(den))
            .map_or(i128::MAX, |n| n / (2 * den));
        let q = i64::try_from(q).unwrap_or(i64::MAX);
        Ok(if amount < 0 { -q } else { q })
    }

    pub fn to_main(&self, amount: i64, from: &Currency) -> Result<i64, FinanceError> {
        self.convert(amount, from, self.main)
    }

    pub fn from_main(&self, amount: i64, to: &Currency) -> Result<i64, FinanceError> {
        self.convert(amount, self.main, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clp() -> &'static Currency {
        currency("CLP").unwrap()
    }
    fn usd() -> &'static Currency {
        currency("USD").unwrap()
    }

    #[test]
    fn parses_clp() {
        for (t, v) in [
            ("17.062", Some(17062)),
            ("17,062", Some(17062)),
            (" 17062 ", Some(17062)),
            ("30,50", None),
            ("30.50", None),
            ("1.2.3", None),
        ] {
            assert_eq!(parse_amount(t, clp()), v, "{t}");
        }
    }

    #[test]
    fn parses_usd() {
        for (t, v) in [
            ("30", Some(3000)),
            ("30,5", Some(3050)),
            ("30.50", Some(3050)),
            ("1.706", Some(170600)),
            ("1,706", Some(170600)),
            ("17.062,05", Some(1706205)),
            ("17,062.05", Some(1706205)),
            ("1.000.000,5", Some(100000050)),
            ("30,5055", None),
            ("1.2.3", None),
            ("1.234.56.78", None),
            ("1,234,56", None),
            ("1234.567.89", None),
            (",50", None),
            ("-5", None),
            ("", None),
            ("abc", None),
            ("1234567890123456789", None),
            ("100000000000.00", Some(MAX_AMOUNT)),
            ("100000000000.01", None),
        ] {
            assert_eq!(parse_amount(t, usd()), v, "{t}");
        }
        assert_eq!(parse_amount("10000000000000", clp()), Some(MAX_AMOUNT));
        assert_eq!(parse_amount("10000000000001", clp()), None);
    }

    #[test]
    fn formats() {
        use AmountStyle::*;
        let eur = currency("USD").unwrap();
        assert_eq!(format_amount(17062, clp(), Spanish), "$ 17.062");
        assert_eq!(format_amount(17062, clp(), English), "$17,062");
        assert_eq!(format_amount(123450, eur, Spanish), "$ 1.234,50");
        assert_eq!(format_amount(123450, eur, English), "$1,234.50");
        assert_eq!(format_amount(-5, eur, English), "\u{2212}$0.05");
        assert_eq!(format_amount(0, clp(), Spanish), "$ 0");
    }

    #[test]
    fn input_round_trips() {
        for style in [AmountStyle::Spanish, AmountStyle::English] {
            for c in CURRENCIES {
                for a in [0, 5, 100, 1234, 123450, 1_706_205, MAX_AMOUNT] {
                    let text = format_input(a, c, style);
                    assert_eq!(parse_amount(&text, c), Some(a), "{text} {}", c.code);
                }
            }
        }
        assert_eq!(format_input(3050, usd(), AmountStyle::Spanish), "30,50");
    }

    #[test]
    fn lookup() {
        assert_eq!(CURRENCIES.len(), 2);
        assert!(currency("XXX").is_none());
    }
}
