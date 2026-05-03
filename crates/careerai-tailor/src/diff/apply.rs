//! Apply a validated [`DiffDoc`] to a [`Profile`], producing a
//! [`ResumeView`].

use std::collections::HashMap;

use careerai_profile::schema::{Experience, Profile, Project};

use crate::error::Result;
use crate::model::{Education, ExperienceView, Personal, ProjectView, ResumeView, Skills};

use super::schema::{BulletPath, DiffDoc, OpKind, Section, SummaryOp};
use super::validate::validate;

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

    // Group ops by (section, entry_index). Consume `doc.ops` by value so
    // `OpKind::Reword { new_text }` moves instead of cloning its String.
    let mut per_entry: HashMap<(Section, usize), Vec<(BulletPath, OpKind)>> = HashMap::new();
    for op in doc.ops {
        let bp = BulletPath::parse(&op.path)?;
        per_entry
            .entry((bp.section.clone(), bp.entry_index))
            .or_default()
            .push((bp, op.kind));
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
