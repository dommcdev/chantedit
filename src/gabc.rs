//! Lossless GABC chord editing. Musical source is retained, never reserialized.

use std::ops::Range;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Chord {
    pub anchor: usize,
    pub text: String,
    pub dx: f64,
    /// Positive is up, in PDF big points (bp).
    pub dy: f64,
    pub automatic_raise: f64,
}

#[derive(Clone, Debug)]
pub struct Anchor {
    pub offset: usize,
    pub end: usize,
}

const NOTE_PREFIX: &str =
    "[nv:\\hbox to0pt{\\kern-\\csname gre@dimen@lastglyphwidth\\endcsname\\GreSetTextAboveLines{";
const NOTE_SUFFIX: &str = "}\\csname gre@currenttextabovelines\\endcsname\\hss}]";

/// Attach a zero-width annotation to the first note without adding an element
/// or glyph. The note-level hook runs at the right edge of its rendered glyph.
fn insertion_offset(music: &str, anchor: &Anchor) -> Result<usize, String> {
    let bytes = music.as_bytes();
    let mut offset = anchor.offset + if bytes[anchor.offset] == b'-' { 2 } else { 1 };
    while offset < bytes.len() {
        if bytes[offset] == b'[' {
            offset = bracket_end(music, offset)?;
        } else if matches!(bytes[offset], b'a'..=b'm' | b'A'..=b'M' | b'/' | b'!' | b'(' | b')' | b'%' | b',' | b';' | b':' | b'`' | b'z' | b'Z')
            || bytes[offset].is_ascii_whitespace()
        {
            break;
        } else {
            offset += 1;
        }
    }
    Ok(offset)
}

fn gap_has_music(source: &str) -> bool {
    let b = source.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            i = source[i..].find('\n').map_or(b.len(), |n| i + n + 1);
        } else if b[i] == b'[' {
            i = bracket_end(source, i).unwrap_or(b.len());
        } else {
            if b[i].is_ascii_alphabetic() || matches!(b[i], b'(' | b')') {
                return true;
            }
            i += 1;
        }
    }
    false
}

pub struct Document {
    original: String,
    pub music: String,
    pub anchors: Vec<Anchor>,
    pub chords: Vec<Chord>,
    initial: Vec<Chord>,
    annotations: Vec<(usize, Chord, String)>,
    pub name: String,
}

fn body_start(source: &str) -> Result<usize, String> {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim() == "%%" {
            return Ok(offset + line.len());
        }
        offset += line.len();
    }
    Err("Missing GABC header separator (%% on its own line)".into())
}

fn bracket_end(source: &str, start: usize) -> Result<usize, String> {
    source[start..]
        .find(']')
        .map(|n| start + n + 1)
        .ok_or_else(|| "Unclosed GABC annotation".into())
}

/// Boundaries already present in the music; no new spaces or glyph cuts.
fn anchors(source: &str) -> Result<Vec<Anchor>, String> {
    let b = source.as_bytes();
    let mut i = body_start(source)?;
    let mut inside = false;
    let mut boundary = true;
    let mut result: Vec<Anchor> = Vec::new();
    while i < b.len() {
        match b[i] {
            b'%' => {
                i = source[i..].find('\n').map_or(b.len(), |n| i + n + 1);
                continue;
            }
            b'[' => {
                i = bracket_end(source, i)?;
                continue;
            }
            b'<' if !inside => {
                if source[i..].starts_with("<v>") {
                    i = source[i..]
                        .find("</v>")
                        .map(|n| i + n + 4)
                        .ok_or("Unclosed <v> tag")?;
                } else {
                    i = source[i..].find('>').map_or(b.len(), |n| i + n + 1);
                }
                continue;
            }
            b'(' => {
                inside = true;
                boundary = true;
            }
            b')' => {
                inside = false;
                boundary = true;
            }
            _ if inside => {
                if b[i] == b'@' || b[i] == b'|' {
                    return Err(
                        "Fused-neume and NABC notation are not yet supported by the editor preview"
                            .into(),
                    );
                }
                // Clefs and accidentals are not chord targets.
                let clef_len = if matches!(b[i], b'c' | b'f') {
                    if b.get(i + 1) == Some(&b'b') && b.get(i + 2).is_some_and(u8::is_ascii_digit) {
                        3
                    } else if b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                        2
                    } else {
                        0
                    }
                } else {
                    0
                };
                if clef_len > 0 {
                    i += clef_len;
                    boundary = true;
                    continue;
                }
                if matches!(b[i], b'a'..=b'm' | b'A'..=b'M') {
                    if b.get(i + 1)
                        .is_some_and(|v| matches!(v, b'x' | b'y' | b'#' | b'+'))
                    {
                        i += 2;
                        continue;
                    }
                    if boundary {
                        let offset = if i > 0 && b[i - 1] == b'-' { i - 1 } else { i };
                        result.push(Anchor { offset, end: i + 1 });
                    }
                    boundary = false;
                    if let Some(a) = result.last_mut() {
                        a.end = i + 1;
                    }
                } else if matches!(
                    b[i],
                    b'/' | b'!'
                        | b' '
                        | b'\t'
                        | b'\r'
                        | b'\n'
                        | b','
                        | b';'
                        | b':'
                        | b'`'
                        | b'z'
                        | b'Z'
                ) {
                    boundary = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if inside {
        return Err("Unclosed music parentheses".into());
    }
    Ok(result)
}

fn plain_chord(text: &str) -> bool {
    fn part(mut text: &str) -> bool {
        if !text
            .as_bytes()
            .first()
            .is_some_and(|c| matches!(c, b'A'..=b'G'))
        {
            return false;
        }
        text = &text[1..];
        if text.starts_with(['#', 'b']) {
            text = &text[1..];
        }
        while !text.is_empty() {
            if text.as_bytes()[0].is_ascii_digit() {
                text = &text[1..];
                continue;
            }
            let Some(prefix) = [
                "maj", "min", "dim", "aug", "sus", "add", "omit", "no", "m", "M", "Δ", "°", "ø",
                "+", "-", "#", "b", ".",
            ]
            .into_iter()
            .find(|p| text.starts_with(p)) else {
                return false;
            };
            text = &text[prefix.len()..];
        }
        true
    }
    let normalized = text
        .replace('♯', "#")
        .replace('♭', "b")
        .replace('−', "-")
        .replace(['(', ')'], "");
    !normalized.trim().is_empty()
        && normalized.split_whitespace().all(|word| {
            if matches!(word, "N.C." | "NC") {
                return true;
            }
            let parts: Vec<_> = word.split('/').collect();
            parts.len() <= 2 && parts.iter().all(|text| part(text))
        })
}

fn take_number(source: &mut &str, prefix: &str, suffix: &str) -> Option<f64> {
    *source = source.strip_prefix(prefix)?;
    let end = source.find(suffix)?;
    let value = source[..end]
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())?;
    *source = &source[end + suffix.len()..];
    Some(value)
}

fn decode(text: &str) -> Option<Chord> {
    if plain_chord(text) {
        return Some(Chord {
            anchor: 0,
            text: text.into(),
            dx: 0.0,
            dy: 0.0,
            automatic_raise: 0.0,
        });
    }
    let mut rest = text;
    let dx = take_number(&mut rest, "\\kern ", "bp")?;
    let automatic_raise = take_number(&mut rest, "\\raise ", "bp\\hbox{")?;
    let dy = take_number(
        &mut rest,
        "\\raise ",
        "bp\\hbox{\\fontsize{9bp}{11bp}\\selectfont\\upshape\\bfseries ",
    )?;
    let text = rest.strip_suffix("}}")?;
    let text = text
        .replace("\\ensuremath{\\sharp}", "♯")
        .replace("\\ensuremath{\\flat}", "♭");
    if !plain_chord(&text) {
        return None;
    }
    Some(Chord {
        anchor: 0,
        text,
        dx,
        dy,
        automatic_raise,
    })
}

fn encode(chord: &Chord) -> Result<String, String> {
    if !plain_chord(&chord.text) {
        return Err(format!("Unsupported chord text: {}", chord.text));
    }
    if ![chord.dx, chord.dy, chord.automatic_raise]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err("Chord offsets must be finite".into());
    }
    let text = chord
        .text
        .replace(['♯', '#'], "\\ensuremath{\\sharp}")
        .replace('♭', "\\ensuremath{\\flat}");
    let content = format!(
        "\\kern {}bp\\raise {}bp\\hbox{{\\raise {}bp\\hbox{{\\fontsize{{9bp}}{{11bp}}\\selectfont\\upshape\\bfseries {}}}}}",
        chord.dx, chord.automatic_raise, chord.dy, text
    );
    Ok(format!("{NOTE_PREFIX}{content}{NOTE_SUFFIX}"))
}

impl Document {
    pub fn parse(source: String) -> Result<Self, String> {
        let start = body_start(&source)?;
        let name = source[..start]
            .lines()
            .find_map(|l| l.strip_prefix("name:"))
            .unwrap_or("Untitled chant")
            .trim()
            .trim_end_matches(';')
            .to_owned();
        let mut removals: Vec<(Range<usize>, Chord)> = Vec::new();
        let mut i = start;
        let b = source.as_bytes();
        while i < b.len() {
            if b[i] == b'%' {
                i = source[i..].find('\n').map_or(b.len(), |n| i + n + 1);
                continue;
            }
            if b[i] == b'<' && source[i..].starts_with("<v>") {
                i = source[i..]
                    .find("</v>")
                    .map(|n| i + n + 4)
                    .ok_or("Unclosed <v> tag")?;
            } else if b[i] == b'<' && source[i..].starts_with("<alt>") {
                let end = source[i + 5..]
                    .find("</alt>")
                    .map(|n| i + 5 + n)
                    .ok_or("Unclosed <alt> tag")?;
                if let Some(chord) = decode(&source[i + 5..end]) {
                    removals.push((i..end + 6, chord));
                }
                i = end + 6;
            } else if b[i] == b'[' {
                let end = bracket_end(&source, i)?;
                if let Some(text) = source[i..end]
                    .strip_prefix("[alt:")
                    .and_then(|s| s.strip_suffix(']'))
                    .or_else(|| {
                        source[i..end]
                            .strip_prefix("[gv:\\GreSetTextAboveLines{")
                            .and_then(|s| s.strip_suffix("}]"))
                    })
                    .or_else(|| {
                        source[i..end]
                            .strip_prefix(NOTE_PREFIX)
                            .and_then(|s| s.strip_suffix(NOTE_SUFFIX))
                    })
                    && let Some(chord) = decode(text)
                {
                    removals.push((i..end, chord));
                }
                i = end;
            } else {
                i += 1;
            }
        }
        let mut music = String::new();
        let mut previous = 0;
        let mut pending = Vec::new();
        for (range, chord) in &removals {
            music.push_str(&source[previous..range.start]);
            pending.push((music.len(), chord.clone(), source[range.clone()].to_owned()));
            previous = range.end;
        }
        music.push_str(&source[previous..]);
        let anchors = anchors(&music)?;
        let mut chords = Vec::new();
        let mut annotations = Vec::new();
        for (offset, mut chord, tag) in pending {
            let index = if tag.starts_with(NOTE_PREFIX) {
                anchors.iter().rposition(|a| a.offset < offset)
            } else {
                anchors.iter().position(|a| a.offset >= offset)
            }
            .ok_or("A chord annotation has no following neume")?;
            chord.anchor = index;
            if tag.starts_with("[alt:") && gap_has_music(&music[offset..anchors[index].offset]) {
                return Err("A chord lies inside a neume or before an accidental. Move its [alt] tag to an existing neume boundary before editing.".into());
            }
            if chords.iter().any(|c: &Chord| c.anchor == index) {
                return Err("Multiple chord annotations at one neume are not supported".into());
            }
            annotations.push((offset, chord.clone(), tag));
            chords.push(chord);
        }
        if anchors.is_empty() {
            return Err("No supported neumes found in the GABC file".into());
        }
        Ok(Self {
            original: source,
            music,
            anchors,
            initial: chords.clone(),
            annotations,
            chords,
            name,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.chords != self.initial
    }

    pub fn serialize(&self) -> Result<String, String> {
        if !self.is_dirty() {
            return Ok(self.original.clone());
        }
        let mut insertions = Vec::new();
        for c in &self.chords {
            if c.text.is_empty() {
                continue;
            }
            let anchor = self.anchors.get(c.anchor).ok_or("Invalid chord anchor")?;
            if let Some((offset, _, tag)) =
                self.annotations.iter().find(|(_, initial, _)| initial == c)
            {
                insertions.push((*offset, tag.clone()));
            } else {
                insertions.push((insertion_offset(&self.music, anchor)?, encode(c)?));
            }
        }
        insertions.sort_by_key(|(offset, _)| *offset);
        let mut result = String::new();
        let mut previous = 0;
        for (offset, text) in insertions {
            result.push_str(&self.music[previous..offset]);
            result.push_str(&text);
            previous = offset;
        }
        result.push_str(&self.music[previous..]);
        Ok(result)
    }

    /// Atomic replacement; no half-written score after an interrupted save.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let source = self.serialize()?;
        let temporary = path.with_file_name(format!(
            ".{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        use std::io::Write;
        let mut created = false;
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            created = true;
            if let Ok(metadata) = std::fs::metadata(path) {
                file.set_permissions(metadata.permissions())?;
            }
            file.write_all(source.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() && created {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|e| e.to_string())?;
        self.original = source;
        self.initial = self.chords.clone();
        Ok(())
    }

    /// Strip comments for the preview without shifting byte/UTF-16 positions.
    pub fn preview_source(&self) -> String {
        let start = body_start(&self.music).expect("validated header");
        let mut result = self.music[..start].to_owned();
        for line in self.music[start..].split_inclusive('\n') {
            if let Some(comment) = line.find('%') {
                result.push_str(&line[..comment]);
                for ch in line[comment..].chars() {
                    if ch == '\n' || ch == '\r' {
                        result.push(ch);
                    } else {
                        result.push_str(&" ".repeat(ch.len_utf16()));
                    }
                }
            } else {
                result.push_str(line);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "name: Test;\r\n% leave this alone\r\n%%\r\n(c4) Dó([alt:C]ghg/h.)mi(hi)nus.(g.) (::) % comment\r\n";

    #[test]
    fn lossless_round_trip_and_targeted_edit() {
        let mut doc = Document::parse(SOURCE.into()).unwrap();
        assert_eq!(doc.serialize().unwrap(), SOURCE);
        assert_eq!(doc.anchors.len(), 4);
        doc.chords[0].text = "Dm".into();
        assert!(doc.serialize().unwrap().contains("\\bfseries Dm"));
        let reload = Document::parse(doc.serialize().unwrap()).unwrap();
        assert_eq!(doc.chords, reload.chords);
        assert_eq!(doc.music, reload.music);
    }

    #[test]
    fn offsets_and_stagger_round_trip_without_musical_changes() {
        let mut doc = Document::parse(SOURCE.into()).unwrap();
        doc.chords[0].dx = -1.25;
        doc.chords[0].dy = 2.5;
        doc.chords[0].automatic_raise = 11.0;
        let saved = doc.serialize().unwrap();
        let mut reload = Document::parse(saved.clone()).unwrap();
        assert_eq!(reload.music, doc.music);
        assert_eq!(reload.chords, doc.chords);
        reload.chords[0].text = "G7".into();
        let again = Document::parse(reload.serialize().unwrap()).unwrap();
        assert_eq!(again.chords, reload.chords);
        assert_eq!(again.music, doc.music);
    }

    #[test]
    fn does_not_treat_clefs_accidentals_or_commands_as_notes() {
        let doc = Document::parse("name:T;\n%%\n(cb4) A(ixghg[cs:a]/h!i.) (::) % (abc)\n".into())
            .unwrap();
        assert_eq!(
            doc.anchors
                .iter()
                .map(|a| &doc.music[a.offset..a.end])
                .collect::<Vec<_>>(),
            ["ghg", "h", "i"]
        );
    }

    #[test]
    fn preserves_non_chord_annotations() {
        let source = "name:T;\n%%\n(c4) A([alt:rit.]g) B([alt:Cantor]h) (::)\n";
        let mut doc = Document::parse(source.into()).unwrap();
        doc.chords.push(Chord {
            anchor: 1,
            text: "C".into(),
            dx: 0.0,
            dy: 0.0,
            automatic_raise: 0.0,
        });
        let saved = doc.serialize().unwrap();
        assert!(saved.contains("[alt:rit.]"));
        assert!(saved.contains("[alt:Cantor]"));
        assert_eq!(Document::parse(saved).unwrap().music, source);
    }

    #[test]
    fn imported_lyric_tags_comments_and_repeated_saves_are_lossless() {
        let source = "name:T;\r\n%%\r\n(c4) <alt>C</alt>A(ghg/ % 😀 leave this\r\n h.) B([alt:G]hi) (::)\r\n";
        let mut doc = Document::parse(source.into()).unwrap();
        assert_eq!(doc.chords.len(), 2);
        assert_eq!(doc.serialize().unwrap(), source);
        doc.chords.push(Chord {
            anchor: 1,
            text: "F♯sus4/C♯".into(),
            dx: -2.0,
            dy: 1.0,
            automatic_raise: 12.6,
        });
        let dir = std::env::temp_dir().join(format!("chantedit-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.gabc");
        doc.save(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("<alt>C</alt>"));
        assert!(first.contains("[alt:G]"));
        assert!(first.contains("% 😀 leave this\r\n"));
        doc.chords[2].dx += 0.5;
        doc.save(&path).unwrap();
        let reload = Document::parse(std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut expected = doc.chords.clone();
        expected.sort_by_key(|c| c.anchor);
        assert_eq!(expected, reload.chords);
        assert_eq!(doc.music, reload.music);
        doc.save(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            reload.serialize().unwrap()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
