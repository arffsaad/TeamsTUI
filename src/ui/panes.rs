//! Main panes for Activity, Calendar, Calls and OneDrive.
use super::*;
use chrono::{Duration, Timelike};

pub fn activity(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(item) = app.activity.get(app.act_sel) else {
        f.render_widget(Paragraph::new(vec![Line::raw("🔔"), Line::raw(""), Line::styled("Your activity feed is empty", dim())]).alignment(Alignment::Center), centered(area, area.width, 3));
        return;
    };
    let card = Rect::new(area.x + 3, area.y + 2, area.width.saturating_sub(6), area.height.saturating_sub(4).min(12));
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(faint()).style(Style::new().bg(PANEL));
    let inner = block.inner(card);
    f.render_widget(block, card);
    let when = item.when.map(|t| t.with_timezone(&Local).format("%A, %B %-d · %-I:%M %p").to_string()).unwrap_or_default();
    let mut lines = vec![
        Line::from(vec![Span::raw(" "), avatar(&item.title), Span::raw(" "), Span::styled(trunc(&item.title, inner.width.saturating_sub(8) as usize), bold())]),
        Line::styled(trunc(&format!("      {when}"), inner.width as usize), dim()),
        Line::raw(""),
    ];
    for l in wrap(&item.preview, inner.width.saturating_sub(8) as usize).into_iter().take(5) {
        lines.push(Line::styled(format!("      {l}"), fg()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("      Enter opens the conversation", accent()));
    f.render_widget(Paragraph::new(lines), inner);
}

fn hour_label(h: u32) -> String {
    match h {
        0 => "12 AM".into(),
        12 => "12 PM".into(),
        h if h < 12 => format!("{h:>2} AM"),
        h => format!("{:>2} PM", h - 12),
    }
}

pub fn calendar(f: &mut Frame, area: Rect, app: &mut App) {
    let mut hits = vec![];
    let week = app.week_start;
    let end = week + Duration::days(6);
    let range = if week.month() == end.month() {
        format!("{} {} – {}, {}", week.format("%B"), week.day(), end.day(), end.year())
    } else {
        format!("{} – {}, {}", week.format("%b %-d"), end.format("%b %-d"), end.year())
    };
    let y = area.y + 1;
    let mut x = area.x + 2;
    for (label, nav) in [(" ‹ ", -1), (" › ", 1), (" Today ", 0)] {
        let r = Rect::new(x, y, w(label) as u16, 1);
        fill(f, r, PANEL);
        put(f, area, x, y, Line::styled(label, fg()));
        hits.push((r, Hit::CalNav(nav)));
        x += r.width + 1;
    }
    put(f, area, x + 1, y, Line::styled(range, bold()));
    if !app.events_loaded {
        put_right(f, Rect::new(area.x, y, area.width.saturating_sub(2), 1), y, Line::styled(format!("{} Loading…", spinner(app)), dim()));
    }

    let gutter = 7u16;
    let col_w = (area.width.saturating_sub(gutter + 1)) / 7;
    if col_w < 6 || area.height < 10 {
        app.hits.extend(hits);
        return;
    }
    let x0 = area.x + gutter;
    let today = Local::now().date_naive();
    let head_y = area.y + 3;
    for d in 0..7u16 {
        let date = week + Duration::days(d as i64);
        let cx = x0 + d * col_w;
        let is_today = date == today;
        let style = if is_today { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { dim() };
        put(f, area, cx + 1, head_y, Line::from(vec![Span::styled(format!("{:>2}", date.day()), if is_today { Style::new().fg(Color::White).bg(BRAND).add_modifier(Modifier::BOLD) } else { bold() }), Span::styled(format!(" {}", date.format("%a")), style)]));
    }
    // All-day row.
    let allday_y = head_y + 1;
    for (i, e) in app.events.iter().enumerate().filter(|(_, e)| e.all_day) {
        for d in 0..7u16 {
            let date = week + Duration::days(d as i64);
            if e.start.date_naive() <= date && date < e.end.date_naive().max(e.start.date_naive() + Duration::days(1)) {
                let r = Rect::new(x0 + d * col_w + 1, allday_y, col_w - 1, 1);
                fill(f, r, if i == app.ev_sel { BRAND } else { OWN });
                put(f, r, r.x, r.y, Line::styled(format!(" {}", trunc(&e.subject, r.width.saturating_sub(1) as usize)), fg()));
                hits.push((r, Hit::CalEvent(i)));
            }
        }
    }
    let grid_y = allday_y + 1;
    put(f, area, area.x, grid_y, Line::styled("─".repeat(area.width as usize), faint()));
    let grid = Rect::new(area.x, grid_y + 1, area.width, area.bottom().saturating_sub(grid_y + 1));
    let hours = (grid.height / 2).max(1) as u32;
    // Pick the first visible hour: working hours by default, following the selection.
    let mut first = 8u32.min(24u32.saturating_sub(hours));
    if let Some(e) = app.events.get(app.ev_sel).filter(|e| !e.all_day) {
        let h = e.start.hour();
        if h < first {
            first = h;
        } else if h >= first + hours {
            first = (h + 1).saturating_sub(hours);
        }
    }
    first = first.min(24u32.saturating_sub(hours));
    for h in 0..hours {
        let gy = grid.y + (h * 2) as u16;
        put(f, grid, grid.x + 1, gy, Line::styled(hour_label(first + h), faint()));
        put(f, grid, x0, gy, Line::styled("┄".repeat((col_w * 7) as usize), Style::new().fg(Color::Rgb(52, 52, 52))));
    }
    for d in 0..=7u16 {
        for gy in grid.y..grid.bottom() {
            put(f, grid, x0 + d * col_w, gy, Line::styled("│", Style::new().fg(Color::Rgb(52, 52, 52))));
        }
    }
    let row_of = |mins: i64| -> i64 { (mins - first as i64 * 60).div_euclid(30) };
    // "Now" line.
    if (week..=end).contains(&today) {
        let now = Local::now();
        let r = row_of((now.hour() * 60 + now.minute()) as i64);
        if r >= 0 && (r as u16) < grid.height {
            let d = (today - week).num_days() as u16;
            put(f, grid, x0 + d * col_w + 1, grid.y + r as u16, Line::styled("━".repeat((col_w - 1) as usize), Style::new().fg(RED)));
        }
    }
    for d in 0..7u16 {
        let date = week + Duration::days(d as i64);
        let day: Vec<usize> = app.events.iter().enumerate().filter(|(_, e)| !e.all_day && e.start.date_naive() == date).map(|(i, _)| i).collect();
        // Side-by-side lanes for overlapping events.
        let mut lane_end: Vec<DateTime<Local>> = vec![];
        let mut lanes = vec![];
        for &i in &day {
            let e = &app.events[i];
            let lane = lane_end.iter().position(|t| *t <= e.start).unwrap_or(lane_end.len());
            if lane == lane_end.len() {
                lane_end.push(e.end);
            } else {
                lane_end[lane] = e.end;
            }
            lanes.push(lane);
        }
        let n = lane_end.len().max(1) as u16;
        for (k, &i) in day.iter().enumerate() {
            let e = &app.events[i];
            let start_m = (e.start.hour() * 60 + e.start.minute()) as i64;
            let end_m = if e.end.date_naive() > date { 24 * 60 } else { (e.end.hour() * 60 + e.end.minute()) as i64 };
            let r0 = row_of(start_m).max(0);
            let r1 = (row_of(end_m - 1) + 1).max(r0 + 1).min(grid.height as i64);
            if r0 >= grid.height as i64 || r1 <= 0 {
                continue;
            }
            let lw = ((col_w - 1) / n).max(3);
            let r = Rect::new(x0 + d * col_w + 1 + lanes[k] as u16 * lw, grid.y + r0 as u16, lw, (r1 - r0) as u16).intersection(grid);
            let selected = i == app.ev_sel;
            let bg = if selected { BRAND } else { OWN };
            f.render_widget(Clear, r);
            fill(f, r, bg);
            let style = if e.cancelled { dim().add_modifier(Modifier::CROSSED_OUT) } else { Style::new().fg(Color::White).add_modifier(Modifier::BOLD) };
            for yy in r.y..r.bottom() {
                put(f, r, r.x, yy, Line::styled("▎", Style::new().fg(if selected { Color::White } else { ACCENT })));
            }
            put(f, r, r.x + 1, r.y, Line::styled(trunc(&e.subject, r.width.saturating_sub(1) as usize), style));
            if r.height > 1 {
                put(f, r, r.x + 1, r.y + 1, Line::styled(trunc(&e.start.format("%-I:%M %p").to_string(), r.width.saturating_sub(1) as usize), Style::new().fg(Color::Rgb(215, 215, 240))));
            }
            hits.push((r, Hit::CalEvent(i)));
        }
    }
    app.hits.extend(hits);
}

pub fn calls(f: &mut Frame, area: Rect, app: &mut App) {
    let contacts = app.contacts();
    let Some(u) = contacts.get(app.call_sel) else {
        f.render_widget(Paragraph::new(vec![Line::raw("📞"), Line::raw(""), Line::styled("Chat with someone first and they'll show up here", dim())]).alignment(Alignment::Center), centered(area, area.width, 3));
        return;
    };
    let a = app.presence.get(&u.id).copied().unwrap_or_default();
    let card = centered(area, 56.min(area.width.saturating_sub(4)), 13);
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(faint()).style(Style::new().bg(PANEL));
    let inner = block.inner(card);
    f.render_widget(block, card);
    let big = Style::new().fg(Color::White).bg(avatar_color(&u.name)).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::raw(""),
        Line::styled("        ", big),
        Line::styled(format!("   {}   ", initials(&u.name)), big),
        Line::styled("        ", big),
        Line::raw(""),
        Line::styled(u.name.clone(), bold()),
        Line::styled(u.email.clone(), dim()),
        Line::from(vec![presence_dot(a), Span::styled(format!(" {}", a.status()), dim())]),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
    let by = inner.y + 9;
    let labels = [(" 📞 Call ", Some(Hit::Call(false))), (" 📹 Video ", Some(Hit::Call(true))), (" 💬 Chat (m) ", None)];
    let total: u16 = labels.iter().map(|(l, _)| w(l) as u16 + 2).sum();
    let mut x = inner.x + inner.width.saturating_sub(total) / 2;
    for (i, (label, hit)) in labels.into_iter().enumerate() {
        let r = Rect::new(x, by, w(label) as u16, 1);
        fill(f, r, if i == 0 { BRAND } else { SEL });
        put(f, inner, x, by, Line::styled(label, Style::new().fg(Color::White)));
        if let Some(h) = hit {
            app.hits.push((r, h));
        }
        x += r.width + 2;
    }
    let note = Rect::new(area.x + 2, card.bottom() + 1, area.width.saturating_sub(4), 2);
    f.render_widget(Paragraph::new("Audio and video can't run inside a terminal — calls hand off to the Teams app or teams.microsoft.com.").style(faint()).alignment(Alignment::Center).wrap(Wrap { trim: true }), note.intersection(area));
}

fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn file_icon(item: &DriveItem) -> &'static str {
    if item.folder {
        return "📁";
    }
    match item.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "doc" | "docx" | "md" | "txt" | "pdf" => "📄",
        "xls" | "xlsx" | "csv" => "📊",
        "ppt" | "pptx" => "📽",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "🖼",
        "mp4" | "mov" | "mkv" => "🎞",
        "zip" | "gz" | "tar" => "🗜",
        _ => "📄",
    }
}

pub fn files(f: &mut Frame, area: Rect, app: &mut App) {
    let mut hits = vec![];
    let crumbs: Vec<String> = app.file_path.iter().map(|p| p.1.clone()).collect();
    put(f, area, area.x + 2, area.y + 1, Line::styled(trunc(&crumbs.join("  ›  "), area.width.saturating_sub(4) as usize), bold()));
    if app.file_path.len() > 1 {
        let r = put_right(f, Rect::new(area.x, area.y + 1, area.width.saturating_sub(2), 1), area.y + 1, Line::styled(" ↑ Up ", Style::new().fg(FG).bg(PANEL)));
        hits.push((r, Hit::FileUp));
    }
    let total = area.width.saturating_sub(4) as usize;
    let (mw, bw, sw) = (14usize, if total > 80 { 22 } else { 0 }, 10usize);
    let nw = total.saturating_sub(mw + bw + sw + 3);
    let hy = area.y + 3;
    let mut head = format!("{}{}", pad("   Name", nw + 3), pad("Modified", mw));
    if bw > 0 {
        head.push_str(&pad("Modified by", bw));
    }
    head.push_str(&format!("{:>sw$}", "Size"));
    put(f, area, area.x + 2, hy, Line::styled(head, dim()));
    put(f, area, area.x + 2, hy + 1, Line::styled("─".repeat(total), faint()));
    let body = Rect::new(area.x + 2, hy + 2, total as u16, area.bottom().saturating_sub(hy + 2));
    if !app.files_loaded {
        put(f, body, body.x + 1, body.y, Line::styled(format!("{} Loading files…", spinner(app)), dim()));
    } else if app.files.is_empty() {
        put(f, body, body.x + 1, body.y, Line::styled("This folder is empty", dim()));
    }
    let visible = body.height as usize;
    let offset = if app.file_sel >= visible { app.file_sel + 1 - visible } else { 0 };
    for (i, item) in app.files.iter().enumerate().skip(offset).take(visible) {
        let y = body.y + (i - offset) as u16;
        let r = Rect::new(body.x, y, body.width, 1);
        if i == app.file_sel {
            fill(f, r, SEL);
        }
        let mut spans = vec![Span::raw(format!("{} ", file_icon(item))), Span::styled(pad(&item.name, nw), if item.folder { bold() } else { fg() }), Span::styled(pad(&short_time(item.modified), mw), dim())];
        if bw > 0 {
            spans.push(Span::styled(pad(&item.modified_by, bw), dim()));
        }
        spans.push(Span::styled(format!("{:>sw$}", if item.folder { String::new() } else { human_size(item.size) }), dim()));
        put(f, r, r.x, y, Line::from(spans));
        hits.push((r, Hit::FileRow(i)));
    }
    app.hits.extend(hits);
}
