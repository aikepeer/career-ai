pub(super) fn prefer_nonempty(a: String, b: String) -> String {
    if a.trim().is_empty() {
        b
    } else {
        a
    }
}

pub(super) fn prefer_longer(a: String, b: String) -> String {
    if b.len() > a.len() {
        b
    } else {
        a
    }
}

pub(super) fn merge_vecs<T>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
    a.extend(b);
    a
}

/// Case-sensitive dedup preserving first-occurrence order.
/// Used for bullets where casing differences (e.g. acronyms) are meaningful.
pub(super) fn dedup_keep_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

/// Case-insensitive dedup preserving first-occurrence order and casing.
/// Used for skills where "Rust" and "RUST" are the same skill.
pub(super) fn dedup_keep_order_ci(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let key = item.to_lowercase();
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}
