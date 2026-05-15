use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
    },
    Frame,
};

use super::app::{App, AppMode, Tab};

const SELECTED_STYLE: Style = Style::new()
    .fg(Color::Yellow)
    .add_modifier(Modifier::BOLD);
const DIM_STYLE: Style = Style::new().fg(Color::DarkGray);
const OK_STYLE:  Style = Style::new().fg(Color::Green);
const ERR_STYLE: Style = Style::new().fg(Color::Red);
const HDR_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

pub fn draw(frame: &mut Frame, app: &mut App) {
    if matches!(app.mode, AppMode::Login) {
        draw_login(frame, app);
        return;
    }

    let area = frame.area();
    let [title_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ]).areas(area);

    draw_tabs(frame, title_area, app);
    draw_body(frame, body_area, app);
    draw_status(frame, status_area, app);

    // Overlays
    if app.new_tok_value.is_some()  { draw_new_token(frame, area, app); }
    if app.confirm.is_some()        { draw_confirm(frame, area, app); }
    if app.publish.is_some()        { draw_publish_form(frame, area, app); }
    if app.tok_create.is_some()     { draw_create_token_form(frame, area, app); }
}

// ── Tabs bar ──────────────────────────────────────────────────────────────────

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles = vec!["1:Packages", "2:Users", "3:Tokens", "4:Audit"];
    let tabs = Tabs::new(titles)
        .select(app.tab.index())
        .block(Block::default()
            .title(format!(
                " freight-registry tui — {}{}",
                app.current_user,
                if app.is_admin { " [admin]" } else { "" }
            ))
            .borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .divider("│");
    frame.render_widget(tabs, area);
}

// ── Body dispatch ─────────────────────────────────────────────────────────────

fn draw_body(frame: &mut Frame, area: Rect, app: &mut App) {
    match app.tab {
        Tab::Packages => draw_packages(frame, area, app),
        Tab::Users    => draw_users(frame, area, app),
        Tab::Tokens   => draw_tokens(frame, area, app),
        Tab::Audit    => draw_audit(frame, area, app),
    }
}

// ── Packages tab ──────────────────────────────────────────────────────────────

fn draw_packages(frame: &mut Frame, area: Rect, app: &mut App) {
    let [search_area, list_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
    ]).areas(area);

    // Search box
    let search_block = Block::default()
        .title(if app.pkg_search_on { " Search (Enter to run, Esc to cancel) " }
               else { " / to search, r to refresh, P to publish " })
        .borders(Borders::ALL)
        .border_style(if app.pkg_search_on { Style::default().fg(Color::Yellow) }
                      else { Style::default() });
    let search_text = Paragraph::new(app.pkg_search.as_str()).block(search_block);
    frame.render_widget(search_text, search_area);

    if app.pkg_detail.is_some() {
        // Split: left=list, right=detail
        let [left, right] = Layout::horizontal([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ]).areas(list_area);
        draw_package_list(frame, left, app);
        draw_package_detail(frame, right, app);
    } else {
        draw_package_list(frame, list_area, app);
    }
}

fn draw_package_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let loading_sfx = if app.loading { " (loading…)" } else { "" };
    let items: Vec<ListItem> = app.packages.iter().map(|p| {
        let ver  = p.latest.as_deref().unwrap_or("?");
        let dl   = p.downloads;
        let line = Line::from(vec![
            Span::raw(format!("{:<28}", &p.name)),
            Span::styled(format!(" {:<10}", ver), DIM_STYLE),
            Span::styled(format!(" ↓{dl}"), DIM_STYLE),
        ]);
        ListItem::new(line)
    }).collect();

    let list = List::new(items)
        .block(Block::default()
            .title(format!(" Packages ({}){} ", app.packages.len(), loading_sfx))
            .borders(Borders::ALL))
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol("► ");

    frame.render_stateful_widget(list, area, &mut app.pkg_state);
}

fn draw_package_detail(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(detail) = &app.pkg_detail else { return };

    let [info_area, ver_area, hint_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(0),
        Constraint::Length(1),
    ]).areas(area);

    // Info block
    let owners = detail.owners.join(", ");
    let desc   = detail.description.as_deref().unwrap_or("—");
    let info = Paragraph::new(vec![
        Line::from(vec![Span::styled("Name:  ", HDR_STYLE), Span::raw(&detail.name)]),
        Line::from(vec![Span::styled("Desc:  ", HDR_STYLE), Span::raw(desc)]),
        Line::from(vec![Span::styled("Owners:", HDR_STYLE), Span::raw(format!(" {owners}"))]),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Detail "));
    frame.render_widget(info, info_area);

    // Versions
    let items: Vec<ListItem> = detail.versions.iter().map(|v| {
        let tag   = if v.yanked { Span::styled(" [yanked]", ERR_STYLE) }
                    else        { Span::styled(" [active]", OK_STYLE) };
        let line  = Line::from(vec![
            Span::raw(format!("{:<14}", v.version)),
            Span::styled(format!("↓{:<8}", v.downloads), DIM_STYLE),
            tag,
        ]);
        ListItem::new(line)
    }).collect();

    let ver_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Versions (j/k, y=yank, u=unyank) "))
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol("► ");
    frame.render_stateful_widget(ver_list, ver_area, &mut app.ver_state);

    // Hints
    let mut hints = vec![Span::raw(" [Esc] back  [y] yank  [u] unyank")];
    if app.is_admin { hints.push(Span::styled("  [d] delete package", ERR_STYLE)); }
    frame.render_widget(Paragraph::new(Line::from(hints)), hint_area);
}

// ── Users tab ─────────────────────────────────────────────────────────────────

fn draw_users(frame: &mut Frame, area: Rect, app: &mut App) {
    let header = Row::new(["ID", "Username", "Email", "Admin"])
        .style(HDR_STYLE);
    let rows: Vec<Row> = app.users.iter().map(|u| {
        let admin = if u.is_admin { "✓" } else { "" };
        Row::new(vec![
            u.id.to_string(),
            u.username.clone(),
            u.email.clone().unwrap_or_default(),
            admin.to_string(),
        ])
    }).collect();

    let hint = if app.is_admin { " [p] promote  [d] demote  [x] remove  [r] refresh" }
               else            { " [r] refresh (read-only — admin access required for mutations)" };

    let [table_area, hint_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
    ]).areas(area);

    let table = Table::new(rows, [
        Constraint::Length(6),
        Constraint::Length(24),
        Constraint::Min(28),
        Constraint::Length(6),
    ])
    .header(header)
    .block(Block::default().borders(Borders::ALL)
        .title(format!(" Users ({}) ", app.users.len())))
    .highlight_style(SELECTED_STYLE)
    .highlight_symbol("► ");

    frame.render_stateful_widget(table, table_area, &mut app.usr_state);
    frame.render_widget(Paragraph::new(hint), hint_area);
}

// ── Tokens tab ────────────────────────────────────────────────────────────────

fn draw_tokens(frame: &mut Frame, area: Rect, app: &mut App) {
    let header = Row::new(["ID", "Name", "Kind", "Expires", "Last used"])
        .style(HDR_STYLE);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let rows: Vec<Row> = app.tokens.iter().map(|t| {
        let expires = t.expires_at.map(|ts| {
            let days = (ts - now) / 86_400;
            if days < 0 { "expired".to_string() } else { format!("{days}d") }
        }).unwrap_or_else(|| "never".to_string());
        let last = t.last_used.map(|ts| {
            let secs = now - ts;
            if secs < 3600 { format!("{}m ago", secs / 60) }
            else           { format!("{}h ago", secs / 3600) }
        }).unwrap_or_else(|| "never".to_string());
        Row::new(vec![
            t.id.to_string(), t.name.clone(), t.kind.clone(), expires, last,
        ])
    }).collect();

    let [table_area, hint_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
    ]).areas(area);

    let table = Table::new(rows, [
        Constraint::Length(6),
        Constraint::Length(24),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(12),
    ])
    .header(header)
    .block(Block::default().borders(Borders::ALL)
        .title(format!(" My Tokens ({}) ", app.tokens.len())))
    .highlight_style(SELECTED_STYLE)
    .highlight_symbol("► ");

    frame.render_stateful_widget(table, table_area, &mut app.tok_state);
    frame.render_widget(
        Paragraph::new(" [n] new token  [x/Del] revoke  [r] refresh"),
        hint_area,
    );
}

// ── Audit tab ─────────────────────────────────────────────────────────────────

fn draw_audit(frame: &mut Frame, area: Rect, app: &mut App) {
    let [filter_area, table_area, hint_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ]).areas(area);

    // Filter box
    let filter_block = Block::default()
        .title(if app.aud_filter_on { " Filter (user:name or action, Enter/Esc to apply) " }
               else { " / to filter, r to refresh " })
        .borders(Borders::ALL)
        .border_style(if app.aud_filter_on { Style::default().fg(Color::Yellow) }
                      else { Style::default() });
    frame.render_widget(
        Paragraph::new(app.aud_filter.as_str()).block(filter_block),
        filter_area,
    );

    // Table
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let header = Row::new(["Time", "User", "Action", "Package", "Version", "IP"])
        .style(HDR_STYLE);

    let rows: Vec<Row> = app.audit.iter().map(|e| {
        let age   = now - e.created_at;
        let time  = if age < 3600 { format!("{}m ago", age / 60) }
                    else { format!("{}h ago", age / 3600) };
        Row::new(vec![
            time,
            e.username.clone().unwrap_or_else(|| "—".into()),
            e.action.clone(),
            e.package.clone().unwrap_or_default(),
            e.version.clone().unwrap_or_default(),
            e.ip_addr.clone().unwrap_or_default(),
        ])
    }).collect();

    let table = Table::new(rows, [
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Length(10),
        Constraint::Min(12),
    ])
    .header(header)
    .block(Block::default().borders(Borders::ALL)
        .title(format!(" Audit Log ({}) ", app.audit.len())))
    .highlight_style(SELECTED_STYLE)
    .highlight_symbol("► ");

    frame.render_stateful_widget(table, table_area, &mut app.aud_state);
    frame.render_widget(Paragraph::new(" j/k or ↑↓ to scroll"), hint_area);
}

// ── Status bar ────────────────────────────────────────────────────────────────

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let spinner = if app.loading { "⣷ " } else { "" };
    let style   = if app.is_err { ERR_STYLE } else { OK_STYLE };
    let line    = Line::from(vec![
        Span::styled(spinner, Style::default().fg(Color::Yellow)),
        Span::styled(&app.status, style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ── Login screen ──────────────────────────────────────────────────────────────

fn draw_login(frame: &mut Frame, app: &App) {
    let area = center_rect(50, 14, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" freight-registry — login ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [url_a, usr_a, pw_a, _, err_a, hint_a] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(inner);

    let field_style = |idx: usize| {
        if app.login.field == idx { Style::default().fg(Color::Yellow) }
        else { Style::default() }
    };

    frame.render_widget(
        Paragraph::new(app.login.url.as_str())
            .block(Block::default().title(" Registry URL ").borders(Borders::ALL)
                .border_style(field_style(0))),
        url_a,
    );
    frame.render_widget(
        Paragraph::new(app.login.username.as_str())
            .block(Block::default().title(" Username ").borders(Borders::ALL)
                .border_style(field_style(1))),
        usr_a,
    );
    let pw_mask: String = "•".repeat(app.login.password.len());
    frame.render_widget(
        Paragraph::new(pw_mask.as_str())
            .block(Block::default().title(" Password ").borders(Borders::ALL)
                .border_style(field_style(2))),
        pw_a,
    );
    if !app.login.error.is_empty() {
        frame.render_widget(
            Paragraph::new(app.login.error.as_str()).style(ERR_STYLE),
            err_a,
        );
    }
    frame.render_widget(
        Paragraph::new(" Tab/↑↓ move  Enter login  Esc quit")
            .style(DIM_STYLE),
        hint_a,
    );
}

// ── Confirm dialog ────────────────────────────────────────────────────────────

fn draw_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let Some(dlg) = &app.confirm else { return };
    let popup = center_rect(50, 7, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [msg_a, _, hint_a] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(inner);

    frame.render_widget(
        Paragraph::new(dlg.message.as_str())
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center),
        msg_a,
    );
    frame.render_widget(
        Paragraph::new(" [y/Enter] confirm  [n/Esc] cancel")
            .style(DIM_STYLE)
            .alignment(Alignment::Center),
        hint_a,
    );
}

// ── Publish form ──────────────────────────────────────────────────────────────

fn draw_publish_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = &app.publish else { return };
    let popup = center_rect(54, 15, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Publish Package ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [na, va, pa, _, ha] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(inner);

    let fs = |idx: usize| if form.field == idx { Style::default().fg(Color::Yellow) }
                          else { Style::default() };

    frame.render_widget(
        Paragraph::new(form.name.as_str())
            .block(Block::default().title(" Package name ").borders(Borders::ALL).border_style(fs(0))),
        na,
    );
    frame.render_widget(
        Paragraph::new(form.vers.as_str())
            .block(Block::default().title(" Version ").borders(Borders::ALL).border_style(fs(1))),
        va,
    );
    frame.render_widget(
        Paragraph::new(form.path.as_str())
            .block(Block::default().title(" Path to .tar.gz (Enter to publish) ").borders(Borders::ALL).border_style(fs(2))),
        pa,
    );
    frame.render_widget(
        Paragraph::new(" Tab/↑↓ move fields  Enter on path to publish  Esc cancel")
            .style(DIM_STYLE),
        ha,
    );
}

// ── Create token form ─────────────────────────────────────────────────────────

fn draw_create_token_form(frame: &mut Frame, area: Rect, app: &App) {
    let Some(form) = &app.tok_create else { return };
    let popup = center_rect(48, 7, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" New Token ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [name_a, _, hint_a] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(inner);

    frame.render_widget(
        Paragraph::new(form.name.as_str())
            .block(Block::default().title(" Token name ").borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))),
        name_a,
    );
    frame.render_widget(
        Paragraph::new(" Enter to create  Esc to cancel").style(DIM_STYLE),
        hint_a,
    );
}

// ── New token reveal ──────────────────────────────────────────────────────────

fn draw_new_token(frame: &mut Frame, area: Rect, app: &App) {
    let Some(raw) = &app.new_tok_value else { return };
    let popup = center_rect(60, 9, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Token created — copy this now! ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Green));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [tok_a, _, hint_a] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(inner);

    frame.render_widget(
        Paragraph::new(raw.as_str())
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: false }),
        tok_a,
    );
    frame.render_widget(
        Paragraph::new(" Press any key to continue").style(DIM_STYLE),
        hint_a,
    );
}

// ── Layout helpers ────────────────────────────────────────────────────────────

fn center_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
}
