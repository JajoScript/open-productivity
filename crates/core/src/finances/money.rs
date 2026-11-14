// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Currencies and amounts in the currency's smallest unit.

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

pub const CURRENCIES: &[Currency] = &[
    cur("CLP", 0, "$"),
    cur("USD", 2, "$"),
    cur("EUR", 2, "€"),
    cur("ARS", 2, "$"),
    cur("BRL", 2, "R$"),
    cur("COP", 2, "$"),
    cur("MXN", 2, "$"),
    cur("PEN", 2, "S/"),
    cur("UYU", 2, "$"),
    cur("GBP", 2, "£"),
    cur("JPY", 0, "¥"),
];

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

/// Symbol first; Spanish: `$ 1.234,50`, English: `$1,234.50`. Negatives get a U+2212 sign.
pub fn format_amount(amount: i64, c: &Currency, style: AmountStyle) -> String {
    let (thousands, decimal) = style.separators();
    let abs = amount.unsigned_abs();
    let scale = 10u64.pow(c.decimals);
    let int = (abs / scale).to_string();
    let mut out = String::new();
    if amount < 0 {
        out.push('\u{2212}');
    }
    out.push_str(c.symbol);
    if style == AmountStyle::Spanish {
        out.push(' ');
    }
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
        let eur = currency("EUR").unwrap();
        assert_eq!(format_amount(17062, clp(), Spanish), "$ 17.062");
        assert_eq!(format_amount(17062, clp(), English), "$17,062");
        assert_eq!(format_amount(123450, eur, Spanish), "€ 1.234,50");
        assert_eq!(format_amount(123450, eur, English), "€1,234.50");
        assert_eq!(format_amount(-5, eur, English), "\u{2212}€0.05");
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
        assert_eq!(CURRENCIES.len(), 11);
        assert!(currency("XXX").is_none());
    }
}
