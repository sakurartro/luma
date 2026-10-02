use super::currency;
use regex::Regex;
use anyhow::{anyhow, Result};

pub fn general_engine(input: String) -> Result<String> {
    let re = Regex::new(r"(\d+(?:\.\d+)?)\s+([A-Za-z]+)\s+to\s+([A-Za-z]+)").unwrap();
    if let Some(caps) = re.captures(&input) {
        let raw_amount = &caps[1];
        let amount: f64 = raw_amount
            .parse()
            .map_err(|e| anyhow!("f64 convert error"))?;
        let from = &caps[2];
        let to = &caps[3];
        let data = currency::convert(from, to, amount)?;
        let rate = data.price;
        let output = format!("{amount}{from} is {rate:.2}{to}");
        return Ok(output);
    }

    Err(anyhow!("currency convert error"))


}
