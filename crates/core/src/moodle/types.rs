//! Respuestas de la API. Moodle mezcla `true`/`1`/`"1"` y a veces manda `null`,
//! así que los campos se leen de forma tolerante.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => !(s.is_empty() || s == "0" || s == "false"),
        _ => false,
    })
}

fn int<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

fn list<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    match Value::deserialize(d)? {
        Value::Array(items) => {
            items.into_iter().map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom)).collect()
        }
        _ => Ok(Vec::new()),
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct SiteInfo {
    pub userid: i64,
    #[serde(default)]
    pub sitename: String,
    #[serde(default)]
    pub fullname: String,
    #[serde(default)]
    pub release: String,
    #[serde(default, deserialize_with = "list")]
    pub functions: Vec<Function>,
}

#[derive(Debug, Deserialize)]
pub struct Function {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UserCourse {
    pub id: i64,
    #[serde(default)]
    pub shortname: String,
    #[serde(default)]
    pub fullname: String,
}

#[derive(Debug, Deserialize)]
pub struct CoursesByField {
    #[serde(default, deserialize_with = "list")]
    pub courses: Vec<CourseDetail>,
}

#[derive(Debug, Deserialize)]
pub struct CourseDetail {
    pub id: i64,
    #[serde(default, deserialize_with = "list")]
    pub contacts: Vec<Contact>,
}

#[derive(Debug, Deserialize)]
pub struct Contact {
    #[serde(default)]
    pub fullname: String,
}

#[derive(Debug, Deserialize)]
pub struct Section {
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "int")]
    pub section: i64,
    #[serde(default, deserialize_with = "list")]
    pub modules: Vec<Module>,
}

#[derive(Debug, Deserialize)]
pub struct Module {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub modname: String,
    #[serde(default)]
    pub modplural: String,
    #[serde(default = "yes", deserialize_with = "flag")]
    pub visible: bool,
    #[serde(default = "yes", deserialize_with = "flag")]
    pub uservisible: bool,
    #[serde(default, deserialize_with = "list")]
    pub contents: Vec<MoodleFile>,
    #[serde(default, deserialize_with = "list")]
    pub dates: Vec<ModuleDate>,
}

/// Archivo de un recurso, carpeta o adjunto.
#[derive(Debug, Deserialize)]
pub struct MoodleFile {
    /// `file` o `url` en contenidos de módulos; vacío en adjuntos.
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub filepath: Option<String>,
    #[serde(default, deserialize_with = "int")]
    pub filesize: i64,
    #[serde(default)]
    pub fileurl: Option<String>,
    #[serde(default, deserialize_with = "int")]
    pub timemodified: i64,
    #[serde(default)]
    pub mimetype: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModuleDate {
    #[serde(default)]
    pub label: String,
    #[serde(default, deserialize_with = "int")]
    pub timestamp: i64,
}

#[derive(Debug, Deserialize)]
pub struct Forum {
    pub id: i64,
    #[serde(default, deserialize_with = "int")]
    pub course: i64,
    /// `news` es el foro de avisos/anuncios.
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default, deserialize_with = "int")]
    pub cmid: i64,
}

#[derive(Debug, Deserialize)]
pub struct Discussions {
    #[serde(default, deserialize_with = "list")]
    pub discussions: Vec<Discussion>,
}

#[derive(Debug, Deserialize)]
pub struct Discussion {
    #[serde(deserialize_with = "int")]
    pub discussion: i64,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub userfullname: Option<String>,
    #[serde(default, deserialize_with = "int")]
    pub created: i64,
    /// Última edición del mensaje (no cambia con las respuestas).
    #[serde(default, deserialize_with = "int")]
    pub modified: i64,
    #[serde(default, deserialize_with = "int")]
    pub timemodified: i64,
    #[serde(default, deserialize_with = "list")]
    pub attachments: Vec<MoodleFile>,
}

#[derive(Debug, Deserialize)]
pub struct AssignmentsResponse {
    #[serde(default, deserialize_with = "list")]
    pub courses: Vec<AssignCourse>,
}

#[derive(Debug, Deserialize)]
pub struct AssignCourse {
    pub id: i64,
    #[serde(default, deserialize_with = "list")]
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Deserialize)]
pub struct Assignment {
    pub id: i64,
    #[serde(deserialize_with = "int")]
    pub cmid: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "int")]
    pub duedate: i64,
    #[serde(default, deserialize_with = "int")]
    pub cutoffdate: i64,
    #[serde(default, deserialize_with = "int")]
    pub timemodified: i64,
    #[serde(default)]
    pub intro: Option<String>,
    #[serde(default, deserialize_with = "list")]
    pub introattachments: Vec<MoodleFile>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SubmissionStatus {
    #[serde(default)]
    pub lastattempt: Option<LastAttempt>,
    #[serde(default)]
    pub feedback: Option<Feedback>,
}

#[derive(Debug, Deserialize)]
pub struct LastAttempt {
    #[serde(default)]
    pub submission: Option<Submission>,
    #[serde(default)]
    pub teamsubmission: Option<Submission>,
}

#[derive(Debug, Deserialize)]
pub struct Submission {
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct Feedback {
    #[serde(default)]
    pub gradefordisplay: Option<String>,
}

impl SubmissionStatus {
    /// `new`, `draft`, `submitted` o `reopened`.
    pub fn state(&self) -> Option<String> {
        let attempt = self.lastattempt.as_ref()?;
        let submission = attempt.submission.as_ref().or(attempt.teamsubmission.as_ref())?;
        (!submission.status.is_empty()).then(|| submission.status.clone())
    }

    pub fn grade(&self) -> Option<String> {
        let html = self.feedback.as_ref()?.gradefordisplay.as_deref()?;
        let text = crate::text::html_to_text(html);
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_campos_tolerantes() {
        let m: Module = serde_json::from_str(
            r#"{"id": 1, "name": "x", "modname": "resource", "visible": 1, "uservisible": false,
                "contents": [{"type": "file", "filename": "a.pdf", "filesize": null, "timemodified": "17"}]}"#,
        )
        .unwrap();
        assert!(m.visible);
        assert!(!m.uservisible);
        assert_eq!(m.contents[0].filesize, 0);
        assert_eq!(m.contents[0].timemodified, 17);

        let d: Discussion = serde_json::from_str(r#"{"discussion": 5, "attachments": false}"#).unwrap();
        assert!(d.attachments.is_empty());
    }

    #[test]
    fn estado_de_entrega() {
        let s: SubmissionStatus = serde_json::from_str(
            r#"{"lastattempt": {"teamsubmission": {"status": "submitted"}}, "feedback": {"gradefordisplay": "<b>6,5</b> / 7,0"}}"#,
        )
        .unwrap();
        assert_eq!(s.state().as_deref(), Some("submitted"));
        assert_eq!(s.grade().as_deref(), Some("6,5 / 7,0"));
    }
}
