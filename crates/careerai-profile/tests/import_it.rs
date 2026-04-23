//! End-to-end import: synthesize a LinkedIn-shaped ZIP, run `import_paths`,
//! snapshot the generated YAML via `insta`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Write};

use careerai_profile::{import_paths, Profile};

fn build_linkedin_zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in files {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(contents.as_bytes()).unwrap();
        }
        zw.finish().unwrap();
    }
    buf
}

#[test]
fn import_linkedin_export_produces_stable_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");

    let profile_csv = "\
First Name,Last Name,Email Address,Geo Location,Summary,Websites
Alice,Kumar,alice@example.com,Delhi NCR,Builder of systems,https://alice.dev
";
    let positions_csv = "\
Company Name,Title,Description,Location,Started On,Finished On
Acme Robotics,Senior Engineer,Led migration\\nCut latency,Remote,Jan 2022,
BetaCorp,Engineer,Built ingest pipeline,Bangalore,Jun 2019,Dec 2021
";
    let education_csv = "\
School Name,Degree Name,Start Date,End Date
IIT Delhi,B.Tech Computer Science,2015,2019
";
    let skills_csv = "\
Name
Rust
Python
Tokio
";

    let bytes = build_linkedin_zip(&[
        ("Profile.csv", profile_csv),
        ("Positions.csv", positions_csv),
        ("Education.csv", education_csv),
        ("Skills.csv", skills_csv),
    ]);
    std::fs::write(&zip_path, &bytes).unwrap();

    let profile = import_paths(&[zip_path.as_path()]).unwrap();
    profile.check().unwrap();

    insta::assert_yaml_snapshot!(profile);
}

#[test]
fn unsupported_extension_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let txt = tmp.path().join("resume.txt");
    std::fs::write(&txt, "hello").unwrap();
    let err = import_paths(&[txt.as_path()]).unwrap_err();
    assert!(matches!(
        err,
        careerai_profile::ProfileError::UnsupportedFormat(_)
    ));
}

#[test]
fn merging_two_sources_dedups() {
    // Two LinkedIn exports with overlapping positions — later overlays base.
    let profile_csv_a = "\
First Name,Last Name,Email Address,Summary
Alice,Kumar,,short
";
    let positions_csv_a = "\
Company Name,Title,Started On
Acme,Engineer,Jan 2022
";
    let profile_csv_b = "\
First Name,Last Name,Email Address,Summary
Alice,Kumar,alice@example.com,a much longer summary describing experience
";
    let positions_csv_b = "\
Company Name,Title,Description,Started On
Acme,Engineer,Led migration,Jan 2022
";

    let tmp = tempfile::tempdir().unwrap();
    let za = tmp.path().join("a.zip");
    let zb = tmp.path().join("b.zip");
    std::fs::write(
        &za,
        build_linkedin_zip(&[
            ("Profile.csv", profile_csv_a),
            ("Positions.csv", positions_csv_a),
        ]),
    )
    .unwrap();
    std::fs::write(
        &zb,
        build_linkedin_zip(&[
            ("Profile.csv", profile_csv_b),
            ("Positions.csv", positions_csv_b),
        ]),
    )
    .unwrap();

    let merged: Profile = import_paths(&[za.as_path(), zb.as_path()]).unwrap();
    assert_eq!(merged.experience.len(), 1);
    assert_eq!(merged.personal.email, "alice@example.com");
    assert!(merged.summary.contains("longer"));
    assert!(merged.experience[0]
        .bullets
        .iter()
        .any(|b| b.contains("migration")));
}
