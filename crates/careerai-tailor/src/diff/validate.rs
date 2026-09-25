//! Validate a [`DiffDoc`] against a [`Profile`].
//!
//! Nine rules in order:
//!
//! 1. Coverage — every profile bullet appears in exactly one op.
//! 2. Paths parse.
//! 3. Paths point to real profile bullets.
//! 4. `move_before` target stays within the same entry.
//! 5. `drop` cannot empty an experience (≥ 1 bullet must remain).
//! 6. Reword length ≤ 280 chars.
//! 7. Reword entity guardrails via `guardrails::forbid_invented_entities`.
//! 8. Summary reword respects employer-token guardrails.
//! 9. Cover letter ≤ 350 whitespace-split words.

use std::collections::{HashMap, HashSet};

use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};
use crate::guardrails;

use super::parse::{entry_bullet_count, original_bullet_text, profile_bullet_paths};
use super::schema::{BulletPath, DiffDoc, DiffOp, OpKind, Section, SummaryOp};

pub(crate) const MAX_BULLET_CHARS: usize = 280;
const MAX_COVER_LETTER_WORDS: usize = 350;
/// Hard char cap on cover letters — catches LLMs that emit one giant
/// no-whitespace string that trivially bypasses `MAX_COVER_LETTER_WORDS`.
/// Sized for ~350 words × average English word length ~6 chars + slack.
const MAX_COVER_LETTER_CHARS: usize = 3500;

/// Validate a `DiffDoc` against a profile.
///
/// Nine rules in order (see module docs). Token sets are built ONCE here
/// and shared across every reword op.
///
/// `jd_text` is the job description text (title + company + description).
/// Proper nouns and vocabulary from the JD are added to the allowed
/// token set so rewords may legitimately align with JD terminology.
#[allow(clippy::too_many_lines)]
pub fn validate(doc: &DiffDoc, profile: &Profile, jd_text: &str) -> Result<()> {
    // Rule 9 first — cheap, independent of the ops list.
    // Enforce both a word cap and a char cap; the char cap closes a
    // word-only-counting bypass where an LLM could emit one huge
    // no-whitespace blob and pass as "1 word".
    let cl_chars = doc.cover_letter.chars().count();
    if cl_chars > MAX_COVER_LETTER_CHARS {
        return Err(TailorError::CoverLetterCharsTooLong {
            chars: cl_chars,
            cap: MAX_COVER_LETTER_CHARS,
        });
    }
    let cl_words = doc.cover_letter.split_whitespace().count();
    if cl_words > MAX_COVER_LETTER_WORDS {
        return Err(TailorError::CoverLetterTooLong {
            words: cl_words,
            cap: MAX_COVER_LETTER_WORDS,
        });
    }

    // Rule 2 — parse every op's path, every move_before target. Collect
    // parsed paths so rules 1/3/4/5 can operate on typed values.
    let mut parsed_ops: Vec<(BulletPath, &DiffOp)> = Vec::with_capacity(doc.ops.len());
    for op in &doc.ops {
        let bp = BulletPath::parse(&op.path)?;
        parsed_ops.push((bp, op));
        if let OpKind::MoveBefore { target_path } = &op.kind {
            // Validate target parses; also validated in rule 4 for entry shape.
            let _ = BulletPath::parse(target_path)?;
        }
    }

    // Rule 3 — every parsed op path must point at a real bullet.
    for (bp, op) in &parsed_ops {
        let Some(count) = entry_bullet_count(profile, &bp.section, bp.entry_index) else {
            return Err(TailorError::Schema(format!(
                "path references missing entry: {}",
                op.path
            )));
        };
        if bp.bullet_index >= count {
            return Err(TailorError::Schema(format!(
                "path references missing bullet: {}",
                op.path
            )));
        }
        // MoveBefore target must also exist.
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let tp = BulletPath::parse(target_path)?;
            let Some(tcount) = entry_bullet_count(profile, &tp.section, tp.entry_index) else {
                return Err(TailorError::Schema(format!(
                    "move_before target missing entry: {target_path}"
                )));
            };
            if tp.bullet_index >= tcount {
                return Err(TailorError::Schema(format!(
                    "move_before target missing bullet: {target_path}"
                )));
            }
        }
    }

    // Rule 1 — coverage + no duplicates.
    // Build the set of expected paths from the profile; check every op
    // appears exactly once as a source (target_path does not count).
    let expected: HashSet<BulletPath> = profile_bullet_paths(profile).into_iter().collect();
    let mut seen: HashSet<BulletPath> = HashSet::with_capacity(parsed_ops.len());
    for (bp, op) in &parsed_ops {
        if !seen.insert(bp.clone()) {
            return Err(TailorError::Schema(format!(
                "duplicate op path: {}",
                op.path
            )));
        }
    }
    for want in &expected {
        if !seen.contains(want) {
            return Err(TailorError::Schema(format!(
                "bullet not covered: {}",
                want.format()
            )));
        }
    }
    // Extra ops not present in the profile were already rejected by rule 3.

    // Rule 4 — cross-entry move guard + no-op / self-target guard.
    // A `move_before` with target == source silently reorders to end in
    // `apply()` (the source is removed before the target lookup runs).
    // Reject it here so mis-behaving LLMs can't hide a no-op that
    // reorders bullets.
    for (bp, op) in &parsed_ops {
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let tp = BulletPath::parse(target_path)?;
            if tp.section != bp.section || tp.entry_index != bp.entry_index {
                return Err(TailorError::Schema(format!(
                    "move_before crosses entries: {} -> {}",
                    op.path, target_path
                )));
            }
            if tp == *bp {
                return Err(TailorError::Schema(format!(
                    "move_before target equals source: {}",
                    op.path
                )));
            }
        }
    }

    // Rule 5 — simulate drops; experiences must retain ≥ 1 bullet.
    // Projects may end empty; `apply()` will skip those projects.
    let mut drops_per_exp: HashMap<usize, usize> = HashMap::new();
    for (bp, op) in &parsed_ops {
        if matches!(op.kind, OpKind::Drop) && bp.section == Section::Experience {
            *drops_per_exp.entry(bp.entry_index).or_insert(0) += 1;
        }
    }
    for (idx, exp) in profile.experience.iter().enumerate() {
        let drops = drops_per_exp.get(&idx).copied().unwrap_or(0);
        if !exp.bullets.is_empty() && drops >= exp.bullets.len() {
            return Err(TailorError::Schema(format!(
                "section emptied by drops: experience[{idx}]"
            )));
        }
    }

    // Rules 6 + 7 — reword length cap + empty-reword guard + entity
    // guardrails. Token sets are built ONCE here and shared across every
    // reword op (previously rebuilt per bullet, O(ops × profile_size)).
    let mut token_sets = guardrails::build_token_sets(profile);
    token_sets.jd_proper_nouns = guardrails::build_jd_token_sets(jd_text);
    for (bp, op) in &parsed_ops {
        if let OpKind::Reword { new_text } = &op.kind {
            if new_text.trim().is_empty() {
                return Err(TailorError::Schema(format!(
                    "reword new_text is empty: {}",
                    op.path
                )));
            }
            if new_text.chars().count() > MAX_BULLET_CHARS {
                return Err(TailorError::InventedContent {
                    path: op.path.clone(),
                    offending_token: format!("len={}", new_text.chars().count()),
                    original_bullet: original_bullet_text(profile, bp).unwrap_or("").to_string(),
                    reason: "bullet over 280 chars",
                });
            }
            let original = original_bullet_text(profile, bp).unwrap_or("");
            guardrails::forbid_invented_entities_with(new_text, original, &token_sets, &op.path)?;
        }
    }

    // Rule 8 — summary reword respects employer proper-noun guardrails.
    if let Some(SummaryOp::Reword { new_text }) = &doc.summary {
        if new_text.trim().is_empty() {
            return Err(TailorError::Schema(
                "summary reword new_text is empty".into(),
            ));
        }
        // Summaries legitimately carry years/numbers from experience; we
        // pass the whole-profile flat text as "original_bullet" so numeric
        // and year tokens from anywhere in the profile are accepted.
        let combined = guardrails::flat_profile_text(profile);
        guardrails::forbid_invented_entities_with(new_text, &combined, &token_sets, "summary")?;
    }

    Ok(())
}

/// Validate a `DiffDoc` against a profile and safely sanitize any individual
/// rewords or summary changes that violate entity guardrails by falling them
/// back to `Keep` / `None`, ensuring 100% safety while preventing batch run failures.
#[allow(clippy::too_many_lines)]
pub fn validate_and_sanitize(doc: &mut DiffDoc, profile: &Profile, jd_text: &str) -> Result<()> {
    let cl_chars = doc.cover_letter.chars().count();
    if cl_chars > MAX_COVER_LETTER_CHARS {
        return Err(TailorError::CoverLetterCharsTooLong {
            chars: cl_chars,
            cap: MAX_COVER_LETTER_CHARS,
        });
    }
    let cl_words = doc.cover_letter.split_whitespace().count();
    if cl_words > MAX_COVER_LETTER_WORDS {
        return Err(TailorError::CoverLetterTooLong {
            words: cl_words,
            cap: MAX_COVER_LETTER_WORDS,
        });
    }

    let mut parsed_ops: Vec<(BulletPath, usize)> = Vec::with_capacity(doc.ops.len());
    for (i, op) in doc.ops.iter().enumerate() {
        let bp = BulletPath::parse(&op.path)?;
        parsed_ops.push((bp, i));
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let _ = BulletPath::parse(target_path)?;
        }
    }

    for (bp, i) in &parsed_ops {
        let op = &doc.ops[*i];
        let Some(count) = entry_bullet_count(profile, &bp.section, bp.entry_index) else {
            return Err(TailorError::Schema(format!(
                "path references missing entry: {}",
                op.path
            )));
        };
        if bp.bullet_index >= count {
            return Err(TailorError::Schema(format!(
                "path references missing bullet: {}",
                op.path
            )));
        }
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let tp = BulletPath::parse(target_path)?;
            let Some(tcount) = entry_bullet_count(profile, &tp.section, tp.entry_index) else {
                return Err(TailorError::Schema(format!(
                    "move_before target missing entry: {target_path}"
                )));
            };
            if tp.bullet_index >= tcount {
                return Err(TailorError::Schema(format!(
                    "move_before target missing bullet: {target_path}"
                )));
            }
        }
    }

    let expected: HashSet<BulletPath> = profile_bullet_paths(profile).into_iter().collect();
    let mut seen: HashSet<BulletPath> = HashSet::with_capacity(parsed_ops.len());
    for (bp, i) in &parsed_ops {
        let op = &doc.ops[*i];
        if !seen.insert(bp.clone()) {
            return Err(TailorError::Schema(format!(
                "duplicate op path: {}",
                op.path
            )));
        }
    }
    for want in &expected {
        if !seen.contains(want) {
            return Err(TailorError::Schema(format!(
                "bullet not covered: {}",
                want.format()
            )));
        }
    }

    for (bp, i) in &parsed_ops {
        let op = &doc.ops[*i];
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let tp = BulletPath::parse(target_path)?;
            if tp.section != bp.section || tp.entry_index != bp.entry_index {
                return Err(TailorError::Schema(format!(
                    "move_before crosses entries: {} -> {}",
                    op.path, target_path
                )));
            }
            if tp == *bp {
                return Err(TailorError::Schema(format!(
                    "move_before target equals source: {}",
                    op.path
                )));
            }
        }
    }

    let mut drops_per_exp: HashMap<usize, usize> = HashMap::new();
    for (bp, i) in &parsed_ops {
        let op = &doc.ops[*i];
        if matches!(op.kind, OpKind::Drop) && bp.section == Section::Experience {
            *drops_per_exp.entry(bp.entry_index).or_insert(0) += 1;
        }
    }
    for (idx, exp) in profile.experience.iter().enumerate() {
        let drops = drops_per_exp.get(&idx).copied().unwrap_or(0);
        if !exp.bullets.is_empty() && drops >= exp.bullets.len() {
            return Err(TailorError::Schema(format!(
                "section emptied by drops: experience[{idx}]"
            )));
        }
    }

    let mut token_sets = guardrails::build_token_sets(profile);
    token_sets.jd_proper_nouns = guardrails::build_jd_token_sets(jd_text);
    for (bp, i) in &parsed_ops {
        let op = &mut doc.ops[*i];
        if let OpKind::Reword { new_text } = &mut op.kind {
            if new_text.trim().is_empty() {
                op.kind = OpKind::Keep;
                continue;
            }
            if new_text.chars().count() > MAX_BULLET_CHARS {
                tracing::warn!(
                    path = %op.path,
                    chars = new_text.chars().count(),
                    "reword bullet exceeded char cap; safely falling back to original bullet"
                );
                op.kind = OpKind::Keep;
                continue;
            }
            let original = original_bullet_text(profile, bp).unwrap_or("");
            if let Err(e) =
                guardrails::forbid_invented_entities_with(new_text, original, &token_sets, &op.path)
            {
                tracing::warn!(
                    path = %op.path,
                    error = %e,
                    "reword entity guardrail triggered; safely falling back to original bullet"
                );
                op.kind = OpKind::Keep;
            }
        }
    }

    if let Some(SummaryOp::Reword { new_text }) = &doc.summary {
        if new_text.trim().is_empty() {
            doc.summary = None;
        } else {
            let combined = guardrails::flat_profile_text(profile);
            if let Err(e) = guardrails::forbid_invented_entities_with(
                new_text,
                &combined,
                &token_sets,
                "summary",
            ) {
                tracing::warn!(
                    error = %e,
                    "summary reword entity guardrail triggered; safely keeping original summary"
                );
                doc.summary = None;
            }
        }
    }

    Ok(())
}
