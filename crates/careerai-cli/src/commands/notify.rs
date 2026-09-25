//! `careerai notify test` — fire a synthetic notification through every
//! configured channel so the operator can verify the pipeline
//! end-to-end without waiting for a real event.

use anyhow::Result;

use careerai_core::config::CoreConfig;

pub async fn run_test(cfg: &CoreConfig) -> Result<()> {
    let pipe = careerai_notify::Pipeline::from_config(&cfg.notify)?;
    let count = pipe.channel_count();
    if count == 0 {
        println!(
            "no notify channels are configured. Edit the resolved XDG \
             config directory's local.yaml and add a slack / telegram / \
             email / ntfy block under `notify.channels`."
        );
        std::process::exit(2);
    }
    // Fire at the configured `min_severity` so the test event is never
    // silently dropped by the pipeline filter. Default `min_severity`
    // is `Warning`, so an `Info` test event would never reach any
    // channel and the operator would (rightly) believe the wiring is
    // broken.
    let severity = cfg.notify.min_severity;
    println!(
        "firing test notification ({severity:?}) through {count} channel(s): {:?}",
        pipe.channel_names()
    );
    pipe.fire(
        careerai_notify::NotifyEvent::SourceUnreachable {
            source: "test".to_string(),
            reason: "manual test via `careerai notify test`".to_string(),
        },
        severity,
    )
    .await;
    println!("done. Check each channel's destination — failures are logged at WARN.");
    Ok(())
}
