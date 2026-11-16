//! Render a decoded image into the terminal using Unicode half-blocks.
//! Each character cell shows two stacked pixels: the upper half (▀, foreground)
//! is the top pixel, the lower half (background) is the bottom pixel. Needs only
//! truecolor, which every modern terminal supports.
use super::*;
use ::image::{DynamicImage, GenericImageView};
use base64::Engine;

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

/// Encode an image as PNG for transmission, bounding the size (Kitty scales to the cell box).
pub fn encode_png(img: &DynamicImage) -> Vec<u8> {
    let max = img.width().max(img.height());
    let scaled;
    let src = if max > 1600 {
        let s = 1600.0 / max as f32;
        scaled = img.resize((img.width() as f32 * s) as u32, (img.height() as f32 * s) as u32, ::image::imageops::FilterType::Triangle);
        &scaled
    } else {
        img
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = src.write_to(&mut buf, ::image::ImageFormat::Png);
    buf.into_inner()
}

/// Kitty graphics escape to transmit a PNG and place it at (x, y), scaled into cols x rows cells.
pub fn kitty_place(id: u32, cols: u16, rows: u16, x: u16, y: u16, png: &[u8]) -> Vec<u8> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    let mut out = format!("\x1b[{};{}H", y + 1, x + 1).into_bytes();
    let data = b64.as_bytes();
    if data.is_empty() {
        return out;
    }
    let mut chunks = data.chunks(4096).peekable();
    let mut first = true;
    while let Some(ch) = chunks.next() {
        let m = if chunks.peek().is_some() { 1 } else { 0 };
        if first {
            // f=100 PNG, a=T transmit+display, C=1 keep cursor, q=2 suppress responses.
            out.extend(format!("\x1b_Gf=100,a=T,i={id},q=2,c={cols},r={rows},C=1,m={m};").into_bytes());
            first = false;
        } else {
            out.extend(format!("\x1b_Gm={m};").into_bytes());
        }
        out.extend_from_slice(ch);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// Kitty escape to delete the image (and its placement) with the given id.
pub fn kitty_delete(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=i,i={id},q=2\x1b\\").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_sequence_is_well_formed() {
        let seq = kitty_place(1337, 20, 10, 4, 2, &[1u8; 5000]);
        let s = String::from_utf8_lossy(&seq);
        assert!(s.starts_with("\x1b[3;5H"), "cursor position first");
        assert!(s.contains("f=100,a=T,i=1337,q=2,c=20,r=10,C=1,m=1;"), "header keys");
        // 5000 bytes -> base64 ~6668 chars -> 2 chunks: first m=1, continuation m=0.
        assert!(s.contains("\x1b_Gm=0;"), "final continuation chunk");
        assert_eq!(s.matches("\x1b\\").count(), 2, "two chunk terminators");
        assert_eq!(kitty_delete(1337), b"\x1b_Ga=d,d=i,i=1337,q=2\x1b\\");
    }

    #[test]
    fn single_chunk_small_image() {
        let seq = kitty_place(1, 2, 1, 0, 0, b"hello");
        let s = String::from_utf8_lossy(&seq);
        assert!(s.contains("m=0;"), "single chunk is final");
        assert_eq!(s.matches("\x1b_G").count(), 1);
    }
}
