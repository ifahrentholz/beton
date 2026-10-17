//! Text für die Darstellung: Markdown-Basisrendering (TUI-001) und Zeilenumbruch nach
//! Anzeigebreite.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::theme;

/// Markdown in logische Zeilen (ohne Umbruch): Überschriften fett, Listen mit `•`, Zitate mit
/// `│`, Code-Blöcke eingerückt und gedämpft, inline `**fett**`, `*kursiv*`, `` `Code` `` und
/// Links als Text.
pub fn markdown(text: &str, base: Style) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut fence: Option<String> = None;
    for raw in text.split('\n') {
        let trimmed = raw.trim_start();
        if let Some(marker) = &fence {
            if trimmed.starts_with(marker.as_str()) {
                fence = None;
            } else {
                out.push(Line::from(Span::styled(
                    format!("  {raw}"),
                    base.patch(theme::code()),
                )));
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed[..3].to_owned());
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let mut spans = inline(trimmed[level..].trim(), base.add_modifier(Modifier::BOLD));
            spans
                .iter_mut()
                .for_each(|s| s.style = s.style.add_modifier(Modifier::BOLD));
            out.push(Line::from(spans));
            continue;
        }
        let indent = &raw[..raw.len() - trimmed.len()];
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let mut spans = vec![Span::styled(format!("{indent}• "), base)];
            spans.extend(inline(rest, base));
            out.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix('>'))
        {
            let mut spans = vec![Span::styled("│ ", theme::dim())];
            spans.extend(inline(rest, base.patch(theme::dim())));
            out.push(Line::from(spans));
            continue;
        }
        let mut spans = Vec::new();
        if !indent.is_empty() {
            spans.push(Span::styled(indent.to_owned(), base));
        }
        spans.extend(inline(trimmed, base));
        out.push(Line::from(spans));
    }
    out
}

/// Inline-Auszeichnung einer Zeile.
fn inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let style = |bold: bool, italic: bool| {
        let mut s = base;
        if bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    };
    let flush = |buf: &mut String, out: &mut Vec<Span<'static>>, s: Style| {
        if !buf.is_empty() {
            out.push(Span::styled(std::mem::take(buf), s));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`'
            && let Some(end) = chars[i + 1..].iter().position(|x| *x == '`')
        {
            flush(&mut buf, &mut out, style(bold, italic));
            let code: String = chars[i + 1..i + 1 + end].iter().collect();
            out.push(Span::styled(code, base.patch(theme::code())));
            i += end + 2;
            continue;
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut buf, &mut out, style(bold, italic));
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_')
            && (italic || chars.get(i + 1).is_some_and(|n| !n.is_whitespace()))
            && (c == '*' || i == 0 || !chars[i - 1].is_alphanumeric())
        {
            flush(&mut buf, &mut out, style(bold, italic));
            italic = !italic;
            i += 1;
            continue;
        }
        if c == '['
            && let Some(close) = chars[i + 1..].iter().position(|x| *x == ']')
            && chars.get(i + close + 2) == Some(&'(')
            && let Some(end) = chars[i + close + 3..].iter().position(|x| *x == ')')
        {
            flush(&mut buf, &mut out, style(bold, italic));
            let label: String = chars[i + 1..i + 1 + close].iter().collect();
            let url: String = chars[i + close + 3..i + close + 3 + end].iter().collect();
            out.push(Span::styled(
                label,
                style(bold, italic).add_modifier(Modifier::UNDERLINED),
            ));
            out.push(Span::styled(format!(" ({url})"), theme::dim()));
            i += close + end + 4;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut out, style(bold, italic));
    out
}

/// Bricht eine Zeile auf `width` Spalten um (an Leerzeichen, zu lange Wörter hart). Folgezeilen
/// beginnen mit `indent` Leerzeichen.
pub fn wrap(line: &Line<'static>, width: u16, indent: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let indent = usize::from(indent).min(width.saturating_sub(1));
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0usize;
    // Wörter samt folgendem Leerraum, mit Stil.
    let mut words: Vec<(String, Style)> = Vec::new();
    for span in &line.spans {
        let mut w = String::new();
        for c in span.content.chars() {
            w.push(c);
            if c == ' ' {
                words.push((std::mem::take(&mut w), span.style));
            }
        }
        if !w.is_empty() {
            words.push((w, span.style));
        }
    }
    let push_row = |rows: &mut Vec<Line<'static>>, cur: &mut Vec<Span<'static>>| {
        rows.push(Line::from(std::mem::take(cur)));
    };
    for (word, style) in words {
        let ww = word.trim_end().width();
        if cur_w + ww > width && cur_w > indent_of(&rows, indent) {
            push_row(&mut rows, &mut cur);
            cur.push(Span::raw(" ".repeat(indent)));
            cur_w = indent;
        }
        if cur_w + ww > width {
            // Wort länger als die Zeile: hart teilen.
            let mut part = String::new();
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if cur_w + cw > width && cur_w > 0 {
                    cur.push(Span::styled(std::mem::take(&mut part), style));
                    push_row(&mut rows, &mut cur);
                    cur.push(Span::raw(" ".repeat(indent)));
                    cur_w = indent;
                }
                part.push(c);
                cur_w += cw;
            }
            cur.push(Span::styled(part, style));
            continue;
        }
        cur_w += word.width();
        cur.push(Span::styled(word, style));
    }
    rows.push(Line::from(cur));
    rows
}

fn indent_of(rows: &[Line<'static>], indent: usize) -> usize {
    if rows.is_empty() { 0 } else { indent }
}

/// Kürzt Text auf `width` Spalten, mit `…` am Ende.
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in text.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Füllt Text rechts mit Leerzeichen auf `width` Spalten auf (vorher gekürzt).
pub fn pad(text: &str, width: usize) -> String {
    let t = truncate(text, width);
    let w = t.width();
    format!("{t}{}", " ".repeat(width.saturating_sub(w)))
}

/// Anzeigebreite einer Zeile.
pub fn line_width(line: &Line<'_>) -> usize {
    line.spans.iter().map(|s| s.content.width()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn tui_001_markdown_basics() {
        let lines = markdown(
            "# Titel\n- **fett** und `code`\n> Zitat\n```rust\nlet x = 1;\n```\n[Link](http://x)",
            Style::default(),
        );
        assert_eq!(
            plain(&lines),
            [
                "Titel",
                "• fett und code",
                "│ Zitat",
                "  let x = 1;",
                "Link (http://x)"
            ]
        );
        assert!(
            lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            lines[1].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn tui_001_wrap_breaks_at_spaces_and_indents() {
        let line = Line::from("eins zwei drei vier");
        assert_eq!(
            plain(&wrap(&line, 10, 2)),
            ["eins zwei ", "  drei ", "  vier"]
        );
        let long = Line::from("abcdefghijkl");
        assert_eq!(plain(&wrap(&long, 5, 0)), ["abcde", "fghij", "kl"]);
        assert_eq!(
            truncate("src/middleware/rate-limit.ts", 22),
            "src/middleware/rate-l…"
        );
    }
}
