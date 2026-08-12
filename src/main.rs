use anker::markdown::{markdown_to_anki_with_typst, upload_media};
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
use tokio::time;

mod parse_file;
use crate::parse_file::{Parts, Types};
use parse_file::parse_file;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AnkiClient::default();
    let mut args = args();
    args.next(); // Skip executable name

    let mut ignore_cache = false;
    let mut add_only = false;

    // Process all command line arguments
    while let Some(s) = args.next() {
        if s == "--no-cache" {
            ignore_cache = true;
        } else if s == "--add-only" || s == "add" {
            add_only = true;
        } else {
            eprintln!(
                "Invalid argument {}\nValid flags: --no-cache, --add-only\n(continuing)",
                s
            )
        }
    }

    let mut num_files = 0;
    traverse(
        &client,
        Path::new("."),
        ignore_cache,
        add_only,
        &mut num_files,
    )
    .await?;

    println!("Successfully handled {} file(s)", num_files);
    Ok(())
}

fn traverse<'a>(
    client: &'a AnkiClient,
    path: &'a Path,
    no_cache: bool,
    add_only: bool,
    num: &'a mut u32,
) -> BoxFuture<'a, Result<(), Box<dyn std::error::Error>>> {
    async move {
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() && path.file_name().and_then(|n| n.to_str()) != Some("artikel") {
                    traverse(client, &path, no_cache, add_only, num).await?;
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
                        add_only,
                        num,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(e) => eprintln!("Failed to handle file:\n{}", e),
                    }
                    time::sleep(time::Duration::from_millis(50)).await;
                }
            }
        }
        Ok(())
    }
    .boxed()
}

#[derive(Debug, Clone, Copy)]
enum CardType<'a> {
    Cloze {
        text: &'a str,
    },
    Basic {
        front: &'a str,
        back: &'a str,
        reversed: bool,
    },
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
    add_only: bool,
    num: &'a mut u32,
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
    *num += 1;
    let mut parsed_file = parse_file(content)?;
    if parsed_file.is_empty() {
        eprint!("File does not contain ankrator items");
    }
    handle_parts(&mut parsed_file, path, client, add_only).await?;
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
            None => return Ok(true),
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

fn append_part_fallback(new_file: &mut String, part: &Parts) {
    match part {
        Parts::DeckName(name) => {
            new_file.push_str(&format!("@deck {}\n", name));
        }
        Parts::Tags(t) => {
            new_file.push_str("@tags ");
            for (i, tag) in t.iter().enumerate() {
                new_file.push_str(tag);
                if i < t.len() - 1 {
                    new_file.push_str(", ");
                } else {
                    new_file.push_str("\n\n");
                }
            }
            if t.is_empty() {
                new_file.push_str("\n\n");
            }
        }
        Parts::CardType(ctype) => match ctype {
            Types::BasicRev => new_file.push_str("# Basic Rev\n"),
            Types::Basic => new_file.push_str("# Basic\n"),
            Types::Cloze => new_file.push_str("# Cloze\n"),
            Types::Unknown => {}
        },
        Parts::Front(cfront) => {
            new_file.push_str(&format!("Front: {}\n", cfront));
        }
        Parts::Back(cback) => {
            new_file.push_str(&format!("Back: {}\n", cback));
        }
        Parts::ClozeLine(line) => {
            new_file.push_str(&format!("Cloze: {}\n", line));
        }
        Parts::CardEnd(cid) => {
            if let Some(id) = cid {
                new_file.push_str(&format!("---NoteID:{}\n\n", id));
            } else {
                new_file.push_str("---\n\n");
            }
        }
        Parts::Comment(c) => {
            new_file.push_str(&format!("//{}\n", c));
        }
        Parts::Fast(fast) => {
            new_file.push_str(&format!("@startfast\n\n{}\n@endfast", fast));
        }
    }
}

async fn handle_parts<'a>(
    parsed_file: &mut Vec<Parts<'a>>,
    path: String,
    client: &AnkiClient,
    add_only: bool, // NEW: Added flag to handle_parts
) -> Result<(), Box<dyn std::error::Error>> {
    let mut deck = "Default";
    let mut tags: Vec<&str> = Vec::new();
    let mut card_type = CardType::Cloze { text: "" };
    let mut num_cards = 0;
    let mut new_file = String::new();

    for (idx, part) in parsed_file.iter().enumerate() {
        let res: Result<(), Box<dyn std::error::Error>> = async {
            match part {
                Parts::DeckName(name) => {
                    new_file.push_str(&format!("@deck {}\n", name));
                    deck = name;
                }
                Parts::Tags(t) => {
                    new_file.push_str("@tags ");
                    for (i, tag) in t.iter().enumerate() {
                        new_file.push_str(tag);
                        if i < t.len() - 1 {
                            new_file.push_str(", ");
                        } else {
                            new_file.push_str("\n\n");
                        }
                    }
                    tags = t.clone();
                }
                Parts::CardType(ctype) => match ctype {
                    Types::BasicRev => {
                        new_file.push_str("# Basic Rev\n");
                        card_type = CardType::Basic {
                            front: "",
                            back: "",
                            reversed: true,
                        };
                    }
                    Types::Basic => {
                        new_file.push_str("# Basic\n");
                        card_type = CardType::Basic {
                            front: "",
                            back: "",
                            reversed: false,
                        };
                    }
                    Types::Cloze => {
                        new_file.push_str("# Cloze\n");
                        card_type = CardType::Cloze { text: "" };
                    }
                    Types::Unknown => return Err("Failed to parse card type".into()),
                },
                Parts::Front(cfront) => match &mut card_type {
                    CardType::Basic {
                        front,
                        back: _,
                        reversed: _,
                    } => {
                        *front = cfront;
                        new_file.push_str(&format!("Front: {}\n", front));
                    }
                    CardType::Cloze { text } => {
                        return Err(format!("Expected basic style card ({})", text).into());
                    }
                },
                Parts::Back(cback) => match &mut card_type {
                    CardType::Basic {
                        front: _,
                        back,
                        reversed: _,
                    } => {
                        *back = cback;
                        new_file.push_str(&format!("Back: {}\n", back));
                    }
                    CardType::Cloze { text } => {
                        return Err(format!("Expected basic style card ({})", text).into());
                    }
                },
                Parts::ClozeLine(line) => match &mut card_type {
                    CardType::Basic {
                        front,
                        back,
                        reversed: _,
                    } => {
                        return Err(
                            format!("Expected cloze style card ({} & {})", front, back).into()
                        )
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
                            let parsed = match i.trim().parse::<i64>() {
                                Ok(i) => i,
                                Err(e) => return Err(e.into()),
                            };
                            Some(parsed)
                        }
                        None => None,
                    };

                    // NEW: If we are in add_only mode and the card has an ID, skip processing it
                    if add_only && id.is_some() {
                        new_file.push_str(&format!("---NoteID:{}\n\n", id.unwrap()));
                        card_type = CardType::default();
                        return Ok(());
                    }

                    let mut fields = HashMap::new();
                    let model_name = match card_type {
                        CardType::Cloze { text } => {
                            let after_media_upload = upload_media(client, text).await?;
                            let _ = fields.insert(
                                "Text".to_string(),
                                markdown_to_anki_with_typst(&format_cloze(&after_media_upload)),
                            );
                            "Cloze".to_string()
                        }
                        CardType::Basic {
                            front,
                            back,
                            reversed,
                        } => {
                            let front_uploaded = upload_media(client, front).await?;
                            let back_uploaded = upload_media(client, back).await?;
                            fields.insert(
                                "Front".to_string(),
                                markdown_to_anki_with_typst(&front_uploaded),
                            );
                            fields.insert(
                                "Back".to_string(),
                                markdown_to_anki_with_typst(&back_uploaded),
                            );
                            if reversed {
                                "Basic (and reversed card)".to_string()
                            } else {
                                "Basic".to_string()
                            }
                        }
                    };

                    // ensure deck exists
                    let _ = client.decks().create_deck(deck).await?;

                    if let Some(id) = id {
                        let update = NoteUpdate {
                            id,
                            fields: Some(&fields),
                            tags: Some(
                                &tags.iter().map(|t| t.to_string()).collect::<Vec<String>>(),
                            ),
                        };
                        client.notes().update_note(&update).await?;
                        client.notes().update_note_deck(id, deck).await?;
                        new_file.push_str(&format!("---NoteID:{}\n\n", id));
                        card_type = CardType::default();
                        return Ok(());
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
                Parts::Fast(fast) => {
                    new_file.push_str(&format!("@startfast\n\n{}\n@endfast", fast));
                }
            }
            Ok(())
        }
        .await;

        if let Err(e) = res {
            eprintln!(
                "Error occurred while processing file '{}': {}. Saving already generated Note IDs...",
                path, e
            );
            new_file.push_str("// Failed right here\n\n");
            for remaining_part in &parsed_file[idx..] {
                append_part_fallback(&mut new_file, remaining_part);
            }
            if let Err(write_err) = fs::write(&path, &new_file) {
                eprintln!("Failed to save recovery file to {}: {}", path, write_err);
            }
            return Err(e);
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
