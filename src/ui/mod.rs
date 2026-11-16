//! Rendering. Every draw function also records clickable regions in `app.hits`.
use crate::app::{App, Focus, Hit, TeamRow, View, VIEWS};
use crate::backend::auth::DeviceCode;
use crate::input::Input;
use crate::model::*;
use chrono::{DateTime, Datelike, Local, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod conv;
pub mod imgview;
mod modals;
mod panes;

// Teams dark theme.
pub const TITLE: Color = Color::Rgb(11, 11, 11);
pub const RAIL: Color = Color::Rgb(11, 11, 11);
pub const LIST: Color = Color::Rgb(20, 20, 20);
pub const MAIN: Color = Color::Rgb(31, 31, 31);
pub const PANEL: Color = Color::Rgb(41, 41, 41);
pub const SEL: Color = Color::Rgb(61, 61, 61);
pub const BUBBLE: Color = Color::Rgb(46, 46, 46);
pub const OWN: Color = Color::Rgb(55, 56, 104);
pub const BRAND: Color = Color::Rgb(91, 95, 199);
pub const ACCENT: Color = Color::Rgb(127, 133, 245);
pub const FG: Color = Color::Rgb(240, 240, 240);
pub const DIM: Color = Color::Rgb(173, 173, 173);
pub const FAINT: Color = Color::Rgb(97, 97, 97);
pub const RED: Color = Color::Rgb(232, 73, 73);
pub const GREEN: Color = Color::Rgb(107, 183, 0);
pub const YELLOW: Color = Color::Rgb(252, 209, 22);

pub fn fg() -> Style {
    Style::new().fg(FG)
}
pub fn dim() -> Style {
    Style::new().fg(DIM)
}
pub fn faint() -> Style {
    Style::new().fg(FAINT)
}
pub fn accent() -> Style {
    Style::new().fg(ACCENT)
}
pub fn bold() -> Style {
    Style::new().fg(FG).add_modifier(Modifier::BOLD)
}

pub fn w(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate to `max` columns, ending with an ellipsis when cut.
pub fn trunc(s: &str, max: usize) -> String {
    if w(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if cur + cw + 1 > max {
            break;
        }
        out.push(c);
        cur += cw;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

pub fn pad(s: &str, width: usize) -> String {
    let t = trunc(s, width);
    let n = width.saturating_sub(w(&t));
    format!("{t}{}", " ".repeat(n))
}

/// Word-wrap text to `width` columns, preserving explicit newlines.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        let mut lw = 0;
        for word in para.split(' ') {
            let ww = w(word);
            if lw > 0 && lw + 1 + ww > width {
                out.push(std::mem::take(&mut line));
                lw = 0;
            }
            if lw > 0 {
                line.push(' ');
                lw += 1;
            }
            if ww <= width {
                line.push_str(word);
                lw += ww;
            } else {
                // Break a very long word (URLs) across lines.
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if lw + cw > width {
                        out.push(std::mem::take(&mut line));
                        lw = 0;
                    }
                    line.push(c);
                    lw += cw;
                }
            }
        }
        out.push(line);
    }
    out
}

pub fn initials(name: &str) -> String {
    let mut it = name.split_whitespace().filter_map(|p| p.chars().find(|c| c.is_alphanumeric()));
    let a = it.next();
    let b = it.next();
    match (a, b) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => format!("{} ", a.to_uppercase()),
        _ => "? ".into(),
    }
}

pub fn avatar_color(name: &str) -> Color {
    const P: [(u8, u8, u8); 8] = [(140, 80, 160), (0, 120, 140), (170, 80, 60), (60, 120, 70), (90, 95, 190), (160, 110, 30), (150, 60, 100), (50, 100, 160)];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = P[h as usize % P.len()];
    Color::Rgb(r, g, b)
}

/// A 4-column "avatar" with the person's initials.
pub fn avatar(name: &str) -> Span<'static> {
    Span::styled(format!(" {} ", initials(name)), Style::new().fg(Color::White).bg(avatar_color(name)).add_modifier(Modifier::BOLD))
}

pub fn presence_dot(a: Availability) -> Span<'static> {
    let (ch, color) = match a {
        Availability::Available => ("●", GREEN),
        Availability::Busy => ("●", RED),
        Availability::DoNotDisturb => ("⊖", RED),
        Availability::BeRightBack | Availability::Away => ("◐", YELLOW),
        Availability::Offline => ("○", DIM),
        Availability::Unknown => (" ", DIM),
    };
    Span::styled(ch, Style::new().fg(color))
}

/// Compact timestamp for lists: time today, weekday this week, date otherwise.
pub fn short_time(t: Option<DateTime<Utc>>) -> String {
    let Some(t) = t else { return String::new() };
    let l = t.with_timezone(&Local);
    let days = (Local::now().date_naive() - l.date_naive()).num_days();
    if days <= 0 {
        l.format("%-I:%M %p").to_string()
    } else if days < 7 {
        l.format("%a").to_string()
    } else {
        format!("{}/{}", l.month(), l.day())
    }
}

pub fn fill(f: &mut Frame, r: Rect, bg: Color) {
    let r = r.intersection(f.area());
    f.buffer_mut().set_style(r, Style::new().bg(bg));
}

/// Draw a line at (x, y), clipped to `clip`.
pub fn put(f: &mut Frame, clip: Rect, x: u16, y: u16, line: Line) {
    let clip = clip.intersection(f.area());
    if y < clip.y || y >= clip.bottom() || x < clip.x || x >= clip.right() {
        return;
    }
    f.buffer_mut().set_line(x, y, &line, clip.right() - x);
}

pub fn put_right(f: &mut Frame, clip: Rect, y: u16, line: Line) -> Rect {
    let lw = line.width() as u16;
    let x = clip.right().saturating_sub(lw).max(clip.x);
    put(f, clip, x, y, line);
    Rect::new(x, y, lw.min(clip.width), 1)
}

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let wd = width.min(area.width);
    let ht = height.min(area.height);
    Rect::new(area.x + (area.width - wd) / 2, area.y + (area.height - ht) / 2, wd, ht)
}

/// Render a single-line text field; returns the cursor position.
pub fn field(f: &mut Frame, area: Rect, input: &Input, placeholder: &str, style: Style) -> Position {
    let width = area.width as usize;
    if input.text.is_empty() {
        put(f, area, area.x, area.y, Line::styled(trunc(placeholder, width), faint()));
        return Position::new(area.x, area.y);
    }
    let before = &input.text[..input.cursor];
    let (shown, cx) = if w(before) < width {
        (trunc(&input.text, width.max(1)), w(before))
    } else {
        // Keep the cursor visible: show the tail of the text before it.
        let mut tail = String::new();
        let mut tw = 0;
        for c in before.chars().rev() {
            let cw = c.width().unwrap_or(0);
            if tw + cw >= width {
                break;
            }
            tail.insert(0, c);
            tw += cw;
        }
        (tail, tw)
    };
    put(f, area, area.x, area.y, Line::styled(shown, style));
    Position::new(area.x + cx as u16, area.y)
}

pub fn spinner(app: &App) -> &'static str {
    const S: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    S[app.spinner % S.len()]
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.hits.clear();
    app.graphics.begin();
    f.buffer_mut().set_style(area, Style::new().bg(MAIN).fg(FG));
    if area.width < 72 || area.height < 16 {
        let msg = Paragraph::new("TeamsTUI needs a terminal of at least 72×16.\nPlease enlarge the window.").alignment(Alignment::Center).style(dim());
        f.render_widget(msg, centered(area, area.width, 2));
        return;
    }
    let top = Rect::new(area.x, area.y, area.width, 1);
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    let rail_w = 10;
    let list_w = if area.width >= 110 { 36 } else { 28 };
    let rail = Rect::new(body.x, body.y, rail_w, body.height);
    let list = Rect::new(body.x + rail_w, body.y, list_w, body.height);
    let main = Rect::new(list.right(), body.y, body.width - rail_w - list_w, body.height);

    draw_top(f, top, app);
    draw_rail(f, rail, app);
    draw_list(f, list, app);
    match app.view {
        View::Chat | View::Teams => conv::draw(f, main, app),
        View::Activity => panes::activity(f, main, app),
        View::Calendar => panes::calendar(f, main, app),
        View::Calls => panes::calls(f, main, app),
        View::Files => panes::files(f, main, app),
    }
    draw_status(f, status, app);
    modals::draw(f, area, app);
}

fn draw_top(f: &mut Frame, area: Rect, app: &mut App) {
    fill(f, area, TITLE);
    put(f, area, area.x + 1, area.y, Line::from(vec![Span::styled(" T ", Style::new().bg(BRAND).fg(Color::White).add_modifier(Modifier::BOLD)), Span::styled("  ‹  ›", faint())]));
    let sw = (area.width * 2 / 5).clamp(24, 50);
    let search = Rect::new(area.x + (area.width - sw) / 2, area.y, sw, 1);
    fill(f, search, PANEL);
    put(f, search, search.x + 1, area.y, Line::styled("🔍 Search (Ctrl+E)", dim()));
    app.hits.push((search, Hit::Search));

    let name = if app.me.name.is_empty() { "…".to_string() } else { app.me.name.clone() };
    let mut spans = vec![presence_dot(app.my_presence), Span::styled(format!(" {}  ", app.my_presence.label()), dim())];
    if !app.status_msg.is_empty() {
        spans.push(Span::styled(format!("“{}”  ", trunc(&app.status_msg, 24)), faint()));
    }
    spans.push(avatar(&name));
    spans.push(Span::styled(format!(" {} ", trunc(&name, 22)), fg()));
    let clip = Rect::new(search.right() + 1, area.y, area.right().saturating_sub(search.right() + 1), 1);
    let mut line = Line::from(spans);
    if line.width() > clip.width as usize {
        // Not enough room: keep the presence dot and the name.
        line = Line::from(vec![presence_dot(app.my_presence), Span::raw(" "), Span::styled(format!("{} ", trunc(&name, clip.width.saturating_sub(3) as usize)), fg())]);
    }
    let r = put_right(f, clip, area.y, line);
    app.hits.push((r, Hit::Profile));
}

fn draw_rail(f: &mut Frame, area: Rect, app: &mut App) {
    fill(f, area, RAIL);
    let mut y = area.y + 1;
    for v in VIEWS {
        if y + 2 > area.bottom() {
            break;
        }
        let selected = v == app.view;
        let r = Rect::new(area.x, y, area.width, 2);
        let style = if selected { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { dim() };
        if selected {
            fill(f, r, Color::Rgb(30, 30, 30));
            put(f, r, r.x, y, Line::styled("▌", accent()));
            put(f, r, r.x, y + 1, Line::styled("▌", accent()));
        }
        let badge = match v {
            View::Chat => app.unread_chats(),
            View::Activity => app.activity.iter().filter(|a| a.unread).count(),
            _ => 0,
        };
        let mut icon = vec![Span::raw(v.icon())];
        if badge > 0 {
            icon.push(Span::styled(format!("{}", badge.min(99)), Style::new().fg(Color::White).bg(RED).add_modifier(Modifier::BOLD)));
        }
        let icon = Line::from(icon);
        let ix = r.x + (r.width.saturating_sub(icon.width() as u16)) / 2;
        put(f, r, ix, y, icon);
        let label = v.label();
        put(f, r, r.x + (r.width.saturating_sub(w(label) as u16)) / 2, y + 1, Line::styled(label, style));
        app.hits.push((r, Hit::Rail(v)));
        y += 3;
    }
    let help = Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1);
    if help.y > y {
        put(f, help, help.x + 2, help.y, Line::styled("? Help", faint()));
        app.hits.push((help, Hit::Help));
    }
}

struct Row {
    avatar: Span<'static>,
    dot: Option<Availability>,
    title: String,
    strong: bool,
    right: String,
    sub: String,
    badge: Option<Span<'static>>,
}

fn draw_rows(f: &mut Frame, area: Rect, rows: Vec<Row>, sel: usize, focused: bool, hits: &mut Vec<(Rect, Hit)>) {
    let per = 3u16;
    let visible = (area.height / per).max(1) as usize;
    let offset = if sel >= visible { sel + 1 - visible } else { 0 };
    for (i, row) in rows.into_iter().enumerate().skip(offset).take(visible) {
        let y = area.y + ((i - offset) as u16) * per;
        let r = Rect::new(area.x, y, area.width, 2);
        if i == sel {
            fill(f, r, if focused { SEL } else { PANEL });
            if focused {
                put(f, r, r.x, y, Line::styled("▎", accent()));
                put(f, r, r.x, y + 1, Line::styled("▎", accent()));
            }
        }
        let tw = area.width as usize;
        let right_w = w(&row.right);
        let title_w = tw.saturating_sub(9 + right_w + 1);
        let tstyle = if row.strong { Style::new().fg(Color::White).add_modifier(Modifier::BOLD) } else { fg() };
        let mut spans = vec![Span::raw(" "), row.avatar, Span::raw(" ")];
        spans.push(row.dot.map(presence_dot).unwrap_or(Span::raw(" ")));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(pad(&row.title, title_w), tstyle));
        spans.push(Span::styled(format!(" {} ", row.right), if row.strong { accent() } else { dim() }));
        put(f, r, r.x, y, Line::from(spans));
        let sub_w = tw.saturating_sub(8 + 3);
        let mut sub = vec![Span::raw("        "), Span::styled(pad(&row.sub.replace('\n', " "), sub_w), if row.strong { fg() } else { dim() })];
        if let Some(b) = row.badge {
            sub.push(Span::raw(" "));
            sub.push(b);
        }
        put(f, r, r.x, y + 1, Line::from(sub));
        hits.push((Rect::new(area.x, y, area.width, per.min(area.bottom() - y)), Hit::List(i)));
    }
}

fn unread_badge() -> Span<'static> {
    Span::styled("●", accent())
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    fill(f, area, LIST);
    app.hits.push((area, Hit::ListArea));
    let focused = app.focus == Focus::List && matches!(app.modal, crate::app::Modal::None);
    let title = match app.view {
        View::Files => "OneDrive",
        v => v.label(),
    };
    put(f, area, area.x + 2, area.y + 1, Line::styled(title, bold().add_modifier(Modifier::BOLD)));
    let action = match app.view {
        View::Chat | View::Activity | View::Calls => Some("✎ New chat"),
        View::Calendar => Some("＋ New meeting"),
        _ => None,
    };
    if let Some(a) = action {
        let clip = Rect::new(area.x, area.y + 1, area.width - 1, 1);
        let r = put_right(f, clip, area.y + 1, Line::styled(a, accent()));
        app.hits.push((r, Hit::NewItem));
    }
    let body = Rect::new(area.x, area.y + 3, area.width, area.height.saturating_sub(3));
    let me = app.me.id.clone();
    let mut hits = vec![];
    let empty = |f: &mut Frame, text: &str| {
        f.render_widget(Paragraph::new(text.to_string()).style(dim()).wrap(Wrap { trim: true }), Rect::new(body.x + 2, body.y, body.width.saturating_sub(4), body.height.min(4)));
    };
    match app.view {
        View::Chat => {
            if !app.chats_loaded {
                empty(f, &format!("{} Loading chats…", spinner(app)));
            } else if app.chats.is_empty() {
                empty(f, "No chats yet. Press Ctrl+N to start one.");
            }
            let rows = app
                .chats
                .iter()
                .map(|c| {
                    let title = c.title(&me);
                    let who = if c.preview_from.id == me {
                        "You: ".to_string()
                    } else if c.kind != ChatKind::OneOnOne && !c.preview_from.name.is_empty() {
                        format!("{}: ", c.preview_from.name.split(' ').next().unwrap_or(""))
                    } else {
                        String::new()
                    };
                    Row {
                        avatar: match c.kind {
                            ChatKind::Meeting => Span::styled(" 📅 ", Style::new().bg(BRAND)),
                            _ => avatar(&title),
                        },
                        dot: (c.kind == ChatKind::OneOnOne).then(|| c.others(&me).next().and_then(|u| app.presence.get(&u.id).copied()).unwrap_or_default()),
                        title,
                        strong: c.unread,
                        right: short_time(c.last_time),
                        sub: format!("{who}{}", c.preview),
                        badge: c.unread.then(unread_badge),
                    }
                })
                .collect();
            draw_rows(f, body, rows, app.chat_sel, focused, &mut hits);
        }
        View::Activity => {
            if app.activity.is_empty() {
                empty(f, if app.chats_loaded { "You're all caught up. Mentions and new messages show up here." } else { "Loading…" });
            }
            let rows = app
                .activity
                .iter()
                .map(|a| Row {
                    avatar: avatar(&a.title),
                    dot: None,
                    title: a.title.clone(),
                    strong: a.unread,
                    right: short_time(a.when),
                    sub: a.preview.clone(),
                    badge: if a.mention { Some(Span::styled("@", Style::new().fg(RED).add_modifier(Modifier::BOLD))) } else { a.unread.then(unread_badge) },
                })
                .collect();
            draw_rows(f, body, rows, app.act_sel, focused, &mut hits);
        }
        View::Calls => {
            let contacts = app.contacts();
            if contacts.is_empty() {
                empty(f, "People you chat with appear here as speed dial.");
            }
            let rows = contacts
                .iter()
                .map(|u| {
                    let a = app.presence.get(&u.id).copied().unwrap_or_default();
                    Row { avatar: avatar(&u.name), dot: Some(a), title: u.name.clone(), strong: false, right: String::new(), sub: a.status().to_string(), badge: None }
                })
                .collect();
            draw_rows(f, body, rows, app.call_sel, focused, &mut hits);
        }
        View::Calendar => {
            if !app.events_loaded {
                empty(f, &format!("{} Loading calendar…", spinner(app)));
            } else if app.events.is_empty() {
                empty(f, "Nothing scheduled this week.");
            }
            let rows = app
                .events
                .iter()
                .map(|e| Row {
                    avatar: Span::styled(format!(" {:>2} ", e.start.day()), Style::new().fg(Color::White).bg(if e.start.date_naive() == Local::now().date_naive() { BRAND } else { SEL }).add_modifier(Modifier::BOLD)),
                    dot: None,
                    title: e.subject.clone(),
                    strong: false,
                    right: String::new(),
                    sub: if e.all_day { format!("{} · All day", e.start.format("%a")) } else { format!("{} {} – {}", e.start.format("%a"), e.start.format("%-I:%M %p"), e.end.format("%-I:%M %p")) },
                    badge: e.join_url.is_some().then(|| Span::styled("📹", accent())),
                })
                .collect();
            draw_rows(f, body, rows, app.ev_sel, focused, &mut hits);
        }
        View::Teams => {
            if !app.teams_loaded {
                empty(f, &format!("{} Loading teams…", spinner(app)));
            } else if app.teams.is_empty() {
                empty(f, "You're not a member of any team yet.");
            }
            let rows = app.team_rows();
            let visible = body.height as usize;
            let offset = if app.team_sel >= visible { app.team_sel + 1 - visible } else { 0 };
            let open = app.chan_conv.as_ref().map(|c| c.target.clone());
            for (i, row) in rows.iter().enumerate().skip(offset).take(visible) {
                let y = body.y + (i - offset) as u16;
                let r = Rect::new(body.x, y, body.width, 1);
                if i == app.team_sel {
                    fill(f, r, if focused { SEL } else { PANEL });
                    if focused {
                        put(f, r, r.x, y, Line::styled("▎", accent()));
                    }
                }
                let line = match *row {
                    TeamRow::Team(ti) => {
                        let t = &app.teams[ti];
                        let arrow = if app.collapsed.contains(&t.id) { "▸" } else { "▾" };
                        Line::from(vec![Span::styled(format!(" {arrow} "), dim()), avatar(&t.name), Span::styled(format!(" {}", trunc(&t.name, body.width.saturating_sub(9) as usize)), bold())])
                    }
                    TeamRow::Channel(ti, ci) => {
                        let (t, c) = (&app.teams[ti], &app.teams[ti].channels[ci]);
                        let is_open = open.as_ref() == Some(&ConvRef::Channel { team: t.id.clone(), channel: c.id.clone() });
                        Line::styled(format!("         {}", trunc(&c.name, body.width.saturating_sub(10) as usize)), if is_open { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { fg() })
                    }
                };
                put(f, r, r.x, y, line);
                hits.push((r, Hit::List(i)));
            }
        }
        View::Files => {
            put(f, body, body.x + 2, body.y, Line::from(vec![Span::styled("📁 ", fg()), Span::styled("My files", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))]));
            let depth = app.file_path.len();
            for (i, (_, name)) in app.file_path.iter().enumerate().skip(1) {
                put(f, body, body.x + 2 + (i as u16) * 2, body.y + i as u16, Line::styled(format!("└ {}", trunc(name, body.width.saturating_sub(8 + i as u16 * 2) as usize)), if i + 1 == depth { fg() } else { dim() }));
            }
        }
    }
    app.hits.extend(hits);
}

fn draw_status(f: &mut Frame, area: Rect, app: &mut App) {
    fill(f, area, TITLE);
    let hint = match (app.view, app.focus) {
        (View::Chat | View::Teams, Focus::Compose) if app.conv().is_some() => "Enter send · Alt+Enter new line · @name+Tab mention · ↑ messages · Tab switch pane · F1 help",
        (View::Chat | View::Teams, Focus::Messages) => "↑↓ select · r react · Enter reply · e edit · d delete · v view image · o open link · Esc back · F1 help",
        (View::Calendar, _) => "↑↓ event · Enter details · ←→ week · t today · n new meeting · J join · Alt+1-6 apps · F1 help",
        (View::Files, _) => "↑↓ select · Enter open · Backspace up · d download · Alt+1-6 apps · F1 help",
        (View::Calls, _) => "↑↓ select · Enter/c call · v video call · m message · Alt+1-6 apps · F1 help",
        _ => "↑↓ select · Enter open · n new · / search · Alt+1-6 apps · Ctrl+P status · F1 help · Ctrl+Q quit",
    };
    put(f, area, area.x + 1, area.y, Line::styled(hint, faint()));
    if let Some((msg, _, err)) = &app.toast {
        let style = if *err { Style::new().fg(Color::White).bg(Color::Rgb(150, 40, 40)) } else { Style::new().fg(Color::White).bg(BRAND) };
        let text = format!(" {} ", trunc(&msg.replace('\n', " "), area.width.saturating_sub(4) as usize));
        put_right(f, area, area.y, Line::styled(text, style.add_modifier(Modifier::BOLD)));
    } else if app.demo {
        put_right(f, area, area.y, Line::styled(" DEMO DATA ", Style::new().fg(Color::Black).bg(YELLOW)));
    }
}

pub struct LoginState {
    pub code: Option<DeviceCode>,
    pub error: Option<String>,
    pub tick: usize,
}

pub fn draw_login(f: &mut Frame, st: &LoginState) {
    let area = f.area();
    f.buffer_mut().set_style(area, Style::new().bg(MAIN).fg(FG));
    let card = centered(area, 64, 15);
    f.render_widget(Clear, card);
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(BRAND)).style(Style::new().bg(PANEL).fg(FG)).title(" TeamsTUI · Sign in ");
    let inner = block.inner(card);
    f.render_widget(block, card);
    const S: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    let spin = S[st.tick % S.len()];
    let mut lines: Vec<Line> = vec![Line::raw("")];
    if let Some(e) = &st.error {
        lines.push(Line::styled("Sign-in failed", Style::new().fg(RED).add_modifier(Modifier::BOLD)));
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), fg()));
        lines.push(Line::raw(""));
        lines.push(Line::styled("If your organisation blocks the default app, pass your own", dim()));
        lines.push(Line::styled("app registration with --client-id (see README).", dim()));
        lines.push(Line::raw(""));
        lines.push(Line::styled("Press r to retry or q to quit", dim()));
    } else if let Some(c) = &st.code {
        lines.push(Line::styled("Sign in with your work or school account", bold()));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("1. Open  ", dim()), Span::styled(c.verification_uri.clone(), Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED))]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("2. Enter the code  ", dim()), Span::styled(format!(" {} ", c.user_code), Style::new().fg(Color::White).bg(BRAND).add_modifier(Modifier::BOLD))]));
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{spin} Waiting for you to finish in the browser…"), dim()));
        lines.push(Line::raw(""));
        lines.push(Line::styled("Enter opens the browser · q quits", faint()));
    } else {
        lines.push(Line::styled(format!("{spin} Contacting Microsoft…"), dim()));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_long_tokens() {
        assert_eq!(wrap("hello brave new world", 11), vec!["hello brave", "new world"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(trunc("hello", 10), "hello");
        assert_eq!(trunc("hello world", 6), "hello…");
        assert_eq!(w(&pad("日本語テキスト", 7)), 7);
        assert_eq!(initials("megan bowen"), "MB");
    }
}
