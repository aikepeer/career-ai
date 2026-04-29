//! Defaults calibration: the embedded `default.yaml` must ship a
//! `match.score_threshold` that the v1 [`JaccardScorer`] can realistically
//! produce against typical profile/JD shapes. Earlier defaults were
//! calibrated for the future BGE-cosine scorer (mid-0.6s), but the
//! shipping scorer is Jaccard over tokens — its empirical ceiling on
//! ~500-token profile vs ~300-token JD is around 0.04, so a 0.62
//! threshold silently shortlists nothing.
//!
//! This test would have caught the regression a real user hit on
//! 2026-04-29: 8853 listings discovered, 0 shortlisted because every
//! score landed in [0.01, 0.04] but threshold demanded 0.62.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::CoreConfig;
use careerai_match::score::{JaccardScorer, Scorer};
use careerai_sources::RawListing;

/// Long, real-shaped profile. Mirrors the size and vocabulary spread of
/// a typical flattened `Profile` (many experience bullets, projects,
/// summaries) — Jaccard against a JD scales down as the union grows, so
/// we need this length for the threshold to reflect production reality.
fn realistic_ai_ml_profile() -> &'static str {
    "kamal pandey senior embedded developer system lead at symx.ai
     graduated savitribai phule pune university bachelor electronics
     telecommunications engineering currently contributing senior embedded
     system engineer involves embedded software development collaborative
     research engineer engineering experience device driver development
     secure system implementation motivated deliver innovative embedded solutions
     committed working teams drive projects align organizational objectives
     create meaningful technological advancements
     skills languages c cpp python bash qt qml golang typescript javascript
     skills frameworks yocto bsp ostree sota uboot openamp rpmsg gstreamer
     v4l2 ros ros2 docker mqtt webrtc rtsp rtp redis postgres kubernetes
     skills tools jtag uart ftrace perf kprobes valgrind gdb jenkins gitlab
     oscilloscopes logic analyzers wireshark git github vscode tmux
     experience symbot6 mining vehicles led end-to-end development deployed
     across mining vehicle fleets enabling real-time telemetry mission-critical
     decision systems extreme industrial vibrations power constraints
     experience linux kernel device drivers built v4l2 video pipeline gstreamer
     plugin chain real-time camera processing ai inference edge devices
     experience yocto bsp custom layers firmware images recipes meta-layer
     integrations across hardware variants secure boot ostree updates
     experience ros2 perception slam autonomous navigation industrial robots
     mining domain integrating sensor fusion lidar camera imu telemetry
     experience power management low-power deep sleep wake sources gpio
     interrupts watchdog recovery hardware error reporting soft reset
     experience systemd service hardening cgroups slices resource caps
     observability journald structured logging metrics prometheus exporters
     projects home automation nodemcu wifi mqtt sensor mesh control panels
     projects gesture controlled robot accelerometer mpu6050 servo motors
     projects custom shell embedded linux device tree overlays kernel modules
     interests machine learning ml engineer ai engineer applied scientist llm
     nlp computer vision deep learning robotics embedded firmware ros ros2
     rtos edge ai slam perception autonomous mlops inference serving"
}

fn realistic_ml_engineer_listing() -> RawListing {
    RawListing {
        source: "greenhouse".into(),
        external_id: "demo-1".into(),
        title: "Machine Learning Engineer".into(),
        company: "Anthropic".into(),
        location: Some("Remote".into()),
        url: "https://example.com/jd/1".into(),
        description: "We are hiring a machine learning engineer to work on llm \
            inference, deep learning, and applied research. Experience with python, \
            pytorch, and computer vision required. Familiarity with embedded \
            deployment, robotics, ros2, and edge ai is a plus. You will collaborate \
            on perception systems, slam, and autonomous decision-making, integrating \
            with c++ device drivers and firmware on linux."
            .into(),
        raw_json: None,
    }
}

#[test]
fn embedded_defaults_threshold_is_reachable_by_jaccard_scorer() {
    let cfg: CoreConfig = serde_yaml::from_str(careerai_core::config::EMBEDDED_DEFAULTS)
        .expect("parse embedded default.yaml");
    let threshold = cfg.matching.score_threshold;

    let scorer = JaccardScorer;
    let listing = realistic_ml_engineer_listing();
    let score = scorer.score(realistic_ai_ml_profile(), &listing);

    assert!(
        score >= threshold,
        "Jaccard score on a clearly-matching ML JD ({score:.4}) is below the \
         shipping default threshold ({threshold}). Either the threshold is \
         calibrated for a scorer this crate doesn't ship, or the scorer can't \
         clear a sensible threshold for any real input. Either way, users would \
         see 0 shortlisted listings against an otherwise rich JD pool."
    );
}

#[test]
fn embedded_defaults_notify_threshold_is_above_score_threshold() {
    let cfg: CoreConfig = serde_yaml::from_str(careerai_core::config::EMBEDDED_DEFAULTS)
        .expect("parse embedded default.yaml");
    assert!(
        cfg.matching.notify_threshold > cfg.matching.score_threshold,
        "notify_threshold ({}) must be > score_threshold ({}) — only the \
         strongest matches should ping the operator",
        cfg.matching.notify_threshold,
        cfg.matching.score_threshold,
    );
}
