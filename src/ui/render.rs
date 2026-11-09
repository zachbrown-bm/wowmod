//! Drawing the two-panel layout.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};

use super::browser::Kind;
use super::{App, Focus, Modal};
use crate::addon;
use crate::flavor::interface_to_version;
use crate::keymap::{Action, Keymap};

const ACCENT: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

fn panel(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(DIM)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default()
            },
        ))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    draw_left(f, app, cols[0]);
    draw_right(f, app, cols[1]);
    draw_status(f, app, rows[1]);

    if app.modal.is_some() {
        draw_modal(f, app, area);
    }
}

fn draw_left(f: &mut Frame, app: &mut App, area: Rect) {
    let preview_height = preview_lines(app).len() as u16 + 2;
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(preview_height.clamp(4, area.height / 2)),
        ])
        .split(area);

    let focused = app.focus == Focus::Browser;
    let title = format!("1 Browse  {}", app.browser.cwd.display());
    let items: Vec<ListItem> = app
        .browser
        .entries
        .iter()
        .map(|e| {
            let (tag, style) = match e.kind {
                Kind::Parent => ("     ", Style::default().fg(DIM)),
                Kind::Dir => ("dir  ", Style::default().fg(Color::Blue)),
                Kind::AddonDir => ("addon", Style::default().fg(Color::Green)),
                Kind::Zip => ("zip  ", Style::default().fg(Color::Magenta)),
                Kind::File => ("     ", Style::default().fg(DIM)),
            };
            let name = if matches!(e.kind, Kind::Dir | Kind::AddonDir) {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{tag} "), style),
                Span::raw(name),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(panel(&title, focused))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, parts[0], &mut app.browser.state);
    if let Some(err) = &app.browser.error {
        let p = Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red));
        let inner = Rect {
            x: parts[0].x + 2,
            y: parts[0].y + 1,
            width: parts[0].width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(p, inner);
    }

    let lines = preview_lines(app);
    let preview = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM))
                .title(" Preview "),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(preview, parts[1]);
}

fn preview_lines(app: &App) -> Vec<Line<'static>> {
    let Some(entry) = app.browser.selected() else {
        return vec![Line::from("Nothing selected.").fg(DIM)];
    };
    if !entry.is_installable() {
        return vec![Line::from("Select a .zip or a folder.").fg(DIM)];
    }
    if app.preview_pending {
        return vec![Line::from(format!("Analysing {} ...", entry.name)).fg(DIM)];
    }
    let Some(p) = &app.preview else {
        return vec![Line::from("").fg(DIM)];
    };
    match &p.result {
        Err(e) => vec![Line::from(e.clone()).fg(Color::Red)],
        Ok(pkg) => {
            let mut out = Vec::new();
            let targets = app.targets();
            out.push(Line::from(vec![
                Span::styled(
                    format!("{} addon folder(s)", pkg.roots.len()),
                    Style::default().bold(),
                ),
                Span::styled(
                    format!(
                        "  -> {}",
                        if targets.is_empty() {
                            "no install selected".to_string()
                        } else {
                            targets
                                .iter()
                                .map(|t| t.label())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ),
                    Style::default().fg(DIM),
                ),
            ]));
            for r in &pkg.roots {
                let title = r
                    .toc
                    .title
                    .clone()
                    .filter(|t| t != &r.name)
                    .map(|t| format!("  {t}"))
                    .unwrap_or_default();
                let ver = r
                    .toc
                    .version
                    .clone()
                    .map(|v| format!("  v{v}"))
                    .unwrap_or_default();
                let exists = targets
                    .iter()
                    .any(|t| t.addons_dir().join(&r.name).exists());
                let mut spans = vec![
                    Span::styled(
                        if exists { "replace " } else { "add     " },
                        Style::default().fg(if exists { ACCENT } else { Color::Green }),
                    ),
                    Span::styled(r.name.clone(), Style::default().bold()),
                    Span::raw(title),
                    Span::styled(ver, Style::default().fg(DIM)),
                ];
                if !r.toc.interface.is_empty() {
                    let iface = r
                        .toc
                        .interface
                        .iter()
                        .map(|i| interface_to_version(*i))
                        .collect::<Vec<_>>()
                        .join(", ");
                    spans.push(Span::styled(
                        format!("  [{iface}]"),
                        Style::default().fg(DIM),
                    ));
                }
                out.push(Line::from(spans));
                for t in &targets {
                    if r.toc.supports(&t.flavor) == Some(false) {
                        out.push(
                            Line::from(format!(
                                "  ! no Interface version for {}",
                                t.flavor.display_name()
                            ))
                            .fg(Color::Red),
                        );
                    }
                }
            }
            out
        }
    }
}

fn draw_right(f: &mut Frame, app: &mut App, area: Rect) {
    let installs_height = (app.installs.len().max(1) as u16 + 2)
        .min(area.height / 3)
        .max(3);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(installs_height), Constraint::Min(3)])
        .split(area);

    // Installs
    let focused = app.focus == Focus::Installs;
    let items: Vec<ListItem> = if app.installs.is_empty() {
        vec![ListItem::new(
            Line::from("No installs found. Press a to add one.").fg(DIM),
        )]
    } else {
        app.installs
            .iter()
            .enumerate()
            .map(|(i, inst)| {
                let mark = if app.marked.contains(&i) {
                    "[x] "
                } else {
                    "[ ] "
                };
                ListItem::new(Line::from(vec![
                    Span::styled(mark, Style::default().fg(ACCENT)),
                    Span::styled(
                        format!("{:<24}", inst.flavor.display_name()),
                        Style::default().bold(),
                    ),
                    Span::styled(inst.path.display().to_string(), Style::default().fg(DIM)),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(panel(
            "2 WoW installs  (Space marks extra targets)",
            focused,
        ))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, parts[0], &mut app.install_state);

    // Installed addons
    let focused = app.focus == Focus::Addons;
    let title = match app.current_install() {
        Some(i) => format!("3 Installed in {}  ({})", i.label(), app.addons.len()),
        None => "3 Installed".to_string(),
    };
    let installed = addon::folder_name_set(&app.addons);
    let flavor = app.current_flavor().cloned();
    let width = parts[1].width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .addons
        .iter()
        .map(|a| {
            let title = a.title();
            let ver = a.version();
            let en = a.primary.enabled;
            let mut flags = Vec::new();
            if !a.members.is_empty() {
                flags.push(format!("+{}", a.members.len()));
            }
            if a.primary.toc.is_none() {
                flags.push("no toc".to_string());
            }
            if let Some(fl) = &flavor
                && a.supports(fl) == Some(false)
            {
                flags.push("wrong flavor".to_string());
            }
            let missing = a.missing_deps(&installed);
            if !missing.is_empty() {
                flags.push(format!("missing {}", missing.join(",")));
            }
            let en_style = match en {
                addon::EnableState::Disabled => Style::default().fg(Color::Red),
                addon::EnableState::Mixed => Style::default().fg(ACCENT),
                _ => Style::default().fg(DIM),
            };
            let flags_text = if flags.is_empty() {
                String::new()
            } else {
                format!(" {}", flags.join(" "))
            };
            let name_w = width.saturating_sub(14 + 6 + flags_text.len()).clamp(8, 40);
            let mut line = vec![
                Span::styled(format!("{:<name_w$.name_w$}", title), Style::default()),
                Span::styled(format!(" {:<13.13}", ver), Style::default().fg(DIM)),
                Span::styled(format!(" {:<5}", en.short()), en_style),
            ];
            if !flags_text.is_empty() {
                let has_warn = flags
                    .iter()
                    .any(|f| f.starts_with("wrong") || f.starts_with("missing") || f == "no toc");
                line.push(Span::styled(
                    flags_text,
                    if has_warn {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(DIM)
                    },
                ));
            }
            ListItem::new(Line::from(line))
        })
        .collect();
    let list = List::new(items)
        .block(panel(&title, focused))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list, parts[1], &mut app.addon_state);
    if app.addons.is_empty() {
        let msg = if app.current_install().is_some() {
            "No addons installed."
        } else {
            ""
        };
        let inner = Rect {
            x: parts[1].x + 2,
            y: parts[1].y + 1,
            width: parts[1].width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(Paragraph::new(msg).fg(DIM), inner);
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let k = |a: Action| app.keys.short(a);
    let keys = match app.focus {
        Focus::Browser => format!(
            "{}/{} install  {} up  {} open  {} go to  {} filter",
            k(Action::Select),
            k(Action::Install),
            k(Action::Back),
            k(Action::Forward),
            k(Action::GoTo),
            k(Action::ToggleFilter)
        ),
        Focus::Installs => format!(
            "{} mark  {} addons  {} uninstall  {} rollback",
            k(Action::Mark),
            k(Action::Select),
            k(Action::Uninstall),
            k(Action::Rollback)
        ),
        Focus::Addons => format!(
            "{} details  {} uninstall  {} rollback",
            k(Action::Select),
            k(Action::Uninstall),
            k(Action::Rollback)
        ),
    };
    let backups = if app.config.backups {
        "backups on"
    } else {
        "backups OFF"
    };
    let left = if app.status.is_empty() {
        keys
    } else {
        app.status.clone()
    };
    let right = format!(
        " {} panels  {} {backups}  {} help  {} quit ",
        k(Action::NextPanel),
        k(Action::ToggleBackups),
        k(Action::Help),
        k(Action::Quit)
    );
    let width = area.width as usize;
    let left_w = width.saturating_sub(right.len());
    let line = Line::from(vec![
        Span::styled(
            format!("{:<left_w$.left_w$}", left),
            Style::default().fg(if app.status.is_empty() {
                DIM
            } else {
                Color::White
            }),
        ),
        Span::styled(right, Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else { return };
    let km = &app.keys;
    let mut scroll = 0;
    let (title, lines, footer): (String, Vec<Line>, String) = match modal {
        Modal::Help { scroll: s } => {
            scroll = *s;
            (
                "Help".into(),
                help_lines(km).into_iter().map(Line::from).collect(),
                format!(
                    " {}/{} scroll    any other key to close ",
                    km.short(Action::Up),
                    km.short(Action::Down)
                ),
            )
        }
        Modal::Message { title, lines } => (
            title.clone(),
            lines.iter().map(|l| Line::from(l.clone())).collect(),
            " any key to close ".into(),
        ),
        Modal::Confirm { title, lines, .. } => (
            title.clone(),
            lines
                .iter()
                .map(|l| {
                    let line = Line::from(l.clone());
                    if l.trim_start().starts_with('!') {
                        line.fg(Color::Red)
                    } else {
                        line
                    }
                })
                .collect(),
            format!(
                " {} confirm    {} cancel ",
                km.label(Action::Confirm),
                km.label(Action::Cancel)
            ),
        ),
        Modal::Uninstall { lines, backup, .. } => {
            let mut out: Vec<Line> = lines.iter().map(|l| Line::from(l.clone())).collect();
            out.push(Line::from(""));
            out.push(Line::from(vec![
                Span::styled(
                    if *backup { "[x] " } else { "[ ] " },
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::raw("Back up folders to AddOns.wowmod-backup first"),
            ]));
            out.push(if *backup {
                Line::from("    Folders are moved to the backup folder and can be rolled back.")
                    .fg(DIM)
            } else {
                Line::from("    ! Folders are deleted permanently.").fg(Color::Red)
            });
            (
                "Uninstall?".into(),
                out,
                format!(
                    " {} toggle backup    {} uninstall    {} cancel ",
                    km.label(Action::Toggle),
                    km.label(Action::Confirm),
                    km.label(Action::Cancel)
                ),
            )
        }
        Modal::Input { title, value, .. } => (
            title.clone(),
            vec![
                Line::from(format!("{value}_")),
                Line::from(""),
                Line::from("Paste or type a path.").fg(DIM),
            ],
            " Enter ok    Esc cancel ".into(),
        ),
    };
    let width = lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(20)
        .max(title.len() + 4)
        .max(footer.len() + 2) as u16
        + 4;
    let height = lines.len() as u16 + 2;
    let rect = centered(area, width.min(area.width.saturating_sub(4)), height);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).bold(),
        ))
        .title_bottom(Line::from(footer).fg(DIM).alignment(Alignment::Right))
        .padding(Padding::horizontal(1));
    let max_scroll = (lines.len() as u16).saturating_sub(rect.height.saturating_sub(2));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(max_scroll), 0)),
        rect,
    );
}

/// The help screen, built from the live key bindings.
fn help_lines(km: &Keymap) -> Vec<String> {
    let mut out = Vec::new();
    match Keymap::path() {
        Some(p) => {
            out.push(format!("Keys are set in {}", p.display()));
            out.push("Edit the file and restart wowmod to apply changes.".into());
        }
        None => out.push("Default keys (no config directory found).".into()),
    }
    let sections: [(&str, &[Action]); 3] = [
        ("Global", Action::GLOBAL),
        ("Navigation inside the focused panel", Action::NAV),
        ("Dialogs", Action::MODAL),
    ];
    for (title, actions) in sections {
        out.push(String::new());
        out.push(title.to_string());
        for &a in actions {
            out.push(format!("  {:<18} {}", km.label(a), a.describe()));
        }
    }
    out.push(String::new());
    out.push(format!(
        "The uninstall dialog has a back-up-first checkbox preset from the {} setting.",
        km.short(Action::ToggleBackups)
    ));
    out.push(
        "Backups live in <flavor>\\Interface\\AddOns.wowmod-backup\\<Folder>-<timestamp>.".into(),
    );
    out.push("Zips are extracted to a temp folder; the addon root is the folder holding".into());
    out.push("<Name>.toc, even if the zip wraps it in another folder (GitHub style).".into());
    out
}
