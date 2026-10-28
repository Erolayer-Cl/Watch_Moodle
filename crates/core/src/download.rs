//! Descarga de archivos a `{download_dir}/{curso}/{sección}/…` sin salirse de esa carpeta.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::model::FileRef;
use crate::moodle::Client;
use crate::{Error, Result};

const MAX_NAME: usize = 120;

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub key: String,
    pub title: String,
    pub file: FileRef,
    pub target: PathBuf,
    pub fingerprint: String,
}

/// Deja un nombre seguro como componente de ruta: sin separadores, sin `..`,
/// sin caracteres de control y con un largo razonable.
pub fn sanitize_component(name: &str) -> String {
    let cleaned: String =
        name.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return "_".into();
    }
    if trimmed.chars().count() <= MAX_NAME {
        return trimmed.to_string();
    }
    match trimmed.rfind('.') {
        Some(dot) if trimmed.len() - dot <= 10 => {
            let ext = &trimmed[dot..];
            let stem: String = trimmed[..dot].chars().take(MAX_NAME - ext.chars().count()).collect();
            format!("{}{ext}", stem.trim_end())
        }
        _ => trimmed.chars().take(MAX_NAME).collect(),
    }
}

pub fn target_path(dir: &Path, course: &str, file: &FileRef) -> PathBuf {
    let mut path = dir.join(sanitize_component(course));
    for part in &file.subdir {
        path.push(sanitize_component(part));
    }
    path.push(sanitize_component(&file.filename));
    path
}

/// `Guía (2).pdf` a partir de `Guía.pdf`.
pub fn numbered(path: &Path, n: u32) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    target.with_file_name(name)
}

/// Descarga en `{destino}.part` y renombra al terminar; si algo falla no queda
/// un archivo a medias con el nombre final.
pub async fn fetch(client: &Client, job: &Job, mut progress: impl FnMut(u64, Option<u64>)) -> Result<()> {
    let url = client.file_url(&job.file.url)?;
    let res = client.http().get(url).send().await?.error_for_status()?;
    let total = res.content_length().or(Some(job.file.size).filter(|s| *s > 0));
    let is_json = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if is_json && !job.file.filename.to_lowercase().ends_with(".json") {
        // Moodle responde los errores de descarga (token inválido, sin permiso) como JSON.
        let body = res.text().await?;
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").or(v.get("message")).and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| body.chars().take(200).collect());
        return Err(Error::Moodle { code: "download".into(), message: client.redact(&message) });
    }

    if let Some(dir) = job.target.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = part_path(&job.target);
    let written = async {
        let mut file = tokio::fs::File::create(&part).await?;
        let mut stream = res.bytes_stream();
        let mut done = 0u64;
        progress(0, total);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok::<_, Error>(())
    }
    .await;
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::rename(&part, &job.target).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, subdir: &[&str]) -> FileRef {
        FileRef {
            filename: name.into(),
            size: 0,
            url: String::new(),
            mimetype: None,
            modified: 0,
            subdir: subdir.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn nombres_seguros() {
        assert_eq!(sanitize_component("Guía 1: Límites.pdf"), "Guía 1_ Límites.pdf");
        assert_eq!(sanitize_component(".."), "_");
        assert_eq!(sanitize_component("../../.bashrc"), "_.._.bashrc");
        assert_eq!(sanitize_component("  .oculto. "), "oculto");
        let long = format!("{}.pdf", "a".repeat(300));
        let short = sanitize_component(&long);
        assert_eq!(short.chars().count(), MAX_NAME);
        assert!(short.ends_with(".pdf"));
    }

    #[test]
    fn rutas_quedan_dentro_de_la_carpeta() {
        let dir = Path::new("/home/u/Moodle");
        let p = target_path(dir, "Cálculo II", &file("../../etc/passwd", &["Unidad 1", ".."]));
        assert_eq!(p, Path::new("/home/u/Moodle/Cálculo II/Unidad 1/_/_.._etc_passwd"));
        assert!(p.starts_with(dir));
        assert_eq!(numbered(Path::new("/a/Guía.pdf"), 2), Path::new("/a/Guía (2).pdf"));
        assert_eq!(numbered(Path::new("/a/LEEME"), 3), Path::new("/a/LEEME (3)"));
    }
}
