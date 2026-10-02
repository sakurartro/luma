use serde::Deserialize;
use anyhow::{Result, anyhow};
use std::collections::HashMap;

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

pub fn convert(_from: &str, _to: &str, amount: f64) -> Result<CurrencyOuput> {
    let _from = _from.to_uppercase();
    let _to = _to.to_uppercase();
    let url = format!("https://open.er-api.com/v6/latest/{}", _from);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let data: ApiResponse = client.get(url).send()?.json()?;

    if data.result != "success" {
        return Err(anyhow!("Currency API error"));
    }
    let rate = data.rates
        .get(&_to)
        .ok_or_else(|| anyhow!("Currency {_to} found"))?;
    let price = amount * rate;
    Ok(CurrencyOuput{_from: _from, _to: _to, price: price})
}
