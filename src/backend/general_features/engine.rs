use super::calc::calc;
use super::currency;
use anyhow::Result;
use regex::Regex;

pub fn general_engine(input: String, currency_enabled: bool) -> Result<String> {
    if currency_enabled && let Some((amount, from, to)) = currency::parse_query(&input) {
        let data = currency::convert(&from, &to, amount)?;
        return Ok(currency::format_result(amount, &from, &to, data.price));
    }
    // Only math characters and at least one binary operator, so "2", "hello" or
    // "2024-10 report" show nothing; a half-typed "2+" shows nothing either.
    let chars_re = Regex::new(r"^[\d\s.+\-*/%^()]+$").unwrap();
    let op_re = Regex::new(r"[\d)]\s*[-+*/%^]\s*[\d(]").unwrap();
    if chars_re.is_match(&input) && op_re.is_match(&input) {
        return Ok(calc(&input).unwrap_or_default());
    }

    Ok("".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> String {
        general_engine(input.into(), false).unwrap()
    }

    #[test]
    fn calc_without_spaces() {
        assert_eq!(run("2+2"), "4");
        assert_eq!(run("10/5"), "2");
        assert_eq!(run("2 + 2"), "4");
        // non-math input stays empty
        assert_eq!(run("hello"), "");
    }

    #[test]
    fn calc_float_division_and_grouping() {
        assert_eq!(run("7/2"), "3.5");
        assert_eq!(run("7-3"), "4");
        assert_eq!(run("(1+2)*3"), "9");
    }

    #[test]
    fn calc_ignores_non_expressions() {
        assert_eq!(run("2024-10 report"), "");
        assert_eq!(run("2"), "");
        assert_eq!(run("2+"), "");
        assert_eq!(run("1/0"), "");
    }

    #[test]
    fn currency_disabled_makes_no_request() {
        // would need the network if the setting were ignored
        assert_eq!(run("100 usd to eur"), "");
    }
}
