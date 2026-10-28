use std::error::Error as _;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No has iniciado sesión. Ejecuta `revision login`.")]
    NotLoggedIn,
    #[error("{0}")]
    Config(String),
    /// Error informado por Moodle (`errorcode` + mensaje legible).
    #[error("{message}")]
    Moodle { code: String, message: String },
    #[error("Error de red: {0}")]
    Http(String),
    #[error("Respuesta inesperada de Moodle ({context}): {detail}")]
    Parse { context: String, detail: String },
    #[error("Llavero del sistema: {0}")]
    Secrets(String),
    #[error("Base de datos: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // without_url: las URLs de descarga llevan el token.
        let e = e.without_url();
        let mut msg = e.to_string();
        let mut source = e.source();
        while let Some(s) = source {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        Error::Http(msg)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
