# Running careerai as a service

career-ai ships a `service` subcommand that installs a **systemd user
service** so `careerai daemon` autostarts on boot and survives logout.
This document covers install, lifecycle, the unit file, and
troubleshooting. Linux only — see the bottom of this file for macOS /
Windows alternatives.

## TL;DR

```bash
# 1. From your initialized project root (where `careerai init` was run)
cd ~/projects/career-ai/career-ai-data       # or wherever your config/ + data/ live
careerai service install

# 2. Enable + start
systemctl --user enable --now careerai

# 3. Confirm
careerai service status
```

`service install` prompts you to enable `loginctl --enable-linger` so
the daemon keeps running after logout. Say `Y` unless you only need
career-ai while you're actively logged in.

## What gets installed

The unit file is written to
`${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/careerai.service` with
this shape (paths are filled in at install time):

```ini
[Unit]
Description=career-ai pipeline daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/kk/toolchains/cargo/bin/careerai daemon
WorkingDirectory=/home/kk/projects/career-ai/career-ai-data
Restart=on-failure
RestartSec=10s
EnvironmentFile=-%h/.config/careerai/env
MemoryMax=2G
CPUQuota=80%

[Install]
WantedBy=default.target
```

Things to know:

- **`WorkingDirectory` is pinned to your `current_dir()` at install
  time.** career-ai resolves `config/`, `data/`, and `profile/` from
  the daemon's cwd; without `WorkingDirectory` set, `systemd --user`
  would start the daemon in the manager's default directory and read
  the wrong tree. Run `service install` from the project root.
- **The unit does NOT contain secrets.** If you want to set
  `ANTHROPIC_API_KEY`, `CAREERAI_ROOT`, or any other env var, drop a
  file at `~/.config/careerai/env`:

  ```ini
  ANTHROPIC_API_KEY=sk-ant-...
  CAREERAI_ROOT=/home/kk/projects/career-ai/career-ai-data
  RUST_LOG=info
  ```

  The `EnvironmentFile=-%h/...` (note the leading `-`) makes the file
  optional — the service starts cleanly even if it doesn't exist.

- **Resource caps.** `MemoryMax=2G` and `CPUQuota=80%` keep a runaway
  daemon from eating the laptop. Override by editing the unit if you
  hit them legitimately.
- **`careerai service install` does NOT auto-enable the unit.** That's
  a deliberate operator action. Run `systemctl --user enable --now
careerai` after install when you're ready.

## Linger

By default systemd-user instances stop when you log out and start when
you log in. `loginctl enable-linger $USER` makes them survive logout —
your daemon keeps running on the laptop even when no shell is open.

`careerai service install` prompts for this, defaulting to yes. If
you decline (or it fails — some distros gate `loginctl` behind polkit
that prompts for a password), enable it manually:

```bash
loginctl enable-linger $USER
```

You'll only need to do this once per machine. Disable with
`loginctl disable-linger $USER`.

## Daily operations

| What                                    | Command                                |
| --------------------------------------- | -------------------------------------- |
| Status (active? failed? last log lines) | `careerai service status`              |
| Live logs                               | `journalctl --user -u careerai -f`     |
| Recent logs (last 100 lines)            | `journalctl --user -u careerai -n 100` |
| Restart after a config change           | `systemctl --user restart careerai`    |
| Pause                                   | `systemctl --user stop careerai`       |
| Resume                                  | `systemctl --user start careerai`      |
| Disable autostart                       | `systemctl --user disable careerai`    |
| Re-enable autostart                     | `systemctl --user enable careerai`     |

`careerai service status` is a thin wrapper around `systemctl --user
status careerai` — its exit code matches systemctl's (0 active, 3
inactive, 4 not-found are all benign; anything else is a real error).

## Updating

When you upgrade the binary (`cargo install --git ... careerai-cli` or
unpack a new release tarball), the existing unit's `ExecStart` keeps
pointing at the same path — usually fine. If you've moved the binary,
re-run `careerai service install --force` to rewrite `ExecStart`, then
`systemctl --user daemon-reload` and `systemctl --user restart
careerai`.

## Uninstall

```bash
careerai service uninstall
```

This:

1. Runs `systemctl --user disable --now careerai` (stops + removes
   from autostart, ignoring "not loaded" errors)
2. Removes the unit file
3. Runs `systemctl --user daemon-reload`

It does **not** disable linger — leave that for `loginctl
disable-linger $USER` if you want to clean it up.

## Troubleshooting

**The service starts then immediately fails.** Tail the journal:

```bash
journalctl --user -u careerai -n 50
```

Common causes:

| Symptom                                                       | Likely cause                                                             | Fix                                                                |
| ------------------------------------------------------------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `failed to load config: ...config/default.yaml: No such file` | `WorkingDirectory` points at a directory without `careerai init` results | Re-run `service install` from the right project root               |
| `database error: failed to open ...data/careerai.sqlite`      | Same as above, or wrong `CAREERAI_ROOT` in env file                      | Check `WorkingDirectory` in the unit + `~/.config/careerai/env`    |
| `LLM backend unavailable`                                     | `claude` CLI isn't reachable, no `ANTHROPIC_API_KEY` set                 | Either `claude login`, or add the key to `~/.config/careerai/env`  |
| Daemon runs but nothing happens after logout                  | Linger isn't enabled                                                     | `loginctl enable-linger $USER`                                     |
| `MemoryMax=2G` killed the process                             | Long-running scoring or render exceeded the cap                          | Edit the unit to raise `MemoryMax`, then `daemon-reload` + restart |

**`careerai service status` reports the unit but `is-active` is
`activating (auto-restart)` in a loop.** The daemon is crashing on
boot and `Restart=on-failure` keeps respawning it. Check the journal,
fix the underlying error, then `systemctl --user restart careerai`.

**`systemctl --user enable` says "no such unit".** The install step
didn't run `daemon-reload`, or you're in a different `XDG_CONFIG_HOME`
than the install. Run `systemctl --user daemon-reload` and try again.

## Non-Linux platforms

Systemd is Linux-only. `careerai service install` exits with code 65
on macOS and Windows.

- **macOS**: write a `~/Library/LaunchAgents/com.careerai.daemon.plist`
  with `RunAtLoad=true` and `KeepAlive=true`, then
  `launchctl load -w ~/Library/LaunchAgents/com.careerai.daemon.plist`.
  PR welcome to add an `install` path here.
- **Windows**: use Task Scheduler with "Run whether user is logged on
  or not" + a "delay 30 seconds" trigger. PR welcome.
