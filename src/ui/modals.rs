//! Overlays: search, new chat, reactions, status, event details, help.
use super::*;
use crate::app::{Modal, SearchItem, MEETING_FIELDS, PRESENCE_EXTRA, REACTIONS};

fn frame(f: &mut Frame, r: Rect, title: &str, hits: &mut Vec<(Rect, Hit)>) -> Rect {
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BRAND))
        .style(Style::new().bg(PANEL).fg(FG))
        .title(Line::styled(format!(" {title} "), Style::new().fg(FG).add_modifier(Modifier::BOLD)));
    let inner = block.inner(r);
    f.render_widget(block, r);
    hits.push((r, Hit::ModalBody));
    Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height)
}

fn item_row(f: &mut Frame, r: Rect, selected: bool, line: Line) {
    if selected {
        fill(f, r, BRAND);
    }
    put(f, r, r.x + 1, r.y, line);
}

const HELP: [(&str, &str); 24] = [
    ("Alt+1 … Alt+6", "Activity, Chat, Teams, Calendar, Calls, OneDrive"),
    ("Ctrl+E  or  /", "Search people, chats, channels and messages"),
    ("Ctrl+N  or  n", "New chat (new meeting in Calendar)"),
    ("Ctrl+P", "Set status, status message, sign out"),
    ("Ctrl+R", "Refresh everything"),
    ("Tab / Shift+Tab", "Move between list, messages and compose box"),
    ("↑ ↓  or  j k", "Move selection"),
    ("Enter", "Open the selected item / send the message"),
    ("Alt+Enter, Ctrl+J", "New line in the compose box"),
    ("@name then Tab", "Mention someone in the conversation"),
    ("↑ (in compose)", "Jump into the message list"),
    ("r", "React to the selected message"),
    ("Enter  or  y", "Reply (quote in chats, thread reply in channels)"),
    ("e", "Edit your message"),
    ("d", "Delete your message"),
    ("o", "Open the message's link, image or attachment"),
    ("c", "Call the people in this chat (opens Teams)"),
    ("PgUp / PgDn", "Scroll the conversation"),
    ("← →  /  t", "Calendar: previous / next week, today"),
    ("J", "Calendar: join the selected meeting"),
    ("Backspace / d", "OneDrive: go up / download file"),
    ("Mouse", "Click anything; wheel scrolls"),
    ("Esc", "Close dialogs, cancel edit or reply"),
    ("Ctrl+Q", "Quit"),
];

pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
    if matches!(app.modal, Modal::None) {
        return;
    }
    let mut hits = vec![(area, Hit::Backdrop)];
    let mut cursor = None;
    match &app.modal {
        Modal::None => {}
        Modal::Help => {
            let r = centered(area, 78, HELP.len() as u16 + 4);
            let inner = frame(f, r, "Keyboard shortcuts", &mut hits);
            for (i, (k, d)) in HELP.iter().enumerate() {
                put(f, inner, inner.x, inner.y + 1 + i as u16, Line::from(vec![Span::styled(pad(k, 20), accent()), Span::styled(*d, fg())]));
            }
        }
        Modal::Search { input, results, sel, loading } => {
            let wd = (area.width * 2 / 3).clamp(50, 90).min(area.width - 2);
            let rows = results.len().min(area.height.saturating_sub(8) as usize).max(1) as u16;
            let r = Rect::new(area.x + (area.width - wd) / 2, area.y, wd, rows + 4);
            let inner = frame(f, r, "Search", &mut hits);
            put(f, inner, inner.x, inner.y, Line::raw("🔍"));
            cursor = Some(field(f, Rect::new(inner.x + 3, inner.y, inner.width.saturating_sub(3), 1), input, "Look for people, chats, channels and messages", fg()));
            put(f, inner, inner.x, inner.y + 1, Line::styled("─".repeat(inner.width as usize), faint()));
            if results.is_empty() {
                let text = if *loading {
                    format!("{} Searching…", spinner(app))
                } else if input.text.trim().is_empty() {
                    "Type to search".to_string()
                } else {
                    "No results".to_string()
                };
                put(f, inner, inner.x + 1, inner.y + 2, Line::styled(text, dim()));
            }
            let visible = rows as usize;
            let offset = if *sel >= visible { *sel + 1 - visible } else { 0 };
            for (i, item) in results.iter().enumerate().skip(offset).take(visible) {
                let rr = Rect::new(inner.x, inner.y + 2 + (i - offset) as u16, inner.width, 1);
                let tw = inner.width.saturating_sub(4) as usize;
                let line = match item {
                    SearchItem::Chat { title, .. } => Line::from(vec![Span::raw("💬 "), Span::styled(trunc(title, tw), fg())]),
                    SearchItem::Channel { title, .. } => Line::from(vec![Span::raw("👥 "), Span::styled(trunc(title, tw), fg())]),
                    SearchItem::Person(u) => Line::from(vec![Span::raw("👤 "), Span::styled(trunc(&u.name, tw / 2), fg()), Span::styled(format!("  {}", trunc(&u.email, tw / 2 - 2)), dim())]),
                    SearchItem::Message(h) => {
                        let from = trunc(&h.from, 18);
                        Line::from(vec![Span::raw("✉  "), Span::styled(format!("{from}: "), bold()), Span::styled(trunc(&h.summary.replace('\n', " "), tw.saturating_sub(w(&from) + 12)), fg()), Span::styled(format!("  {}", short_time(h.when)), dim())])
                    }
                };
                item_row(f, rr, i == *sel, line);
                hits.push((rr, Hit::ModalItem(i)));
            }
            if *loading && !results.is_empty() {
                put_right(f, inner, inner.y, Line::styled(spinner(app), dim()));
            }
        }
        Modal::NewChat { input, results, sel, picked } => {
            let r = centered(area, 64, 16);
            let inner = frame(f, r, "New chat", &mut hits);
            let mut spans = vec![Span::styled("To: ", dim())];
            for p in picked {
                spans.push(Span::styled(format!(" {} ", trunc(&p.name, 18)), Style::new().fg(Color::White).bg(BRAND)));
                spans.push(Span::raw(" "));
            }
            let used = (spans.iter().map(|s| w(&s.content)).sum::<usize>() as u16).min(inner.width.saturating_sub(12));
            put(f, Rect::new(inner.x, inner.y + 1, used, 1), inner.x, inner.y + 1, Line::from(spans));
            cursor = Some(field(f, Rect::new(inner.x + used, inner.y + 1, inner.width - used, 1), input, "Enter a name or email", fg()));
            put(f, inner, inner.x, inner.y + 2, Line::styled("─".repeat(inner.width as usize), faint()));
            for (i, u) in results.iter().enumerate().take(inner.height.saturating_sub(5) as usize) {
                let rr = Rect::new(inner.x, inner.y + 3 + i as u16, inner.width, 1);
                let a = app.presence.get(&u.id).copied().unwrap_or_default();
                item_row(f, rr, i == *sel, Line::from(vec![avatar(&u.name), Span::raw(" "), presence_dot(a), Span::styled(format!(" {}", trunc(&u.name, 24)), fg()), Span::styled(format!("  {}", trunc(&u.email, 26)), dim())]));
                hits.push((rr, Hit::ModalItem(i)));
            }
            let hint = if picked.is_empty() { "Type a name · Enter adds the person" } else { "Enter adds · Enter on an empty field starts the chat · Backspace removes" };
            put(f, inner, inner.x, inner.bottom().saturating_sub(1), Line::styled(hint, faint()));
        }
        Modal::React { sel } => {
            let r = centered(area, 40, 5);
            let inner = frame(f, r, "React", &mut hits);
            let mut x = inner.x + 1;
            for (i, e) in REACTIONS.iter().enumerate() {
                let rr = Rect::new(x, inner.y + 1, 5, 1);
                if i == *sel {
                    fill(f, rr, BRAND);
                }
                put(f, rr, x + 1, rr.y, Line::raw(*e));
                hits.push((rr, Hit::ModalItem(i)));
                x += 6;
            }
            put(f, inner, inner.x, inner.y + 2, Line::styled("←/→ then Enter, or press 1-6", faint()));
        }
        Modal::Presence { sel } => {
            let n = Availability::SETTABLE.len();
            let r = Rect::new(area.right().saturating_sub(36), area.y + 1, 34.min(area.width), (n + PRESENCE_EXTRA.len() + 6) as u16).intersection(area);
            let inner = frame(f, r, "Account", &mut hits);
            put(f, inner, inner.x, inner.y, Line::from(vec![avatar(&app.me.name), Span::styled(format!(" {}", trunc(&app.me.name, 22)), bold())]));
            put(f, inner, inner.x, inner.y + 1, Line::styled(trunc(&app.me.email, inner.width as usize), dim()));
            for (i, a) in Availability::SETTABLE.iter().enumerate() {
                let rr = Rect::new(inner.x, inner.y + 3 + i as u16, inner.width, 1);
                let current = if *a == app.my_presence { "  ✓" } else { "" };
                item_row(f, rr, i == *sel, Line::from(vec![presence_dot(*a), Span::styled(format!(" {}{current}", a.label()), fg())]));
                hits.push((rr, Hit::ModalItem(i)));
            }
            for (j, label) in PRESENCE_EXTRA.iter().enumerate() {
                let i = n + j;
                let rr = Rect::new(inner.x, inner.y + 4 + i as u16, inner.width, 1);
                item_row(f, rr, i == *sel, Line::styled(format!("  {label}"), if j == 2 { Style::new().fg(RED) } else { fg() }));
                hits.push((rr, Hit::ModalItem(i)));
            }
        }
        Modal::StatusMsg { input } => {
            let r = centered(area, 60, 6);
            let inner = frame(f, r, "Set status message", &mut hits);
            cursor = Some(field(f, Rect::new(inner.x, inner.y + 1, inner.width, 1), input, "What's your status?", fg()));
            put(f, inner, inner.x, inner.y + 3, Line::styled("Enter saves (empty clears) · Esc cancels", faint()));
        }
        Modal::ConfirmDelete => {
            let r = centered(area, 46, 6);
            let inner = frame(f, r, "Delete message", &mut hits);
            put(f, inner, inner.x, inner.y + 1, Line::styled("Delete this message for everyone?", fg()));
            put(f, inner, inner.x, inner.y + 3, Line::from(vec![Span::styled(" y ", Style::new().fg(Color::White).bg(RED)), Span::styled(" Delete    ", fg()), Span::styled(" Esc ", Style::new().bg(SEL)), Span::styled(" Cancel", fg())]));
        }
        Modal::Event(i) => {
            if let Some(e) = app.events.get(*i) {
                let r = centered(area, 70, 18);
                let inner = frame(f, r, "Meeting details", &mut hits);
                let when = if e.all_day { format!("{} · All day", e.start.format("%A, %B %-d")) } else { format!("{} · {} – {}", e.start.format("%A, %B %-d"), e.start.format("%-I:%M %p"), e.end.format("%-I:%M %p")) };
                let response = match e.response.as_str() {
                    "accepted" => "Accepted",
                    "tentativelyAccepted" => "Tentative",
                    "declined" => "Declined",
                    "organizer" => "You're the organizer",
                    _ => "Not responded",
                };
                let tw = inner.width as usize;
                let mut lines = vec![
                    Line::raw(""),
                    Line::styled(trunc(&e.subject, tw), bold()),
                    Line::styled(if e.cancelled { format!("{when}  (cancelled)") } else { when }, accent()),
                    Line::raw(""),
                    Line::from(vec![Span::styled("Where      ", dim()), Span::styled(trunc(if e.location.is_empty() { "—" } else { &e.location }, tw - 11), fg())]),
                    Line::from(vec![Span::styled("Organizer  ", dim()), Span::styled(trunc(&e.organizer, tw - 11), fg())]),
                    Line::from(vec![Span::styled("Response   ", dim()), Span::styled(response, fg())]),
                    Line::from(vec![Span::styled("Attendees  ", dim()), Span::styled(trunc(&e.attendees.join(", "), tw - 11), fg())]),
                    Line::raw(""),
                ];
                lines.extend(wrap(&e.preview.replace(['\r', '\n'], " "), tw).into_iter().take(4).map(|l| Line::styled(l, dim())));
                f.render_widget(Paragraph::new(lines), inner);
                let join = if e.join_url.is_some() { Span::styled(" j Join ", Style::new().fg(Color::White).bg(BRAND)) } else { Span::styled(" No online meeting ", faint()) };
                put(f, inner, inner.x, inner.bottom().saturating_sub(1), Line::from(vec![join, Span::styled("  a accept · t tentative · d decline · o open in Outlook · Esc", faint())]));
            }
        }
        Modal::NewMeeting { fields, focus } => {
            let r = centered(area, 66, (fields.len() * 3 + 4) as u16);
            let inner = frame(f, r, "New meeting", &mut hits);
            for (i, input) in fields.iter().enumerate() {
                let y = inner.y + 1 + (i as u16) * 3;
                let active = i == *focus;
                put(f, inner, inner.x, y, Line::styled(MEETING_FIELDS[i], if active { accent() } else { dim() }));
                let fr = Rect::new(inner.x, y + 1, inner.width, 1);
                fill(f, fr, if active { SEL } else { Color::Rgb(50, 50, 50) });
                let pos = field(f, Rect::new(fr.x + 1, fr.y, fr.width.saturating_sub(2), 1), input, "", fg());
                if active {
                    cursor = Some(pos);
                }
                hits.push((Rect::new(fr.x, y, fr.width, 2), Hit::ModalItem(i)));
            }
            put(f, inner, inner.x, inner.bottom().saturating_sub(1), Line::styled("Tab next field · Enter schedule a Teams meeting · Esc cancel", faint()));
        }
    }
    if let Some(c) = cursor {
        f.set_cursor_position(c);
    }
    app.hits.extend(hits);
}
