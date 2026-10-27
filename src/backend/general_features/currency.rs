use serde::Deserialize;
use anyhow::{Result, anyhow};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Deserialize, Debug)]
pub struct CurrencyOuput {
    pub _from: String,
    pub _to: String,
    pub price: f64,
}

#[derive(Deserialize)]
struct ApiResponse {
    result: String,
    rates: HashMap<String, f64>,
}

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
type Rates = HashMap<String, f64>;
// Rates per base currency, so typing doesn't refetch on every keystroke.
static RATES: LazyLock<Mutex<HashMap<String, (Instant, Rates)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// "<amount> <CODE> to <CODE>" with 3-letter codes (any case) → (amount, FROM, TO).
pub fn parse_query(input: &str) -> Option<(f64, String, String)> {
    let re = Regex::new(r"(?i)^\s*(\d+(?:\.\d+)?)\s+([a-z]{3})\s+to\s+([a-z]{3})\s*$").unwrap();
    let caps = re.captures(input)?;
    Some((caps[1].parse().ok()?, caps[2].to_uppercase(), caps[3].to_uppercase()))
}

pub fn format_result(amount: f64, from: &str, to: &str, price: f64) -> String {
    format!("{amount} {from} = {price:.2} {to}")
}

pub fn convert(_from: &str, _to: &str, amount: f64) -> Result<CurrencyOuput> {
    let _from = _from.to_uppercase();
    let _to = _to.to_uppercase();
    // Held across the fetch: concurrent keystrokes wait for one request
    // instead of each firing their own.
    let mut cache = RATES.lock().unwrap();
    if !cache.get(&_from).is_some_and(|(at, _)| at.elapsed() < CACHE_TTL) {
        let url = format!("https://open.er-api.com/v6/latest/{}", _from);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()?;
        let data: ApiResponse = client.get(url).send()?.json()?;

        if data.result != "success" {
            return Err(anyhow!("Currency API error"));
        }
        cache.insert(_from.clone(), (Instant::now(), data.rates));
    }
    let rate = cache[&_from].1
        .get(&_to)
        .ok_or_else(|| anyhow!("Currency {_to} not found"))?;
    let price = amount * rate;
    Ok(CurrencyOuput{_from: _from, _to: _to, price: price})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_only_three_letter_codes() {
        assert_eq!(parse_query("100 usd to EUR"), Some((100.0, "USD".into(), "EUR".into())));
        assert_eq!(parse_query("1.5 USD TO eur"), Some((1.5, "USD".into(), "EUR".into())));
        // mid-typing / not a currency code
        assert_eq!(parse_query("100 usd to eu"), None);
        assert_eq!(parse_query("5 apps to install"), None);
    }

    #[test]
    fn output_format() {
        assert_eq!(format_result(100.0, "USD", "EUR", 92.3149), "100 USD = 92.31 EUR");
    }
}
