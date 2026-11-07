use super::{App, Confirm, LogKind, Mode, Pane};
use haccerlink::calc::Screenshot;
use haccerlink::names;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Gauge, Paragraph, Row, Table, Wrap};

const ACCENT: Color = Color::Cyan;
const SELECTED: Color = Color::Yellow;

fn human(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1}M", n as f64 / (1024.0 * 1024.0))
    } else if n >= 10 * 1024 {
        format!("{}K", n / 1024)
    } else {
        n.to_string()
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.image_area = None;
    let [header, body, log, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(9),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);
    draw_local(f, app, left);
    draw_calc(f, app, right);
    draw_log(f, app, log);
    draw_footer(f, app, footer);

    match &app.mode {
        Mode::Normal => {}
        Mode::Confirm(c) => {
            let text = match c {
                Confirm::Delete(v) => {
                    let names: Vec<_> = v.iter().map(|v| v.name.as_str()).collect();
                    format!(
                        "Delete {} from the calculator?\n\n{}",
                        plural(v.len(), "variable"),
                        names.join(", ")
                    )
                }
                Confirm::SyncClock => "Set the calculator's clock to this computer's time?".into(),
            };
            popup(f, "Confirm", &format!("{text}\n\n[y] yes   [n] no"), 50, 9);
        }
        Mode::Rename { var, input } => {
            popup(
                f,
                "Rename",
                &format!(
                    "New name for {}:\n\n> {input}_\n\n[Enter] ok   [Esc] cancel",
                    var.name
                ),
                44,
                8,
            );
        }
        Mode::Screen | Mode::Remote => draw_screen_popup(f, app),
        Mode::Info => {
            let text = app.info.as_ref().map_or_else(
                || "no information".to_string(),
                |i| {
                    i.lines()
                        .iter()
                        .map(|(k, v)| format!("{k:<17} {v}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            );
            popup(
                f,
                "Calculator",
                &format!("{text}\n\n(any key to close)"),
                60,
                18,
            );
        }
        Mode::Help => popup(f, "Help", HELP, 66, 28),
    }
}

fn plural(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

const HELP: &str = "\
Tab / ← →     switch pane            ↑ ↓ j k   move
Space         mark / unmark          PgUp PgDn Home End

Left pane (computer)
  Enter       open folder / send file to calculator
  s           send marked (or current) files
  S           send to archive
  Backspace   parent folder          .   show all files

Right pane (calculator)
  Enter / g   receive into the current folder
  d / Del     delete                 a   archive <-> RAM
  r           rename                 x   run program

Anywhere
  p           screenshot (s saves PNG, r refreshes, k remote)
  K           remote control: type on the calculator, Esc to stop
              F1-F5 = Y= WINDOW ZOOM TRACE GRAPH, F6 MODE, F7 QUIT
              Backspace = DEL, Delete = CLEAR
  i           calculator info        t   set clock to PC time
  R / F5      refresh                c   reconnect
  l           toggle packet log      q   quit

(any key to close)";

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::styled(
        " haccerlink ",
        Style::new().bold().fg(Color::Black).bg(ACCENT),
    )];
    match &app.connected {
        Some((desc, info)) => {
            spans.push(Span::raw(format!(
                " {} · OS {} · ",
                info.model(),
                info.os_version.as_deref().unwrap_or("?")
            )));
            if let Some((ram, flash)) = app.mem {
                spans.push(Span::raw(format!(
                    "RAM {} free · Archive {} free · ",
                    human(ram),
                    human(flash)
                )));
            }
            spans.push(Span::styled(desc.clone(), Style::new().dim()));
        }
        None => spans.push(Span::styled(
            if app.opts.sim {
                " connecting to simulator…"
            } else {
                " not connected (c to connect)"
            },
            Style::new().fg(Color::Red),
        )),
    }
    f.render_widget(Line::from(spans), area);
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(ACCENT)
    } else {
        Style::new().dim()
    };
    Block::bordered().title(title).border_style(style)
}

fn draw_local(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Pane::Local;
    let block = pane_block(
        format!(
            " {} ",
            short_path(&app.local_dir, area.width.saturating_sub(6))
        ),
        focused,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [list_area, detail] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    let rows = app.local.iter().map(|e| {
        let marked = app.local_sel.contains(&e.path);
        let name = if e.is_dir {
            format!("{}/", e.name)
        } else {
            e.name.clone()
        };
        let style = if marked {
            Style::new().fg(SELECTED).bold()
        } else if e.is_dir {
            Style::new().fg(Color::Blue).bold()
        } else {
            Style::new()
        };
        Row::new(vec![
            Cell::from(if marked { "●" } else { " " }),
            Cell::from(name),
            Cell::from(if e.is_dir {
                String::new()
            } else {
                human(e.size)
            }),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(7),
        ],
    )
    .row_highlight_style(highlight(focused));
    f.render_stateful_widget(table, list_area, &mut app.local_state);

    if let Some((_, d)) = &app.local_detail {
        f.render_widget(Paragraph::new(d.clone()).style(Style::new().dim()), detail);
    }
}

/// `~`-relative path, cut from the left to fit `max` columns.
fn short_path(p: &std::path::Path, max: u16) -> String {
    let mut s = p.display().to_string();
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().into_owned();
        if !home.is_empty() && s.starts_with(&home) {
            s = format!("~{}", &s[home.len()..]);
        }
    }
    let n = s.chars().count();
    let max = max.max(4) as usize;
    if n > max {
        s = format!("…{}", s.chars().skip(n - max + 1).collect::<String>());
    }
    s
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::UNDERLINED)
    }
}

fn draw_calc(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Pane::Calc;
    let n_apps = app.vars.iter().filter(|v| v.is_app()).count();
    let title = format!(
        " Calculator: {} · {} ",
        plural(app.vars.len() - n_apps, "variable"),
        plural(n_apps, "app")
    );
    let block = pane_block(title, focused);
    let header = Row::new(["", "Name", "Type", "Size", "Where"]).style(Style::new().dim().bold());
    let rows = app.vars.iter().map(|v| {
        let marked = app.calc_sel.contains(&(v.name.clone(), v.type_id));
        let style = if marked {
            Style::new().fg(SELECTED).bold()
        } else {
            Style::new()
        };
        Row::new(vec![
            Cell::from(if marked { "●" } else { " " }),
            Cell::from(v.name.clone()),
            Cell::from(names::type_name(v.type_id)),
            Cell::from(v.size.to_string()),
            Cell::from(if v.archived {
                Span::styled("archive", Style::new().fg(Color::Magenta))
            } else {
                Span::raw("RAM")
            }),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(highlight(focused));
    f.render_stateful_widget(table, area, &mut app.calc_state);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.show_packets {
        " Packets (l: messages) "
    } else {
        " Messages (l: packets) "
    };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::new().dim());
    let inner = block.inner(area);
    let h = inner.height as usize;
    let lines: Vec<Line> = if app.show_packets {
        let skip = app.packets.len().saturating_sub(h);
        app.packets
            .iter()
            .skip(skip)
            .map(|s| {
                let style = if s.starts_with("PC") {
                    Style::new().fg(Color::Green)
                } else {
                    Style::new().fg(Color::Blue)
                };
                Line::styled(s.clone(), style)
            })
            .collect()
    } else {
        let skip = app.log.len().saturating_sub(h);
        app.log
            .iter()
            .skip(skip)
            .map(|(k, s)| match k {
                LogKind::Info => Line::raw(s.clone()),
                LogKind::Error => Line::styled(s.clone(), Style::new().fg(Color::Red)),
            })
            .collect()
    };
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Some(label) = &app.busy {
        if let Some((done, total)) = app.progress {
            let ratio = if total == 0 {
                1.0
            } else {
                (done as f64 / total as f64).clamp(0.0, 1.0)
            };
            let g = Gauge::default()
                .ratio(ratio)
                .label(format!(
                    "{label}: {} / {}",
                    human(done as u64),
                    human(total as u64)
                ))
                .gauge_style(Style::new().fg(ACCENT).bg(Color::Black));
            f.render_widget(g, area);
        } else {
            f.render_widget(
                Line::styled(format!(" ⏳ {label}…"), Style::new().fg(ACCENT)),
                area,
            );
        }
        return;
    }
    let keys = match app.focus {
        Pane::Local => {
            "Enter send/open · s send · S send to archive · Space mark · Tab calc · p screen · ? help · q quit"
        }
        Pane::Calc => {
            "Enter/g receive · d delete · a archive · r rename · Space mark · Tab files · p screen · ? help"
        }
    };
    f.render_widget(Line::styled(format!(" {keys}"), Style::new().dim()), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn popup(f: &mut Frame, title: &str, text: &str, w: u16, h: u16) {
    let area = centered(f.area(), w, h);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(ACCENT));
    f.render_widget(
        Paragraph::new(text.to_string())
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn draw_screen_popup(f: &mut Frame, app: &mut App) {
    let remote = matches!(app.mode, Mode::Remote);
    let full = f.area();
    // 320x240 at one pixel per column / two per row is 160 rows; fit what we can.
    let area = centered(
        full,
        full.width.saturating_sub(4).min(164),
        full.height.saturating_sub(2).min(124),
    );
    f.render_widget(Clear, area);
    let title = if remote {
        " Remote control — typing goes to the calculator · Esc stop "
    } else {
        " Screenshot · s save PNG · r refresh · k remote · Esc close "
    };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::new().fg(if remote { Color::Red } else { ACCENT }));
    let inner = block.inner(area);
    f.render_widget(block, area);
    match &app.screen {
        // The kitty image goes on top of blank cells after the frame is drawn.
        Some(_) if app.kitty.is_some() => app.image_area = Some(inner),
        Some(s) => render_screen(f, inner, s),
        None => f.render_widget(Paragraph::new("waiting for screenshot…"), inner),
    }
}

/// Draw the calculator screen with half blocks: each cell shows two pixels
/// (foreground = top, background = bottom), box-filtered when shrinking.
fn render_screen(f: &mut Frame, area: Rect, s: &Screenshot) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (sw, sh) = (s.width as f64, s.height as f64);
    let scale = (area.width as f64 / sw).min(area.height as f64 * 2.0 / sh);
    let ow = ((sw * scale) as u16).max(1);
    let oh = ((sh * scale) as u16).max(2);
    let x0 = area.x + (area.width - ow.min(area.width)) / 2;
    let y0 = area.y + (area.height - (oh / 2).min(area.height)) / 2;

    let sample = |px: u16, py: u16| -> Color {
        let sx0 = (px as f64 / scale) as usize;
        let sy0 = (py as f64 / scale) as usize;
        let sx1 = (((px + 1) as f64 / scale) as usize).clamp(sx0 + 1, s.width);
        let sy1 = (((py + 1) as f64 / scale) as usize).clamp(sy0 + 1, s.height);
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for y in sy0.min(s.height - 1)..sy1 {
            for x in sx0.min(s.width - 1)..sx1 {
                let (pr, pg, pb) = s.pixel(x, y);
                r += pr as u32;
                g += pg as u32;
                b += pb as u32;
                n += 1;
            }
        }
        let n = n.max(1);
        Color::Rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
    };

    let buf = f.buffer_mut();
    for cy in 0..(oh / 2).min(area.height) {
        for cx in 0..ow.min(area.width) {
            let top = sample(cx, cy * 2);
            let bottom = sample(cx, cy * 2 + 1);
            if let Some(cell) = buf.cell_mut((x0 + cx, y0 + cy)) {
                cell.set_char('▀').set_fg(top).set_bg(bottom);
            }
        }
    }
}
