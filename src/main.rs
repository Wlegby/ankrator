use anker::notes::Note;
use anker::AnkiClient;
use std::collections::HashMap;
use std::fs;

mod parse_file;
use parse_file::parse_file;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // anker::launch_anki()?;

    let content = fs::read_to_string("input").unwrap();

    let parsed_file = parse_file(content.as_str())?;

    dbg!(parsed_file);

    Ok(())
}
