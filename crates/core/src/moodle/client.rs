use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Url;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{Error, Result};

/// Servicio de la app móvil oficial: viene habilitado en la mayoría de los Moodle.
const SERVICE: &str = "moodle_mobile_app";
const API_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    base_url: Url,
    token: Arc<Mutex<String>>,
    /// Usuario y contraseña para pedir un token nuevo si el actual vence.
    credentials: Option<Arc<(String, String)>>,
}

impl Client {
    /// Inicia sesión con usuario y contraseña (`login/token.php`).
    pub async fn login(base: &str, username: &str, password: &str) -> Result<Client> {
        let client = Client::with_token(base, "", Some((username.into(), password.into())))?;
        client.relogin().await?;
        Ok(client)
    }

    pub fn with_token(base: &str, token: &str, credentials: Option<(String, String)>) -> Result<Client> {
        let base_url = Url::parse(base).map_err(|_| Error::Config(format!("URL inválida: {base}")))?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("revision/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Client {
            http,
            base: base.trim_end_matches('/').to_string(),
            base_url,
            token: Arc::new(Mutex::new(token.to_string())),
            credentials: credentials.map(Arc::new),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> String {
        self.token.lock().unwrap().clone()
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn relogin(&self) -> Result<()> {
        let (username, password) = self.credentials.as_deref().ok_or(Error::NotLoggedIn)?;
        let token = request_token(&self.http, &self.base, username, password).await?;
        *self.token.lock().unwrap() = token;
        Ok(())
    }

    /// Llama a una función del web service. Si el token venció y hay credenciales,
    /// pide uno nuevo y reintenta una sola vez.
    pub async fn call<T: DeserializeOwned>(&self, function: &str, params: Value) -> Result<T> {
        let value = match self.call_raw(function, &params).await {
            Err(Error::Moodle { code, .. })
                if matches!(code.as_str(), "invalidtoken" | "accessexception") && self.credentials.is_some() =>
            {
                self.relogin().await?;
                self.call_raw(function, &params).await?
            }
            other => other?,
        };
        serde_json::from_value(value).map_err(|e| Error::Parse { context: function.into(), detail: e.to_string() })
    }

    async fn call_raw(&self, function: &str, params: &Value) -> Result<Value> {
        let mut form = vec![
            ("wstoken".to_string(), self.token()),
            ("wsfunction".to_string(), function.to_string()),
            ("moodlewsrestformat".to_string(), "json".to_string()),
        ];
        flatten(params, "", &mut form);
        let res = self
            .http
            .post(format!("{}/webservice/rest/server.php", self.base))
            .form(&form)
            .timeout(API_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        let value: Value = serde_json::from_slice(&res.bytes().await?)
            .map_err(|e| Error::Parse { context: function.into(), detail: e.to_string() })?;
        if value.get("exception").is_some() {
            let field = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            return Err(Error::Moodle {
                code: field("errorcode"),
                message: format!("{function}: {}", field("message")),
            });
        }
        Ok(value)
    }

    /// URL de descarga con el token. Usa `webservice/pluginfile.php` y se niega a
    /// mandar el token a un sitio distinto del aula virtual.
    pub fn file_url(&self, fileurl: &str) -> Result<Url> {
        let mut url =
            Url::parse(fileurl).map_err(|_| Error::Parse { context: "fileurl".into(), detail: fileurl.into() })?;
        if url.scheme() != self.base_url.scheme()
            || url.host_str() != self.base_url.host_str()
            || url.port_or_known_default() != self.base_url.port_or_known_default()
        {
            return Err(Error::Config(format!(
                "El archivo está en otro sitio ({}); no se envía el token.",
                url.host_str().unwrap_or("?")
            )));
        }
        if !url.path().contains("/webservice/pluginfile.php/") {
            let path = url.path().replacen("/pluginfile.php/", "/webservice/pluginfile.php/", 1);
            url.set_path(&path);
        }
        let pairs: Vec<(String, String)> =
            url.query_pairs().filter(|(k, _)| k != "token").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        url.query_pairs_mut().clear().extend_pairs(pairs).append_pair("token", &self.token());
        Ok(url)
    }

    /// Oculta el token en textos que vienen del servidor.
    pub fn redact(&self, text: &str) -> String {
        let token = self.token();
        if token.is_empty() { text.to_string() } else { text.replace(&token, "***") }
    }
}

async fn request_token(http: &reqwest::Client, base: &str, username: &str, password: &str) -> Result<String> {
    let res = http
        .post(format!("{base}/login/token.php"))
        .form(&[("username", username), ("password", password), ("service", SERVICE)])
        .timeout(API_TIMEOUT)
        .send()
        .await?;
    let status = res.status();
    let value: Value = serde_json::from_slice(&res.bytes().await?).map_err(|_| Error::Moodle {
        code: "notmoodle".into(),
        message: format!("La dirección no respondió como un Moodle (HTTP {status}). Revisa la URL."),
    })?;
    if let Some(token) = value.get("token").and_then(Value::as_str) {
        return Ok(token.to_string());
    }
    let code = value.get("errorcode").and_then(Value::as_str).unwrap_or_default().to_string();
    let original = value.get("error").and_then(Value::as_str).unwrap_or("error desconocido");
    let message = match code.as_str() {
        "invalidlogin" => "Usuario o contraseña incorrectos.".to_string(),
        "enablewsdescription" | "servicenotavailable" | "webservicesnotenabled" => format!(
            "Tu Moodle no permite conectarse como la app móvil ({original}). \
             Habría que leer la página web en vez de usar la API."
        ),
        _ => format!("Moodle rechazó el inicio de sesión: {original}"),
    };
    Err(Error::Moodle { code, message })
}

/// Convierte parámetros anidados al formato `clave[0][sub]=valor` que espera Moodle.
pub fn flatten(value: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}[{k}]") };
                flatten(v, &key, out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                flatten(v, &format!("{prefix}[{i}]"), out);
            }
        }
        Value::Bool(b) => out.push((prefix.to_string(), if *b { "1" } else { "0" }.to_string())),
        Value::Number(n) => out.push((prefix.to_string(), n.to_string())),
        Value::String(s) => out.push((prefix.to_string(), s.clone())),
        Value::Null => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn aplana_parametros() {
        let mut out = Vec::new();
        flatten(&json!({"userid": 5, "courseids": [1, 2], "options": [{"name": "x", "value": true}]}), "", &mut out);
        out.sort();
        assert_eq!(
            out,
            [
                ("courseids[0]".into(), "1".into()),
                ("courseids[1]".into(), "2".into()),
                ("options[0][name]".into(), "x".into()),
                ("options[0][value]".into(), "1".into()),
                ("userid".into(), "5".into()),
            ]
        );
    }

    #[test]
    fn url_de_archivo_con_token() {
        let c = Client::with_token("https://aula.uni.cl", "abc", None).unwrap();
        let url = c.file_url("https://aula.uni.cl/pluginfile.php/10/mod_resource/content/1/Gu%C3%ADa.pdf?forcedownload=1&token=viejo").unwrap();
        assert_eq!(
            url.as_str(),
            "https://aula.uni.cl/webservice/pluginfile.php/10/mod_resource/content/1/Gu%C3%ADa.pdf?forcedownload=1&token=abc"
        );
        let ws = c.file_url("https://aula.uni.cl/webservice/pluginfile.php/1/a.pdf").unwrap();
        assert_eq!(ws.as_str(), "https://aula.uni.cl/webservice/pluginfile.php/1/a.pdf?token=abc");
    }

    #[test]
    fn no_envia_token_a_otro_sitio() {
        let c = Client::with_token("https://aula.uni.cl", "abc", None).unwrap();
        assert!(c.file_url("https://otro.cl/pluginfile.php/1/a.pdf").is_err());
        assert!(c.file_url("http://aula.uni.cl/pluginfile.php/1/a.pdf").is_err());
        assert_eq!(c.redact("error con token abc"), "error con token ***");
    }
}
