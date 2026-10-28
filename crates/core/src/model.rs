use serde::{Deserialize, Serialize};

use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Announcement,
    Assignment,
    Activity,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Announcement => "announcement",
            Kind::Assignment => "assignment",
            Kind::Activity => "activity",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::File => "Archivos",
            Kind::Announcement => "Anuncios",
            Kind::Assignment => "Tareas",
            Kind::Activity => "Actividades",
        }
    }
}

/// De qué consulta salió un elemento. Sirve para saber qué desapareció: solo se
/// marca como eliminado lo que faltó en una consulta que terminó completa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Contents,
    Forum,
    Assign,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Contents => "contents",
            Source::Forum => "forum",
            Source::Assign => "assign",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    pub filename: String,
    pub size: u64,
    /// `fileurl` tal como lo entrega Moodle (sin token).
    pub url: String,
    #[serde(default)]
    pub mimetype: Option<String>,
    pub modified: i64,
    /// Carpetas bajo la del curso donde se guarda (sección, carpeta de Moodle…).
    #[serde(default)]
    pub subdir: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabeledDate {
    pub label: String,
    pub timestamp: i64,
}

/// Algo visible en el aula virtual: archivo, anuncio, tarea u otra actividad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Clave estable: `file:{cmid}:{ruta}{nombre}`, `ann:{discusión}`, `assign:{id}`,
    /// `mod:{cmid}`, `annfile:…`, `assignfile:…`.
    pub key: String,
    pub kind: Kind,
    pub source: Source,
    pub course_id: i64,
    pub course: String,
    pub title: String,
    #[serde(default)]
    pub section: Option<String>,
    /// Dónde está: nombre del recurso, anuncio o tarea al que pertenece un archivo.
    #[serde(default)]
    pub context: Option<String>,
    /// Elemento dueño de un adjunto (`ann:…` o `assign:…`).
    #[serde(default)]
    pub parent: Option<String>,
    /// Tipo de actividad legible (`cuestionario`, `enlace`…).
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub file: Option<FileRef>,
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub cutoff: Option<i64>,
    #[serde(default)]
    pub dates: Vec<LabeledDate>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    pub modified: i64,
    /// Estado de entrega de una tarea: `new`, `draft`, `submitted` o `reopened`.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub grade: Option<String>,
}

impl Item {
    pub fn new(key: String, kind: Kind, source: Source, course_id: i64, course: &str, title: String) -> Item {
        Item {
            key,
            kind,
            source,
            course_id,
            course: course.to_string(),
            title,
            section: None,
            context: None,
            parent: None,
            module: None,
            web_url: None,
            file: None,
            due: None,
            cutoff: None,
            dates: Vec::new(),
            body_html: None,
            author: None,
            created: None,
            modified: 0,
            status: None,
            grade: None,
        }
    }

    /// Campos cuyo cambio cuenta como «actualizado».
    pub fn fingerprint(&self) -> String {
        match self.kind {
            Kind::File => self.file.as_ref().map(|f| format!("{}:{}", f.modified, f.size)).unwrap_or_default(),
            Kind::Announcement => self.modified.to_string(),
            Kind::Assignment => {
                format!("{}:{}:{}", self.due.unwrap_or(0), self.cutoff.unwrap_or(0), self.modified)
            }
            Kind::Activity => String::new(),
        }
    }

    pub fn status_label(&self) -> Option<&'static str> {
        Some(match self.status.as_deref()? {
            "submitted" => "entregada",
            "draft" => "borrador",
            "new" => "sin entrega",
            "reopened" => "reabierta",
            _ => return None,
        })
    }
}

/// Describe qué cambió entre dos versiones de un elemento.
pub fn describe_change(old: &Item, new: &Item) -> String {
    match new.kind {
        Kind::File => "archivo reemplazado".into(),
        Kind::Announcement => "anuncio editado".into(),
        Kind::Activity => "actualizada".into(),
        Kind::Assignment => {
            let date = |ts: Option<i64>| ts.map(text::fmt_datetime).unwrap_or_else(|| "sin fecha".into());
            let mut parts = Vec::new();
            if old.due != new.due {
                parts.push(format!("entrega: {} → {}", date(old.due), date(new.due)));
            }
            if old.cutoff != new.cutoff {
                parts.push(format!("cierre: {} → {}", date(old.cutoff), date(new.cutoff)));
            }
            if parts.is_empty() {
                parts.push("enunciado actualizado".into());
            }
            parts.join(" · ")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    New,
    Updated,
}

#[derive(Debug, Clone, Serialize)]
pub struct Download {
    pub path: String,
    pub fingerprint: String,
    pub at: i64,
}

/// Elemento guardado, con su estado de novedad y descarga.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub item: Item,
    pub change: Option<Change>,
    pub change_detail: Option<String>,
    pub seen: bool,
    pub first_seen: i64,
    pub last_changed: i64,
    pub downloaded: Option<Download>,
}

impl Entry {
    /// Ya se descargó, pero Moodle tiene una versión más nueva.
    pub fn download_outdated(&self) -> bool {
        self.downloaded.as_ref().is_some_and(|d| d.fingerprint != self.item.fingerprint())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Course {
    pub id: i64,
    pub shortname: String,
    pub fullname: String,
    pub teachers: Vec<String>,
    pub unseen: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    /// Primera revisión: todo quedó como punto de partida, sin novedades.
    pub baseline: bool,
    pub courses: usize,
    pub items: usize,
    pub new: usize,
    pub updated: usize,
    pub removed: usize,
    /// Novedades sin ver en total (incluye las de revisiones anteriores).
    pub unseen: usize,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detalle_de_cambio_en_tareas() {
        let mut old = Item::new("assign:1".into(), Kind::Assignment, Source::Assign, 1, "C", "T".into());
        old.due = Some(1_700_000_000);
        let mut new = old.clone();
        assert_eq!(describe_change(&old, &new), "enunciado actualizado");
        new.due = None;
        assert!(describe_change(&old, &new).starts_with("entrega: "));
        assert!(describe_change(&old, &new).ends_with(" → sin fecha"));
        assert_ne!(old.fingerprint(), new.fingerprint());
    }
}
