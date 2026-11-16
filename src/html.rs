//! Minimal HTML <-> plain text conversion for Teams message bodies.

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(j) = rest.find(';').filter(|j| *j <= 10) {
            let ent = &rest[1..j];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &rest[j + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    for q in ['"', '\''] {
        let pat = format!("{name}={q}");
        let mut from = 0;
        while let Some(i) = lower[from..].find(&pat) {
            let at = from + i;
            // must be preceded by whitespace so "data-alt" does not match "alt"
            if at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace() {
                let start = at + pat.len();
                let end = tag[start..].find(q)? + start;
                return Some(decode_entities(&tag[start..end]));
            }
            from = at + pat.len();
        }
    }
    None
}

fn nl(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn push_text(out: &mut String, s: &str) {
    if s.is_empty() {
        return;
    }
    let t = decode_entities(s).replace(['\r', '\n'], " ");
    // drop pure-whitespace runs at the start of a line
    if (out.is_empty() || out.ends_with('\n')) && t.trim().is_empty() {
        return;
    }
    out.push_str(&t);
}

/// Convert a Teams HTML message body to readable plain text.
pub fn to_text(html: &str) -> String {
    let mut out = String::new();
    let mut links: Vec<(String, usize)> = vec![];
    let mut quotes: Vec<usize> = vec![];
    let mut rest = html;
    while !rest.is_empty() {
        let Some(i) = rest.find('<') else {
            push_text(&mut out, rest);
            break;
        };
        push_text(&mut out, &rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('>') else {
            push_text(&mut out, &rest[i..]);
            break;
        };
        let tag = &after[..j];
        rest = &after[j + 1..];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match name.as_str() {
            "br" => out.push('\n'),
            "p" | "div" | "tr" | "ul" | "ol" | "table" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => nl(&mut out),
            "li" => {
                nl(&mut out);
                if !closing {
                    out.push_str("• ");
                }
            }
            "hr" => {
                nl(&mut out);
                out.push_str("────────\n");
            }
            "td" | "th" if closing => out.push_str("  "),
            "blockquote" => {
                if !closing {
                    nl(&mut out);
                    quotes.push(out.len());
                } else if let Some(start) = quotes.pop() {
                    let inner = out.split_off(start.min(out.len()));
                    for line in inner.trim().lines() {
                        out.push_str("▎ ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
            "a" => {
                if !closing {
                    links.push((attr(tag, "href").unwrap_or_default(), out.len()));
                } else if let Some((href, start)) = links.pop() {
                    let text = out[start.min(out.len())..].trim().to_string();
                    if href.starts_with("http") && !text.contains(href.trim_end_matches('/')) {
                        if text.is_empty() {
                            out.push_str(&href);
                        } else {
                            out.push_str(&format!(" ({href})"));
                        }
                    }
                }
            }
            "img" => {
                let alt = attr(tag, "alt").unwrap_or_default();
                if tag.contains("Emoji") && !alt.is_empty() {
                    out.push_str(&alt);
                } else {
                    let gif = tag.to_ascii_lowercase().contains("gif") || tag.contains("giphy") || alt.to_ascii_lowercase().contains("gif");
                    out.push_str(if gif { "[GIF]" } else { "[image]" });
                }
            }
            "emoji" if !closing => out.push_str(&attr(tag, "alt").unwrap_or_default()),
            "at" if !closing => out.push('@'),
            _ => {}
        }
    }
    let mut res = String::new();
    let mut blank = 0;
    for line in out.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        res.push_str(line);
        res.push('\n');
    }
    res.trim().to_string()
}

/// Sources of content (non-emoji) images in a message body, each with a GIF flag.
pub fn images(html: &str) -> Vec<(String, bool)> {
    let mut out = vec![];
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(j) = rest.find('>') else { break };
        let tag = &rest[..j];
        rest = &rest[j + 1..];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("img") && !lower.contains("emoji") {
            if let Some(src) = attr(tag, "src").filter(|s| s.starts_with("http")) {
                let gif = src.to_ascii_lowercase().contains("gif") || lower.contains("giphy") || attr(tag, "alt").unwrap_or_default().to_ascii_lowercase().contains("gif");
                out.push((src, gif));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_markup() {
        assert_eq!(to_text("<p>Hello <b>world</b></p><p>Second&nbsp;line &amp; more</p>"), "Hello world\nSecond line & more");
        assert_eq!(to_text("a<br>b<br/>c"), "a\nb\nc");
    }

    #[test]
    fn mentions_emoji_links() {
        assert_eq!(to_text("<div><at id=\"0\">Megan Bowen</at> hi</div>"), "@Megan Bowen hi");
        assert_eq!(to_text("ok <emoji id=\"smile\" alt=\"🙂\" title=\"Smile\"></emoji>"), "ok 🙂");
        assert_eq!(to_text("<a href=\"https://x.io/a\">docs</a>"), "docs (https://x.io/a)");
        assert_eq!(to_text("<a href=\"https://x.io\">https://x.io</a>"), "https://x.io");
        assert_eq!(to_text("<img src=\"https://x/y.png\" alt=\"image\">"), "[image]");
    }

    #[test]
    fn quotes_and_lists() {
        assert_eq!(to_text("<blockquote>Bob<br>hi</blockquote>yes"), "▎ Bob\n▎ hi\nyes");
        assert_eq!(to_text("<ul><li>one</li><li>two</li></ul>"), "• one\n• two");
    }

    #[test]
    fn escape_roundtrip() {
        let s = "a < b && \"c\"\nnext";
        assert_eq!(to_text(&escape(s)), s);
    }

    #[test]
    fn extracts_content_images() {
        let html = "<div>see <img src=\"https://x/a.png\" alt=\"pic\"> and <img itemtype=\"Emoji\" src=\"https://e/s.png\" alt=\"\u{1f642}\"> <img src=\"https://media.giphy.com/x.gif\" alt=\"gif\"></div>";
        assert_eq!(images(html), vec![("https://x/a.png".to_string(), false), ("https://media.giphy.com/x.gif".to_string(), true)]);
        assert_eq!(to_text("<img src=\"https://media.giphy.com/x.gif\" alt=\"gif\">"), "[GIF]");
        assert_eq!(to_text("<img src=\"https://x/y.png\" alt=\"image\">"), "[image]");
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("&#39;x&#x41; &bogus; & é"), "'xA &bogus; & é");
    }
}
