//! Drawing. Every screen shares the frame, top to bottom:
//!
//!   header   brand · where you are (or workspace tabs) · scan status
//!   cards    repos · on disk · freeable at the chosen level · kept away
//!   body     Home: menu │ workspaces   Repos: table │ detail   Sources: list
//!   footer   keycaps for the current screen

use keepsake_core::{
    analysis::{RepoAnalysis, RepoStatus},
    fmt,
    plan::{self, Level},
    safety::{Severity, Verdict},
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Cell, Clear, HighlightSpacing, List, ListItem, Padding, Paragraph, Row,
        Table, Wrap,
    },
    Frame,
};

use super::{
    app::{App, Entry, MenuAction, Prompt, Screen, SourceKind, ToastKind},
    theme,
    widgets::{self, keycap},
};
use super::actions::{Action, Confirm};

pub fn draw(f: &mut Frame, app: &mut App) {
    let [header, _, cards, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    draw_cards(f, app, cards);

    match app.screen {
        Screen::Home => draw_home(f, app, body),
        Screen::Repos => draw_repos(f, app, body),
        Screen::Sources => draw_sources(f, app, body),
    }

    draw_footer(f, app, footer);
    draw_toast(f, app, body);
    let screen = f.area();
    if let Some(prompt) = &app.prompt {
        draw_prompt(f, prompt, screen);
    }
    if let Some(confirm) = &app.confirm {
        draw_confirm(f, confirm, app.tick, screen);
    }
    if app.show_help {
        draw_help(f, app, screen);
    }
}

fn draw_repos(f: &mut Frame, app: &mut App, body: Rect) {
    if body.width >= 112 {
        let [list, detail] =
            Layout::horizontal([Constraint::Percentage(57), Constraint::Percentage(43)])
                .spacing(1)
                .areas(body);
        draw_table(f, app, list);
        draw_detail(f, app, detail);
    } else {
        let [list, detail] = Layout::vertical([Constraint::Min(8), Constraint::Length(18)]).areas(body);
        draw_table(f, app, list);
        draw_detail(f, app, detail);
    }
}

// ── home (launch page) ──────────────────────────────────────────────────────

fn draw_home(f: &mut Frame, app: &mut App, body: Rect) {
    let wide = body.width >= 100;
    let [menu_area, side] = if wide {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .spacing(1)
            .areas(body)
    } else {
        // Three rows per option, plus the border and top padding.
        let height = app.menu_items().len() as u16 * 3 + 3;
        Layout::vertical([Constraint::Length(height), Constraint::Min(0)]).areas(body)
    };

    let items: Vec<ListItem<'static>> = app
        .menu_items()
        .iter()
        .map(|item| {
            let quiet = item.action == MenuAction::Quit;
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(format!(" {} ", item.key), theme::bold(theme::ORANGE)),
                    Span::raw("  "),
                    Span::styled(
                        item.title.to_string(),
                        if quiet { theme::muted() } else { theme::bold(theme::TEXT) },
                    ),
                ]),
                Line::from(Span::styled(
                    format!("     {}", app.menu_description(item.action)),
                    theme::faint(),
                )),
                Line::default(),
            ]))
        })
        .collect();

    let block = panel(vec![Span::styled(" Start here ", theme::bold(theme::TEXT))])
        .padding(Padding::new(1, 1, 1, 0));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(theme::SELECTION))
        .highlight_symbol("▍")
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, menu_area, &mut app.menu);

    if side.height > 2 {
        draw_workspace_list(f, app, side);
    }
}

fn draw_workspace_list(f: &mut Frame, app: &App, area: Rect) {
    let block = panel(vec![
        Span::styled(" Workspaces ", theme::bold(theme::TEXT)),
        Span::styled(format!("{} ", app.workspaces.len()), theme::faint()),
    ])
    .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    if app.workspaces.is_empty() {
        lines.push(Line::from(Span::styled("No .code-workspace files found yet.", theme::text())));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Press w to choose the folders where you keep them. \
             Code Keepsake remembers them for next time (so you don't have to).",
            theme::muted(),
        )));
    }
    for ws in &app.workspaces {
        let repos = app
            .entries
            .iter()
            .filter(|e| e.target.workspaces.contains(&ws.name))
            .count();
        lines.push(Line::from(vec![
            Span::styled("◆ ", theme::fg(theme::ACCENT)),
            Span::styled(ws.name.clone(), theme::bold(theme::TEXT)),
            Span::styled(
                format!("  {} folder{}", repos, if repos == 1 { "" } else { "s" }),
                theme::faint(),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", widgets::short_path(&ws.path, width.saturating_sub(2))),
            theme::faint(),
        )));
    }
    for warning in &app.load_warnings {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(format!("◆ {warning}"), theme::fg(theme::WARN))));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ── sources ─────────────────────────────────────────────────────────────────

fn draw_sources(f: &mut Frame, app: &mut App, body: Rect) {
    let [list_area, info_area] =
        Layout::vertical([Constraint::Min(6), Constraint::Length(7)]).areas(body);

    let items: Vec<ListItem<'static>> = app
        .sources
        .iter()
        .map(|s| {
            let (tag, tag_style) = match s.kind {
                SourceKind::Saved => ("saved", theme::fg(theme::KEPT)),
                SourceKind::Default => ("current folder, used until you add one", theme::muted()),
                SourceKind::Launch => ("this launch only", theme::muted()),
            };
            let found = if !s.exists {
                Span::styled("not found on disk", theme::fg(theme::BAD))
            } else {
                Span::styled(
                    format!("{} workspace file{}", s.files, if s.files == 1 { "" } else { "s" }),
                    if s.files == 0 { theme::fg(theme::WARN) } else { theme::text() },
                )
            };
            ListItem::new(Text::from(vec![
                Line::from(Span::styled(s.path.display().to_string(), theme::bold(theme::TEXT))),
                Line::from(vec![
                    found,
                    Span::styled("  ·  ", theme::faint()),
                    Span::styled(tag, tag_style),
                ]),
                Line::default(),
            ]))
        })
        .collect();

    let block = panel(vec![
        Span::styled(" Workspace sources ", theme::bold(theme::TEXT)),
        Span::styled(format!("{} ", app.sources.len()), theme::faint()),
    ])
    .padding(Padding::new(1, 1, 1, 0));
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(theme::SELECTION))
        .highlight_symbol("▍")
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, list_area, &mut app.source_list);

    let config_path = keepsake_core::config::Config::path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "nowhere (no config folder found)".into());
    let info = vec![
        Line::from(Span::styled(
            format!(
                "Each source is a folder searched {} levels deep for .code-workspace files, or a single workspace file.",
                app.config.search_depth
            ),
            theme::muted(),
        )),
        Line::from(Span::styled(
            "Your workspace files are never changed. They're only read to find repos.",
            theme::muted(),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled("Saved in ", theme::faint()),
            Span::styled(config_path, theme::faint()),
        ]),
    ];
    let block = panel(vec![]).padding(Padding::new(2, 2, 1, 0));
    f.render_widget(Paragraph::new(info).wrap(Wrap { trim: false }).block(block), info_area);
}

// ── prompt ──────────────────────────────────────────────────────────────────

fn draw_prompt(f: &mut Frame, prompt: &Prompt, screen: Rect) {
    let area = widgets::centered(screen, 76, 9);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(theme::fg(theme::ACCENT))
        .title(Line::from(Span::styled(
            format!(" {} ", prompt.purpose.title()),
            theme::bold(theme::ACCENT),
        )))
        .padding(Padding::new(2, 2, 1, 0));
    let inner_width = area.width.saturating_sub(8) as usize;
    let shown = if prompt.input.chars().count() > inner_width {
        let skip = prompt.input.chars().count() - inner_width;
        format!("…{}", prompt.input.chars().skip(skip + 1).collect::<String>())
    } else {
        prompt.input.clone()
    };
    let lines = vec![
        Line::from(Span::styled(prompt.purpose.hint(), theme::muted())),
        Line::default(),
        Line::from(vec![
            Span::styled("› ", theme::bold(theme::ACCENT)),
            Span::styled(shown, theme::text()),
            Span::styled("▏", theme::fg(theme::ACCENT)),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled("enter", theme::muted()),
            Span::styled(" confirm   ", theme::faint()),
            Span::styled("esc", theme::muted()),
            Span::styled(" cancel   ", theme::faint()),
            Span::styled("quotes from \"Copy as path\" are fine", theme::faint()),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ── confirm ─────────────────────────────────────────────────────────────────

fn draw_confirm(f: &mut Frame, confirm: &Confirm, tick: u64, screen: Rect) {
    let area = widgets::centered(screen, 78, 22);
    f.render_widget(Clear, area);
    let color = if confirm.action.needs_name() { theme::WARN } else { theme::ACCENT };
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(theme::fg(color))
        .title(Line::from(Span::styled(
            format!(" {} {} ", confirm.action.label(), confirm.target.name),
            theme::bold(color),
        )))
        .padding(Padding::new(2, 2, 1, 0));
    let width = area.width.saturating_sub(6) as usize;
    let mut lines = vec![
        Line::from(Span::styled(widgets::short_path(&confirm.target.path, width), theme::faint())),
        Line::default(),
    ];
    let muted = |s: &str| Line::from(Span::styled(s.to_string(), theme::muted()));

    match (confirm.action, &confirm.preview) {
        (Action::Restore, _) => {
            lines.push(muted("Brings back .git (if it was removed) and every tracked file."));
            lines.push(muted("Existing files aren't overwritten. Build folders aren't rebuilt."));
        }
        (_, None) => lines.push(Line::from(vec![
            Span::styled(widgets::spinner(tick), theme::fg(theme::ACCENT)),
            Span::styled(" Checking what would happen…", theme::muted()),
        ])),
        (_, Some(Err(e))) => lines.push(Line::from(Span::styled(e.clone(), theme::fg(theme::BAD)))),
        (_, Some(Ok(report))) if !report.blocked.is_empty() => {
            lines.push(Line::from(Span::styled("Blocked. Nothing will be changed.", theme::bold(theme::BAD))));
            lines.extend(report.blocked.iter().map(|b| muted(&format!("• {b}"))));
        }
        (_, Some(Ok(report))) if report.actions.is_empty() => {
            lines.push(muted("Nothing to do: there are no build folders here."));
        }
        (action, Some(Ok(report))) => {
            lines.push(Line::from(Span::styled(
                format!("Frees {}", fmt::bytes(report.bytes_freed)),
                theme::bold(theme::OK),
            )));
            lines.extend(report.actions.iter().take(6).map(|a| muted(&format!("• {a}"))));
            if report.actions.len() > 6 {
                lines.push(muted(&format!("  and {} more", report.actions.len() - 6)));
            }
            lines.push(Line::default());
            match action {
                Action::PutAway(Level::Clean) => lines.push(muted("Source, history and settings are not touched.")),
                Action::PutAway(level) => {
                    lines.push(muted("Other ignored files (.env, local data) stay where they are."));
                    if level == Level::Full {
                        lines.push(muted("Local commits and stashes are bundled in .keepsake/."));
                        lines.push(muted("Commits only reachable from the reflog are not kept."));
                    }
                }
                Action::Restore => {}
            }
        }
    }

    lines.push(Line::default());
    if confirm.ready() {
        if confirm.action.needs_name() {
            lines.push(Line::from(vec![
                Span::styled("Type ", theme::muted()),
                Span::styled(confirm.target.name.clone(), theme::bold(theme::TEXT)),
                Span::styled(" and press enter  › ", theme::muted()),
                Span::styled(confirm.typed.clone(), theme::text()),
                Span::styled("▏", theme::fg(color)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("y", theme::bold(color)),
                Span::styled(" to confirm", theme::muted()),
            ]));
        }
    }
    lines.push(Line::from(Span::styled("esc to cancel", theme::faint())));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

fn panel(title: Vec<Span<'static>>) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(theme::fg(theme::BORDER))
        .style(Style::new().bg(theme::PANEL))
        .title(Line::from(title))
}

// ── header ──────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut left = vec![
        Span::styled(" ◆ ", theme::bold(theme::ACCENT)),
        Span::styled("code keepsake", theme::bold(theme::TEXT)),
    ];
    if app.demo.is_some() {
        left.push(Span::styled(" DEMO · safe to delete ", theme::bold(theme::WARN)));
    }
    left.push(Span::styled("   ", theme::faint()));
    let place = match app.screen {
        Screen::Home => Some("home"),
        Screen::Sources => Some("home › workspace sources"),
        Screen::Repos => None,
    };
    if let Some(place) = place {
        left.push(Span::styled(place, theme::muted()));
    }
    let tabs = if app.screen == Screen::Repos { app.tab_names() } else { Vec::new() };
    for (i, name) in tabs.iter().enumerate() {
        let count = app.tab_count(i);
        if i == app.tab {
            let depth = if i == 0 {
                String::new()
            } else {
                format!(" · depth {}", app.config.depth_for(name))
            };
            left.push(Span::styled(
                format!(" {name} {count}{depth} "),
                Style::new()
                    .fg(theme::TEXT)
                    .bg(theme::SELECTION)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            left.push(Span::styled(format!(" {name} "), theme::muted()));
            left.push(Span::styled(format!("{count} "), theme::faint()));
        }
        left.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);

    let status = if app.scan.running {
        vec![
            Span::styled(widgets::spinner(app.tick), theme::fg(theme::ACCENT)),
            Span::styled(
                format!(" scanning {}/{} ", app.scan.done, app.scan.total),
                theme::muted(),
            ),
        ]
    } else if let Some(took) = app.scan.took {
        vec![
            Span::styled("✓", theme::fg(theme::OK)),
            Span::styled(format!(" scanned in {:.1}s ", took.as_secs_f32()), theme::faint()),
        ]
    } else {
        Vec::new()
    };
    let mut right = status;
    if app.screen == Screen::Repos {
        right.push(Span::styled(" sorted by ", theme::faint()));
        right.push(Span::styled(format!("{} ", app.sort.label()), theme::muted()));
    }
    f.render_widget(Paragraph::new(Line::from(right).right_aligned()), area);
}

// ── cards ───────────────────────────────────────────────────────────────────

fn card(f: &mut Frame, area: Rect, label: &str, value: String, color: Color, sub: String) {
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(theme::fg(theme::BORDER))
        .style(Style::new().bg(theme::PANEL))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![
        Line::from(Span::styled(label.to_string(), theme::label())),
        Line::from(vec![
            Span::styled(value, theme::bold(color)),
            Span::styled(format!("  {sub}"), theme::faint()),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_cards(f: &mut Frame, app: &App, area: Rect) {
    let t = app.totals();
    let [a, b, c, d] = Layout::horizontal([Constraint::Fill(1); 4]).spacing(1).areas(area);

    let workspaces = if app.tab == 0 {
        format!("in {} workspaces", app.workspaces.len())
    } else {
        format!("in {}", app.tab_names()[app.tab])
    };
    card(f, a, "REPOS", t.repos.to_string(), theme::ACCENT, workspaces);
    card(
        f,
        b,
        "ON DISK",
        fmt::bytes(t.on_disk),
        theme::ORANGE,
        format!("{} is build output", fmt::bytes(t.build)),
    );
    card(
        f,
        c,
        &format!("FREEABLE · {}", app.level.label().to_uppercase()),
        fmt::bytes(t.freeable),
        theme::OK,
        format!("from {} repos", t.freeable_repos),
    );
    let (kept_value, kept_color, kept_sub) = if t.kept > 0 {
        (t.kept.to_string(), theme::KEPT, format!("{} saved", fmt::bytes(t.kept_saved)))
    } else if t.needs_attention > 0 {
        (t.needs_attention.to_string(), theme::WARN, "need a look".to_string())
    } else {
        ("0".to_string(), theme::MUTED, "nothing put away yet".to_string())
    };
    let kept_label = if t.kept == 0 && t.needs_attention > 0 { "ATTENTION" } else { "KEPT AWAY" };
    card(f, d, kept_label, kept_value, kept_color, kept_sub);
}

// ── table ───────────────────────────────────────────────────────────────────

fn idle_color(secs: i64) -> Color {
    match secs / 86_400 {
        d if d < 30 => theme::MUTED,
        d if d < 180 => theme::TEXT,
        _ => theme::ACCENT,
    }
}

fn status_cell(entry: &Entry, tick: u64) -> Span<'static> {
    let Some(a) = &entry.analysis else {
        return Span::styled(widgets::spinner(tick), theme::faint());
    };
    if entry.busy {
        return Span::styled(widgets::spinner(tick), theme::fg(theme::ACCENT));
    }
    if a.error.is_some() {
        return Span::styled("✕", theme::fg(theme::BAD));
    }
    match a.status {
        RepoStatus::Kept => Span::styled("◇", theme::fg(theme::KEPT)),
        RepoStatus::Missing => Span::styled("!", theme::bold(theme::BAD)),
        RepoStatus::NotGit => Span::styled("·", theme::faint()),
        RepoStatus::Active => match a.verdict() {
            Some(Verdict::Ready) => Span::styled("●", theme::fg(theme::OK)),
            Some(Verdict::Preserve(_)) => Span::styled("●", theme::fg(theme::WARN)),
            Some(Verdict::Blocked(_)) => Span::styled("●", theme::fg(theme::BAD)),
            None => Span::styled("●", theme::faint()),
        },
    }
}

fn two_lines(top: Line<'static>, bottom: Line<'static>) -> Cell<'static> {
    Cell::from(Text::from(vec![top, bottom]))
}

fn right(spans: Vec<Span<'static>>) -> Line<'static> {
    Line::from(spans).right_aligned()
}

fn repo_row(entry: &Entry, app: &App) -> Row<'static> {
    let name = Line::from(Span::styled(entry.target.name.clone(), theme::bold(theme::TEXT)));
    let path = Line::from(Span::styled(widgets::short_path(&entry.target.path, 46), theme::faint()));
    let empty = || Line::default();

    let cells = match &entry.analysis {
        None => vec![
            two_lines(empty(), empty()),
            two_lines(
                right(vec![Span::styled("measuring", theme::faint())]),
                right(vec![Span::styled("·".repeat(12), theme::fg(theme::BORDER))]),
            ),
            two_lines(empty(), empty()),
            two_lines(empty(), empty()),
        ],
        Some(a) => analysis_cells(a, entry.busy, app),
    };

    let mut row = vec![
        Cell::from(Text::from(vec![Line::from(status_cell(entry, app.tick))])),
        two_lines(name, path),
    ];
    row.extend(cells);
    Row::new(row).height(2).bottom_margin(1)
}

fn analysis_cells(a: &RepoAnalysis, busy: bool, app: &App) -> Vec<Cell<'static>> {
    let dim = |s: &str| Span::styled(s.to_string(), theme::faint());

    match a.status {
        RepoStatus::Kept => {
            let saved = a.stub.as_ref().map_or(0, |s| s.level.bytes_freed(&s.sizes_before));
            let level = a.stub.as_ref().map_or("", |s| s.level.label());
            vec![
                two_lines(
                    Line::from(Span::styled(
                        a.idle_secs(app.now).map(fmt::age_short).unwrap_or_default(),
                        theme::fg(theme::KEPT),
                    )),
                    Line::from(dim("put away")),
                ),
                two_lines(right(vec![dim("stub")]), right(vec![dim(level)])),
                two_lines(
                    right(vec![Span::styled(fmt::bytes(saved), theme::bold(theme::KEPT))]),
                    right(vec![dim("saved")]),
                ),
                two_lines(
                    Line::from(Span::styled("◇ kept", theme::fg(theme::KEPT))),
                    Line::from(dim("s to restore")),
                ),
            ]
        }
        RepoStatus::Missing => vec![
            two_lines(empty_line(), empty_line()),
            two_lines(right(vec![dim("—")]), empty_line()),
            two_lines(right(vec![dim("—")]), empty_line()),
            two_lines(
                Line::from(Span::styled("! missing", theme::fg(theme::BAD))),
                Line::from(dim("no folder on disk")),
            ),
        ],
        RepoStatus::NotGit => vec![
            two_lines(empty_line(), empty_line()),
            two_lines(right(vec![dim("—")]), empty_line()),
            two_lines(right(vec![dim("—")]), empty_line()),
            two_lines(Line::from(dim("not a repo")), Line::from(dim("skipped"))),
        ],
        RepoStatus::Active => {
            let idle = a.idle_secs(app.now);
            let branch = a
                .info
                .as_ref()
                .and_then(|i| i.head.as_ref())
                .map(|h| h.branch.clone().unwrap_or_else(|| format!("@{}", h.short_commit())))
                .unwrap_or_default();
            let idle_cell = two_lines(
                Line::from(Span::styled(
                    idle.map(fmt::age_short).unwrap_or_else(|| "—".into()),
                    theme::fg(idle.map_or(theme::FAINT, idle_color)),
                )),
                Line::from(dim(&fmt::middle_truncate(&branch, 9))),
            );

            let size_cell = match a.sizes {
                Some(s) => two_lines(
                    right(vec![Span::styled(fmt::bytes(s.total()), theme::text())]),
                    right(widgets::stacked_bar(&s, 12, "━")),
                ),
                None => two_lines(right(vec![dim("—")]), empty_line()),
            };

            let total = a.sizes.map_or(0, |s| s.total());
            let free_cell = match a.freed_at(app.level) {
                Some(freed) => {
                    let pct = if total == 0 { 0 } else { freed * 100 / total };
                    two_lines(
                        right(vec![Span::styled(fmt::bytes(freed), theme::bold(theme::ACCENT))]),
                        right(vec![dim(&format!("{pct}%"))]),
                    )
                }
                None => two_lines(right(vec![dim("—")]), right(vec![dim("blocked")])),
            };

            let safety_cell = if busy {
                two_lines(
                    Line::from(vec![
                        Span::styled(widgets::spinner(app.tick), theme::fg(theme::ACCENT)),
                        Span::styled(" checking", theme::muted()),
                    ]),
                    Line::from(dim("fetching")),
                )
            } else if let Some(error) = &a.error {
                two_lines(
                    Line::from(Span::styled("✕ error", theme::fg(theme::BAD))),
                    Line::from(dim(&fmt::middle_truncate(error, 14))),
                )
            } else {
                let (badge, color) = match a.verdict() {
                    Some(Verdict::Ready) => ("✓ ready".to_string(), theme::OK),
                    Some(Verdict::Preserve(n)) => (format!("◆ {n} to save"), theme::WARN),
                    Some(Verdict::Blocked(n)) => (format!("✕ {n} blocking"), theme::BAD),
                    None => ("—".to_string(), theme::FAINT),
                };
                let fetch = match &a.safety {
                    Some(s) if s.fetch_error.is_some() => Span::styled("fetch failed", theme::fg(theme::BAD)),
                    Some(s) if s.fetched => Span::styled("fetched now", theme::fg(theme::OK)),
                    Some(s) if !s.has_remote => dim("no remote"),
                    _ => dim("last fetch"),
                };
                two_lines(Line::from(Span::styled(badge, theme::fg(color))), Line::from(fetch))
            };

            vec![idle_cell, size_cell, free_cell, safety_cell]
        }
    }
}

fn empty_line() -> Line<'static> {
    Line::default()
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    let mut title = vec![
        Span::styled(" Repositories ", theme::bold(theme::TEXT)),
        Span::styled(format!("{} ", app.view.len()), theme::faint()),
    ];
    if !app.filter.is_empty() || app.filtering {
        title.push(Span::styled("· filter ", theme::faint()));
        title.push(Span::styled(app.filter.clone(), theme::fg(theme::ACCENT)));
        if app.filtering {
            title.push(Span::styled("▏", theme::fg(theme::ACCENT)));
        }
        title.push(Span::raw(" "));
    }
    let block = panel(title)
        .border_style(theme::fg(if app.filtering { theme::BORDER_FOCUS } else { theme::BORDER }))
        .padding(Padding::new(1, 1, 1, 0));

    if app.view.is_empty() {
        let message = if app.entries.is_empty() {
            "No repos yet. Press esc, then w to add workspace sources."
        } else {
            "No repos match. Press esc to clear the filter."
        };
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(message, theme::muted())).centered()),
            widgets::centered(inner, inner.width, 1),
        );
        return;
    }

    let header = Row::new(
        ["", "REPOSITORY", "IDLE", "SIZE", "FREES", "SAFETY"]
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let line = Line::from(Span::styled(h, theme::label()));
                Cell::from(if i == 3 || i == 4 { line.right_aligned() } else { line })
            }),
    )
    .bottom_margin(1);

    let shared: &App = app;
    let rows: Vec<Row<'static>> = shared
        .view
        .iter()
        .map(|&i| repo_row(&shared.entries[i], shared))
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(9),
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(14),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(2)
        .row_highlight_style(Style::new().bg(theme::SELECTION).fg(theme::ACCENT))
        .highlight_symbol("▍")
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(table, area, &mut app.table);
}

// ── detail ──────────────────────────────────────────────────────────────────

fn kv(key: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{key:<9}"), theme::faint())];
    spans.extend(value);
    Line::from(spans)
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let Some(entry) = app.selected() else {
        f.render_widget(panel(vec![]), area);
        return;
    };
    let block = panel(vec![Span::styled(
        format!(" {} ", entry.target.name),
        theme::bold(theme::ACCENT),
    )])
    .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;

    let mut lines = vec![
        Line::from(Span::styled(
            widgets::short_path(&entry.target.path, width),
            theme::faint(),
        )),
        Line::default(),
    ];

    match &entry.analysis {
        None => lines.push(Line::from(vec![
            Span::styled(widgets::spinner(app.tick), theme::fg(theme::ACCENT)),
            Span::styled(" Measuring and checking…", theme::muted()),
        ])),
        Some(a) => detail_lines(a, entry, app, width, &mut lines),
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn detail_lines(a: &RepoAnalysis, entry: &Entry, app: &App, width: usize, lines: &mut Vec<Line<'static>>) {
    let text = |s: String| Span::styled(s, theme::text());
    let muted = |s: String| Span::styled(s, theme::muted());

    if let Some(error) = &a.error {
        lines.push(Line::from(Span::styled("Couldn't read this repo", theme::bold(theme::BAD))));
        lines.push(Line::from(muted(error.clone())));
        return;
    }

    match a.status {
        RepoStatus::Missing => {
            lines.push(Line::from(Span::styled("Folder not found", theme::bold(theme::BAD))));
            lines.push(Line::default());
            lines.push(Line::from(muted(format!(
                "It's listed in {}, but nothing is on disk at this path. If you moved it, \
                 update the workspace file; if it was deleted, its remote isn't recorded anywhere \
                 Code Keepsake can see.",
                entry.target.workspaces.join(", ")
            ))));
            return;
        }
        RepoStatus::NotGit => {
            lines.push(Line::from(muted(
                "Not inside a git repository. Code Keepsake only puts away git repos.".into(),
            )));
            return;
        }
        RepoStatus::Kept => {
            if let Some(stub) = &a.stub {
                lines.push(kv(
                    "Kept",
                    vec![
                        Span::styled(stub.level.label().to_string(), theme::bold(theme::KEPT)),
                        muted(format!("  {}", fmt::age_long(app.now - stub.stored_at_unix))),
                    ],
                ));
                lines.push(kv("Saved", vec![text(fmt::bytes(stub.level.bytes_freed(&stub.sizes_before)))]));
                for r in stub.remotes.iter().take(2) {
                    lines.push(kv("Remote", vec![muted(format!("{} ", r.name)), text(r.fetch_url.clone())]));
                }
                lines.push(Line::default());
                lines.push(kv("Record", vec![text(keepsake_core::stub::STUB_FILE.into())]));
                if stub.level == Level::Full {
                    lines.push(kv("Extras", vec![text(format!("{}/", keepsake_core::stub::STUB_DIR))]));
                }
                lines.push(Line::default());
                lines.push(Line::from(muted("Press e to open the folder, s to restore.".into())));
                if let Some(head) = &stub.head {
                    lines.push(kv(
                        "Branch",
                        vec![text(head.branch.clone().unwrap_or_else(|| "(detached)".into()))],
                    ));
                }
            }
            return;
        }
        RepoStatus::Active => {}
    }

    // Identity
    if let Some(info) = &a.info {
        if info.remotes.is_empty() {
            lines.push(kv("Remote", vec![Span::styled("none".to_string(), theme::fg(theme::WARN))]));
        }
        for r in info.remotes.iter().take(2) {
            lines.push(kv("Remote", vec![muted(format!("{} ", r.name)), text(r.fetch_url.clone())]));
        }
        if let Some(head) = &info.head {
            let mut branch = vec![text(head.branch.clone().unwrap_or_else(|| "detached".into()))];
            match &info.upstream {
                Some(u) => branch.push(muted(format!(" → {u}"))),
                None if !head.detached => branch.push(Span::styled(" no upstream", theme::faint())),
                None => {}
            }
            lines.push(kv("Branch", branch));
            lines.push(kv(
                "Commit",
                vec![
                    Span::styled(head.short_commit().to_string(), theme::fg(theme::GIT)),
                    muted(format!(
                        "  {}",
                        a.idle_secs(app.now).map(fmt::age_long).unwrap_or_default()
                    )),
                ],
            ));
        }
    }
    if !a.workspaces.is_empty() {
        lines.push(kv("In", vec![muted(a.workspaces.join(", "))]));
    }
    if !entry.target.subfolders.is_empty() {
        let subs: Vec<String> = entry
            .target
            .subfolders
            .iter()
            .filter_map(|p| p.strip_prefix(&a.path).ok())
            .map(|p| p.display().to_string())
            .collect();
        lines.push(kv("Opens", vec![muted(subs.join(", "))]));
    }

    // Disk
    if let Some(sizes) = a.sizes {
        lines.push(Line::default());
        lines.push(widgets::section("DISK", Some(fmt::bytes(sizes.total())), width));
        lines.push(Line::from(widgets::stacked_bar(&sizes, width, "█")));
        let total = sizes.total().max(1);
        for (bytes, color, label) in widgets::categories(&sizes) {
            if bytes == 0 {
                continue;
            }
            lines.push(Line::from(vec![
                Span::styled("■ ", theme::fg(color)),
                Span::styled(format!("{label:<15}"), theme::muted()),
                Span::styled(format!("{:>9}", fmt::bytes(bytes)), theme::text()),
                Span::styled(format!("{:>6}", format!("{}%", bytes * 100 / total)), theme::faint()),
            ]));
        }
    }

    // Safety
    if let Some(safety) = &a.safety {
        lines.push(Line::default());
        let summary = match safety.verdict() {
            Verdict::Ready => Span::styled("ready", theme::fg(theme::OK)),
            Verdict::Preserve(n) => Span::styled(format!("{n} to save"), theme::fg(theme::WARN)),
            Verdict::Blocked(n) => Span::styled(format!("{n} blocking"), theme::fg(theme::BAD)),
        };
        let mut heading = widgets::section("SAFETY", None, width.saturating_sub(summary.content.len()));
        heading.spans.push(summary);
        lines.push(heading);

        let findings = safety.findings();
        if findings.iter().all(|f| f.severity == Severity::Note) {
            lines.push(Line::from(vec![
                Span::styled("✓ ", theme::fg(theme::OK)),
                Span::styled("Everything here is also on a remote", theme::text()),
            ]));
        }
        for finding in findings {
            let (icon, color) = match finding.severity {
                Severity::Blocker => ("✕ ", theme::BAD),
                Severity::Preserve => ("◆ ", theme::WARN),
                Severity::Note => ("· ", theme::FAINT),
            };
            let title_style = if finding.severity == Severity::Note { theme::muted() } else { theme::text() };
            lines.push(Line::from(vec![
                Span::styled(icon, theme::fg(color)),
                Span::styled(finding.title, title_style),
            ]));
            if let Some(detail) = finding.detail {
                lines.push(Line::from(Span::styled(format!("  {detail}"), theme::faint())));
            }
        }
    }

    // Put away
    if let Some(sizes) = a.sizes {
        lines.push(Line::default());
        lines.push(widgets::section("PUT AWAY", Some("1 2 3 to choose".into()), width));
        let total = sizes.total().max(1);
        let bar_width = width.saturating_sub(30).clamp(6, 24);
        for level in Level::ALL {
            let chosen = level == app.level;
            let p = plan::plan(level, &sizes, a.safety.as_ref());
            let blocked = !p.blocked_by.is_empty();
            let marker = if chosen { "› " } else { "  " };
            let label_style = if chosen { theme::bold(theme::ACCENT) } else { theme::text() };

            let mut spans = vec![
                Span::styled(marker, theme::bold(theme::ACCENT)),
                Span::styled(format!("{:<7}", level.label()), label_style),
            ];
            if blocked {
                spans.push(Span::styled(format!("{:<width$}", "blocked", width = bar_width + 1), theme::fg(theme::BAD)));
            } else {
                let color = if chosen { theme::ACCENT } else { theme::MUTED };
                spans.extend(widgets::ratio_bar(p.bytes_freed as f64 / total as f64, bar_width, color));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                format!("{:>9}", if blocked { String::new() } else { fmt::bytes(p.bytes_freed) }),
                if chosen { theme::bold(theme::TEXT) } else { theme::muted() },
            ));
            lines.push(Line::from(spans));

            if chosen {
                lines.push(Line::from(Span::styled(format!("         {}", level.tagline()), theme::faint())));
                let items = if blocked { &p.blocked_by } else { &p.steps };
                let (bullet, style) = if blocked {
                    ("✕", theme::fg(theme::BAD))
                } else {
                    ("·", theme::muted())
                };
                for item in items {
                    lines.push(Line::from(vec![
                        Span::styled(format!("         {bullet} "), style),
                        Span::styled(item.clone(), theme::muted()),
                    ]));
                }
            }
        }
    }
}

// ── footer, toast, help ─────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let keys: &[(&str, &str)] = match app.screen {
        Screen::Home => &[
            ("↑↓", "choose"),
            ("enter", "open"),
            (if app.demo.is_some() { "b c r f w e d x" } else { "b c r f w e d" }, "shortcuts"),
            ("1 2 3", "level"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Screen::Sources => &[
            ("↑↓", "choose"),
            ("a", "add"),
            ("d", "remove"),
            ("esc", "back"),
            ("?", "help"),
        ],
        Screen::Repos if app.filtering => &[("type", "to filter"), ("enter", "done"), ("esc", "clear")],
        Screen::Repos => &[
            ("↑↓", "move"),
            ("tab", "workspace"),
            ("p m", "depth"),
            ("1 2 3", "level"),
            ("s", "put away / restore"),
            ("e", "open folder"),
            ("f", "fetch"),
            ("o", "sort"),
            ("/", "filter"),
            ("esc", "home"),
            ("?", "help"),
        ],
    };
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in keys {
        spans.extend(keycap(key, label));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_toast(f: &mut Frame, app: &App, body: Rect) {
    let Some(toast) = &app.toast else { return };
    let (icon, color) = match toast.kind {
        ToastKind::Info => ("●", theme::ACCENT),
        ToastKind::Success => ("✓", theme::OK),
        ToastKind::Warn => ("◆", theme::WARN),
    };
    let width = (toast.text.chars().count() as u16 + 8).min(body.width.saturating_sub(4));
    if width < 10 || body.height < 4 {
        return;
    }
    let area = Rect {
        x: body.right().saturating_sub(width + 2),
        y: body.bottom().saturating_sub(4),
        width,
        height: 3,
    };
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(theme::fg(color))
        .padding(Padding::horizontal(1));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{icon} "), theme::fg(color)),
            Span::styled(toast.text.clone(), theme::text()),
        ]))
        .block(block),
        area,
    );
}

fn draw_help(f: &mut Frame, app: &App, screen: Rect) {
    let area = widgets::centered(screen, 66, 24);
    f.render_widget(Clear, area);
    let block = panel(vec![Span::styled(" Keys ", theme::bold(theme::ACCENT))]).padding(Padding::new(2, 2, 1, 1));

    let row = |key: &str, text: &str| {
        let mut spans = keycap(key, "");
        spans.pop();
        spans.push(Span::raw(" ".repeat(9usize.saturating_sub(key.chars().count() + 2))));
        spans.push(Span::styled(text.to_string(), theme::text()));
        Line::from(spans)
    };
    let mut lines = match app.screen {
        Screen::Home => vec![
            row("↑ ↓", "choose (also j k, tab)"),
            row("enter", "open the highlighted option"),
            row("letter", "each option's key opens it directly"),
            row("1 2 3", "level used for the freeable totals"),
            row("q", "quit"),
        ],
        Screen::Sources => vec![
            row("↑ ↓", "choose a source"),
            row("a", "add a folder or a .code-workspace file"),
            row("d", "remove the highlighted source"),
            row("esc", "back to home"),
        ],
        Screen::Repos => vec![
            row("↑ ↓", "move (also j k, PgUp PgDn, g G)"),
            row("tab", "next workspace (shift+tab, ← → also work)"),
            row("o", "change sort order"),
            row("/", "filter by name or path"),
            row("p m", "look deeper (p) or shallower (m) for repos inside this workspace's folders"),
            row("f", "fetch the selected repo and check it again"),
            row("r", "rescan everything"),
            row("1 2 3", "choose how far to put repos away"),
            row("s", "put the selected repo away, or restore it (also enter)"),
            row("e", "open the repo's folder in the file manager"),
            row("esc", "back to home"),
            row("q", "quit"),
        ],
    };
    lines.push(Line::default());
    lines.push(Line::from(Span::styled("LEVELS", theme::label())));
    for level in Level::ALL {
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", level.key()), theme::fg(theme::ACCENT)),
            Span::styled(format!("{:<8}", level.label()), theme::bold(theme::TEXT)),
            Span::styled(level.tagline().to_string(), theme::muted()),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled("Press any key to close", theme::faint())).centered());
    f.render_widget(Paragraph::new(lines).block(block), area);
}
