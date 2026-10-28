use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{Config, Error, Result};

/// Dónde se guardan token y contraseña. En producción, el llavero del sistema;
/// en pruebas, memoria.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// Llavero del sistema (Secret Service en Linux: KWallet o GNOME Keyring).
pub struct Keyring;

const SERVICE: &str = "revision";

fn entry(name: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, name).map_err(|e| Error::Secrets(e.to_string()))
}

impl SecretStore for Keyring {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Secrets(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        entry(name)?.set_password(value).map_err(|e| Error::Secrets(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<()> {
        match entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Secrets(e.to_string())),
        }
    }
}

#[derive(Clone, Default)]
pub struct MemorySecrets(Arc<Mutex<HashMap<String, String>>>);

impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// Nombres de las entradas del llavero: `token:usuario@host` y `password:usuario@host`.
pub(crate) struct SecretNames {
    pub token: String,
    pub password: String,
}

impl SecretNames {
    pub fn for_config(config: &Config) -> SecretNames {
        let host = reqwest::Url::parse(&config.url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_else(|| config.url.clone());
        let id = format!("{}@{host}", config.username);
        SecretNames { token: format!("token:{id}"), password: format!("password:{id}") }
    }
}
