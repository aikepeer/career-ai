use super::*;

#[test]
fn confirm_profile_import_at_promotes_draft_and_backs_up_target() {
    let tmp = tempfile::tempdir().unwrap();
    let draft = tmp.path().join("profile.draft.yaml");
    let target = tmp.path().join("profile.yaml");
    std::fs::write(&draft, "name: Draft\n").unwrap();
    std::fs::write(&target, "name: Previous\n").unwrap();

    let applied = confirm_profile_import_at(&draft, &target).unwrap();

    assert_eq!(applied, target);
    assert!(!draft.exists(), "draft should be consumed");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "name: Draft\n");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("profile.yaml.bak")).unwrap(),
        "name: Previous\n"
    );
}

#[test]
fn confirm_profile_import_at_missing_draft_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let draft = tmp.path().join("profile.draft.yaml");
    let target = tmp.path().join("profile.yaml");
    std::fs::write(&target, "name: Previous\n").unwrap();

    let err = confirm_profile_import_at(&draft, &target).unwrap_err();
    assert!(err.contains("no pending profile import"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "name: Previous\n"
    );
}

#[test]
fn confirm_profile_import_at_creates_target_without_backup_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let draft = tmp.path().join("profile.draft.yaml");
    let target = tmp.path().join("profile.yaml");
    std::fs::write(&draft, "name: Draft\n").unwrap();

    confirm_profile_import_at(&draft, &target).unwrap();

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "name: Draft\n");
    assert!(!tmp.path().join("profile.yaml.bak").exists());
}

#[tokio::test]
async fn save_profile_at_writes_atomically_and_creates_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested").join("profile.yaml");
    let mut profile = Profile::default();
    profile.personal.name = "Alice".to_string();

    let written = save_profile_at(&profile, &path).await.unwrap();

    assert_eq!(written, path);
    let parsed: Profile = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(parsed.personal.name, "Alice");
}

#[tokio::test]
async fn save_profile_to_disk_at_backs_up_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("profile.yaml");
    std::fs::write(&target, "personal:\n  name: OldName\n").unwrap();

    let mut profile = Profile::default();
    profile.personal.name = "NewName".to_string();

    let written = save_profile_to_disk_at(&profile, &target).await.unwrap();
    assert_eq!(written, target);

    let bak = tmp.path().join("profile.yaml.bak");
    assert!(bak.exists(), "backup must exist");
    assert_eq!(
        std::fs::read_to_string(&bak).unwrap(),
        "personal:\n  name: OldName\n"
    );

    let current = std::fs::read_to_string(&target).unwrap();
    assert!(current.contains("NewName"));
}

#[test]
fn load_profile_view_at_parses_existing_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("profile.yaml");
    let yaml = r#"
personal:
  name: Kamal Pandey
  email: pandeykamal13526@gmail.com
  phone: "+91 7767984205"
  location: Gurugram, Haryana, India
summary: "Target Roles: Senior Embedded Lead, Rust SWE"
skills:
  languages:
    - Rust
    - C++
  frameworks:
    - Yocto
  tools:
    - OSTree
experience:
  - title: Principal Engineer
    company: Acme Systems
    start: 2020-01
    end: present
    bullets:
      - Led a production platform from architecture through launch.
education:
  - degree: B.E. Computer Engineering
    institution: Example Institute
    start: "2014"
    end: "2018"
    achievements:
      - Graduated with distinction.
"#;
    std::fs::write(&target, yaml).unwrap();
    let view = load_profile_view_at(&target).expect("profile view loaded");
    assert_eq!(view.name, "Kamal Pandey");
    assert_eq!(view.email, "pandeykamal13526@gmail.com");
    assert_eq!(view.target_roles, vec!["Senior Embedded Lead", "Rust SWE"]);
    assert_eq!(view.languages, vec!["Rust", "C++"]);
    assert_eq!(view.frameworks, vec!["Yocto"]);
    assert_eq!(view.tools, vec!["OSTree"]);
    assert_eq!(view.skill_count, 4);
    assert_eq!(view.experience_count, 1);
    assert_eq!(view.education_count, 1);
    assert_eq!(view.career_story.len(), 2);
    assert_eq!(view.career_story[0].kind, "education");
    assert_eq!(view.career_story[1].organization, "Acme Systems");
}
