use crate::schema::{Education, Experience, Profile, Project, Skills};

use super::keys::{edu_key, exp_key, exp_loose_key, looks_like_date_garbage};
use super::utils::{
    dedup_keep_order, dedup_keep_order_ci, merge_vecs, prefer_longer, prefer_nonempty,
};

#[must_use]
pub fn merge_all(profiles: Vec<Profile>) -> Profile {
    profiles.into_iter().fold(Profile::default(), merge_pair)
}

#[must_use]
pub fn merge_pair(base: Profile, other: Profile) -> Profile {
    Profile {
        personal: merge_personal(base.personal, other.personal),
        summary: prefer_longer(base.summary, other.summary),
        skills: merge_skills(base.skills, other.skills),
        experience: merge_experience(base.experience, other.experience),
        education: merge_education(base.education, other.education),
        projects: merge_projects(base.projects, other.projects),
        target_roles: {
            let mut roles = dedup_keep_order_ci(merge_vecs(base.target_roles, other.target_roles));
            roles.dedup();
            roles
        },
    }
}

fn merge_personal(
    base: crate::schema::Personal,
    other: crate::schema::Personal,
) -> crate::schema::Personal {
    crate::schema::Personal {
        name: prefer_nonempty(base.name, other.name),
        email: prefer_nonempty(base.email, other.email),
        phone: prefer_nonempty(base.phone, other.phone),
        location: prefer_nonempty(base.location, other.location),
        links: crate::schema::Links {
            github: prefer_nonempty(base.links.github, other.links.github),
            linkedin: prefer_nonempty(base.links.linkedin, other.links.linkedin),
            portfolio: prefer_nonempty(base.links.portfolio, other.links.portfolio),
        },
    }
}

fn merge_skills(base: Skills, other: Skills) -> Skills {
    Skills {
        languages: dedup_keep_order_ci(merge_vecs(base.languages, other.languages)),
        frameworks: dedup_keep_order_ci(merge_vecs(base.frameworks, other.frameworks)),
        tools: dedup_keep_order_ci(merge_vecs(base.tools, other.tools)),
        platforms: dedup_keep_order_ci(merge_vecs(base.platforms, other.platforms)),
        devops: dedup_keep_order_ci(merge_vecs(base.devops, other.devops)),
        debugging: dedup_keep_order_ci(merge_vecs(base.debugging, other.debugging)),
        protocols: dedup_keep_order_ci(merge_vecs(base.protocols, other.protocols)),
    }
}

fn merge_experience(base: Vec<Experience>, other: Vec<Experience>) -> Vec<Experience> {
    let mut out = base;
    for new in other {
        if new.start.is_empty() {
            out.push(new);
            continue;
        }
        let strict_key = exp_key(&new);
        let loose_key = exp_loose_key(&new);
        let mut idx = out
            .iter()
            .position(|e| !e.start.is_empty() && exp_key(e) == strict_key);
        if idx.is_none() {
            let new_title_garbage = looks_like_date_garbage(&new.title);
            idx = out.iter().position(|e| {
                !e.start.is_empty()
                    && exp_loose_key(e) == loose_key
                    && (new_title_garbage || looks_like_date_garbage(&e.title))
            });
        }
        if let Some(i) = idx {
            let existing = &mut out[i];
            existing.bullets = dedup_keep_order(merge_vecs(existing.bullets.clone(), new.bullets));
            existing.location = prefer_nonempty(existing.location.clone(), new.location);
            existing.end = prefer_nonempty(existing.end.clone(), new.end);
            if looks_like_date_garbage(&existing.title) && !looks_like_date_garbage(&new.title) {
                existing.title = new.title;
            }
        } else {
            out.push(new);
        }
    }
    out
}

fn merge_education(base: Vec<Education>, other: Vec<Education>) -> Vec<Education> {
    let mut out = base;
    for new in other {
        let key = edu_key(&new);
        if !out.iter().any(|e| edu_key(e) == key) {
            out.push(new);
        }
    }
    out
}

fn merge_projects(base: Vec<Project>, other: Vec<Project>) -> Vec<Project> {
    let mut out = base;
    for new in other {
        let key = new.name.to_lowercase();
        if let Some(existing) = out.iter_mut().find(|p| p.name.to_lowercase() == key) {
            existing.url = prefer_nonempty(existing.url.clone(), new.url);
            existing.bullets = dedup_keep_order(merge_vecs(existing.bullets.clone(), new.bullets));
        } else {
            out.push(new);
        }
    }
    out
}
