use crate::app::AppState;
use crate::app::Screen;
use crate::monitor::ServiceStatus;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render(frame: &mut Frame, state: &AppState) {
    match &state.screen {
        Screen::Main => render_main(frame, state),
        Screen::Detail {
            host_index,
            service_index,
        } => render_detail(frame, state, *host_index, *service_index),
    }
}

fn render_main(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    render_grid(frame, state, chunks[0]);
    render_status_bar(frame, state, chunks[1]);
}

fn render_grid(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.hosts.is_empty() || state.service_names.is_empty() {
        let msg = Paragraph::new("No data. Press 'r' to refresh or check your config files.")
            .block(Block::default().borders(Borders::ALL).title("Services"));
        frame.render_widget(msg, area);
        return;
    }

    // Build header row
    let mut header_cells = vec![Cell::from("Host").style(Style::default().add_modifier(Modifier::BOLD))];
    for name in &state.service_names {
        header_cells.push(
            Cell::from(name.as_str()).style(Style::default().add_modifier(Modifier::BOLD)),
        );
    }
    let header = Row::new(header_cells).height(1);

    // Build data rows
    let rows: Vec<Row> = state
        .grid
        .iter()
        .enumerate()
        .map(|(row_idx, row)| {
            let mut cells = vec![Cell::from(state.hosts[row_idx].address.as_str())];
            for (col_idx, hs) in row.iter().enumerate() {
                let style = status_style(&hs.status);
                let selected = row_idx == state.cursor_row && col_idx == state.cursor_col;
                let style = if selected {
                    style.add_modifier(Modifier::REVERSED)
                } else {
                    style
                };
                cells.push(Cell::from(hs.status.display()).style(style));
            }
            Row::new(cells)
        })
        .collect();

    // Column widths
    let mut widths = vec![Constraint::Length(20)]; // host column
    for name in &state.service_names {
        widths.push(Constraint::Length((name.len().max(10) + 2) as u16));
    }

    let table = Table::new(rows, &widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Services"))
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(table, area);
}

fn render_status_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    let status_text = if state.refreshing {
        "Refreshing..."
    } else {
        "r:refresh  Enter:detail  c:ssh  s:stop  t:restart  q:quit"
    };

    let host_info = if !state.hosts.is_empty() && !state.service_names.is_empty() {
        format!(
            " | {}/{}",
            state.hosts[state.cursor_row].address,
            state.service_names.get(state.cursor_col).map(|s| s.as_str()).unwrap_or("?")
        )
    } else {
        String::new()
    };

    let bar = Paragraph::new(Line::from(vec![
        Span::styled(status_text, Style::default().fg(Color::DarkGray)),
        Span::styled(host_info, Style::default().fg(Color::Cyan)),
    ]));
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

fn status_style(status: &ServiceStatus) -> Style {
    match status {
        ServiceStatus::Active => Style::default().fg(Color::Green),
        ServiceStatus::Failed => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ServiceStatus::Inactive => Style::default().fg(Color::Yellow),
        ServiceStatus::NotFound => Style::default().fg(Color::DarkGray),
        ServiceStatus::Unknown => Style::default().fg(Color::Gray),
        ServiceStatus::Error(_) => Style::default().fg(Color::Red),
    }
}
