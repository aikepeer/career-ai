//! Constrained-diff grammar + validator + applier.
//!
//! This is the non-negotiable safety boundary: LLM output can only
//! reorder or reword existing profile bullets. It cannot invent
//! experience, titles, dates, employers, or numbers. Nine validator
//! rules enforce that boundary; see `validate()` below.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use careerai_profile::schema::{Experience, Profile, Project};

use crate::error::{Result, TailorError};
use crate::guardrails;
use crate::model::{Education, ExperienceView, Personal, ProjectView, ResumeView, Skills};

/// One JSON blob emitted by the tailor LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffDoc {
    pub prompt_version: String,
    #[serde(default)]
    pub summary: Option<SummaryOp>,
    pub ops: Vec<DiffOp>,
    pub cover_letter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
pub enum SummaryOp {
    Keep,
    Reword { new_text: String },
}

// Note: no `deny_unknown_fields` on `DiffOp` — it's incompatible with
// `#[serde(flatten)]` on the `kind` field because serde can't tell at
// parse time which fields belong to the flattened subtype. The inner
// `OpKind` variants each carry `deny_unknown_fields`, which is where the
// airtightness actually lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOp {
    pub path: String,
    #[serde(flatten)]
    pub kind: OpKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
pub enum OpKind {
    Keep,
    Reword { new_text: String },
    Drop,
    MoveBefore { target_path: String },
}

/// Parsed bullet path. Stable equality + hashing for set membership.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BulletPath {
    pub section: Section,
    pub entry_index: usize,
    pub bullet_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    Experience,
    Projects,
}

impl Section {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Experience => "experience",
            Self::Projects => "projects",
        }
    }
}

fn path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Safe: hard-coded, tested.
        #[allow(clippy::unwrap_used)]
        Regex::new(r"^(experience|projects)\[(\d+)\]\.bullets\[(\d+)\]$").unwrap()
    })
}

impl BulletPath {
    pub fn parse(s: &str) -> Result<Self> {
        let caps = path_regex()
            .captures(s)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?;
        let section = match caps
            .get(1)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
        {
            "experience" => Section::Experience,
            "projects" => Section::Projects,
            other => return Err(TailorError::BadPath(other.to_string())),
        };
        let entry_index: usize = caps
            .get(2)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
            .parse()
            .map_err(|_| TailorError::BadPath(s.to_string()))?;
        let bullet_index: usize = caps
            .get(3)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
            .parse()
            .map_err(|_| TailorError::BadPath(s.to_string()))?;
        Ok(Self {
            section,
            entry_index,
            bullet_index,
        })
    }

    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{}[{}].bullets[{}]",
            self.section.as_str(),
            self.entry_index,
            self.bullet_index
        )
    }
}

const MAX_BULLET_CHARS: usize = 280;
const MAX_COVER_LETTER_WORDS: usize = 350;

/// Enumerate every `(section, entry_index, bullet_index)` tuple in the
/// profile. Drives the coverage rule.
fn profile_bullet_paths(profile: &Profile) -> Vec<BulletPath> {
    let mut out = Vec::new();
    for (i, exp) in profile.experience.iter().enumerate() {
        for j in 0..exp.bullets.len() {
            out.push(BulletPath {
                section: Section::Experience,
                entry_index: i,
                bullet_index: j,
            });
        }
    }
    for (i, proj) in profile.projects.iter().enumerate() {
        for j in 0..proj.bullets.len() {
            out.push(BulletPath {
                section: Section::Projects,
                entry_index: i,
                bullet_index: j,
            });
        }
    }
    out
}

fn entry_bullet_count(profile: &Profile, section: &Section, idx: usize) -> Option<usize> {
    match section {
        Section::Experience => profile.experience.get(idx).map(|e| e.bullets.len()),
        Section::Projects => profile.projects.get(idx).map(|p| p.bullets.len()),
    }
}

fn original_bullet_text<'a>(profile: &'a Profile, bp: &BulletPath) -> Option<&'a str> {
    match bp.section {
        Section::Experience => profile
            .experience
            .get(bp.entry_index)
            .and_then(|e| e.bullets.get(bp.bullet_index))
            .map(String::as_str),
        Section::Projects => profile
            .projects
            .get(bp.entry_index)
            .and_then(|p| p.bullets.get(bp.bullet_index))
            .map(String::as_str),
    }
}

/// Validate a `DiffDoc` against a profile.
///
/// Nine rules in order:
///
/// 1. Coverage — every profile bullet appears in exactly one op.
/// 2. Paths parse.
/// 3. Paths point to real profile bullets.
/// 4. `move_before` target stays within the same entry.
/// 5. `drop` cannot empty an experience (≥ 1 bullet must remain).
/// 6. Reword length ≤ 280 chars.
/// 7. Reword entity guardrails via `guardrails::forbid_invented_entities`.
/// 8. Summary reword respects employer-token guardrails.
/// 9. Cover letter ≤ 350 whitespace-split words.
#[allow(clippy::too_many_lines)]
pub fn validate(doc: &DiffDoc, profile: &Profile) -> Result<()> {
    // Rule 9 first — cheap, independent of the ops list.
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

    // Rule 4 — cross-entry move guard.
    for (bp, op) in &parsed_ops {
        if let OpKind::MoveBefore { target_path } = &op.kind {
            let tp = BulletPath::parse(target_path)?;
            if tp.section != bp.section || tp.entry_index != bp.entry_index {
                return Err(TailorError::Schema(format!(
                    "move_before crosses entries: {} -> {}",
                    op.path, target_path
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

    // Rules 6 + 7 — reword length cap and entity guardrails.
    for (bp, op) in &parsed_ops {
        if let OpKind::Reword { new_text } = &op.kind {
            if new_text.chars().count() > MAX_BULLET_CHARS {
                return Err(TailorError::InventedContent {
                    path: op.path.clone(),
                    offending_token: format!("len={}", new_text.chars().count()),
                    original_bullet: original_bullet_text(profile, bp).unwrap_or("").to_string(),
                    reason: "bullet over 280 chars",
                });
            }
            let original = original_bullet_text(profile, bp).unwrap_or("");
            guardrails::forbid_invented_entities(new_text, original, profile, &op.path)?;
        }
    }

    // Rule 8 — summary reword respects employer proper-noun guardrails.
    if let Some(SummaryOp::Reword { new_text }) = &doc.summary {
        // Summaries legitimately carry years/numbers from experience; we
        // only police invented *proper nouns* here. We piggyback on
        // `forbid_invented_entities` by feeding a large "original_bullet"
        // composed of the whole profile text so numbers/years always pass.
        let combined = profile_flat_text(profile);
        guardrails::forbid_invented_entities(new_text, &combined, profile, "summary")?;
    }

    Ok(())
}

fn profile_flat_text(profile: &Profile) -> String {
    let mut out = String::new();
    out.push_str(&profile.summary);
    out.push(' ');
    for exp in &profile.experience {
        out.push_str(&exp.title);
        out.push(' ');
        out.push_str(&exp.company);
        out.push(' ');
        out.push_str(&exp.location);
        out.push(' ');
        out.push_str(&exp.start);
        out.push(' ');
        out.push_str(&exp.end);
        out.push(' ');
        for b in &exp.bullets {
            out.push_str(b);
            out.push(' ');
        }
    }
    for ed in &profile.education {
        out.push_str(&ed.degree);
        out.push(' ');
        out.push_str(&ed.institution);
        out.push(' ');
        out.push_str(&ed.start);
        out.push(' ');
        out.push_str(&ed.end);
        out.push(' ');
    }
    for p in &profile.projects {
        out.push_str(&p.name);
        out.push(' ');
        out.push_str(&p.url);
        out.push(' ');
        for b in &p.bullets {
            out.push_str(b);
            out.push(' ');
        }
    }
    out
}

/// Apply a validated `DiffDoc` to the profile and return a `ResumeView`.
///
/// Callers should treat `validate` + `apply` as a single unit; we call
/// `validate` defensively here too so mis-ordered callers can't bypass
/// the safety gates.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn apply(doc: DiffDoc, profile: Profile) -> Result<ResumeView> {
    validate(&doc, &profile)?;

    // Summary.
    let summary = match doc.summary.as_ref() {
        Some(SummaryOp::Reword { new_text }) => new_text.clone(),
        _ => profile.summary.clone(),
    };

    // Group ops by (section, entry_index).
    let mut per_entry: HashMap<(Section, usize), Vec<(BulletPath, OpKind)>> = HashMap::new();
    for op in &doc.ops {
        let bp = BulletPath::parse(&op.path)?;
        per_entry
            .entry((bp.section.clone(), bp.entry_index))
            .or_default()
            .push((bp, op.kind.clone()));
    }

    let experience = profile
        .experience
        .iter()
        .enumerate()
        .map(|(i, exp)| build_experience_view(i, exp, per_entry.remove(&(Section::Experience, i))))
        .collect::<Result<Vec<_>>>()?;

    let projects = profile
        .projects
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            match build_project_view(i, p, per_entry.remove(&(Section::Projects, i))) {
                Ok(Some(v)) => Some(Ok(v)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<Vec<_>>>()?;

    let education: Vec<Education> = profile.education.clone();
    let personal: Personal = profile.personal.clone();
    let skills: Skills = profile.skills.clone();

    Ok(ResumeView {
        personal,
        summary,
        skills,
        experience,
        education,
        projects,
    })
}

#[allow(clippy::needless_pass_by_value)]
fn apply_ops_to_bullets(
    original: &[String],
    ops: Vec<(BulletPath, OpKind)>,
) -> Result<Vec<String>> {
    // Build a mutable order vector of original indexes; drop-marked indexes
    // are filtered at emit time. Reword/Keep annotations live in a map.
    #[derive(Clone)]
    enum Effect {
        Keep,
        Reword(String),
        Drop,
    }

    let mut effect: HashMap<usize, Effect> = HashMap::new();
    let mut order: Vec<usize> = (0..original.len()).collect();

    // First pass — Keep/Reword/Drop. MoveBefore handled in a second pass so
    // ordering stays deterministic regardless of op emission order.
    for (bp, kind) in &ops {
        match kind {
            OpKind::Keep => {
                effect.insert(bp.bullet_index, Effect::Keep);
            }
            OpKind::Reword { new_text } => {
                effect.insert(bp.bullet_index, Effect::Reword(new_text.clone()));
            }
            OpKind::Drop => {
                effect.insert(bp.bullet_index, Effect::Drop);
            }
            OpKind::MoveBefore { .. } => {
                // Marks a Keep if no other effect was set — moves don't
                // change text.
                effect.entry(bp.bullet_index).or_insert(Effect::Keep);
            }
        }
    }

    // Second pass — apply each MoveBefore by removing source idx from
    // `order` then inserting it before target idx in the current vector.
    for (bp, kind) in &ops {
        if let OpKind::MoveBefore { target_path } = kind {
            let tp = BulletPath::parse(target_path)?;
            // Safe: validated earlier that section/entry match source.
            let src_idx = bp.bullet_index;
            let tgt_idx = tp.bullet_index;
            if let Some(pos) = order.iter().position(|&i| i == src_idx) {
                order.remove(pos);
            }
            if let Some(pos) = order.iter().position(|&i| i == tgt_idx) {
                order.insert(pos, src_idx);
            } else {
                // Target was dropped or already moved past; push to end.
                order.push(src_idx);
            }
        }
    }

    let mut out = Vec::with_capacity(order.len());
    for idx in order {
        let eff = effect.get(&idx).cloned().unwrap_or(Effect::Keep);
        match eff {
            Effect::Keep => {
                if let Some(t) = original.get(idx) {
                    out.push(t.clone());
                }
            }
            Effect::Reword(text) => out.push(text),
            Effect::Drop => {}
        }
    }
    Ok(out)
}

fn build_experience_view(
    _idx: usize,
    exp: &Experience,
    ops: Option<Vec<(BulletPath, OpKind)>>,
) -> Result<ExperienceView> {
    let bullets = match ops {
        Some(ops) => apply_ops_to_bullets(&exp.bullets, ops)?,
        None => exp.bullets.clone(),
    };
    let location = if exp.location.is_empty() {
        None
    } else {
        Some(exp.location.clone())
    };
    Ok(ExperienceView {
        title: exp.title.clone(),
        company: exp.company.clone(),
        location,
        start: exp.start.clone(),
        end: exp.end.clone(),
        bullets,
    })
}

fn build_project_view(
    _idx: usize,
    p: &Project,
    ops: Option<Vec<(BulletPath, OpKind)>>,
) -> Result<Option<ProjectView>> {
    let bullets = match ops {
        Some(ops) => apply_ops_to_bullets(&p.bullets, ops)?,
        None => p.bullets.clone(),
    };
    // Projects that end with zero bullets get dropped from the output.
    if bullets.is_empty() {
        return Ok(None);
    }
    let url = if p.url.is_empty() {
        None
    } else {
        Some(p.url.clone())
    };
    Ok(Some(ProjectView {
        name: p.name.clone(),
        url,
        bullets,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Experience, Personal, Profile, Project, Skills};

    fn fixture_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Ada Lovelace".into(),
                email: "ada@example.com".into(),
                ..Default::default()
            },
            summary: "Senior Rust engineer with embedded experience.".into(),
            skills: Skills {
                languages: vec!["Rust".into(), "Python".into()],
                frameworks: vec!["Tokio".into(), "Kubernetes".into()],
                tools: vec!["SQLite".into()],
            },
            experience: vec![
                Experience {
                    title: "Senior Engineer".into(),
                    company: "Acme Robotics".into(),
                    location: "Remote".into(),
                    start: "2022-01".into(),
                    end: "present".into(),
                    bullets: vec![
                        "Shipped Rust LLM pipeline reducing latency 35%.".into(),
                        "Led team of 4 on embedded perception.".into(),
                    ],
                },
                Experience {
                    title: "Engineer".into(),
                    company: "Widget Corp".into(),
                    location: String::new(),
                    start: "2018-06".into(),
                    end: "2021-12".into(),
                    bullets: vec!["Built Python services on Kubernetes.".into()],
                },
            ],
            education: vec![],
            projects: vec![Project {
                name: "openLLM".into(),
                url: "https://example.com/openllm".into(),
                bullets: vec!["Tokenizer in Rust supporting 5 languages.".into()],
            }],
        }
    }

    fn minimal_doc(ops: Vec<DiffOp>) -> DiffDoc {
        DiffDoc {
            prompt_version: "tailor.v1".into(),
            summary: Some(SummaryOp::Keep),
            ops,
            cover_letter: "short letter".into(),
        }
    }

    fn keep(path: &str) -> DiffOp {
        DiffOp {
            path: path.into(),
            kind: OpKind::Keep,
        }
    }

    fn full_coverage_ops() -> Vec<DiffOp> {
        vec![
            keep("experience[0].bullets[0]"),
            keep("experience[0].bullets[1]"),
            keep("experience[1].bullets[0]"),
            keep("projects[0].bullets[0]"),
        ]
    }

    #[test]
    fn rejects_path_outside_profile() {
        let profile = fixture_profile();
        // experience[0] only has 2 bullets — [2] is out of range.
        let mut ops = full_coverage_ops();
        ops.push(keep("experience[0].bullets[2]"));
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(matches!(err, TailorError::Schema(ref s) if s.contains("missing bullet")));
    }

    #[test]
    fn rejects_invented_employer() {
        let profile = fixture_profile();
        let mut ops = full_coverage_ops();
        // Replace experience[0].bullets[0] with a reword mentioning Google.
        ops[0] = DiffOp {
            path: "experience[0].bullets[0]".into(),
            kind: OpKind::Reword {
                new_text: "Shipped Rust pipeline at Google reducing latency 35%.".into(),
            },
        };
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_fabricated_number() {
        let profile = fixture_profile();
        let mut ops = full_coverage_ops();
        ops[0] = DiffOp {
            path: "experience[0].bullets[0]".into(),
            kind: OpKind::Reword {
                // 97% appears nowhere in the profile.
                new_text: "Shipped Rust pipeline reducing latency 97%.".into(),
            },
        };
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented number"),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_cross_entry_move() {
        let profile = fixture_profile();
        let mut ops = full_coverage_ops();
        ops[0] = DiffOp {
            path: "experience[0].bullets[0]".into(),
            kind: OpKind::MoveBefore {
                // Crosses into experience[1]
                target_path: "experience[1].bullets[0]".into(),
            },
        };
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("crosses entries")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_empty_experience_after_drops() {
        let profile = fixture_profile();
        // experience[1] has 1 bullet — dropping it empties the entry.
        let mut ops = full_coverage_ops();
        ops[2] = DiffOp {
            path: "experience[1].bullets[0]".into(),
            kind: OpKind::Drop,
        };
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("emptied by drops")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_missing_coverage() {
        let profile = fixture_profile();
        // Omit experience[1].bullets[0].
        let ops = vec![
            keep("experience[0].bullets[0]"),
            keep("experience[0].bullets[1]"),
            keep("projects[0].bullets[0]"),
        ];
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("not covered")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_duplicate_op_paths() {
        let profile = fixture_profile();
        let mut ops = full_coverage_ops();
        ops.push(keep("experience[0].bullets[0]"));
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("duplicate op path")),
            "got {err:?}"
        );
    }

    #[test]
    fn accepts_valid_reorder_and_keep() {
        let profile = fixture_profile();
        let ops = vec![
            DiffOp {
                path: "experience[0].bullets[1]".into(),
                kind: OpKind::MoveBefore {
                    target_path: "experience[0].bullets[0]".into(),
                },
            },
            keep("experience[0].bullets[0]"),
            keep("experience[1].bullets[0]"),
            keep("projects[0].bullets[0]"),
        ];
        let doc = minimal_doc(ops);
        validate(&doc, &profile).unwrap();
        let view = apply(doc, profile).unwrap();
        // Moved bullet[1] to front of experience[0].
        assert_eq!(view.experience[0].bullets.len(), 2);
        assert!(view.experience[0].bullets[0].contains("Led team"));
    }

    #[test]
    fn accepts_skill_injection_from_profile_skills() {
        let profile = fixture_profile();
        let mut ops = full_coverage_ops();
        // Reword the Python services bullet to mention Rust + Tokio (both
        // in profile.skills).
        ops[2] = DiffOp {
            path: "experience[1].bullets[0]".into(),
            kind: OpKind::Reword {
                new_text: "Built Rust Tokio services on Kubernetes.".into(),
            },
        };
        let doc = minimal_doc(ops);
        validate(&doc, &profile).unwrap();
    }

    #[test]
    fn bullet_length_cap_enforced() {
        let profile = fixture_profile();

        // Under cap: 280 chars exactly (build a string of 280 'a's — but
        // this must still pass proper-noun + number guardrails; all
        // lowercase letters satisfy both.).
        let ok_text: String = "a".repeat(MAX_BULLET_CHARS);
        let mut ops = full_coverage_ops();
        ops[0] = DiffOp {
            path: "experience[0].bullets[0]".into(),
            kind: OpKind::Reword { new_text: ok_text },
        };
        let doc = minimal_doc(ops);
        validate(&doc, &profile).unwrap();

        // Over cap: 281 chars.
        let long_text: String = "a".repeat(MAX_BULLET_CHARS + 1);
        let mut ops = full_coverage_ops();
        ops[0] = DiffOp {
            path: "experience[0].bullets[0]".into(),
            kind: OpKind::Reword {
                new_text: long_text,
            },
        };
        let doc = minimal_doc(ops);
        let err = validate(&doc, &profile).unwrap_err();
        assert!(
            matches!(err, TailorError::InventedContent { reason, .. } if reason == "bullet over 280 chars"),
            "got {err:?}"
        );
    }

    #[test]
    fn bullet_path_parse_roundtrip() {
        for s in [
            "experience[0].bullets[0]",
            "experience[12].bullets[34]",
            "projects[1].bullets[2]",
        ] {
            let bp = BulletPath::parse(s).unwrap();
            assert_eq!(bp.format(), s);
        }
    }

    #[test]
    fn bullet_path_parse_rejects_garbage() {
        for s in [
            "summary",
            "experience[0]",
            "experience[a].bullets[0]",
            "experience[0].bullets[a]",
            "education[0].bullets[0]",
        ] {
            let err = BulletPath::parse(s).unwrap_err();
            assert!(matches!(err, TailorError::BadPath(_)));
        }
    }
}
