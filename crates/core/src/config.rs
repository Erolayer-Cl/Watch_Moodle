use std::path::{Path, PathBuf};

use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{Error, Result};

const HEADER: &str = "# Configuración de Revisión. La contraseña y el token se guardan en el llavero del sistema.\n\n";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// URL base del aula virtual, sin barra final.
    pub url: String,
    pub username: String,
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    /// Filtro opcional de cursos (id o nombre corto). Vacío = todos.
    #[serde(default, deserialize_with = "course_list")]
    pub courses: Vec<String>,
}

fn default_download_dir() -> String {
    "~/Moodle".into()
}

/// Acepta `courses = ["MAT101", 1234]`: TOML permite mezclar números y textos.
fn course_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CourseRef {
        Id(i64),
        Name(String),
    }
    Ok(Vec::<CourseRef>::deserialize(d)?
        .into_iter()
        .map(|c| match c {
            CourseRef::Id(id) => id.to_string(),
            CourseRef::Name(name) => name,
        })
        .collect())
}

impl Config {
    pub fn new(url: String, username: String) -> Config {
        Config { url, username, download_dir: default_download_dir(), courses: Vec::new() }
    }

    pub fn load(path: &Path) -> Result<Option<Config>> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map(Some).map_err(|e| Error::Config(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, format!("{HEADER}{text}"))?;
        Ok(())
    }

    pub fn download_dir(&self) -> PathBuf {
        expand_home(&self.download_dir)
    }
}

/// Rutas de configuración y datos, compartidas por la CLI y la app de escritorio.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub db_file: PathBuf,
}

impl Paths {
    /// `~/.config/revision/config.toml` y `~/.local/share/revision/revision.db` en Linux.
    pub fn standard() -> Result<Paths> {
        let dirs = directories::ProjectDirs::from("", "", "revision")
            .ok_or_else(|| Error::Config("No se pudo determinar la carpeta personal.".into()))?;
        Ok(Paths { config_file: dirs.config_dir().join("config.toml"), db_file: dirs.data_dir().join("revision.db") })
    }

    pub fn in_dir(dir: &Path) -> Paths {
        Paths { config_file: dir.join("config.toml"), db_file: dir.join("revision.db") }
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    let home =
        || directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Normaliza lo que el usuario pega (p. ej. `aula.uni.cl/my/`) a la URL base del sitio.
/// Exige https, salvo para localhost (pruebas).
pub fn normalize_url(input: &str) -> Result<String> {
    let input = input.trim();
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| Error::Config(format!("URL inválida: {input}")))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err(Error::Config("La URL del aula virtual debe usar https://".into()));
    }
    url.set_query(None);
    url.set_fragment(None);
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/login/index.php", "/login", "/my/courses.php", "/my", "/course/view.php", "/index.php"] {
        if let Some(base) = path.strip_suffix(suffix) {
            path = base.to_string();
            break;
        }
    }
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_urls_pegadas() {
        assert_eq!(normalize_url("aula.uni.cl").unwrap(), "https://aula.uni.cl");
        assert_eq!(normalize_url("https://aula.uni.cl/my/").unwrap(), "https://aula.uni.cl");
        assert_eq!(normalize_url("https://uni.cl/moodle/login/index.php").unwrap(), "https://uni.cl/moodle");
        assert_eq!(normalize_url(" https://uni.cl/moodle/course/view.php?id=5 ").unwrap(), "https://uni.cl/moodle");
        assert_eq!(normalize_url("http://127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert!(normalize_url("http://aula.uni.cl").is_err());
    }

    #[test]
    fn acepta_cursos_mixtos() {
        let c: Config =
            toml::from_str("url = \"https://a.cl\"\nusername = \"u\"\ncourses = [\"MAT101\", 1234]").unwrap();
        assert_eq!(c.courses, ["MAT101", "1234"]);
        assert_eq!(c.download_dir, "~/Moodle");
    }

    #[test]
    fn guarda_y_carga() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        assert!(Config::load(&path).unwrap().is_none());
        let c = Config::new("https://a.cl".into(), "u".into());
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), Some(c));
    }
}
