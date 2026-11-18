use std::error::Error;
use std::path::Path;

use super::{Record, decode};

pub(super) async fn read_records(input: &Path) -> Result<Vec<Record>, Box<dyn Error>> {
    let contents = std::fs::read_to_string(input)?;
    let mut records = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<Record>(line).map_err(|error| {
            format!(
                "{}:{}: malformed JSONL record: {error}",
                input.display(),
                line_number + 1
            )
        })?;
        if decode(&record.key_b64).is_none() || decode(&record.value_b64).is_none() {
            return Err(format!(
                "{}:{}: invalid base64 payload",
                input.display(),
                line_number + 1
            )
            .into());
        }
        records.push(record);
    }
    Ok(records)
}
