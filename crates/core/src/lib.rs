//! Núcleo de Revisión: consulta Moodle, detecta qué hay de nuevo o cambió
//! (archivos, anuncios, tareas, actividades) y descarga archivos.
//! Lo usan la CLI y la app de escritorio, que comparten configuración, llavero y BD.

pub mod config;
pub mod download;
mod error;
pub mod model;
pub mod moodle;
pub mod secrets;
pub mod store;
pub mod sync;
pub mod text;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub use config::{Config, Paths};
pub use download::Job;
pub use error::{Error, Result};
pub use model::{Change, Course, Entry, Item, Kind, SyncReport};
pub use moodle::Client;
pub use secrets::{Keyring, MemorySecrets, SecretStore};
pub use store::{Query, Store};

use secrets::SecretNames;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Sesión abierta: configuración, cliente con token y BD local.
pub struct Revision {
    config: Config,
    store: Store,
    client: Client,
    secrets: Box<dyn SecretStore>,
    saved_token: String,
}

impl Revision {
    /// Inicia sesión, guarda token y contraseña en el llavero y la configuración en disco.
    /// Si cambia el aula virtual, se borra el estado anterior.
    pub async fn login(
        paths: &Paths,
        secrets: Box<dyn SecretStore>,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Revision> {
        let url = config::normalize_url(url)?;
        let client = Client::login(&url, username, password).await?;

        let previous = Config::load(&paths.config_file)?;
        let site_changed = previous.as_ref().is_some_and(|p| p.url != url);
        let config = match previous {
            Some(p) => Config { url, username: username.to_string(), ..p },
            None => Config::new(url, username.to_string()),
        };
        let names = SecretNames::for_config(&config);
        secrets.set(&names.token, &client.token())?;
        secrets.set(&names.password, password)?;
        config.save(&paths.config_file)?;

        let mut store = Store::open(&paths.db_file)?;
        if site_changed {
            store.reset()?;
        }
        Ok(Revision { saved_token: client.token(), config, store, client, secrets })
    }

    /// Abre la sesión guardada (configuración + llavero).
    pub fn open(paths: &Paths, secrets: Box<dyn SecretStore>) -> Result<Revision> {
        let config = Config::load(&paths.config_file)?.ok_or(Error::NotLoggedIn)?;
        let names = SecretNames::for_config(&config);
        let token = secrets.get(&names.token)?;
        let password = secrets.get(&names.password)?;
        if token.is_none() && password.is_none() {
            return Err(Error::NotLoggedIn);
        }
        let credentials = password.map(|p| (config.username.clone(), p));
        let client = Client::with_token(&config.url, token.as_deref().unwrap_or_default(), credentials)?;
        let store = Store::open(&paths.db_file)?;
        Ok(Revision { saved_token: token.unwrap_or_default(), config, store, client, secrets })
    }

    /// Borra token y contraseña del llavero. Devuelve `false` si no había sesión.
    pub fn logout(paths: &Paths, secrets: &dyn SecretStore) -> Result<bool> {
        let Some(config) = Config::load(&paths.config_file)? else { return Ok(false) };
        let names = SecretNames::for_config(&config);
        secrets.delete(&names.token)?;
        secrets.delete(&names.password)?;
        Ok(true)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Consulta Moodle y registra las novedades.
    pub async fn sync(&mut self) -> Result<SyncReport> {
        let now = now();
        let fetched = sync::fetch(&self.client, &self.config.courses, now).await?;
        let report = self.store.apply(&fetched, now)?;
        self.persist_token()?;
        Ok(report)
    }

    /// Si el cliente tuvo que pedir un token nuevo, se guarda en el llavero.
    fn persist_token(&mut self) -> Result<()> {
        let token = self.client.token();
        if token != self.saved_token {
            self.secrets.set(&SecretNames::for_config(&self.config).token, &token)?;
            self.saved_token = token;
        }
        Ok(())
    }

    /// Prepara las descargas: decide la ruta de cada archivo sin pisar archivos
    /// ajenos. Un archivo actualizado reemplaza a su versión descargada antes.
    pub fn plan_downloads(&self, keys: &[String]) -> Result<Vec<Job>> {
        let dir = self.config.download_dir();
        let mut planned = HashSet::new();
        let mut jobs = Vec::new();
        for key in keys {
            let Some(entry) = self.store.entry(key)? else { continue };
            let Some(file) = entry.item.file.clone() else { continue };
            let previous = entry.downloaded.as_ref().map(|d| PathBuf::from(&d.path)).filter(|p| p.exists());
            let target = match previous {
                Some(path) if !planned.contains(&path) => path,
                _ => {
                    let base = download::target_path(&dir, &entry.item.course, &file);
                    if !base.starts_with(&dir) {
                        return Err(Error::Config(format!("Ruta de descarga inválida: {}", base.display())));
                    }
                    let mut candidate = base.clone();
                    let mut n = 2;
                    while planned.contains(&candidate)
                        || (candidate.exists() && self.store.download_owner(&candidate)?.as_deref() != Some(key))
                    {
                        candidate = download::numbered(&base, n);
                        n += 1;
                    }
                    candidate
                }
            };
            planned.insert(target.clone());
            jobs.push(Job {
                key: key.clone(),
                title: entry.item.title.clone(),
                fingerprint: entry.item.fingerprint(),
                file,
                target,
            });
        }
        Ok(jobs)
    }

    pub fn record_download(&mut self, job: &Job) -> Result<()> {
        self.store.record_download(&job.key, &job.target, &job.fingerprint, now())
    }
}
