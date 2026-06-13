use rparse::{literal, split_at, take_until, Parser};

#[derive(Debug)]
pub enum Parts<'a> {
    DeckName(&'a str),
    Tags(Vec<&'a str>),
    CardType(Types),
    Front(&'a str),
    Back(&'a str),
    ClozeLine(&'a str),
    CardEnd,
    Comment(&'a str),
}

#[derive(Debug)]
pub enum Types {
    Basic,
    Cloze,
    Unknown,
}

pub fn parse_deck<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("@deck ")
        .and(take_until(|c| c == '\n' || c == '\r'))
        .map(|(_, deck)| Parts::DeckName(deck))
}

pub fn parse_tags<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("@tags ")
        .and(take_until(|c| c == '\n' || c == '\r'))
        .map(|(_, tag)| {
            let tags = tag.split(',').map(|t| t.trim()).collect::<Vec<&str>>();
            Parts::Tags(tags)
        })
}
pub fn parse_card_end<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("---").map(|_| Parts::CardEnd)
}

pub fn parse_card_type<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("# ")
        .and(take_until(|c| c == '\n' || c == '\r'))
        .map(|(_, ctype)| {
            let _type = match ctype.to_lowercase().as_str() {
                "basic" => Types::Basic,
                "cloze" => Types::Cloze,
                _ => Types::Unknown,
            };
            Parts::CardType(_type)
        })
}
pub fn parse_front<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("Front: ")
        .and(split_at("Back:"))
        .map(|(_, text)| Parts::Front(text))
}

pub fn parse_back<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("Back: ")
        .and(split_at("---"))
        .map(|(_, text)| Parts::Back(text))
}

pub fn parse_cloze<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("Cloze: ")
        .and(split_at("---"))
        .map(|(_, text)| Parts::ClozeLine(text))
}
pub fn parse_comment<'a>() -> impl Parser<'a, Parts<'a>> {
    literal("//")
        .and(take_until(|c| c == '\n' || c == '\r'))
        .map(|(_, comment)| Parts::Comment(comment))
}

pub fn parse_file<'a>(mut input: &'a str) -> Result<Vec<Parts<'a>>, &'a str> {
    let mut parts = Vec::new();

    let parts_parser = parse_deck()
        .or(parse_tags())
        .or(parse_card_type())
        .or(parse_front())
        .or(parse_back())
        .or(parse_cloze())
        .or(parse_card_end())
        .or(parse_comment());

    let whitespace = literal("\n").or(literal("\r")).or(literal("\r\n"));

    while !input.is_empty() {
        match whitespace.parse(input) {
            Ok(_) => input = input.trim(),
            Err(_) => {}
        }

        match parts_parser.parse(input) {
            Ok((rest, part)) => {
                parts.push(part);
                input = rest;
            }
            Err(i) => return Err(i),
        }
    }

    Ok(parts)
}
