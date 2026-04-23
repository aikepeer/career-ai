//! DOCX → plain text. DOCX is a ZIP containing `word/document.xml`; we walk
//! it with `quick-xml` collecting `<w:t>` runs and emit a blank line between
//! paragraphs so the heuristic splitter can see section boundaries.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::Result;

/// Extract text from a DOCX file on disk.
pub fn extract_text_from_path(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    extract_from_archive(&mut archive)
}

/// Extract text from DOCX bytes (tests).
pub fn extract_text_from_bytes(bytes: &[u8]) -> Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    extract_from_archive(&mut archive)
}

fn extract_from_archive<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<String> {
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")?
        .read_to_string(&mut xml)?;
    Ok(xml_to_text(&xml)?)
}

fn xml_to_text(xml: &str) -> std::result::Result<String, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event()? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"t" => in_text = true,
            Event::End(e) if local_name(e.name().as_ref()) == b"t" => in_text = false,
            Event::Text(e) if in_text => {
                out.push_str(&e.unescape()?);
            }
            Event::Empty(e) | Event::Start(e) if local_name(e.name().as_ref()) == b"br" => {
                out.push('\n');
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"p" => {
                out.push('\n');
                out.push('\n');
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_paragraph_breaks() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="x">
  <w:body>
    <w:p><w:r><w:t>Alice Kumar</w:t></w:r></w:p>
    <w:p><w:r><w:t>Senior Engineer</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let text = xml_to_text(xml).unwrap();
        assert!(text.contains("Alice Kumar"));
        assert!(text.contains("Senior Engineer"));
        // Paragraph closers inject the blank line the heuristic splitter expects.
        assert!(text.contains("\n\n"));
    }

    #[test]
    fn handles_namespaced_and_bare_tags() {
        let xml = r"<root><t>bare</t><w:t xmlns:w='x'>ns</w:t></root>";
        let text = xml_to_text(xml).unwrap();
        assert!(text.contains("bare"));
        assert!(text.contains("ns"));
    }

    #[test]
    fn unescapes_xml_entities() {
        let xml = "<root><t>A &amp; B</t></root>";
        let text = xml_to_text(xml).unwrap();
        assert!(text.contains("A & B"));
    }
}
