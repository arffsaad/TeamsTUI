//! The conversation pane: header, message list and compose box.
use super::*;
use crate::app::{Conv, Modal};

fn time_label(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%-I:%M %p").to_string()
}

fn date_label(d: chrono::NaiveDate) -> String {
    let today = Local::now().date_naive();
    match (today - d).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ if d.year() == today.year() => d.format("%A, %B %-d").to_string(),
        _ => d.format("%B %-d, %Y").to_string(),
    }
}

fn content(m: &Message, maxw: usize) -> Vec<(String, Style)> {
    if m.deleted {
        return vec![("This message has been deleted.".into(), dim().add_modifier(Modifier::ITALIC))];
    }
    let mut rows = vec![];
    if let Some(s) = &m.subject {
        rows.extend(wrap(s, maxw).into_iter().map(|l| (l, bold())));
    }
    if !m.body.is_empty() || m.attachments.is_empty() {
        for para in m.body.split('\n') {
            let style = if para.starts_with('▎') { dim() } else { fg() };
            rows.extend(wrap(para, maxw).into_iter().map(|l| (l, style)));
        }
    }
    for a in &m.attachments {
        rows.push((format!("📎 {}", trunc(&a.name, maxw.saturating_sub(3))), accent().add_modifier(Modifier::UNDERLINED)));
    }
    rows
}

fn reactions(m: &Message, me: &str) -> Vec<Span<'static>> {
    let mut groups: Vec<(String, usize, bool)> = vec![];
    for r in &m.reactions {
        match groups.iter_mut().find(|g| g.0 == r.emoji) {
            Some(g) => {
                g.1 += 1;
                g.2 |= r.user_id == me;
            }
            None => groups.push((r.emoji.clone(), 1, r.user_id == me)),
        }
    }
    let mut spans = vec![];
    for (emoji, n, mine) in groups {
        spans.push(Span::styled(format!(" {emoji} {n} "), Style::new().fg(FG).bg(if mine { BRAND } else { SEL })));
        spans.push(Span::raw(" "));
    }
    spans
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| w(&s.content)).sum()
}

struct Built {
    lines: Vec<Line<'static>>,
    ranges: Vec<(usize, usize)>,
}

fn center(text: String, width: usize, style: Style) -> Line<'static> {
    let padl = width.saturating_sub(w(&text)) / 2;
    Line::from(vec![Span::raw(" ".repeat(padl)), Span::styled(text, style)])
}

fn date_sep(b: &mut Built, d: chrono::NaiveDate, width: usize) {
    let label = format!(" {} ", date_label(d));
    let side = width.saturating_sub(w(&label) + 4) / 2;
    b.lines.push(Line::raw(""));
    b.lines.push(Line::from(vec![Span::raw("  "), Span::styled("─".repeat(side), faint()), Span::styled(label, dim()), Span::styled("─".repeat(side), faint())]));
}

fn header_meta(m: &Message, me: &str) -> Vec<Span<'static>> {
    let mut v = vec![Span::styled(time_label(m.created), dim())];
    if m.edited && !m.deleted {
        v.push(Span::styled("  Edited", faint()));
    }
    if m.id.starts_with("pending-") {
        v.push(Span::styled("  Sending…", faint()));
    }
    if m.mentions.iter().any(|x| x == me) {
        v.push(Span::styled("  @", Style::new().fg(RED).add_modifier(Modifier::BOLD)));
    }
    v
}

/// Chat layout: bubbles, other people on the left, your messages on the right.
fn build_chat(conv: &Conv, me: &str, width: usize) -> Built {
    let mut b = Built { lines: vec![], ranges: vec![] };
    let maxw = (width * 2 / 3).clamp(16, width.saturating_sub(10).max(16));
    let mut prev: Option<(String, DateTime<Utc>)> = None;
    let mut prev_date = None;
    for (i, fm) in conv.flat().iter().enumerate() {
        let m = fm.m;
        let date = m.created.with_timezone(&Local).date_naive();
        if prev_date != Some(date) {
            date_sep(&mut b, date, width);
            prev_date = Some(date);
            prev = None;
        }
        let selected = conv.sel == Some(i);
        let start = b.lines.len();
        if m.system {
            b.lines.push(Line::raw(""));
            b.lines.push(center(m.body.replace('\n', " "), width, if selected { accent() } else { faint().add_modifier(Modifier::ITALIC) }));
            b.ranges.push((start, b.lines.len()));
            prev = None;
            continue;
        }
        let mine = m.from.id == me;
        let grouped = prev.as_ref().is_some_and(|(id, t)| *id == m.from.id && (m.created - *t).num_minutes() < 5);
        let rows = content(m, maxw - 2);
        let bw = rows.iter().map(|(l, _)| w(l)).max().unwrap_or(0) + 2;
        let bg = match (selected, mine) {
            (true, _) => BRAND,
            (false, true) => OWN,
            (false, false) => BUBBLE,
        };
        if !grouped {
            b.lines.push(Line::raw(""));
            let meta = header_meta(m, me);
            if mine {
                let padl = width.saturating_sub(spans_width(&meta) + 2);
                let mut spans = vec![Span::raw(" ".repeat(padl))];
                spans.extend(meta);
                b.lines.push(Line::from(spans));
            } else {
                let mut spans = vec![Span::raw(" "), avatar(&m.from.name), Span::raw(" "), Span::styled(trunc(&m.from.name, width.saturating_sub(30)), bold()), Span::raw("  ")];
                spans.extend(meta);
                b.lines.push(Line::from(spans));
            }
        }
        let mark = |on: bool, ch: &'static str| if on { Span::styled(ch, accent()) } else { Span::raw(" ") };
        for (text, style) in rows {
            let cell = Span::styled(format!(" {} ", pad(&text, bw - 2)), style.bg(bg));
            if mine {
                b.lines.push(Line::from(vec![Span::raw(" ".repeat(width.saturating_sub(bw + 2))), cell, Span::raw(" "), mark(selected, "▐")]));
            } else {
                b.lines.push(Line::from(vec![mark(selected, "▌"), Span::raw("     "), cell]));
            }
        }
        let chips = reactions(m, me);
        if !chips.is_empty() && !m.deleted {
            let padl = if mine { width.saturating_sub(spans_width(&chips) + 1) } else { 6 };
            let mut spans = vec![Span::raw(" ".repeat(padl))];
            spans.extend(chips);
            b.lines.push(Line::from(spans));
        }
        b.ranges.push((start, b.lines.len()));
        prev = Some((m.from.id.clone(), m.created));
    }
    b.lines.push(Line::raw(""));
    b
}

/// Channel layout: each post is a card with its replies threaded underneath.
fn build_channel(conv: &Conv, me: &str, width: usize) -> Built {
    let mut b = Built { lines: vec![], ranges: vec![] };
    let cw = width.saturating_sub(4).max(20);
    let card = |selected: bool, spans: Vec<Span<'static>>| -> Line<'static> {
        let bg = if selected { SEL } else { PANEL };
        let used = spans_width(&spans);
        let mut out = vec![if selected { Span::styled("▌", accent()) } else { Span::raw(" ") }, Span::raw(" ")];
        out.extend(spans.into_iter().map(|s| {
            let st = if s.style.bg.is_some() { s.style } else { s.style.bg(bg) };
            Span::styled(s.content, st)
        }));
        out.push(Span::styled(" ".repeat(cw.saturating_sub(used)), Style::new().bg(bg)));
        Line::from(out)
    };
    let flat = conv.flat();
    for (i, fm) in flat.iter().enumerate() {
        let m = fm.m;
        let selected = conv.sel == Some(i);
        let reply = fm.parent.is_some();
        if !reply {
            b.lines.push(Line::raw(""));
        }
        let start = b.lines.len();
        let (gutter, indent, maxw) = if reply { ("  │ ", "  │      ", cw.saturating_sub(11)) } else { (" ", "      ", cw.saturating_sub(8)) };
        let when = m.created.with_timezone(&Local);
        let stamp = if when.date_naive() == Local::now().date_naive() { time_label(m.created) } else { format!("{} {}", when.format("%b %-d"), time_label(m.created)) };
        let mut head = vec![Span::styled(gutter, faint()), avatar(&m.from.name), Span::raw(" "), Span::styled(trunc(&m.from.name, 28), bold()), Span::raw("  "), Span::styled(stamp, dim())];
        head.extend(header_meta(m, me).into_iter().skip(1));
        b.lines.push(card(selected, head));
        for (text, style) in content(m, maxw) {
            b.lines.push(card(selected, vec![Span::styled(indent, faint()), Span::styled(text, style)]));
        }
        let chips = reactions(m, me);
        if !chips.is_empty() && !m.deleted {
            let mut spans = vec![Span::styled(indent, faint())];
            spans.extend(chips);
            b.lines.push(card(selected, spans));
        }
        b.ranges.push((start, b.lines.len()));
        let last_of_thread = flat.get(i + 1).map_or(true, |n| n.parent.is_none());
        if last_of_thread {
            b.lines.push(card(false, vec![Span::styled("  ↩ Reply", faint())]));
        }
    }
    b.lines.push(Line::raw(""));
    b
}

pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
    let focus = app.focus;
    let modal_open = !matches!(app.modal, Modal::None);
    let me = app.me.id.clone();
    let spin = spinner(app);
    let Some(conv) = app.conv() else {
        let (icon, text) = match app.view {
            View::Teams => ("👥", "Pick a channel on the left to see its posts"),
            _ => ("💬", "Select a chat, or press Ctrl+N to start a new one"),
        };
        let r = centered(area, area.width.saturating_sub(4), 3);
        f.render_widget(Paragraph::new(vec![Line::raw(icon), Line::raw(""), Line::styled(text, dim())]).alignment(Alignment::Center), r);
        return;
    };
    let is_channel = conv.is_channel();

    // Header.
    let head = Rect::new(area.x, area.y, area.width, 3);
    let mut title = vec![Span::raw(" "), avatar(conv.title.rsplit(" › ").next().unwrap_or(&conv.title)), Span::raw(" "), Span::styled(trunc(&conv.title, area.width.saturating_sub(34) as usize), bold())];
    let mut call_hits = vec![];
    if let ConvRef::Chat(id) = &conv.target {
        if let Some(c) = app.chats.iter().find(|c| &c.id == id).filter(|c| c.kind == ChatKind::OneOnOne) {
            let a = c.others(&me).next().and_then(|u| app.presence.get(&u.id).copied()).unwrap_or_default();
            if a != Availability::Unknown {
                title.push(Span::raw("  "));
                title.push(presence_dot(a));
                title.push(Span::styled(format!(" {}", a.status()), dim()));
            }
        }
        let clip = Rect::new(head.x, head.y, head.width.saturating_sub(1), 1);
        let call = put_right(f, clip, head.y, Line::styled(" 📞 Call ", dim()));
        let clip2 = Rect::new(head.x, head.y, call.x.saturating_sub(head.x), 1);
        let video = put_right(f, clip2, head.y, Line::styled(" 📹 Video ", dim()));
        call_hits.push((call, Hit::Call(false)));
        call_hits.push((video, Hit::Call(true)));
    }
    put(f, Rect::new(head.x, head.y, head.width.saturating_sub(22), 1), head.x, head.y, Line::from(title));
    let tab = if is_channel { "Posts" } else { "Chat" };
    let mut tabs = vec![Span::raw("  "), Span::styled(tab, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))];
    if !conv.subtitle.is_empty() {
        tabs.push(Span::styled(format!("    {}", trunc(&conv.subtitle.replace('\n', " "), area.width.saturating_sub(16) as usize)), faint()));
    }
    put(f, head, head.x, head.y + 1, Line::from(tabs));
    put(f, head, head.x, head.y + 2, Line::from(vec![Span::styled("──", faint()), Span::styled("━".repeat(tab.len()), accent()), Span::styled("─".repeat(area.width.saturating_sub(2 + tab.len() as u16) as usize), faint())]));

    // Compose box geometry.
    let inner_w = area.width.saturating_sub(6) as usize;
    let (rows, cur) = app.compose.layout(inner_w);
    let shown = rows.len().clamp(1, 6);
    let ch = shown as u16 + 2;
    let compose = Rect::new(area.x + 1, area.bottom().saturating_sub(ch), area.width.saturating_sub(2), ch);
    let msgs = Rect::new(area.x, head.bottom(), area.width, compose.y.saturating_sub(head.bottom()));

    // Messages.
    let width = msgs.width as usize;
    let built = if is_channel { build_channel(conv, &me, width) } else { build_chat(conv, &me, width) };
    let total = built.lines.len();
    let h = msgs.height as usize;
    let max_scroll = total.saturating_sub(h);
    let mut scroll = conv.scroll;
    if conv.follow {
        if let Some(&(s, e)) = conv.sel.and_then(|i| built.ranges.get(i)) {
            let top = max_scroll.saturating_sub(scroll.min(max_scroll));
            if e > top + h {
                scroll = total.saturating_sub(e);
            } else if s < top + 1 {
                scroll = total.saturating_sub(s.saturating_sub(1) + h);
            }
        }
    }
    scroll = scroll.min(max_scroll);
    let top = max_scroll - scroll;
    let y0 = msgs.y + (h.saturating_sub(total)) as u16;
    let (loaded, empty, loading_older, has_more) = (conv.loaded, conv.msgs.is_empty(), conv.loading_older, conv.cursor.is_some());
    let mut hits = vec![(msgs, Hit::Messages)];
    if !loaded {
        f.render_widget(Paragraph::new(format!("{spin} Loading conversation…")).style(dim()).alignment(Alignment::Center), centered(msgs, msgs.width, 1));
    } else if empty {
        let text = if is_channel { "No posts yet. Start a new post below." } else { "No messages yet. Say hi 👋" };
        f.render_widget(Paragraph::new(text).style(dim()).alignment(Alignment::Center), centered(msgs, msgs.width, 1));
    } else {
        for (row, line) in built.lines.into_iter().skip(top).take(h).enumerate() {
            put(f, msgs, msgs.x, y0 + row as u16, line);
        }
        for (i, &(s, e)) in built.ranges.iter().enumerate() {
            let (vs, ve) = (s.max(top), e.min(top + h));
            if vs < ve {
                hits.push((Rect::new(msgs.x, y0 + (vs - top) as u16, msgs.width, (ve - vs) as u16), Hit::Message(i)));
            }
        }
        if loading_older {
            put(f, msgs, msgs.x + 2, msgs.y, Line::styled(format!("{spin} Loading earlier messages…"), dim()));
        } else if scroll > 0 {
            put_right(f, Rect::new(msgs.x, msgs.bottom().saturating_sub(1), msgs.width.saturating_sub(2), 1), msgs.bottom().saturating_sub(1), Line::styled(" ↓ Latest (Esc) ", Style::new().fg(Color::White).bg(BRAND)));
        }
    }

    // Compose box.
    let focused = focus == Focus::Compose && !modal_open;
    let (title, placeholder) = if app.editing.is_some() {
        (" ✎ Editing message · Esc to cancel ".to_string(), "Edit your message")
    } else if let Some(r) = &app.reply {
        (format!(" ↩ Replying to {} · Esc to cancel ", trunc(&r.who, 30)), "Type a reply")
    } else if is_channel {
        (" New post ".to_string(), "Start a new post")
    } else {
        (String::new(), "Type a message")
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { accent() } else { faint() })
        .style(Style::new().bg(PANEL))
        .title(Line::styled(title, accent()))
        .title_bottom(Line::styled(if focused { " Enter ➤ " } else { "" }, accent()).right_aligned());
    let inner = block.inner(compose);
    f.render_widget(block, compose);
    let first = (cur.1 + 1).saturating_sub(shown);
    if app.compose.text.is_empty() {
        put(f, inner, inner.x + 1, inner.y, Line::styled(placeholder, faint()));
    } else {
        for (i, row) in rows.iter().skip(first).take(shown).enumerate() {
            put(f, inner, inner.x + 1, inner.y + i as u16, Line::styled(row.clone(), fg()));
        }
    }
    if focused {
        f.set_cursor_position(Position::new(inner.x + 1 + cur.0 as u16, inner.y + (cur.1 - first) as u16));
    }
    hits.push((compose, Hit::Compose));
    hits.extend(call_hits);

    // Write back what only the renderer knows.
    let want_older = loaded && has_more && scroll >= max_scroll;
    if let Some(c) = match app.view {
        View::Teams => app.chan_conv.as_mut(),
        _ => app.chat_conv.as_mut(),
    } {
        c.scroll = scroll;
        c.follow = false;
        c.want_older |= want_older;
    }
    app.hits.extend(hits);
}
