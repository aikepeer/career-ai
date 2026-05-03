#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn slugify_strips_non_alnum_and_lowercases() {
    assert_eq!(support::slugify("Acme Robotics"), "acmerobotics");
    assert_eq!(support::slugify("Foo, Inc."), "fooinc");
    assert_eq!(support::slugify(""), "unknown");
}

#[test]
fn sanitize_external_id_blocks_path_traversal() {
    assert_eq!(sanitize_external_id("../../admin"), "______admin");
    assert_eq!(sanitize_external_id("job-42"), "job-42");
    assert_eq!(sanitize_external_id("job_42_v2"), "job_42_v2");
    assert_eq!(sanitize_external_id("job.v2"), "job_v2");
    assert_eq!(sanitize_external_id("foo/bar"), "foo_bar");
    assert_eq!(sanitize_external_id("foo?bar=baz#frag"), "foo_bar_baz_frag");
    assert_eq!(sanitize_external_id(""), "unknown");
}

#[test]
fn truncate_chars_respects_char_boundaries() {
    let s = "héllo";
    let t = support::truncate_chars(s, 2);
    assert!(s.starts_with(&t));
    assert!(t.is_char_boundary(t.len()));
}
