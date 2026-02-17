use crate::app::{AppState, FlatEntry, Screen};
use crate::monitor::ServiceStatus;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render(frame: &mut Frame, state: &mut AppState) {
    match state.screen.clone() {
        Screen::Main => render_main(frame, state),
        Screen::Detail {
            host_index,
            service_index,
        } => render_detail(frame, state, host_index, service_index),
    }
}

fn render_main(frame: &mut Frame, state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    render_service_list(frame, state, chunks[0]);
    render_status_bar(frame, state, chunks[1]);
}

fn render_service_list(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let entries = state.flat_entries();

    if entries.is_empty() {
        let msg = if state.refreshing {
            "Refreshing..."
        } else {
            "No data. Press 'r' to refresh or check your config files."
        };
        let paragraph = Paragraph::new(msg)
            .block(Block::default().borders(Borders::ALL).title("Services"));
        frame.render_widget(paragraph, area);
        return;
    }

    // Header
    let header = Row::new(vec![
        Cell::from("Service").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Host").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .height(1);

    // Data rows
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| match entry {
            FlatEntry::Service { host_idx, svc_idx } => {
                let hs = &state.grid[*host_idx][*svc_idx];
                let status_style = status_color(&hs.status);

                Row::new(vec![
                    Cell::from(hs.service_name.as_str()),
                    Cell::from(hs.host_address.as_str()),
                    Cell::from(hs.status.display()).style(status_style),
                ])
            }
            FlatEntry::UnreachableHost { host_idx, reason } => {
                let host = &state.hosts[*host_idx].address;
                let style = Style::default().fg(Color::Red);

                Row::new(vec![
                    Cell::from("").style(style),
                    Cell::from(host.as_str()).style(style),
                    Cell::from(reason.as_str()).style(style),
                ])
            }
        })
        .collect();

    let widths = [
        Constraint::Length(25),
        Constraint::Length(20),
        Constraint::Min(10),
    ];

    let table = Table::new(rows, &widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Services"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(table, area, &mut state.table_state);
}

fn render_status_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    let status_text = if state.refreshing {
        "Refreshing..."
    } else {
        "r:refresh  Enter:detail  c:ssh  s:stop  t:restart  q:quit"
    };

    let bar = Paragraph::new(Line::from(Span::styled(
        status_text,
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(bar, area);
}

fn render_detail(frame: &mut Frame, state: &AppState, host_idx: usize, svc_idx: usize) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let hs = &state.grid[host_idx][svc_idx];

    let mut items: Vec<ListItem> = Vec::new();

    if !hs.config.files.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "--- Files ---",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))));
        for f in &hs.config.files {
            items.push(ListItem::new(format!("  {}", f)));
        }
    }

    if !hs.config.commands.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "--- Commands ---",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))));
        for c in &hs.config.commands {
            items.push(ListItem::new(format!("  {}", c)));
        }
    }

    // Highlight the selected item
    let items: Vec<ListItem> = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            if i == state.detail_cursor {
                item.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect();

    let title = format!(
        " {}:{} [{}] ",
        hs.host_address,
        hs.service_name,
        hs.status.display()
    );

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );

    frame.render_widget(list, chunks[0]);

    let bar = Paragraph::new(Line::from(Span::styled(
        "Enter:view in vim  r:refresh  c:ssh  s:stop  t:restart  q:back",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(bar, chunks[1]);
}

fn status_color(status: &ServiceStatus) -> Style {
    match status {
        ServiceStatus::Active => Style::default().fg(Color::Green),
        ServiceStatus::Failed => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ServiceStatus::Inactive => Style::default().fg(Color::Yellow),
        ServiceStatus::NotFound => Style::default().fg(Color::DarkGray),
        ServiceStatus::Unknown => Style::default().fg(Color::Gray),
        ServiceStatus::Error(_) => Style::default().fg(Color::Red),
    }
}
