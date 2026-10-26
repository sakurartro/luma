use anyhow::{Result, anyhow};
use evalexpr::eval_number;
use regex::Regex;

pub fn calc(input: &str) -> Result<String> {
    // evalexpr divides two ints as ints (7/2 = 3) and has no float-only mode:
    // turn every integer literal into a float literal.
    let num_re = Regex::new(r"\d+(\.\d+)?").unwrap();
    let input = num_re.replace_all(input, |c: &regex::Captures| {
        if c.get(1).is_some() { c[0].to_string() } else { format!("{}.0", &c[0]) }
    });
    match eval_number(&input) {
        // f64 Display already drops a trailing ".0" (4.0 -> "4")
        Ok(value) if value.is_finite() => Ok(value.to_string()),
        _ => Err(anyhow!("Error calc")),
    }
}
