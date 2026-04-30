//! Profile-walking helpers: enumerate bullet paths, look up original
//! bullet text, count bullets per entry.

use careerai_profile::schema::Profile;

use super::schema::{BulletPath, Section};

/// Enumerate every `(section, entry_index, bullet_index)` tuple in the
/// profile. Drives the coverage rule.
pub(crate) fn profile_bullet_paths(profile: &Profile) -> Vec<BulletPath> {
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

pub(crate) fn entry_bullet_count(
    profile: &Profile,
    section: &Section,
    idx: usize,
) -> Option<usize> {
    match section {
        Section::Experience => profile.experience.get(idx).map(|e| e.bullets.len()),
        Section::Projects => profile.projects.get(idx).map(|p| p.bullets.len()),
    }
}

pub(crate) fn original_bullet_text<'a>(profile: &'a Profile, bp: &BulletPath) -> Option<&'a str> {
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
