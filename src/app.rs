use crate::config::{Host, ServiceConfig};
use crate::monitor::status::{build_grid, refresh_cell};
use crate::monitor::{GridResult, HostService, ServiceStatus};
use crate::ssh::SessionManager;
use crate::tui;
use crate::tui::event::{poll_event, AppEvent};
use crate::tui::ui::render;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::HashSet;
use std::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Screen {
    Main,
    Detail {
        host_index: usize,
        service_index: usize,
    },
}

pub enum RefreshResult {
    FullGrid(GridResult),
}

#[derive(Debug, Clone, Copy)]
pub enum FlatEntry {
    Service { host_idx: usize, svc_idx: usize },
    UnreachableHost { host_idx: usize },
}

pub struct AppState {
    pub hosts: Vec<Host>,
    pub service_configs: Vec<ServiceConfig>,
    pub service_names: Vec<String>,
    pub grid: Vec<Vec<HostService>>,
    pub unreachable_hosts: HashSet<usize>,
    pub screen: Screen,
    pub cursor: usize,
    pub table_state: TableState,
    pub detail_cursor: usize,
    pub refreshing: bool,
    pub should_quit: bool,
    pub ssh_user: Option<String>,
}

impl AppState {
    pub fn new(hosts: Vec<Host>, service_configs: Vec<ServiceConfig>, ssh_user: Option<String>) -> Self {
        Self {
            hosts,
            service_configs,
            service_names: Vec::new(),
            grid: Vec::new(),
            unreachable_hosts: HashSet::new(),
            screen: Screen::Main,
            cursor: 0,
            table_state: TableState::default().with_selected(0),
            detail_cursor: 0,
            refreshing: false,
            should_quit: false,
            ssh_user,
        }
    }

    /// Build a flat list of entries for the main screen.
    /// Unreachable hosts and failed services are sorted to the top.
    pub fn flat_entries(&self) -> Vec<FlatEntry> {
        let mut failed = Vec::new();
        let mut rest = Vec::new();

        for (host_idx, row) in self.grid.iter().enumerate() {
            if self.unreachable_hosts.contains(&host_idx) {
                failed.push(FlatEntry::UnreachableHost { host_idx });
            } else {
                for (svc_idx, hs) in row.iter().enumerate() {
                    let entry = FlatEntry::Service { host_idx, svc_idx };
                    if hs.status == ServiceStatus::Failed {
                        failed.push(entry);
                    } else {
                        rest.push(entry);
                    }
                }
            }
        }

        failed.extend(rest);
        failed
    }

    /// Total number of entries in the flat list.
    pub fn flat_len(&self) -> usize {
        self.flat_entries().len()
    }

    /// Get the flat entry at the current cursor position.
    pub fn selected_entry(&self) -> Option<FlatEntry> {
        self.flat_entries().get(self.cursor).copied()
    }

    /// Get the list of detail items (files + commands) for the current detail view.
    fn detail_items(&self, host_idx: usize, svc_idx: usize) -> Vec<DetailItem> {
        let hs = &self.grid[host_idx][svc_idx];
        let mut items = Vec::new();

        if !hs.config.files.is_empty() {
            items.push(DetailItem::Header(()));
            for f in &hs.config.files {
                items.push(DetailItem::File(f.clone()));
            }
        }

        if !hs.config.commands.is_empty() {
            items.push(DetailItem::Header(()));
            for c in &hs.config.commands {
                items.push(DetailItem::Command(c.clone()));
            }
        }

        items
    }

    fn detail_item_count(&self, host_idx: usize, svc_idx: usize) -> usize {
        self.detail_items(host_idx, svc_idx).len()
    }

    fn apply_grid_result(&mut self, result: GridResult) {
        self.service_names = result.service_names;
        self.grid = result.grid;
        self.unreachable_hosts = result.unreachable_hosts;
        self.refreshing = false;
        // Clamp cursor
        let len = self.flat_len();
        if len > 0 && self.cursor >= len {
            self.cursor = len - 1;
        }
    }
}

#[derive(Debug, Clone)]
enum DetailItem {
    Header(()),
    File(String),
    Command(String),
}

pub async fn run(
    hosts: Vec<Host>,
    service_configs: Vec<ServiceConfig>,
    ssh_user: Option<String>,
) -> Result<()> {
    let mut state = AppState::new(hosts, service_configs, ssh_user);
    let mut terminal = tui::init()?;

    // Set up async refresh channel
    let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel::<RefreshResult>();

    // Initial refresh (non-blocking so the UI stays responsive)
    log::info!("Starting initial refresh");
    spawn_full_refresh(&mut state, &refresh_tx);

    loop {
        state.table_state.select(Some(state.cursor));
        terminal.draw(|f| render(f, &mut state))?;

        // Drain async refresh results
        while let Ok(result) = refresh_rx.try_recv() {
            match result {
                RefreshResult::FullGrid(grid_result) => {
                    log::info!(
                        "Refresh complete: {} services, {} unreachable hosts",
                        grid_result.service_names.len(),
                        grid_result.unreachable_hosts.len()
                    );
                    state.apply_grid_result(grid_result);
                }
            }
        }

        // Poll keyboard with 200ms timeout
        match poll_event(200)? {
            AppEvent::Key(key) => {
                handle_key(&mut state, key, &refresh_tx, &mut terminal).await?;
            }
            AppEvent::None => {}
        }

        if state.should_quit {
            break;
        }
    }

    tui::restore()?;
    Ok(())
}

async fn handle_key(
    state: &mut AppState,
    key: KeyEvent,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
    terminal: &mut tui::Tui,
) -> Result<()> {
    match &state.screen {
        Screen::Main => handle_main_key(state, key, refresh_tx, terminal).await,
        Screen::Detail {
            host_index,
            service_index,
        } => {
            let hi = *host_index;
            let si = *service_index;
            handle_detail_key(state, key, hi, si, refresh_tx, terminal).await
        }
    }
}

async fn handle_main_key(
    state: &mut AppState,
    key: KeyEvent,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
    terminal: &mut tui::Tui,
) -> Result<()> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            log::info!("Quit requested");
            state.should_quit = true;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            log::info!("Ctrl+C quit requested");
            state.should_quit = true;
        }
        KeyCode::Up => {
            if state.cursor > 0 {
                state.cursor -= 1;
            }
        }
        KeyCode::Down => {
            let len = state.flat_len();
            if len > 0 && state.cursor + 1 < len {
                state.cursor += 1;
            }
        }
        KeyCode::Enter => {
            if let Some(FlatEntry::Service { host_idx, svc_idx }) = state.selected_entry() {
                log::info!(
                    "Opening detail view for {}:{}",
                    state.hosts[host_idx].address,
                    state.grid[host_idx][svc_idx].service_name
                );
                state.screen = Screen::Detail {
                    host_index: host_idx,
                    service_index: svc_idx,
                };
                state.detail_cursor = 0;
            }
        }
        KeyCode::Char('r') => {
            log::info!("Full refresh requested");
            spawn_full_refresh(state, refresh_tx);
        }
        KeyCode::Char('c') => {
            let host_idx = match state.selected_entry() {
                Some(FlatEntry::Service { host_idx, .. }) => Some(host_idx),
                Some(FlatEntry::UnreachableHost { host_idx }) => Some(host_idx),
                None => None,
            };
            if let Some(hi) = host_idx {
                let host = state.hosts[hi].address.clone();
                let ssh_dest = match &state.ssh_user {
                    Some(user) => format!("{}@{}", user, host),
                    None => host.clone(),
                };
                log::info!("Opening SSH session to {}", ssh_dest);
                suspend_and_run(terminal, &["ssh", &ssh_dest])?;
                log::info!("Returned from SSH session to {}", ssh_dest);
            }
        }
        KeyCode::Char('s') => {
            if let Some(FlatEntry::Service { host_idx, svc_idx }) = state.selected_entry() {
                let host = state.hosts[host_idx].address.clone();
                let svc = state.grid[host_idx][svc_idx].service_name.clone();
                log::info!("Stopping service {} on {}", svc, host);
                run_service_action(state, &host, &svc, "stop", host_idx, svc_idx).await;
            }
        }
        KeyCode::Char('t') => {
            if let Some(FlatEntry::Service { host_idx, svc_idx }) = state.selected_entry() {
                let host = state.hosts[host_idx].address.clone();
                let svc = state.grid[host_idx][svc_idx].service_name.clone();
                log::info!("Restarting service {} on {}", svc, host);
                run_service_action(state, &host, &svc, "restart", host_idx, svc_idx).await;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_detail_key(
    state: &mut AppState,
    key: KeyEvent,
    host_idx: usize,
    svc_idx: usize,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
    terminal: &mut tui::Tui,
) -> Result<()> {
    let item_count = state.detail_item_count(host_idx, svc_idx);

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            log::info!("Ctrl+C quit requested from detail screen");
            state.should_quit = true;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            log::info!("Returning to main screen");
            state.screen = Screen::Main;
            state.detail_cursor = 0;
        }
        KeyCode::Up => {
            if state.detail_cursor > 0 {
                state.detail_cursor -= 1;
            }
        }
        KeyCode::Down => {
            if state.detail_cursor + 1 < item_count {
                state.detail_cursor += 1;
            }
        }
        KeyCode::Enter => {
            let items = state.detail_items(host_idx, svc_idx);
            if let Some(item) = items.get(state.detail_cursor) {
                let host = &state.hosts[host_idx].address;
                match item {
                    DetailItem::File(path) => {
                        log::info!("Viewing file {} on {}", path, host);
                        let cmd = format!("cat {}", path);
                        open_in_vim(terminal, host, &cmd, &state.ssh_user).await?;
                    }
                    DetailItem::Command(cmd) => {
                        log::info!("Running command '{}' on {} and viewing in vim", cmd, host);
                        open_in_vim(terminal, host, cmd, &state.ssh_user).await?;
                    }
                    DetailItem::Header(_) => {}
                }
            }
        }
        KeyCode::Char('r') => {
            spawn_full_refresh(state, refresh_tx);
        }
        KeyCode::Char('c') => {
            let host = state.hosts[host_idx].address.clone();
            let ssh_dest = match &state.ssh_user {
                Some(user) => format!("{}@{}", user, host),
                None => host.clone(),
            };
            suspend_and_run(terminal, &["ssh", &ssh_dest])?;
        }
        KeyCode::Char('s') => {
            let host = state.hosts[host_idx].address.clone();
            let svc = state.grid[host_idx][svc_idx].service_name.clone();
            run_service_action(state, &host, &svc, "stop", host_idx, svc_idx).await;
        }
        KeyCode::Char('t') => {
            let host = state.hosts[host_idx].address.clone();
            let svc = state.grid[host_idx][svc_idx].service_name.clone();
            run_service_action(state, &host, &svc, "restart", host_idx, svc_idx).await;
        }
        _ => {}
    }
    Ok(())
}

fn spawn_full_refresh(
    state: &mut AppState,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
) {
    if state.refreshing {
        return;
    }
    state.refreshing = true;

    let hosts = state.hosts.clone();
    let configs = state.service_configs.clone();
    let ssh_user = state.ssh_user.clone();
    let tx = refresh_tx.clone();

    tokio::spawn(async move {
        let mut session_mgr = SessionManager::new(ssh_user);
        let grid_result = build_grid(&mut session_mgr, &hosts, &configs).await;
        let _ = tx.send(RefreshResult::FullGrid(grid_result));
        session_mgr.close_all().await;
    });
}

async fn run_service_action(
    state: &mut AppState,
    host: &str,
    service: &str,
    action: &str,
    host_idx: usize,
    svc_idx: usize,
) {
    let mut session_mgr = SessionManager::new(state.ssh_user.clone());
    let cmd = format!("sudo systemctl {} {}", action, service);
    match session_mgr.run_command(host, &cmd).await {
        Ok(_) => log::info!("Service action '{}' succeeded for {} on {}", action, service, host),
        Err(e) => log::error!("Service action '{}' failed for {} on {}: {}", action, service, host, e),
    }

    // Refresh the cell
    let status = refresh_cell(&mut session_mgr, host, service).await;
    log::info!("Status after {} for {}:{} = {:?}", action, host, service, status);
    if host_idx < state.grid.len() && svc_idx < state.grid[host_idx].len() {
        state.grid[host_idx][svc_idx].status = status;
    }
    session_mgr.close_all().await;
}

async fn open_in_vim(
    terminal: &mut tui::Tui,
    host: &str,
    cmd: &str,
    ssh_user: &Option<String>,
) -> Result<()> {
    // Run the command on the remote host, write output to a temp file, open in vim
    let mut session_mgr = SessionManager::new(ssh_user.clone());
    let output = session_mgr
        .run_command(host, cmd)
        .await
        .unwrap_or_else(|e| format!("Error: {}", e));
    session_mgr.close_all().await;

    let tmp = std::env::temp_dir().join(format!(
        "sctl-{}-{}.txt",
        host.replace('.', "_"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));

    std::fs::write(&tmp, &output)?;

    suspend_and_run(terminal, &["vim", "-R", tmp.to_str().unwrap()])?;

    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn suspend_and_run(terminal: &mut tui::Tui, args: &[&str]) -> Result<()> {
    tui::suspend()?;

    let status = Command::new(args[0])
        .args(&args[1..])
        .status();

    match status {
        Ok(_) => {}
        Err(e) => eprintln!("Failed to run {:?}: {}", args, e),
    }

    let new_terminal = tui::resume()?;
    *terminal = new_terminal;
    Ok(())
}
