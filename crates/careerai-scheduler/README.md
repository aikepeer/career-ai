# careerai-scheduler

`tokio-cron-scheduler`-driven daemon. Wakes up on per-source
cadences and walks listings through the pipeline stages.

## Boundary

| Owns | Never does |
|---|---|
| `Scheduler::from_config(cwd, cfg)` constructor | Pipeline stage logic (calls into `careerai-pipeline`) |
| Cron job registration per source / per stage | DB writes (delegates) |
| Graceful shutdown on SIGINT / SIGTERM (Unix) and Ctrl-C (Windows, cfg-gated) | LLM calls |
| Error handling per-tick — one failing tick never kills the daemon |  |

## Cadences

Each source has its own cadence under `cfg.scheduler.sources.<name>.cron`.
The scheduler also runs:

* `match` after every successful discover tick (per source)
* `tailor` then `render` then `apply` — currently driven through the CLI;
  the daemon owns only discover + match. (Future work: full pipeline ticks.)

## Cross-platform

`SIGINT`/`SIGTERM` handling is `#[cfg(unix)]`-gated (PR #44 fix);
Windows uses `tokio::signal::ctrl_c()`. CI's `windows-check` job
catches regressions at PR time.

## Tests

```bash
cargo test -p careerai-scheduler
```

Unit tests cover cron parsing + shutdown signal handling. The
end-to-end "tick fires the right pipeline call" test lives in
`careerai-pipeline`'s integration tests.

## Operator usage

```bash
careerai daemon                  # foreground; press Ctrl-C to exit
careerai service install         # systemd-user wrapper for autostart
journalctl --user -u careerai -f # tail logs
```

See [`docs/SERVICE.md`](../../docs/SERVICE.md) for the full
walkthrough.
