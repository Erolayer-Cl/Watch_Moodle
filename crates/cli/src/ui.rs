use std::collections::BTreeMap;
use std::fmt;
use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::{InquireError, MultiSelect};
use revision_core::text::{fmt_datetime, fmt_short, html_to_text, human_size};
use revision_core::{Change, Entry, Kind};

/// Orden en que se muestran los tipos dentro de cada curso.
const KIND_ORDER: [Kind; 4] = [Kind::Announcement, Kind::Assignment, Kind::File, Kind::Activity];
const EXCERPT_CHARS: usize = 140;

pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("plantilla válida"));
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn print_news(entries: &[Entry], now: i64) {
    let mut by_course: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        by_course.entry(&e.item.course).or_default().push(e);
    }
    let total = entries.len();
    println!("{}", style(format!("{total} novedad{}", if total == 1 { "" } else { "es" })).bold());
    for (course, items) in by_course {
        println!("\n{}", style(course).bold().underlined());
        for kind in KIND_ORDER {
            let group: Vec<&Entry> = items.iter().copied().filter(|e| e.item.kind == kind).collect();
            if group.is_empty() {
                continue;
            }
            println!("  {}", style(kind.label()).dim());
            for e in group {
                print_news_line(e, now);
            }
        }
    }
    println!();
}

fn print_news_line(e: &Entry, now: i64) {
    let item = &e.item;
    let marker = match e.change {
        Some(Change::Updated) => style("~").yellow().bold(),
        _ => style("+").green().bold(),
    };
    let mut details: Vec<String> = Vec::new();
    if let (Some(Change::Updated), Some(detail)) = (e.change, &e.change_detail) {
        details.push(style(detail).yellow().to_string());
    }
    match item.kind {
        Kind::File => {
            if let Some(f) = &item.file {
                details.push(human_size(f.size));
            }
            if let Some(ctx) = item.context.as_ref().filter(|c| **c != item.title) {
                details.push(format!("en «{ctx}»"));
            }
            if let Some(section) = &item.section {
                details.push(section.clone());
            }
        }
        Kind::Announcement => {
            if let Some(author) = &item.author {
                details.push(author.clone());
            }
            if let Some(created) = item.created {
                details.push(fmt_short(created));
            }
        }
        Kind::Assignment => {
            details.push(due_text(item.due, now));
            if let Some(status) = item.status_label() {
                details.push(status.to_string());
            }
        }
        Kind::Activity => {
            if let Some(module) = &item.module {
                details.push(module.clone());
            }
            for d in &item.dates {
                details.push(format!("{} {}", d.label.to_lowercase(), fmt_short(d.timestamp)));
            }
        }
    }
    let sep = style(" · ").dim().to_string();
    let details = if details.is_empty() { String::new() } else { format!("{sep}{}", details.join(&sep)) };
    println!("    {marker} {}{details}", item.title);

    if let (Kind::Announcement, Some(body)) = (item.kind, &item.body_html) {
        let text = html_to_text(body).replace('\n', " ");
        if !text.is_empty() {
            let excerpt: String = text.chars().take(EXCERPT_CHARS).collect();
            let more = if text.chars().count() > EXCERPT_CHARS { "…" } else { "" };
            println!("      {}", style(format!("{excerpt}{more}")).dim());
        }
    }
}

pub fn due_text(due: Option<i64>, now: i64) -> String {
    let Some(due) = due else { return "sin fecha de entrega".into() };
    let left = due - now;
    let relative = if left < 0 {
        "vencida".to_string()
    } else if left < 3600 {
        "en menos de 1 hora".into()
    } else if left < 86_400 {
        format!("en {} h", left / 3600)
    } else {
        let days = left / 86_400;
        format!("en {days} día{}", if days == 1 { "" } else { "s" })
    };
    format!("vence {} ({relative})", fmt_short(due))
}

struct FileOption {
    key: String,
    label: String,
}

impl fmt::Display for FileOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Lista de selección múltiple. Devuelve las claves elegidas, o `None` si el
/// usuario interrumpió con Ctrl+C. Esc equivale a no descargar nada.
pub fn choose_files(entries: &[&Entry], with_section: bool) -> anyhow::Result<Option<Vec<String>>> {
    let options: Vec<FileOption> =
        entries.iter().map(|e| FileOption { key: e.item.key.clone(), label: file_label(e, with_section) }).collect();
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        if with_section {
            for o in &options {
                println!("  {o}");
            }
        }
        println!("(Para elegir archivos y descargarlos, ejecuta el comando en una terminal interactiva.)");
        return Ok(Some(Vec::new()));
    }
    let prompt = MultiSelect::new("¿Qué archivos descargar?", options).with_page_size(15).with_help_message(
        "↑↓ mover · espacio marcar · → todos · ← ninguno · escribe para filtrar · enter descargar · esc omitir",
    );
    match prompt.prompt() {
        Ok(selected) => Ok(Some(selected.into_iter().map(|o| o.key).collect())),
        Err(InquireError::OperationCanceled) => Ok(Some(Vec::new())),
        Err(InquireError::OperationInterrupted) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn file_label(e: &Entry, with_section: bool) -> String {
    let item = &e.item;
    let size = item.file.as_ref().map(|f| human_size(f.size)).unwrap_or_default();
    let mut label = if with_section {
        format!("{} › {} › {} ({size})", item.course, item.section.as_deref().unwrap_or("-"), item.title)
    } else {
        format!("{} · {} ({size})", item.course, item.title)
    };
    if e.downloaded.is_some() {
        label.push_str(if e.download_outdated() { "  [descargado, hay versión nueva]" } else { "  [descargado]" });
    } else if !with_section {
        label.push_str(if e.change == Some(Change::Updated) { "  [actualizado]" } else { "  [nuevo]" });
    }
    label
}

pub fn print_assignments(list: &[Entry], now: i64) {
    println!("{}", style("Próximas entregas").bold());
    for e in list {
        let item = &e.item;
        let text = due_text(item.due, now);
        let due = match item.due {
            Some(d) if d - now < 86_400 => style(text).red().bold(),
            Some(d) if d - now < 3 * 86_400 => style(text).yellow(),
            _ => style(text),
        };
        let status = item.status_label().map(|s| format!(" · {s}")).unwrap_or_default();
        println!("\n  {}{}", style(&item.title).bold(), style(status).dim());
        println!("    {} · {due}", item.course);
        if let Some(url) = &item.web_url {
            println!("    {}", style(url).dim());
        }
    }
}

pub fn print_announcements(list: &[Entry], files: &[Entry]) {
    for e in list {
        let item = &e.item;
        let mut meta = vec![item.course.clone()];
        meta.extend(item.created.map(fmt_datetime));
        meta.extend(item.author.clone());
        let new = if e.seen { String::new() } else { format!(" {}", style("nuevo").green().bold()) };
        println!("\n{}{new}", style(&item.title).bold());
        println!("{}", style(meta.join(" · ")).dim());
        if let Some(body) = &item.body_html {
            println!("\n{}", html_to_text(body));
        }
        let attachments: Vec<&str> = files
            .iter()
            .filter(|f| f.item.parent.as_deref() == Some(item.key.as_str()))
            .map(|f| f.item.title.as_str())
            .collect();
        if !attachments.is_empty() {
            println!("\n{} {}", style("Adjuntos:").dim(), attachments.join(", "));
        }
        if let Some(url) = &item.web_url {
            println!("{}", style(url).dim());
        }
        println!("{}", style("─".repeat(60)).dim());
    }
}

#[cfg(test)]
mod tests {
    use revision_core::model::{Download, FileRef, Item, Source};

    use super::*;

    fn entry(key: &str, kind: Kind, change: Change) -> Entry {
        let mut item = Item::new(key.into(), kind, Source::Contents, 1, "Cálculo II", format!("{key}.pdf"));
        item.section = Some("Unidad 1".into());
        Entry {
            item,
            change: Some(change),
            change_detail: None,
            seen: false,
            first_seen: 0,
            last_changed: 0,
            downloaded: None,
        }
    }

    fn with_file(mut e: Entry, modified: i64) -> Entry {
        e.item.file = Some(FileRef {
            filename: e.item.title.clone(),
            size: 1536,
            url: String::new(),
            mimetype: None,
            modified,
            subdir: vec![],
        });
        e
    }

    #[test]
    fn fechas_de_entrega_relativas() {
        let now = 1_000_000;
        assert_eq!(due_text(None, now), "sin fecha de entrega");
        assert!(due_text(Some(now - 1), now).ends_with("(vencida)"));
        assert!(due_text(Some(now + 600), now).ends_with("(en menos de 1 hora)"));
        assert!(due_text(Some(now + 5 * 3600), now).ends_with("(en 5 h)"));
        assert!(due_text(Some(now + 86_400 + 60), now).ends_with("(en 1 día)"));
        assert!(due_text(Some(now + 3 * 86_400), now).ends_with("(en 3 días)"));
    }

    #[test]
    fn etiquetas_de_archivos() {
        let nuevo = with_file(entry("guia", Kind::File, Change::New), 1);
        assert_eq!(file_label(&nuevo, false), "Cálculo II · guia.pdf (1,5 KB)  [nuevo]");
        assert_eq!(file_label(&nuevo, true), "Cálculo II › Unidad 1 › guia.pdf (1,5 KB)");

        let mut bajado = with_file(entry("pauta", Kind::File, Change::Updated), 2);
        bajado.downloaded = Some(Download { path: "/x".into(), fingerprint: "1:1536".into(), at: 0 });
        assert!(file_label(&bajado, false).ends_with("[descargado, hay versión nueva]"));
        bajado.downloaded.as_mut().unwrap().fingerprint = bajado.item.fingerprint();
        assert!(file_label(&bajado, false).ends_with("  [descargado]"));
    }

    #[test]
    fn muestra_todos_los_tipos_sin_fallar() {
        let mut anuncio = entry("aviso", Kind::Announcement, Change::New);
        anuncio.item.body_html = Some("<p>La prueba se cambia a la <b>sala B-12</b>.</p>".into());
        anuncio.item.author = Some("Ana Rojas".into());
        let mut tarea = entry("tarea", Kind::Assignment, Change::Updated);
        tarea.change_detail = Some("entrega: 01-11-2026 23:59 → 03-11-2026 23:59".into());
        tarea.item.due = Some(2_000_000);
        tarea.item.status = Some("new".into());
        let actividad = entry("control", Kind::Activity, Change::New);
        let entries = [anuncio, tarea, with_file(entry("guia", Kind::File, Change::New), 1), actividad];
        print_news(&entries, 1_000_000);
        print_assignments(&entries[1..2], 1_000_000);
        print_announcements(&entries[..1], &entries);
    }
}
