//! Rendering. Every function here takes `&App` and a `Frame`, so the whole UI
//! can be rendered against a `TestBackend`.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use chiave_core::{EntryView, ExposeSecret, FieldValue};

use crate::app::{App, Focus, Mode};
use crate::form::{FieldKind, FIXED};

const MASK: &str = "••••••••";
const ACCENT: Color = Color::Cyan;

fn focused_style(active: bool) -> Style {
    if active {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn pane(title: &str, active: bool) -> Block<'static> {
    Block::bordered()
        .border_type(if active {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(focused_style(active))
        .title(Span::styled(format!(" {title} "), focused_style(active)))
}

/// Which panes are visible at this width.
struct Panes {
    tree: Option<Rect>,
    entries: Option<Rect>,
    detail: Option<Rect>,
}

fn layout_panes(app: &App, area: Rect) -> Panes {
    if area.width < 60 {
        // One pane at a time; Esc walks back towards the tree.
        return match app.focus {
            Focus::Tree => Panes {
                tree: Some(area),
                entries: None,
                detail: None,
            },
            Focus::Entries => Panes {
                tree: None,
                entries: Some(area),
                detail: None,
            },
            Focus::Detail => Panes {
                tree: None,
                entries: None,
                detail: Some(area),
            },
        };
    }
    if area.width < 100 {
        // The tree only earns its space when it has the focus.
        let [a, b] = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .areas(area);
        return if app.focus == Focus::Tree {
            Panes {
                tree: Some(a),
                entries: Some(b),
                detail: None,
            }
        } else {
            Panes {
                tree: None,
                entries: Some(a),
                detail: Some(b),
            }
        };
    }
    let [a, b, c] = Layout::horizontal([
        Constraint::Length(28),
        Constraint::Percentage(35),
        Constraint::Min(20),
    ])
    .areas(area);
    Panes {
        tree: Some(a),
        entries: Some(b),
        detail: Some(c),
    }
}

pub fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let [body, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    if app.mode == Mode::Locked {
        draw_unlock(app, frame, body);
        draw_status(app, frame, status);
        return;
    }

    let panes = layout_panes(app, body);
    if let Some(r) = panes.tree {
        draw_tree(app, frame, r);
    }
    if let Some(r) = panes.entries {
        draw_entries(app, frame, r);
    }
    if let Some(r) = panes.detail {
        draw_detail(app, frame, r);
    }
    draw_status(app, frame, status);

    if app.form.is_some() {
        draw_form(app, frame, body);
    }
    if app.picker.is_some() {
        draw_picker(app, frame, body);
    }
    if app.prompt_box.is_some() {
        draw_prompt(app, frame, body);
    }
    if app.gen.is_some() {
        draw_gen(app, frame, body);
    }
    if app.dialog.is_some() {
        draw_dialog(app, frame, body);
    }
    if app.mode == Mode::Help {
        draw_help(frame, body);
    }
}

// ----- left pane ------------------------------------------------------------

fn draw_tree(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.focus == Focus::Tree;
    let items: Vec<ListItem> = app
        .tree
        .rows
        .iter()
        .map(|row| {
            let marker = if row.has_children {
                if row.expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };
            let icon = if row.is_recycle_bin { "♻ " } else { "" };
            let text = format!("{}{marker}{icon}{}", "  ".repeat(row.depth), row.name);
            let style = if row.is_recycle_bin {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Span::styled(text, style))
        })
        .collect();
    let list = List::new(items)
        .block(pane("Groups", active))
        .highlight_style(selection_style(active))
        .highlight_symbol("");
    let mut state = ListState::default().with_selected(Some(app.tree.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn selection_style(active: bool) -> Style {
    if active {
        Style::default()
            .bg(ACCENT)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

// ----- middle pane ----------------------------------------------------------

fn draw_entries(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.focus == Focus::Entries;
    let (title, items, selected) = if app.search.active {
        let title = format!(
            "Search: {}{}",
            app.search.query,
            if app.mode == Mode::Search { "▏" } else { "" }
        );
        let items: Vec<ListItem> = app
            .search
            .hits
            .iter()
            .map(|hit| {
                let mut spans = vec![Span::raw(hit.path.clone())];
                if hit.has_otp {
                    spans.push(Span::styled(" [otp]", Style::default().fg(Color::Magenta)));
                }
                if hit.expired {
                    spans.push(Span::styled(" EXPIRED", Style::default().fg(Color::Red)));
                }
                if hit.in_recycle_bin {
                    spans.push(Span::styled(" ♻", Style::default().fg(Color::DarkGray)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        (title, items, app.search.selected)
    } else {
        // Line the usernames up in a column, but never past half the pane.
        let title_col = app
            .entries
            .iter()
            .filter(|e| e.username.is_some())
            .map(|e| e.title.width())
            .max()
            .unwrap_or(0)
            .min(area.width.saturating_sub(4) as usize / 2);
        let items: Vec<ListItem> = app
            .entries
            .iter()
            .map(|e| {
                let mut spans = vec![Span::raw(e.title.clone())];
                if let Some(u) = &e.username {
                    let pad = (title_col + 2).saturating_sub(e.title.width()).max(2);
                    spans.push(Span::styled(
                        format!("{}{u}", " ".repeat(pad)),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                if e.has_otp {
                    spans.push(Span::styled(" [otp]", Style::default().fg(Color::Magenta)));
                }
                if e.expired {
                    spans.push(Span::styled(" EXPIRED", Style::default().fg(Color::Red)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        ("Entries".to_string(), items, app.entry_sel)
    };

    if items.is_empty() {
        let hint = if app.search.active {
            "no matches"
        } else {
            "no entries in this group"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)))
                .block(pane(&title, active)),
            area,
        );
        return;
    }
    let list = List::new(items)
        .block(pane(&title, active))
        .highlight_style(selection_style(active));
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

// ----- right pane -----------------------------------------------------------

fn label(text: &str) -> Span<'static> {
    Span::styled(format!("{text:<11}"), Style::default().fg(Color::DarkGray))
}

fn draw_detail(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.focus == Focus::Detail;
    let block = pane("Details", active);
    let view = app
        .vault()
        .zip(app.selected_entry_id())
        .and_then(|(v, id)| v.entry(id).ok());
    let Some(view) = view else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no entry selected",
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            area,
        );
        return;
    };
    let lines = detail_lines(app, &view);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0)),
        area,
    );
}

fn detail_lines(app: &App, view: &EntryView) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        label("Title"),
        Span::styled(
            view.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        label("Username"),
        Span::raw(view.username.clone().unwrap_or_default()),
    ]));
    let password = match (&view.password, app.reveal) {
        (Some(p), true) => Span::styled(
            p.expose_secret().to_string(),
            Style::default().fg(Color::Yellow),
        ),
        (Some(_), false) => Span::raw(MASK.to_string()),
        (None, _) => Span::styled("(none)", Style::default().fg(Color::DarkGray)),
    };
    lines.push(Line::from(vec![label("Password"), password]));
    lines.push(Line::from(vec![
        label("URL"),
        Span::raw(view.url.clone().unwrap_or_default()),
    ]));

    if let Some(code) = &app.otp {
        let width = 16usize;
        let filled = if code.period_secs == 0 {
            0
        } else {
            (code.valid_for_secs as usize * width) / code.period_secs as usize
        };
        let bar = format!(
            "{}{}",
            "█".repeat(filled.min(width)),
            "░".repeat(width - filled.min(width))
        );
        lines.push(Line::from(vec![
            label("TOTP"),
            Span::styled(
                code.code.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(bar, Style::default().fg(Color::Magenta)),
            Span::raw(format!(" {}s", code.valid_for_secs)),
        ]));
    }

    if let Some(notes) = &view.notes {
        lines.push(Line::from(""));
        lines.push(Line::from(label("Notes")));
        for line in notes.lines() {
            lines.push(Line::from(Span::raw(format!("  {line}"))));
        }
    }

    if !view.custom.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(label("Fields")));
        for (name, value) in &view.custom {
            let shown = match value {
                FieldValue::Plain(s) => Span::raw(s.clone()),
                FieldValue::Protected(s) if app.reveal => Span::styled(
                    s.expose_secret().to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                FieldValue::Protected(_) => Span::raw(MASK.to_string()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {name:<9}"), Style::default().fg(Color::DarkGray)),
                shown,
            ]));
        }
    }

    if !view.tags.is_empty() {
        lines.push(Line::from(vec![
            label("Tags"),
            Span::raw(view.tags.join(", ")),
        ]));
    }
    if !view.attachments.is_empty() {
        lines.push(Line::from(vec![
            label("Files"),
            Span::raw(view.attachments.join(", ")),
        ]));
    }

    lines.push(Line::from(""));
    for (name, when) in [
        ("Created", view.created),
        ("Modified", view.modified),
        ("Accessed", view.accessed),
    ] {
        if let Some(t) = when {
            lines.push(Line::from(vec![
                label(name),
                Span::styled(
                    t.format("%Y-%m-%d %H:%M").to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }
    let expiry = match view.expires {
        Some(t) => {
            let text = t.format("%Y-%m-%d").to_string();
            if view.expired {
                Span::styled(
                    format!("{text} EXPIRED"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(text)
            }
        }
        None => Span::styled("never", Style::default().fg(Color::DarkGray)),
    };
    lines.push(Line::from(vec![label("Expires"), expiry]));
    lines
}

// ----- status line ----------------------------------------------------------

fn hints(app: &App) -> &'static str {
    match app.mode {
        Mode::Locked => "Enter unlock · Ctrl-p external prompt · Esc quit",
        Mode::Search => "type to filter · Enter keep results · Esc clear",
        Mode::Form => "Tab field · Ctrl-s save · Ctrl-g generate · Alt-g options · Ctrl-r reveal · Ctrl-n/Ctrl-d field · Esc cancel",
        Mode::Prompt => "Enter confirm · Esc cancel",
        Mode::Picker => "j/k choose group · Enter move · Esc cancel",
        Mode::GenOptions => "h/l length · s specials · a ambiguous · Enter generate · Esc cancel",
        Mode::Help => "Esc close",
        Mode::Dialog => "choose an option",
        Mode::Browse => match app.focus {
            Focus::Tree => "j/k move · h/l fold · Enter entries · N new group · r rename · d delete · Tab pane · ? help",
            Focus::Entries => "j/k move · / search · y password · u user · U url · o otp · v reveal · n new · e edit · d delete · ? help",
            Focus::Detail => "j/k scroll · v reveal · y password · e edit · Tab pane · ? help",
        },
    }
}

fn draw_status(app: &App, frame: &mut Frame, area: Rect) {
    let message = app.message();
    let left = if message.is_empty() {
        Span::styled(hints(app), Style::default().fg(Color::DarkGray))
    } else if app.message_is_error {
        Span::styled(message.clone(), Style::default().fg(Color::Red))
    } else {
        Span::styled(message.clone(), Style::default().fg(Color::Green))
    };

    let mut right = String::new();
    if app.has_unsaved_changes() {
        right.push('*');
    }
    let read_only = app.opts.read_only || app.vault().map(|v| !v.can_save()).unwrap_or(false);
    if read_only && !app.is_locked() {
        if !right.is_empty() {
            right.push(' ');
        }
        right.push_str("[RO]");
    }

    let used = left.content.width() + right.width();
    let pad = (area.width as usize).saturating_sub(used + 1);
    let line = Line::from(vec![
        Span::raw(" "),
        left,
        Span::raw(" ".repeat(pad)),
        Span::styled(
            right,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ----- overlays -------------------------------------------------------------

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn overlay(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn draw_dialog(app: &App, frame: &mut Frame, area: Rect) {
    let Some(d) = app.dialog.as_ref() else { return };
    let width = d
        .body
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(30)
        .max(d.title.width() as u16)
        .max(40)
        + 4;
    let rect = popup(area, width.min(area.width), d.body.len() as u16 + 4);
    let inner = overlay(frame, rect, &d.title);
    let mut lines: Vec<Line> = d.body.iter().map(|b| Line::from(b.clone())).collect();
    lines.push(Line::from(""));
    let choices: Vec<Span> = d
        .choices
        .iter()
        .flat_map(|c| {
            vec![
                Span::styled(
                    format!("[{}]", c.key),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}   ", c.label)),
            ]
        })
        .collect();
    lines.push(Line::from(choices));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_prompt(app: &App, frame: &mut Frame, area: Rect) {
    let Some(p) = app.prompt_box.as_ref() else {
        return;
    };
    let rect = popup(area, 50, 5);
    let inner = overlay(frame, rect, &p.title);
    let text = with_cursor(&p.input.value, p.input.cursor);
    frame.render_widget(Paragraph::new(Line::from(text)), inner);
}

fn draw_picker(app: &App, frame: &mut Frame, area: Rect) {
    let Some(p) = app.picker.as_ref() else { return };
    let height = (p.groups.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let rect = popup(area, 52, height);
    let inner = overlay(frame, rect, &format!("Move {}", p.label));
    let items: Vec<ListItem> = p
        .groups
        .iter()
        .map(|(_, depth, name)| ListItem::new(format!("{}{name}", "  ".repeat(*depth))))
        .collect();
    let list = List::new(items).highlight_style(selection_style(true));
    let mut state = ListState::default().with_selected(Some(p.selected));
    frame.render_stateful_widget(list, inner, &mut state);
}

fn draw_gen(app: &App, frame: &mut Frame, area: Rect) {
    let Some(g) = app.gen.as_ref() else { return };
    let rect = popup(area, 46, 7);
    let inner = overlay(frame, rect, "Generate password");
    let on = |b: bool| if b { "on" } else { "off" };
    let lines = vec![
        Line::from(format!("Length              {}", g.length)),
        Line::from(format!("Special characters  {}", on(g.special))),
        Line::from(format!("Exclude ambiguous   {}", on(g.exclude_ambiguous))),
        Line::from(""),
        Line::from(Span::styled(
            "h/l length · s specials · a ambiguous · Enter",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn with_cursor(value: &str, cursor: usize) -> String {
    let at = value
        .char_indices()
        .nth(cursor)
        .map(|(b, _)| b)
        .unwrap_or(value.len());
    format!("{}▏{}", &value[..at], &value[at..])
}

fn draw_form(app: &App, frame: &mut Frame, area: Rect) {
    let Some(form) = app.form.as_ref() else {
        return;
    };
    let title = if form.target.is_some() {
        "Edit entry"
    } else {
        "New entry"
    };
    let rows: usize = form
        .fields
        .iter()
        .map(|f| f.input.value.matches('\n').count() + 1)
        .sum();
    let rect = popup(
        area,
        area.width.saturating_sub(6).min(76),
        area.height.saturating_sub(2).min(rows as u16 + 6),
    );
    let inner = overlay(frame, rect, title);

    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let masked = match field.kind {
            FieldKind::Secret => !form.reveal,
            FieldKind::CustomValue { protected } => protected && !form.reveal,
            _ => false,
        };
        let raw = &field.input.value;
        let shown = if masked {
            "•".repeat(raw.chars().count())
        } else if focused {
            with_cursor(raw, field.input.cursor)
        } else {
            raw.clone()
        };
        let name = if i >= FIXED && field.kind == FieldKind::CustomName {
            "Field".to_string()
        } else if i >= FIXED {
            "  value".to_string()
        } else {
            field.label.clone()
        };
        let marker = if focused { "› " } else { "  " };
        let label_style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut parts = shown.split('\n');
        let first = parts.next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled(marker, label_style),
            Span::styled(format!("{name:<10}"), label_style),
            Span::raw(first.to_string()),
        ]));
        for rest in parts {
            lines.push(Line::from(Span::raw(format!("            {rest}"))));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Ctrl-s save · Ctrl-g generate · Ctrl-r reveal · Ctrl-n/Ctrl-d field · Esc",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let rows = [
        ("Tab / Shift-Tab", "cycle panes"),
        ("j k ↑ ↓", "move · h l ← → fold the tree"),
        ("Enter", "tree → entries → detail"),
        ("/", "fuzzy search, Esc clears"),
        ("y or p", "copy the password"),
        ("u / U / o", "copy username / URL / TOTP"),
        ("x", "clear the clipboard"),
        ("v", "reveal the password for this entry"),
        ("n / e", "new / edit entry"),
        ("d / D", "delete (recycle bin) / delete for good"),
        ("m", "move entry or group"),
        ("r / N", "rename group / new group"),
        ("s", "save the vault"),
        ("L", "lock now"),
        ("? / q", "this help / quit"),
        ("Ctrl-c", "quit without saving"),
        ("in the form", "Ctrl-s save, Ctrl-g generate, Alt-g options"),
    ];
    let rect = popup(area, 62, rows.len() as u16 + 2);
    let inner = overlay(frame, rect, "Keys");
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(
                    format!("{k:<16}"),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(*v),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn draw_unlock(app: &App, frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let rect = popup(area, 52, 9);
    let inner = overlay(frame, rect, "Locked");
    let dots = "•".repeat(app.unlock.input.value.chars().count());
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} is locked.", app.file_label),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Password  ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{dots}▏")),
        ]),
        Line::from(""),
    ];
    if let Some(err) = &app.unlock.error {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        inner,
    );
}
