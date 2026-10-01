//! Position-aware XML events built on quick-xml.
//!
//! The POM editor needs byte offsets for every element so it can splice text
//! in place instead of re-serialising the document (which would destroy
//! formatting and comments).

use anyhow::{Result, anyhow};
use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ev {
    Start {
        name: String,
        start: usize,
        end: usize,
    },
    End {
        name: String,
        start: usize,
        end: usize,
    },
    Empty {
        name: String,
        start: usize,
        end: usize,
    },
    /// Text, entity references and CDATA merged into one run; offsets cover the raw source.
    Text {
        start: usize,
        end: usize,
    },
}

pub fn events(xml: &str) -> Result<Vec<Ev>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut out: Vec<Ev> = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let ev = reader.read_event().map_err(|e| anyhow!("XML error at byte {start}: {e}"))?;
        let end = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) => out.push(Ev::Start { name: local(e.local_name().as_ref()), start, end }),
            Event::End(e) => out.push(Ev::End { name: local(e.local_name().as_ref()), start, end }),
            Event::Empty(e) => out.push(Ev::Empty { name: local(e.local_name().as_ref()), start, end }),
            Event::Text(_) | Event::GeneralRef(_) | Event::CData(_) => {
                if let Some(Ev::Text { end: prev_end, .. }) = out.last_mut()
                    && *prev_end == start
                {
                    *prev_end = end;
                } else {
                    out.push(Ev::Text { start, end });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn local(name: &str) -> String {
    name.to_string()
}

/// Decodes the five predefined XML entities and numeric references.
pub fn unescape(raw: &str) -> String {
    let raw = raw.trim();
    let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(raw);
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(j) = rest.find(';') {
            let ent = &rest[1..j];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &rest[j + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_roundtrip() {
        let xml = "<a>\n  <b>x &amp; y</b>\n</a>";
        let evs = events(xml).unwrap();
        let text: Vec<_> = evs
            .iter()
            .filter_map(|e| if let Ev::Text { start, end } = e { Some(unescape(&xml[*start..*end])) } else { None })
            .collect();
        assert_eq!(text, vec!["", "x & y", ""]);
        let Ev::Start { start, end, name } = &evs[2] else { panic!() };
        assert_eq!(name, "b");
        assert_eq!(&xml[*start..*end], "<b>");
    }

    #[test]
    fn unescapes() {
        assert_eq!(unescape("a &lt; b &#65;"), "a < b A");
        assert_eq!(unescape("<![CDATA[x<y]]>"), "x<y");
    }
}
