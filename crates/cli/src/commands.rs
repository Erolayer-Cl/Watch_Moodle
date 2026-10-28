use std::io::IsTerminal;

use anyhow::{Result, bail};
use console::style;
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use inquire::{Password, PasswordDisplayMode, Text};
use revision_core::config::normalize_url;
use revision_core::{Config, Entry, Keyring, Kind, Paths, Query, Revision, SyncReport, now};

use crate::{Command, ui};

const PARALLEL_DOWNLOADS: usize = 3;

pub async fn run(command: Command) -> Result<()> {
    let paths = Paths::standard()?;
    match command {
        Command::Login { url, usuario } => login(&paths, url, usuario).await,
        Command::Logout => logout(&paths),
        command => {
            let mut rev = Revision::open(&paths, Box::new(Keyring))?;
            match command {
                Command::Revisar { curso, no_marcar } => revisar(&mut rev, curso, no_marcar).await,
                Command::Archivos { curso, offline } => archivos(&mut rev, curso, offline).await,
                Command::Tareas { todas, offline } => tareas(&mut rev, todas, offline).await,
                Command::Anuncios { curso, cantidad, offline } => anuncios(&mut rev, curso, cantidad, offline).await,
                Command::Cursos { offline } => cursos(&mut rev, offline).await,
                Command::Login { .. } | Command::Logout => unreachable!(),
            }
        }
    }
}

async fn login(paths: &Paths, url: Option<String>, usuario: Option<String>) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("Para iniciar sesión ejecuta `revision login` en una terminal: la contraseña se escribe a mano.");
    }
    let previous = Config::load(&paths.config_file)?;
    let url = match url {
        Some(url) => url,
        None => {
            let mut prompt =
                Text::new("Dirección del aula virtual:").with_placeholder("https://aulavirtual.tu-universidad.cl");
            if let Some(p) = &previous {
                prompt = prompt.with_default(&p.url);
            }
            prompt.prompt()?
        }
    };
    // Una URL inválida se avisa antes de pedir la contraseña.
    let url = normalize_url(&url)?;
    let username = match usuario {
        Some(u) => u,
        None => {
            let mut prompt = Text::new("Usuario:");
            if let Some(p) = &previous {
                prompt = prompt.with_default(&p.username);
            }
            prompt.prompt()?
        }
    };
    let password =
        Password::new("Contraseña:").without_confirmation().with_display_mode(PasswordDisplayMode::Masked).prompt()?;

    let pb = ui::spinner("Iniciando sesión…");
    let result = Revision::login(paths, Box::new(Keyring), &url, username.trim(), &password).await;
    pb.finish_and_clear();
    let mut rev = result?;
    println!("{} Sesión iniciada en {} como {}.", style("✓").green(), rev.config().url, rev.config().username);
    println!("  La contraseña quedó en el llavero del sistema, no en texto plano.");

    let report = sync(&mut rev, "Revisando tus cursos…").await?;
    print_courses(&rev)?;
    if report.baseline {
        println!("\n{} {} elementos guardados como punto de partida.", style("✓").green(), report.items);
        println!("  Desde ahora, {} te mostrará solo lo nuevo.", style("revision revisar").cyan());
    } else if report.unseen > 0 {
        println!("\nTienes {} novedad(es) sin ver: ejecuta {}.", report.unseen, style("revision revisar").cyan());
    }
    Ok(())
}

fn logout(paths: &Paths) -> Result<()> {
    if Revision::logout(paths, &Keyring)? {
        println!("{} Token y contraseña borrados del llavero.", style("✓").green());
    } else {
        println!("No había una sesión guardada.");
    }
    Ok(())
}

async fn sync(rev: &mut Revision, message: &str) -> Result<SyncReport> {
    let pb = ui::spinner(message);
    let result = rev.sync().await;
    pb.finish_and_clear();
    let report = result?;
    for warning in &report.warnings {
        eprintln!("{} {warning}", style("aviso:").yellow());
    }
    Ok(report)
}

fn course_filter(rev: &Revision, curso: Option<String>) -> Result<Option<i64>> {
    Ok(match curso {
        Some(q) => Some(rev.store().find_course(&q)?.id),
        None => None,
    })
}

async fn revisar(rev: &mut Revision, curso: Option<String>, no_marcar: bool) -> Result<()> {
    let report = sync(rev, "Revisando Moodle…").await?;
    if report.baseline {
        println!(
            "{} Primera revisión: {} elementos guardados como punto de partida. La próxima vez verás lo nuevo.",
            style("✓").green(),
            report.items
        );
        return Ok(());
    }
    let course_id = course_filter(rev, curso)?;
    let entries = rev.store().entries(&Query { unseen_only: true, course_id, ..Query::default() })?;
    if entries.is_empty() {
        println!("{} Sin novedades desde tu última revisión.", style("✓").green());
        return Ok(());
    }
    ui::print_news(&entries, now());

    let files: Vec<&Entry> = entries.iter().filter(|e| e.item.file.is_some()).collect();
    if !files.is_empty() {
        // Ctrl+C: se sale sin marcar nada como visto.
        let Some(keys) = ui::choose_files(&files, false)? else { return Ok(()) };
        download(rev, &keys).await?;
    }
    if no_marcar {
        println!("(Las novedades siguen marcadas como no vistas.)");
    } else {
        let keys: Vec<String> = entries.iter().map(|e| e.item.key.clone()).collect();
        rev.store_mut().mark_seen(&keys)?;
    }
    Ok(())
}

async fn archivos(rev: &mut Revision, curso: Option<String>, offline: bool) -> Result<()> {
    if !offline {
        sync(rev, "Revisando Moodle…").await?;
    }
    let course_id = course_filter(rev, curso)?;
    let mut entries = rev.store().entries(&Query { kind: Some(Kind::File), course_id, ..Query::default() })?;
    if entries.is_empty() {
        println!("No hay archivos.");
        return Ok(());
    }
    entries.sort_by(|a, b| {
        (&a.item.course, &a.item.section, &a.item.title).cmp(&(&b.item.course, &b.item.section, &b.item.title))
    });
    let refs: Vec<&Entry> = entries.iter().collect();
    if let Some(keys) = ui::choose_files(&refs, true)? {
        download(rev, &keys).await?;
    }
    Ok(())
}

async fn tareas(rev: &mut Revision, todas: bool, offline: bool) -> Result<()> {
    if !offline {
        sync(rev, "Revisando Moodle…").await?;
    }
    let now = now();
    let mut list: Vec<Entry> = rev
        .store()
        .entries(&Query { kind: Some(Kind::Assignment), ..Query::default() })?
        .into_iter()
        .filter(|e| e.item.due.is_none_or(|d| d >= now) && (todas || e.item.status.as_deref() != Some("submitted")))
        .collect();
    if list.is_empty() {
        println!("{} No tienes entregas pendientes.", style("✓").green());
        return Ok(());
    }
    list.sort_by_key(|e| (e.item.due.is_none(), e.item.due));
    ui::print_assignments(&list, now);
    Ok(())
}

async fn anuncios(rev: &mut Revision, curso: Option<String>, cantidad: usize, offline: bool) -> Result<()> {
    if !offline {
        sync(rev, "Revisando Moodle…").await?;
    }
    let course_id = course_filter(rev, curso)?;
    let mut list = rev.store().entries(&Query { kind: Some(Kind::Announcement), course_id, ..Query::default() })?;
    if list.is_empty() {
        println!("No hay anuncios.");
        return Ok(());
    }
    list.sort_by_key(|e| std::cmp::Reverse(e.item.created.unwrap_or(e.item.modified)));
    list.truncate(cantidad);
    let files = rev.store().entries(&Query { kind: Some(Kind::File), course_id, ..Query::default() })?;
    ui::print_announcements(&list, &files);
    // Leer un anuncio completo cuenta como verlo.
    let keys: Vec<String> = list.iter().map(|e| e.item.key.clone()).collect();
    rev.store_mut().mark_seen(&keys)?;
    Ok(())
}

async fn cursos(rev: &mut Revision, offline: bool) -> Result<()> {
    if !offline {
        sync(rev, "Revisando Moodle…").await?;
    }
    print_courses(rev)
}

fn print_courses(rev: &Revision) -> Result<()> {
    let courses = rev.store().courses()?;
    if courses.is_empty() {
        println!("No estás inscrito en ningún curso visible.");
        return Ok(());
    }
    println!("\n{}", style(format!("Tus cursos ({})", courses.len())).bold());
    for c in &courses {
        let unseen =
            if c.unseen > 0 { style(format!(" · {} sin ver", c.unseen)).yellow().to_string() } else { String::new() };
        println!("  {} {}{unseen}", c.fullname, style(format!("[{}]", c.shortname)).dim());
        if !c.teachers.is_empty() {
            println!("    {}", style(c.teachers.join(", ")).dim());
        }
    }
    Ok(())
}

/// Descarga en paralelo con una barra por archivo y registra lo descargado.
async fn download(rev: &mut Revision, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let jobs = rev.plan_downloads(keys)?;
    let multi = MultiProgress::new();
    let bar_style = ProgressStyle::with_template("  {bar:24.cyan/blue} {bytes:>10} / {total_bytes:<10} {wide_msg}")?
        .progress_chars("━╸─");
    let client = rev.client().clone();
    let results: Vec<_> = stream::iter(jobs.iter().map(|job| {
        let pb = multi.add(ProgressBar::new(job.file.size));
        pb.set_style(bar_style.clone());
        pb.set_message(job.title.clone());
        let client = &client;
        async move {
            let result = revision_core::download::fetch(client, job, |done, total| {
                if let Some(total) = total {
                    pb.set_length(total);
                }
                pb.set_position(done);
            })
            .await;
            pb.finish_and_clear();
            (job, result)
        }
    }))
    .buffer_unordered(PARALLEL_DOWNLOADS)
    .collect()
    .await;

    let dir = rev.config().download_dir();
    let mut ok = 0;
    for (job, result) in results {
        match result {
            Ok(()) => {
                rev.record_download(job)?;
                ok += 1;
                println!("{} {}", style("↓").green(), job.target.strip_prefix(&dir).unwrap_or(&job.target).display());
            }
            Err(e) => eprintln!("{} {}: {e}", style("✗").red(), job.title),
        }
    }
    if ok > 0 {
        println!("{} {ok} archivo(s) en {}", style("✓").green(), dir.display());
    }
    Ok(())
}
