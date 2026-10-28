//! Formato de fechas, tamaños y conversión de HTML (anuncios, enunciados) a texto.

use chrono::{Local, TimeZone};

/// `27-10-2026 14:30` en hora local.
pub fn fmt_datetime(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).earliest().map(|d| d.format("%d-%m-%Y %H:%M").to_string()).unwrap_or_default()
}

/// `27-10 14:30` en hora local.
pub fn fmt_short(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).earliest().map(|d| d.format("%d-%m %H:%M").to_string()).unwrap_or_default()
}

/// `1,2 MB`, con coma decimal.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
}

/// Convierte el HTML de Moodle en texto legible: respeta párrafos y listas,
/// agrega la dirección de los enlaces y descarta scripts y estilos.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut link: Option<(usize, String)> = None;
    let mut skipping: Option<&'static str> = None;

    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skipping.is_none() {
                push_text(&mut out, rest);
            }
            break;
        };
        if skipping.is_none() {
            push_text(&mut out, &rest[..lt]);
        }
        let after = &rest[lt..];
        if let Some(comment) = after.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let end = tag_end(after);
        let tag = after[1..end].trim();
        rest = after.get(end + 1..).unwrap_or("");

        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if let Some(skip) = skipping {
            if closing && name == skip {
                skipping = None;
            }
            continue;
        }
        match (name.as_str(), closing) {
            ("script", false) => skipping = Some("script"),
            ("style", false) => skipping = Some("style"),
            ("br", _) => out.push('\n'),
            ("p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "blockquote" | "pre", _) => {
                blank_line(&mut out)
            }
            ("div" | "tr" | "hr", _) => new_line(&mut out),
            ("li", false) => {
                new_line(&mut out);
                out.push_str("• ");
            }
            ("td" | "th", false) => {
                if !out.ends_with(['\n', ' ']) && !out.is_empty() {
                    out.push(' ');
                }
            }
            ("a", false) => link = attr(tag, "href").map(|href| (out.len(), decode_entities(&href))),
            ("a", true) => {
                if let Some((start, href)) = link.take() {
                    let text = out[start..].trim();
                    if !href.is_empty() && !href.starts_with('#') && text != href {
                        out.push_str(&format!(" ({href})"));
                    }
                }
            }
            ("img", _) => {
                if let Some(alt) = attr(tag, "alt").filter(|a| !a.trim().is_empty()) {
                    out.push_str(&format!("[imagen: {}]", decode_entities(alt.trim())));
                }
            }
            _ => {}
        }
    }

    // Limpieza: espacios al final de cada línea y como máximo una línea en blanco seguida.
    let mut clean = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines().map(str::trim) {
        if line.is_empty() {
            blank += 1;
            if blank > 1 || clean.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        clean.push_str(line);
        clean.push('\n');
    }
    clean.trim_end().to_string()
}

/// Índice del `>` que cierra la etiqueta, ignorando los que van entre comillas.
fn tag_end(s: &str) -> usize {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '>') => return i,
            _ => {}
        }
    }
    s.len()
}

fn push_text(out: &mut String, raw: &str) {
    for c in decode_entities(raw).chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

fn new_line(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn blank_line(out: &mut String) {
    new_line(out);
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        let after_name = &lower[start + name.len()..];
        let rest = after_name.trim_start();
        let boundary = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        if boundary && rest.starts_with('=') {
            let eq = start + name.len() + (after_name.len() - rest.len());
            let value = tag[eq + 1..].trim_start();
            return Some(match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default().to_string(),
                _ => value.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default().to_string(),
            });
        }
        from = start + name.len();
    }
    None
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';').filter(|&end| end <= 10).and_then(|end| Some((end, entity(&after[..end])?))) {
            Some((end, c)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "aacute" => 'á',
        "eacute" => 'é',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "Aacute" => 'Á',
        "Eacute" => 'É',
        "Iacute" => 'Í',
        "Oacute" => 'Ó',
        "Uacute" => 'Ú',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "uuml" => 'ü',
        "Uuml" => 'Ü',
        "iexcl" => '¡',
        "iquest" => '¿',
        "laquo" => '«',
        "raquo" => '»',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "deg" => '°',
        "ordm" => 'º',
        "ordf" => 'ª',
        "euro" => '€',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convierte_html_de_anuncios() {
        let html = r#"<p>Estimados:</p><p>La prueba se cambia a la <b>sala&nbsp;B-12</b>.</p>
            <ul><li>Traer calculadora</li><li>Ver <a href="https://zoom.us/j/1">el link</a></li></ul>
            <script>alert(1)</script><!-- nota --><p>Saludos,<br>Ana &amp; equipo</p>"#;
        assert_eq!(
            html_to_text(html),
            "Estimados:\n\nLa prueba se cambia a la sala B-12.\n\n• Traer calculadora\n• Ver el link (https://zoom.us/j/1)\n\nSaludos,\nAna & equipo"
        );
    }

    #[test]
    fn decodifica_entidades() {
        assert_eq!(decode_entities("Cr&eacute;ditos &#241; &#xF1; &foo; & x"), "Créditos ñ ñ &foo; & x");
        assert_eq!(html_to_text("a <img alt=\"gráfico\" src=x> b"), "a [imagen: gráfico] b");
        assert_eq!(html_to_text("texto sin etiquetas"), "texto sin etiquetas");
        assert_eq!(html_to_text("roto <b"), "roto");
    }

    #[test]
    fn tamanos() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1,5 KB");
        assert_eq!(human_size(1_258_291), "1,2 MB");
    }
}
