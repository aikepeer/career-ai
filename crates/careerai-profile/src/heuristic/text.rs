use crate::dates;

pub(super) fn split_title_company(line: &str) -> (String, String) {
    for sep in ["—", "–", " - ", " at ", ", ", " @ ", "|"] {
        if let Some(idx) = line.find(sep) {
            let (a, b) = line.split_at(idx);
            return (a.trim().to_string(), b[sep.len()..].trim().to_string());
        }
    }
    (line.trim().to_string(), String::new())
}

pub(super) fn split_name_url(line: &str) -> (String, String) {
    if let Some(start) = line.find("http") {
        return (
            line[..start]
                .trim_end_matches([' ', '—', '-', '|'])
                .trim()
                .to_string(),
            line[start..].trim().to_string(),
        );
    }
    (line.trim().to_string(), String::new())
}

pub(super) fn split_dates(line: &str) -> (String, String) {
    for sep in [" - ", " – ", " — ", " to "] {
        if let Some(idx) = line.find(sep) {
            let (a, b) = line.split_at(idx);
            return (dates::normalize(a), dates::normalize_end(&b[sep.len()..]));
        }
    }
    (dates::normalize(line), String::new())
}

pub(super) fn strip_bullet(line: &str) -> &str {
    let trimmed = line.trim_start();
    for marker in ["-", "•", "·", "*", "–", "—"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    trimmed
}
