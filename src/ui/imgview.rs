//! Render a decoded image into the terminal using Unicode half-blocks.
//! Each character cell shows two stacked pixels: the upper half (▀, foreground)
//! is the top pixel, the lower half (background) is the bottom pixel. Needs only
//! truecolor, which every modern terminal supports.
use super::*;
use ::image::{DynamicImage, GenericImageView};

pub fn render(img: &DynamicImage, area: Rect, bg: Color) -> Vec<Line<'static>> {
    if area.width == 0 || area.height == 0 {
        return vec![];
    }
    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return vec![];
    }
    // Fit within `area` columns and `area.height * 2` pixel rows, preserving aspect.
    let scale = (area.width as f32 / iw as f32).min((area.height as f32 * 2.0) / ih as f32);
    let pw = ((iw as f32 * scale).round() as u32).clamp(1, area.width as u32);
    let ph = ((ih as f32 * scale).round() as u32).clamp(1, area.height as u32 * 2);
    let small = img.resize_exact(pw, ph, ::image::imageops::FilterType::Triangle).to_rgba8();
    let (bg_r, bg_g, bg_b) = match bg {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (30, 30, 30),
    };
    let blend = |px: &::image::Rgba<u8>| -> Color {
        let a = px[3] as f32 / 255.0;
        let m = |c: u8, b: u8| (c as f32 * a + b as f32 * (1.0 - a)).round() as u8;
        Color::Rgb(m(px[0], bg_r), m(px[1], bg_g), m(px[2], bg_b))
    };
    let pad = (area.width as u32).saturating_sub(pw) as usize / 2;
    let rows = ph.div_ceil(2);
    let mut lines = Vec::with_capacity(rows as usize);
    for ry in 0..rows {
        let mut spans = Vec::with_capacity(pw as usize + 1);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        for x in 0..pw {
            let top = blend(small.get_pixel(x, ry * 2));
            let bottom = if ry * 2 + 1 < ph { blend(small.get_pixel(x, ry * 2 + 1)) } else { Color::Rgb(bg_r, bg_g, bg_b) };
            spans.push(Span::styled("▀", Style::new().fg(top).bg(bottom)));
        }
        lines.push(Line::from(spans));
    }
    lines
}
