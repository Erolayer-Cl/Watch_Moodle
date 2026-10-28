//! Consulta Moodle y convierte las respuestas en una lista plana de [`Item`].

use std::collections::{HashMap, HashSet};

use futures::stream::{self, StreamExt};

use crate::Result;
use crate::model::{FileRef, Item, Kind, LabeledDate, Source};
use crate::moodle::Client;
use crate::moodle::types::{Assignment, Discussion, MoodleFile, Section};

const CONCURRENCY: usize = 4;
const DISCUSSIONS_PER_FORUM: u32 = 25;
/// Se consulta el estado de entrega de tareas vigentes o vencidas hace menos de esto.
const STATUS_WINDOW: i64 = 14 * 86_400;
/// Módulos que no se reportan como actividad: los recursos y carpetas se reportan
/// como archivos, las tareas y los anuncios por su propia API.
const SKIP_MODULES: &[&str] = &["resource", "folder", "assign", "label", "qbank", "subsection"];

#[derive(Debug, Clone)]
pub struct CourseInfo {
    pub id: i64,
    pub shortname: String,
    pub fullname: String,
    pub teachers: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Fetched {
    pub courses: Vec<CourseInfo>,
    pub items: Vec<Item>,
    /// Consultas que terminaron bien, por curso: lo que falte de ellas fue eliminado.
    pub complete: HashSet<(i64, Source)>,
    pub warnings: Vec<String>,
}

/// Descarga el estado actual de los cursos. Los errores de un curso o de un tipo de
/// contenido se informan como advertencias y no detienen la revisión.
pub async fn fetch(client: &Client, filter: &[String], now: i64) -> Result<Fetched> {
    let site = client.site_info().await?;
    let has = |f: &str| site.functions.is_empty() || site.functions.iter().any(|x| x.name == f);
    let mut out = Fetched::default();

    let mut courses: Vec<CourseInfo> = client
        .user_courses(site.userid)
        .await?
        .into_iter()
        .filter(|c| {
            filter.is_empty() || filter.iter().any(|f| *f == c.id.to_string() || f.eq_ignore_ascii_case(&c.shortname))
        })
        .map(|c| CourseInfo {
            id: c.id,
            fullname: if c.fullname.trim().is_empty() { c.shortname.clone() } else { c.fullname.trim().to_string() },
            shortname: c.shortname,
            teachers: Vec::new(),
        })
        .collect();
    let ids: Vec<i64> = courses.iter().map(|c| c.id).collect();
    if ids.is_empty() {
        out.courses = courses;
        return Ok(out);
    }

    // Profesores, para poder ordenar por profesor más adelante.
    if has("core_course_get_courses_by_field") {
        match client.courses_by_ids(&ids).await {
            Ok(details) => {
                for d in details {
                    if let Some(c) = courses.iter_mut().find(|c| c.id == d.id) {
                        c.teachers = d.contacts.into_iter().map(|t| t.fullname).filter(|n| !n.is_empty()).collect();
                    }
                }
            }
            Err(e) => out.warnings.push(format!("profesores: {e}")),
        }
    }
    let course = |id: i64| courses.iter().find(|c| c.id == id);

    // Anuncios primero: sus foros se excluyen de las actividades.
    let mut news_cmids = HashSet::new();
    let legacy_forum = !has("mod_forum_get_forum_discussions");
    if has("mod_forum_get_forums_by_courses") && (!legacy_forum || has("mod_forum_get_forum_discussions_paginated")) {
        match client.forums(&ids).await {
            Ok(forums) => {
                let news: Vec<_> = forums.into_iter().filter(|f| f.kind == "news").collect();
                news_cmids.extend(news.iter().map(|f| f.cmid));
                let results: Vec<_> =
                    stream::iter(news.iter().map(|f| async move {
                        (f, client.discussions(f.id, DISCUSSIONS_PER_FORUM, legacy_forum).await)
                    }))
                    .buffer_unordered(CONCURRENCY)
                    .collect()
                    .await;
                for (forum, res) in results {
                    let Some(c) = course(forum.course) else { continue };
                    match res {
                        Ok(list) => {
                            for d in &list {
                                out.items.extend(announcement_items(client.base(), c, d));
                            }
                        }
                        Err(e) => out.warnings.push(format!("anuncios de {}: {e}", c.fullname)),
                    }
                }
            }
            Err(e) => out.warnings.push(format!("anuncios: {e}")),
        }
    }

    // Archivos y actividades de cada curso.
    let mut module_sections = HashMap::new();
    let results: Vec<_> = stream::iter(courses.iter().map(|c| async move { (c, client.course_contents(c.id).await) }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    for (c, res) in results {
        match res {
            Ok(sections) => {
                for s in &sections {
                    for m in &s.modules {
                        module_sections.insert(m.id, section_name(s));
                    }
                }
                out.items.extend(content_items(c, &sections, &news_cmids));
                out.complete.insert((c.id, Source::Contents));
            }
            Err(e) => out.warnings.push(format!("contenidos de {}: {e}", c.fullname)),
        }
    }

    // Tareas y su estado de entrega.
    if has("mod_assign_get_assignments") {
        match client.assignments(&ids).await {
            Ok(assign_courses) => {
                let mut items = Vec::new();
                let mut pending = Vec::new();
                for ac in &assign_courses {
                    let Some(c) = course(ac.id) else { continue };
                    out.complete.insert((c.id, Source::Assign));
                    for a in &ac.assignments {
                        if a.duedate == 0 || a.duedate >= now - STATUS_WINDOW {
                            pending.push((items.len(), a.id));
                        }
                        items.extend(assignment_items(client.base(), c, a, module_sections.get(&a.cmid)));
                    }
                }
                if has("mod_assign_get_submission_status") {
                    let statuses: Vec<_> = stream::iter(
                        pending.into_iter().map(|(idx, id)| async move { (idx, client.submission_status(id).await) }),
                    )
                    .buffer_unordered(CONCURRENCY)
                    .collect()
                    .await;
                    let failed = statuses.iter().filter(|(_, r)| r.is_err()).count();
                    for (idx, status) in statuses {
                        if let Ok(s) = status {
                            items[idx].status = s.state();
                            items[idx].grade = s.grade();
                        }
                    }
                    if failed > 0 {
                        out.warnings.push(format!("no se pudo leer el estado de entrega de {failed} tarea(s)"));
                    }
                }
                out.items.extend(items);
            }
            Err(e) => out.warnings.push(format!("tareas: {e}")),
        }
    }

    out.courses = courses;
    Ok(out)
}

fn section_name(s: &Section) -> String {
    let name = s.name.trim();
    if !name.is_empty() {
        name.to_string()
    } else if s.section == 0 {
        "General".into()
    } else {
        format!("Sección {}", s.section)
    }
}

fn module_label(modname: &str, modplural: &str) -> String {
    match modname {
        "quiz" => "cuestionario",
        "forum" => "foro",
        "url" => "enlace",
        "page" => "página",
        "book" => "libro",
        "choice" => "consulta",
        "feedback" | "survey" | "questionnaire" => "encuesta",
        "lesson" => "lección",
        "workshop" => "taller",
        "glossary" => "glosario",
        "data" => "base de datos",
        "h5pactivity" | "hvp" => "H5P",
        "lti" => "herramienta externa",
        "bigbluebuttonbn" | "zoom" => "videoconferencia",
        "attendance" => "asistencia",
        "scorm" => "SCORM",
        "wiki" => "wiki",
        "chat" => "chat",
        _ if !modplural.is_empty() => return modplural.to_lowercase(),
        other => other,
    }
    .to_string()
}

fn file_ref(f: &MoodleFile, subdir: Vec<String>) -> Option<FileRef> {
    Some(FileRef {
        filename: f.filename.clone(),
        size: f.filesize.max(0) as u64,
        url: f.fileurl.clone()?,
        mimetype: f.mimetype.clone(),
        modified: f.timemodified,
        subdir,
    })
}

pub fn content_items(course: &CourseInfo, sections: &[Section], news_cmids: &HashSet<i64>) -> Vec<Item> {
    let mut items = Vec::new();
    for s in sections {
        let section = section_name(s);
        for m in &s.modules {
            if !m.visible || !m.uservisible || news_cmids.contains(&m.id) {
                continue;
            }
            if m.modname == "resource" || m.modname == "folder" {
                for f in m.contents.iter().filter(|f| f.kind == "file" && !f.filename.is_empty()) {
                    let filepath = f.filepath.clone().unwrap_or_else(|| "/".into());
                    let mut subdir = vec![section.clone()];
                    if m.modname == "folder" {
                        subdir.push(m.name.clone());
                        subdir.extend(filepath.split('/').filter(|p| !p.is_empty()).map(String::from));
                    }
                    let Some(file) = file_ref(f, subdir) else { continue };
                    let key = format!("file:{}:{filepath}{}", m.id, f.filename);
                    let mut item =
                        Item::new(key, Kind::File, Source::Contents, course.id, &course.fullname, f.filename.clone());
                    item.section = Some(section.clone());
                    item.context = Some(m.name.clone());
                    item.web_url = m.url.clone();
                    item.modified = f.timemodified;
                    item.file = Some(file);
                    items.push(item);
                }
            } else if !SKIP_MODULES.contains(&m.modname.as_str()) {
                let mut item = Item::new(
                    format!("mod:{}", m.id),
                    Kind::Activity,
                    Source::Contents,
                    course.id,
                    &course.fullname,
                    m.name.clone(),
                );
                item.section = Some(section.clone());
                item.module = Some(module_label(&m.modname, &m.modplural));
                item.web_url = m.url.clone();
                item.dates = m
                    .dates
                    .iter()
                    .filter(|d| d.timestamp > 0)
                    .map(|d| LabeledDate {
                        label: d.label.trim().trim_end_matches(':').to_string(),
                        timestamp: d.timestamp,
                    })
                    .collect();
                items.push(item);
            }
        }
    }
    items
}

pub fn announcement_items(base: &str, course: &CourseInfo, d: &Discussion) -> Vec<Item> {
    let title = if d.subject.trim().is_empty() { d.name.trim() } else { d.subject.trim() }.to_string();
    let key = format!("ann:{}", d.discussion);
    let mut item =
        Item::new(key.clone(), Kind::Announcement, Source::Forum, course.id, &course.fullname, title.clone());
    item.web_url = Some(format!("{base}/mod/forum/discuss.php?d={}", d.discussion));
    item.body_html = Some(d.message.clone());
    item.author = d.userfullname.clone();
    item.created = (d.created > 0).then_some(d.created);
    item.modified = if d.modified > 0 { d.modified } else { d.timemodified };
    item.section = Some("Anuncios".into());

    let mut items = vec![item];
    for f in &d.attachments {
        let Some(file) = file_ref(f, vec!["Anuncios".into()]) else { continue };
        let path = f.filepath.as_deref().unwrap_or("/");
        let mut att = Item::new(
            format!("annfile:{}:{path}{}", d.discussion, f.filename),
            Kind::File,
            Source::Forum,
            course.id,
            &course.fullname,
            f.filename.clone(),
        );
        att.section = Some("Anuncios".into());
        att.context = Some(title.clone());
        att.parent = Some(key.clone());
        att.modified = f.timemodified;
        att.file = Some(file);
        items.push(att);
    }
    items
}

pub fn assignment_items(base: &str, course: &CourseInfo, a: &Assignment, section: Option<&String>) -> Vec<Item> {
    let key = format!("assign:{}", a.id);
    let mut item =
        Item::new(key.clone(), Kind::Assignment, Source::Assign, course.id, &course.fullname, a.name.clone());
    item.web_url = Some(format!("{base}/mod/assign/view.php?id={}", a.cmid));
    item.section = section.cloned();
    item.due = (a.duedate > 0).then_some(a.duedate);
    item.cutoff = (a.cutoffdate > 0).then_some(a.cutoffdate);
    item.body_html = a.intro.clone().filter(|i| !i.trim().is_empty());
    item.modified = a.timemodified;

    let mut items = vec![item];
    for f in &a.introattachments {
        let Some(file) = file_ref(f, vec!["Tareas".into(), a.name.clone()]) else { continue };
        let path = f.filepath.as_deref().unwrap_or("/");
        let mut att = Item::new(
            format!("assignfile:{}:{path}{}", a.id, f.filename),
            Kind::File,
            Source::Assign,
            course.id,
            &course.fullname,
            f.filename.clone(),
        );
        att.section = section.cloned();
        att.context = Some(a.name.clone());
        att.parent = Some(key.clone());
        att.modified = f.timemodified;
        att.file = Some(file);
        items.push(att);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn course() -> CourseInfo {
        CourseInfo { id: 1, shortname: "MAT".into(), fullname: "Cálculo".into(), teachers: vec![] }
    }

    #[test]
    fn archivos_y_actividades_de_un_curso() {
        let sections: Vec<Section> = serde_json::from_str(
            r#"[{"name": "", "section": 0, "modules": [
                    {"id": 9, "name": "Avisos", "modname": "forum"},
                    {"id": 10, "name": "Bienvenida", "modname": "label"}]},
                {"name": "Unidad 1", "section": 1, "modules": [
                    {"id": 11, "name": "Guía", "modname": "resource", "url": "https://a/mod/resource/view.php?id=11",
                     "contents": [{"type": "file", "filename": "guia.pdf", "filepath": "/", "filesize": 10,
                                   "fileurl": "https://a/webservice/pluginfile.php/1/guia.pdf", "timemodified": 5}]},
                    {"id": 12, "name": "Material", "modname": "folder",
                     "contents": [{"type": "file", "filename": "e.pdf", "filepath": "/sem1/", "fileurl": "https://a/f/e.pdf"}]},
                    {"id": 13, "name": "Oculto", "modname": "quiz", "uservisible": false},
                    {"id": 14, "name": "Control", "modname": "quiz", "dates": [{"label": "Cierre:", "timestamp": 99}]},
                    {"id": 15, "name": "Tarea", "modname": "assign"}]}]"#,
        )
        .unwrap();
        let items = content_items(&course(), &sections, &HashSet::from([9]));
        let keys: Vec<_> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["file:11:/guia.pdf", "file:12:/sem1/e.pdf", "mod:14"]);
        assert_eq!(items[0].section.as_deref(), Some("Unidad 1"));
        assert_eq!(items[1].file.as_ref().unwrap().subdir, ["Unidad 1", "Material", "sem1"]);
        assert_eq!(items[2].module.as_deref(), Some("cuestionario"));
        assert_eq!(items[2].dates, [LabeledDate { label: "Cierre".into(), timestamp: 99 }]);
    }
}
