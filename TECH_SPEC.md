# Tech Spec: system-controller-rust

## Context

A Rust TUI application that monitors systemd services across remote hosts via SSH. It parses an Ansible-style inventory and a YAML services config, displays a host×service grid with live statuses, and supports actions like refresh, stop, restart, SSH connect, and viewing files/command output in vim.

## Dependencies (Cargo.toml)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
ratatui = "0.29"
crossterm = "0.28"
openssh = { version = "0.11", features = ["native-mux"] }
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
configparser = "3"
glob-match = "0.2"
anyhow = "1"
```

- **openssh** — shells out to system `ssh`, inherits `~/.ssh/config` and agent for free
- **configparser** — handles Ansible INI (section headers = groups, bare lines = hosts)
- **glob-match** — in-memory pattern matching for service name globs

## Module Structure

```
src/
  main.rs              -- CLI args, tokio::main, panic hook, launches app
  app.rs               -- AppState, event loop, keyboard dispatch
  config/
    mod.rs
    inventory.rs       -- parse inventory.ini → Vec<Host>
    services.rs        -- parse services.yaml → Vec<ServiceConfig>
  ssh/
    mod.rs
    session.rs         -- SessionManager: connection pool via ControlMaster
  monitor/
    mod.rs
    status.rs          -- fetch statuses, glob expansion, batch refresh
  tui/
    mod.rs             -- terminal setup/teardown
    ui.rs              -- render main screen table + detail screen list
    event.rs           -- crossterm event polling
```

## Data Model

### Core structs

- **Host** { address: String, group: String }
- **ServiceConfig** { name_pattern: String, files: Vec<String>, commands: Vec<String>, is_glob: bool }
- **ServiceStatus** enum: Unknown, Active, Inactive, Failed, NotFound, Error(String)
- **HostService** { host_address: String, service_name: String, config: ServiceConfig, status: ServiceStatus }

### App state

- **Screen** enum: Main, Detail { host_index, service_index }
- **AppState** — holds hosts, service_configs, grid (Vec<Vec<HostService>>), UI cursor state, SessionManager, mpsc channel for async refresh results

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| SSH crate | `openssh` with `connect_mux` | Inherits all SSH config/agent; ControlMaster multiplexing = one TCP conn per host |
| Async strategy | Tokio tasks for SSH commands, sync-ish event loop | Simple; UI never blocks |
| TUI↔async bridge | `mpsc::channel` | Refresh results trickle in without blocking UI |
| Grid model | `Vec<Vec<HostService>>` | Direct mapping to table rows×columns |
| Glob matching | Query `systemctl list-units` once per host, match in-memory with `glob-match` | Avoids complex remote expansion |
| External commands | Suspend TUI, run child process (vim/ssh), resume TUI | Standard Unix pattern |
| Error handling | `anyhow` everywhere; SSH errors displayed in grid cells, never crash | Resilient UX |

## SSH Layer

- **SessionManager** wraps a `HashMap<String, openssh::Session>`
- `get_session(host)` — connect on first use via `Session::connect_mux`, reuse thereafter
- `run_command(host, cmd)` — runs via `sh -c "..."` so pipes/redirects work
- Batch optimization: `systemctl is-active svc1 svc2 svc3` returns one status per line, reducing SSH round trips to 1 per host

## Glob Expansion

For patterns like `docker-*`: run `systemctl list-units --type=service --all --no-legend --no-pager` on each host, strip `.service` suffix, match with `glob_match`. Column set = union of all concrete names across all hosts.

## TUI Screens

**Main screen:** `ratatui::widgets::Table` — rows=hosts, columns=services, cells color-coded (green=active, red=failed, yellow=inactive, gray=unknown). Status bar at bottom.

**Detail screen:** `ratatui::widgets::List` — shows files and commands for the selected host+service. Enter opens output in vim via temp file.

## Event Loop (app.rs)

```
loop {
    terminal.draw(|f| render(f, &state));
    // drain async refresh results (non-blocking)
    while let Ok(result) = refresh_rx.try_recv() { update grid cell }
    // poll keyboard with 200ms timeout
    if poll(200ms) { handle key }
    if should_quit { break }
}
```

## Keyboard Handling

| Key | Main Screen | Detail Screen |
|-----|-------------|---------------|
| Enter | Go to detail screen | Open file/cmd output in vim |
| r | Spawn full refresh (async) | Same |
| c | Suspend TUI, `ssh <host>` | Same |
| s | `sudo systemctl stop <svc>` then refresh cell | Same |
| t | `sudo systemctl restart <svc>` then refresh cell | Same |
| q | Quit app | Back to main screen |
| Arrow keys | Navigate grid | Navigate list |

## Verification

1. **Config parsing:** Create sample `inventory.ini` and `services.yaml` from README examples, run binary and verify parsed output
2. **SSH connectivity:** Test against a reachable host — verify session creation, command execution, connection reuse
3. **TUI rendering:** Launch app with sample configs, verify grid displays with correct statuses
4. **Keyboard controls:** Test each key binding (navigation, refresh, stop, restart, connect, detail view, quit)
5. **Glob expansion:** Add a glob pattern service, verify it expands correctly per host
6. **Vim integration:** Select a file/command in detail screen, verify vim opens with correct content
7. **Error resilience:** Test with an unreachable host — verify error displays in grid without crashing
