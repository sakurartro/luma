use super::calc::calc;
use super::currency;
use anyhow::{Result, anyhow};
use regex::Regex;

pub fn general_engine(input: String) -> Result<String> {
    let cur_re = Regex::new(r"(\d+(?:\.\d+)?)\s+([A-Za-z]+)\s+to\s+([A-Za-z]+)").unwrap();
    if let Some(caps) = cur_re.captures(&input) {
        let raw_amount = &caps[1];
        let amount: f64 = raw_amount
            .parse()
            .map_err(|_e| anyhow!("f64 convert error"))?;
        let from = &caps[2];
        let to = &caps[3];
        let data = currency::convert(from, to, amount)?;
        let rate = data.price;
        let output = format!("{amount}{from} is {rate:.2}{to}");
        return Ok(output);
    }
    let calc_re = Regex::new(r"(\d+)\s*([+-/*])\s(\d+)").unwrap();
    if let Some(_caps) = calc_re.captures(&input) {
        let ans = calc(&input)?;
        return Ok(ans);
    }

    Ok("".to_string())
}
