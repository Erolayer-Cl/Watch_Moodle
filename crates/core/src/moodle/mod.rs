//! Cliente de la API de web services de Moodle (la misma que usa la app móvil).

mod api;
mod client;
pub mod types;

pub use client::{Client, flatten};
