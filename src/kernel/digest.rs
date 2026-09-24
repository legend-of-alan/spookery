use std::fs;
use std::path::Path;

use crate::error::Code;
use crate::firewall::allows;
use crate::model::{Clock, ViewerId};
use crate::store::Shard;

pub enum DigestViewer {
    Host,
    Player(String),
}

pub fn idle_digest(shard: &Shard, viewer: DigestViewer) -> Result<String, Code> {
    let mut rows = Vec::new();
    match viewer {
        DigestViewer::Host => {
            rows.extend(effect_rows(shard.root())?);
            for (id, clock) in &shard.now().clocks {
                rows.push(clock_line(id, clock));
            }
        }
        DigestViewer::Player(id) => {
            let mut messages: Vec<_> = shard
                .now()
                .messages
                .iter()
                .filter(|message| message.to.iter().any(|to| to == &id))
                .collect();
            messages.sort_by(|left, right| left.id.cmp(&right.id));
            for message in messages {
                rows.push(format!("message {} {}", message.id, message.text));
            }
            let viewer = ViewerId::Player(id.clone());
            let mut rumors: Vec<_> = shard
                .now()
                .rumors
                .iter()
                .filter(|rumor| rumor.known_by.iter().any(|known| known == &id))
                .collect();
            rumors.sort_by(|left, right| left.id.cmp(&right.id));
            for rumor in rumors {
                let mut facts: Vec<_> = rumor
                    .fact_ids
                    .iter()
                    .filter(|fact| allows(shard.canon(), shard.now(), &viewer, fact))
                    .cloned()
                    .collect();
                facts.sort();
                facts.dedup();
                for fact in facts {
                    rows.push(format!("rumor {} {fact}", rumor.id));
                }
            }
            for (clock_id, clock) in &shard.now().clocks {
                if clock.owners.iter().any(|owner| owner == &id) {
                    rows.push(clock_line(clock_id, clock));
                }
            }
        }
    }
    if rows.is_empty() {
        Ok(String::new())
    } else {
        Ok(rows.join("\n") + "\n")
    }
}

fn clock_line(id: &str, clock: &Clock) -> String {
    let state = if clock.paused { "paused" } else { "running" };
    format!("clock {id} {} {} {state}", clock.value, clock.max)
}

fn effect_rows(root: &Path) -> Result<Vec<String>, Code> {
    let log_dir = root.join("state/log");
    if !log_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&log_dir).map_err(|_| Code::Schema)? {
        let entry = entry.map_err(|_| Code::Schema)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|_| Code::Schema)?;
        let (tick_index, effects) = parse_effects(&text)?;
        for (class, payload) in effects {
            rows.push((tick_index, format!("effect {class} {tick_index} {payload}")));
        }
    }
    rows.sort_by_key(|(tick_index, _)| *tick_index);
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}

fn parse_effects(text: &str) -> Result<(u64, Vec<(String, String)>), Code> {
    let bytes = text.as_bytes();
    let mut index = 0;
    skip_ws(bytes, &mut index);
    expect(bytes, &mut index, b'{')?;
    let mut tick_index = None;
    let mut effects = None;
    loop {
        skip_ws(bytes, &mut index);
        if index < bytes.len() && bytes[index] == b'}' {
            break;
        }
        if index < bytes.len() && bytes[index] == b',' {
            index += 1;
            skip_ws(bytes, &mut index);
        }
        let key = parse_string(bytes, &mut index)?;
        skip_ws(bytes, &mut index);
        expect(bytes, &mut index, b':')?;
        skip_ws(bytes, &mut index);
        if key == "tick_index" {
            tick_index = Some(parse_u64(bytes, &mut index)?);
        } else if key == "effects" {
            effects = Some(parse_effect_array(bytes, &mut index)?);
        } else {
            skip_value(bytes, &mut index)?;
        }
    }
    Ok((
        tick_index.ok_or(Code::Schema)?,
        effects.ok_or(Code::Schema)?,
    ))
}

fn parse_effect_array(bytes: &[u8], index: &mut usize) -> Result<Vec<(String, String)>, Code> {
    expect(bytes, index, b'[')?;
    let mut effects = Vec::new();
    loop {
        skip_ws(bytes, index);
        if *index < bytes.len() && bytes[*index] == b']' {
            *index += 1;
            break;
        }
        if *index < bytes.len() && bytes[*index] == b',' {
            *index += 1;
            skip_ws(bytes, index);
        }
        expect(bytes, index, b'{')?;
        let mut class = None;
        let mut payload = None;
        loop {
            skip_ws(bytes, index);
            if *index < bytes.len() && bytes[*index] == b'}' {
                *index += 1;
                break;
            }
            if *index < bytes.len() && bytes[*index] == b',' {
                *index += 1;
                skip_ws(bytes, index);
            }
            let key = parse_string(bytes, index)?;
            skip_ws(bytes, index);
            expect(bytes, index, b':')?;
            skip_ws(bytes, index);
            if key == "class" {
                class = Some(parse_string(bytes, index)?);
            } else if key == "payload" {
                payload = Some(raw_value(bytes, index)?);
            } else {
                skip_value(bytes, index)?;
            }
        }
        effects.push((class.ok_or(Code::Schema)?, payload.ok_or(Code::Schema)?));
    }
    Ok(effects)
}

fn raw_value(bytes: &[u8], index: &mut usize) -> Result<String, Code> {
    let start = *index;
    skip_value(bytes, index)?;
    let slice = bytes.get(start..*index).ok_or(Code::Schema)?;
    std::str::from_utf8(slice)
        .map(|text| text.to_string())
        .map_err(|_| Code::Schema)
}

fn skip_value(bytes: &[u8], index: &mut usize) -> Result<(), Code> {
    skip_ws(bytes, index);
    if *index >= bytes.len() {
        return Err(Code::Schema);
    }
    match bytes[*index] {
        b'{' => skip_container(bytes, index, b'{', b'}'),
        b'[' => skip_container(bytes, index, b'[', b']'),
        b'"' => {
            parse_string(bytes, index)?;
            Ok(())
        }
        b't' => skip_literal(bytes, index, b"true"),
        b'f' => skip_literal(bytes, index, b"false"),
        b'n' => skip_literal(bytes, index, b"null"),
        b'-' | b'0'..=b'9' => skip_number(bytes, index),
        _ => Err(Code::Schema),
    }
}

fn skip_container(bytes: &[u8], index: &mut usize, open: u8, close: u8) -> Result<(), Code> {
    expect(bytes, index, open)?;
    let mut depth = 1usize;
    while *index < bytes.len() && depth > 0 {
        match bytes[*index] {
            b'"' => {
                parse_string(bytes, index)?;
            }
            byte if byte == open => {
                depth += 1;
                *index += 1;
            }
            byte if byte == close => {
                depth -= 1;
                *index += 1;
            }
            _ => *index += 1,
        }
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(Code::Schema)
    }
}

fn skip_literal(bytes: &[u8], index: &mut usize, literal: &[u8]) -> Result<(), Code> {
    if bytes.get(*index..).is_some_and(|rest| rest.starts_with(literal)) {
        *index += literal.len();
        Ok(())
    } else {
        Err(Code::Schema)
    }
}

fn skip_number(bytes: &[u8], index: &mut usize) -> Result<(), Code> {
    if *index < bytes.len() && bytes[*index] == b'-' {
        *index += 1;
    }
    let start = *index;
    while *index < bytes.len() && bytes[*index].is_ascii_digit() {
        *index += 1;
    }
    if start == *index {
        Err(Code::Schema)
    } else {
        Ok(())
    }
}

fn parse_u64(bytes: &[u8], index: &mut usize) -> Result<u64, Code> {
    if *index < bytes.len() && bytes[*index] == b'-' {
        return Err(Code::Schema);
    }
    let start = *index;
    while *index < bytes.len() && bytes[*index].is_ascii_digit() {
        *index += 1;
    }
    if start == *index {
        return Err(Code::Schema);
    }
    std::str::from_utf8(&bytes[start..*index])
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(Code::Schema)
}

fn parse_string(bytes: &[u8], index: &mut usize) -> Result<String, Code> {
    expect(bytes, index, b'"')?;
    let mut out = String::new();
    while *index < bytes.len() {
        let byte = bytes[*index];
        *index += 1;
        match byte {
            b'"' => return Ok(out),
            b'\\' => {
                if *index >= bytes.len() {
                    return Err(Code::Schema);
                }
                let escaped = bytes[*index];
                *index += 1;
                match escaped {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = bytes.get(*index..*index + 4).ok_or(Code::Schema)?;
                        let text = std::str::from_utf8(hex).map_err(|_| Code::Schema)?;
                        let code = u32::from_str_radix(text, 16).map_err(|_| Code::Schema)?;
                        out.push(char::from_u32(code).ok_or(Code::Schema)?);
                        *index += 4;
                    }
                    _ => return Err(Code::Schema),
                }
            }
            _ => out.push(byte as char),
        }
    }
    Err(Code::Schema)
}

fn expect(bytes: &[u8], index: &mut usize, byte: u8) -> Result<(), Code> {
    if *index < bytes.len() && bytes[*index] == byte {
        *index += 1;
        Ok(())
    } else {
        Err(Code::Schema)
    }
}

fn skip_ws(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_effects;

    #[test]
    fn effect_payload_keeps_a_negative_delta() {
        let text = r#"{"tick_index":1,"effects":[{"class":"stock","payload":{"delta":-1}}]}"#;
        let (tick_index, effects) = parse_effects(text).unwrap();
        assert_eq!(tick_index, 1);
        assert_eq!(
            effects,
            vec![("stock".to_string(), r#"{"delta":-1}"#.to_string())]
        );
    }
}
