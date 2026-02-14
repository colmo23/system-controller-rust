use crate::config::{Host, ServiceConfig};
use crate::monitor::status::{build_grid, refresh_cell};
use crate::monitor::HostService;
use crate::ssh::SessionManager;
use crate::tui;
use crate::tui::event::{poll_event, AppEvent};
use crate::tui::ui::render;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
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
    FullGrid {
        service_names: Vec<String>,
        grid: Vec<Vec<HostService>>,
    },
}

pub struct AppState {
    pub hosts: Vec<Host>,
    pub service_configs: Vec<ServiceConfig>,
    pub service_names: Vec<String>,
    pub grid: Vec<Vec<HostService>>,
    pub screen: Screen,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub detail_cursor: usize,
    pub refreshing: bool,
    pub should_quit: bool,
}

impl AppState {
    pub fn new(hosts: Vec<Host>, service_configs: Vec<ServiceConfig>) -> Self {
        Self {
            hosts,
            service_configs,
            service_names: Vec::new(),
            grid: Vec::new(),
            screen: Screen::Main,
            cursor_row: 0,
            cursor_col: 0,
            detail_cursor: 0,
            refreshing: false,
            should_quit: false,
        }
    }

    fn max_rows(&self) -> usize {
        self.hosts.len()
    }

    fn max_cols(&self) -> usize {
        self.service_names.len()
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
) -> Result<()> {
    let mut state = AppState::new(hosts, service_configs);
    let mut terminal = tui::init()?;

    // Set up async refresh channel
    let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel::<RefreshResult>();

    // Initial refresh
    {
        let mut session_mgr = SessionManager::new();
        state.refreshing = true;
        terminal.draw(|f| render(f, &state))?;

        let (names, grid) =
            build_grid(&mut session_mgr, &state.hosts, &state.service_configs).await;
        state.service_names = names;
        state.grid = grid;
        state.refreshing = false;

        // Keep session manager for later use - we'll recreate as needed
        session_mgr.close_all().await;
    }

    loop {
        terminal.draw(|f| render(f, &state))?;

        // Drain async refresh results
        while let Ok(result) = refresh_rx.try_recv() {
            match result {
                RefreshResult::FullGrid {
                    service_names,
                    grid,
                } => {
                    state.service_names = service_names;
                    state.grid = grid;
                    state.refreshing = false;
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
        KeyCode::Char('q') => {
            state.should_quit = true;
        }
        KeyCode::Up => {
            if state.cursor_row > 0 {
                state.cursor_row -= 1;
            }
        }
        KeyCode::Down => {
            if state.cursor_row + 1 < state.max_rows() {
                state.cursor_row += 1;
            }
        }
        KeyCode::Left => {
            if state.cursor_col > 0 {
                state.cursor_col -= 1;
            }
        }
        KeyCode::Right => {
            if state.cursor_col + 1 < state.max_cols() {
                state.cursor_col += 1;
            }
        }
        KeyCode::Enter => {
            if !state.grid.is_empty() && !state.service_names.is_empty() {
                state.screen = Screen::Detail {
                    host_index: state.cursor_row,
                    service_index: state.cursor_col,
                };
                state.detail_cursor = 0;
            }
        }
        KeyCode::Char('r') => {
            spawn_full_refresh(state, refresh_tx);
        }
        KeyCode::Char('c') => {
            if !state.hosts.is_empty() {
                let host = state.hosts[state.cursor_row].address.clone();
                suspend_and_run(terminal, &["ssh", &host])?;
            }
        }
        KeyCode::Char('s') => {
            if !state.grid.is_empty() {
                let host = state.hosts[state.cursor_row].address.clone();
                let svc = state.service_names[state.cursor_col].clone();
                run_service_action(state, &host, &svc, "stop", state.cursor_row, state.cursor_col)
                    .await;
            }
        }
        KeyCode::Char('t') => {
            if !state.grid.is_empty() {
                let host = state.hosts[state.cursor_row].address.clone();
                let svc = state.service_names[state.cursor_col].clone();
                run_service_action(
                    state,
                    &host,
                    &svc,
                    "restart",
                    state.cursor_row,
                    state.cursor_col,
                )
                .await;
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
        KeyCode::Char('q') => {
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
                        let cmd = format!("cat {}", path);
                        open_in_vim(terminal, host, &cmd).await?;
                    }
                    DetailItem::Command(cmd) => {
                        open_in_vim(terminal, host, cmd).await?;
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
            suspend_and_run(terminal, &["ssh", &host])?;
        }
        KeyCode::Char('s') => {
            let host = state.hosts[host_idx].address.clone();
            let svc = state.service_names[svc_idx].clone();
            run_service_action(state, &host, &svc, "stop", host_idx, svc_idx).await;
        }
        KeyCode::Char('t') => {
            let host = state.hosts[host_idx].address.clone();
            let svc = state.service_names[svc_idx].clone();
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
    let tx = refresh_tx.clone();

    tokio::spawn(async move {
        let mut session_mgr = SessionManager::new();
        let (names, grid) = build_grid(&mut session_mgr, &hosts, &configs).await;
        let _ = tx.send(RefreshResult::FullGrid {
            service_names: names,
            grid,
        });
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
    let mut session_mgr = SessionManager::new();
    let cmd = format!("sudo systemctl {} {}", action, service);
    let _ = session_mgr.run_command(host, &cmd).await;

    // Refresh the cell
    let status = refresh_cell(&mut session_mgr, host, service).await;
    if host_idx < state.grid.len() && svc_idx < state.grid[host_idx].len() {
        state.grid[host_idx][svc_idx].status = status;
    }
    session_mgr.close_all().await;
}

async fn open_in_vim(
    terminal: &mut tui::Tui,
    host: &str,
    cmd: &str,
) -> Result<()> {
    // Run the command on the remote host, write output to a temp file, open in vim
    let mut session_mgr = SessionManager::new();
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
