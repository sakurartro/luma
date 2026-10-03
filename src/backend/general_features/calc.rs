use anyhow::{Result, anyhow};
use evalexpr::eval;

pub fn calc(input: &str) -> Result<String> {
    let result = match eval(input) {
        Ok(value) => value.to_string(),
        Err(_) => return Err(anyhow!("Error calc")),
    };
    Ok(result)
}
