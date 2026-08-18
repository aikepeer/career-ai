#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn req(command: &str) -> CliRunRequest {
    CliRunRequest {
        command: command.to_string(),
        args: CliRunArgs::default(),
    }
}

fn args_str(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}

#[test]
fn whitelisted_commands_build_expected_argv() {
    assert_eq!(
        args_str(&build_command_args(&req("discover")).unwrap()),
        ["discover"]
    );
    assert_eq!(
        args_str(&build_command_args(&req("match")).unwrap()),
        ["match"]
    );

    let mut r = req("match");
    r.args.tune = Some(true);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["match", "--tune"]
    );

    assert_eq!(args_str(&build_command_args(&req("run")).unwrap()), ["run"]);
    let mut r = req("run");
    r.args.auto_submit = Some(true);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["run", "--auto-submit"]
    );

    let mut r = req("shortlist show");
    r.args.limit = Some(5);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["shortlist", "show", "--limit", "5"]
    );

    let mut r = req("tailor");
    r.args.listing_id = Some("abc-123".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["tailor", "abc-123"]
    );

    let mut r = req("render");
    r.args.application_id = Some("app_1".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["render", "app_1"]
    );

    let mut r = req("apply");
    r.args.all = Some(true);
    r.args.auto_submit = Some(true);
    r.args.source = Some("greenhouse".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["apply", "--all", "--auto-submit", "--source", "greenhouse"]
    );

    let mut r = req("applied");
    r.args.limit = Some(3);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["applied", "--limit", "3"]
    );

    let mut r = req("inspect");
    r.args.application_id = Some("app_1".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["inspect", "app_1"]
    );

    assert_eq!(
        args_str(&build_command_args(&req("review")).unwrap()),
        ["review"]
    );

    let mut r = req("retry");
    r.args.application_id = Some("app_1".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["retry", "app_1"]
    );

    let mut r = req("config generate");
    r.args.force = Some(true);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["config", "generate", "--force"]
    );

    let mut r = req("sources sync");
    r.args.apply = Some(true);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["sources", "sync", "--apply"]
    );

    let mut r = req("digest");
    r.args.since = Some("7d".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["digest", "--since", "7d"]
    );

    assert_eq!(
        args_str(&build_command_args(&req("llm probe")).unwrap()),
        ["llm", "probe"]
    );
    assert_eq!(
        args_str(&build_command_args(&req("notify test")).unwrap()),
        ["notify", "test"]
    );
}

#[test]
fn apply_application_id_is_positional() {
    let mut r = req("apply");
    r.args.application_id = Some("app_1".into());
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["apply", "app_1"]
    );
}

#[test]
fn discover_validates_and_joins_sources() {
    let mut r = req("discover");
    r.args.sources = Some(vec!["greenhouse".into(), "lever".into()]);
    assert_eq!(
        args_str(&build_command_args(&r).unwrap()),
        ["discover", "--source", "greenhouse,lever"]
    );

    r.args.sources = Some(vec!["not-a-source".into()]);
    assert!(matches!(
        build_command_args(&r),
        Err(CliRunValidationError::UnknownSource(_))
    ));
}

#[test]
fn rejects_unknown_and_disallowed_commands() {
    assert!(matches!(
        build_command_args(&req("nonsense")),
        Err(CliRunValidationError::UnknownCommand(_))
    ));
    for disallowed in [
        "daemon",
        "status serve",
        "service install",
        "cookies refresh",
    ] {
        assert!(matches!(
            build_command_args(&req(disallowed)),
            Err(CliRunValidationError::DisallowedCommand(_))
        ));
    }
}

#[test]
fn rejects_shell_metacharacters_in_ids() {
    for evil in ["abc; rm -rf /", "x$(whoami)", "a b", "a/b", "a'b"] {
        let mut r = req("tailor");
        r.args.listing_id = Some(evil.into());
        assert!(matches!(
            build_command_args(&r),
            Err(CliRunValidationError::InvalidId(_))
        ));
    }
}

#[test]
fn rejects_arg_shape_mismatches() {
    let mut r = req("tailor");
    r.args.application_id = Some("app_1".into());
    assert!(matches!(
        build_command_args(&r),
        Err(CliRunValidationError::MissingArgument(_, _))
    ));

    let r = req("apply");
    assert!(matches!(
        build_command_args(&r),
        Err(CliRunValidationError::MissingArgument(_, _))
    ));

    let r = req("retry");
    assert!(matches!(
        build_command_args(&r),
        Err(CliRunValidationError::MissingArgument(_, _))
    ));
}
