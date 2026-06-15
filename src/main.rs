use anker::markdown::markdown_to_anki_with_typst;
use anker::notes::{format_cloze, Note, NoteUpdate};
use anker::AnkiClient;
use futures::future::BoxFuture;
use futures::FutureExt;
use std::collections::HashMap;
use std::env::args;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::time::SystemTime;

mod parse_file;
use parse_file::parse_file;

use crate::parse_file::{Parts, Types};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AnkiClient::default();
    let mut args = args();
    args.next();

    let mut ignore_cache = false;
    match args.next() {
        Some(s) => {
            if &s == "--no-cache" {
                ignore_cache = true;
            } else {
                eprintln!(
                    "Invalid argument {}\nonly --no-cache valid\n(continuing)",
                    s
                )
            }
        }
        None => {}
    }

    let mut num_files = 0;

    traverse(&client, Path::new("."), ignore_cache, &mut num_files).await?;

    println!("Successfully handled {} file(s)", num_files);
    Ok(())
}

fn traverse<'a>(
    client: &'a AnkiClient,
    path: &'a Path,
    no_cache: bool,
    num: &'a mut u32,
) -> BoxFuture<'a, Result<(), Box<dyn std::error::Error>>> {
    async move {
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    traverse(client, &path, no_cache, num).await?;
                } else if let Some(ex) = path.extension() {
                    if ex != "ak" {
                        continue;
                    }
                    let content = fs::read_to_string(&path)?;
                    match handle_file(
                        &content,
                        path.to_string_lossy().to_string(),
                        &client,
                        no_cache,
                    )
                    .await
                    {
                        Ok(_) => *num += 1,
                        Err(e) => eprintln!("Failed to handle file:\n{}", e),
                    }
                }
            }
        }
        Ok(())
    }
    .boxed()
}

#[derive(Debug, Clone, Copy)]
enum CardType<'a> {
    Cloze { text: &'a str },
    Basic { front: &'a str, back: &'a str },
}
impl<'a> Default for CardType<'a> {
    fn default() -> Self {
        Self::Cloze { text: "" }
    }
}

async fn handle_file<'a>(
    content: &'a str,
    path: String,
    client: &AnkiClient,
    no_cache: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !no_cache {
        match file_changed(&path) {
            Ok(changed) => {
                if !changed {
                    return Ok(());
                }
            }
            Err(i) => eprintln!(
                "Failed to check if file changed ({}), going to parse it anyways",
                i
            ),
        }
    }

    let mut parsed_file = parse_file(content)?;

    if parsed_file.is_empty() {
        eprint!("File does not contain ankrator items");
    }

    handle_parts(&mut parsed_file, path, client).await?;

    Ok(())
}

fn file_changed<'a>(path: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(mut dir) = dirs::cache_dir() {
        dir = dir.join("ankrator");
        fs::create_dir_all(&dir)?;
        dir = dir.join("cache.json");

        if !dir.exists() {
            fs::write(&dir, "{}")?;
        }

        let cache = fs::read_to_string(&dir)?;

        let map: HashMap<String, SystemTime> = serde_json::from_str(&cache)?;

        let last_time = match map.get(path) {
            Some(last_time) => *last_time,
            None => SystemTime::now(),
        };

        let metadata = fs::metadata(path)?;
        let modified_time: SystemTime = metadata.modified().unwrap();

        Ok(modified_time > last_time)
    } else {
        eprint!("Failed to get cache dir");
        Ok(true)
    }
}

fn add_cache(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mut dir) = dirs::cache_dir() {
        dir = dir.join("ankrator");
        fs::create_dir_all(&dir)?;
        dir = dir.join("cache.json");

        if !dir.exists() {
            fs::write(&dir, "{}")?;
        }

        let cache = fs::read_to_string(&dir)?;

        let mut map: HashMap<String, SystemTime> = serde_json::from_str(&cache)?;

        match map.get_mut(path) {
            Some(last_time) => {
                *last_time = SystemTime::now();
            }
            None => {
                map.insert(path.to_string(), SystemTime::now());
            }
        }

        let file = File::create(&dir)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &map)?;

        Ok(())
    } else {
        Err("Failed to get cache dir".into())
    }
}

async fn handle_parts<'a>(
    parsed_file: &mut Vec<Parts<'a>>,
    path: String,
    client: &AnkiClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut deck = "Default";
    let mut tags: Vec<&str> = Vec::new();
    let mut card_type = CardType::Cloze { text: "" };

    let mut new_file = String::new();
    let mut num_cards = 0;

    for part in parsed_file.iter_mut() {
        match part {
            Parts::DeckName(name) => {
                new_file.push_str(&format!("@deck {}\n", name));
                deck = name;
            }
            Parts::Tags(t) => {
                new_file.push_str(&format!("@tags "));
                for (i, tag) in t.iter().enumerate() {
                    new_file.push_str(&format!("{}", tag));
                    if i < t.len() - 1 {
                        new_file.push_str(&format!(", "));
                    } else {
                        new_file.push_str("\n\n");
                    }
                }

                tags = t.clone();
            }
            Parts::CardType(ctype) => match ctype {
                Types::Basic => {
                    new_file.push_str(&format!("# Basic\n"));
                    card_type = CardType::Basic {
                        front: "",
                        back: "",
                    }
                }
                Types::Cloze => {
                    new_file.push_str(&format!("# Cloze\n"));
                    card_type = CardType::Cloze { text: "" }
                }
                Types::Unknown => return Err("Failed to parse card type".into()),
            },
            Parts::Front(cfront) => match &mut card_type {
                CardType::Basic { front, back: _ } => {
                    *front = cfront;
                    new_file.push_str(&format!("Front: {}\n", front));
                }
                CardType::Cloze { text } => {
                    return Err(format!("Expected basic style card ({})", text).into());
                }
            },
            Parts::Back(cback) => match &mut card_type {
                CardType::Basic { front: _, back } => {
                    *back = cback;
                    new_file.push_str(&format!("Back: {}\n", back));
                }
                CardType::Cloze { text } => {
                    return Err(format!("Expected basic style card ({})", text).into())
                }
            },
            Parts::ClozeLine(line) => match &mut card_type {
                CardType::Basic { front, back } => {
                    return Err(format!("Expected basic style card ({} & {})", front, back).into())
                }
                CardType::Cloze { text } => {
                    *text = line;
                    new_file.push_str(&format!("Cloze: {}\n", text));
                }
            },
            Parts::CardEnd(cid) => {
                num_cards += 1;
                let id = match cid {
                    Some(i) => {
                        new_file.push_str(&format!("---NoteID:{}\n\n", i));
                        let parsed = match i.trim().parse::<i64>() {
                            Ok(i) => i,
                            Err(e) => return Err(e.into()),
                        };
                        Some(parsed)
                    }
                    None => None,
                };

                let mut fields = HashMap::new();
                let model_name = match card_type {
                    CardType::Cloze { text } => {
                        let _ = fields.insert(
                            "Text".to_string(),
                            markdown_to_anki_with_typst(&format_cloze(text)),
                        );
                        "Cloze".to_string()
                    }
                    CardType::Basic { front, back } => {
                        fields.insert("Front".to_string(), markdown_to_anki_with_typst(front));
                        fields.insert("Back".to_string(), markdown_to_anki_with_typst(back));
                        "Basic".to_string()
                    }
                };

                //ensure deck exists
                let _ = client.decks().create_deck(deck).await?;

                if let Some(id) = id {
                    let update = NoteUpdate {
                        id,
                        fields: Some(&fields),
                        tags: Some(&tags.iter().map(|t| t.to_string()).collect::<Vec<String>>()),
                    };

                    client.notes().update_note(&update).await?;
                    client.notes().update_note_deck(id, deck).await?;

                    continue;
                }

                let note = Note {
                    deck_name: deck.to_string(),
                    model_name,
                    fields,
                    tags: tags.iter().map(|t| t.to_string()).collect(),
                };

                let id = client.notes().add_note(&note).await?;
                new_file.push_str(&format!("---NoteID:{}\n\n", id));
                card_type = CardType::default();
            }
            Parts::Comment(c) => {
                new_file.push_str(&format!("//{}\n", c));
            }
        }
    }

    if num_cards == 0 {
        eprint!("{} Did not contain any cards", path);
    } else {
        add_cache(&path)?;
    }

    fs::write(&path, new_file)?;
    Ok(())
}
