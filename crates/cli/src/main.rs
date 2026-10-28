mod commands;
mod ui;

use clap::{ArgAction, Parser, Subcommand};
use console::style;
use inquire::InquireError;

/// Plantilla de ayuda en español (clap la trae en inglés).
const HELP: &str = "{about-with-newline}\nUso: {usage}\n\n{all-args}{after-help}";

/// Revisa las novedades de tu aula virtual Moodle: archivos, anuncios y tareas.
/// Sin subcomando, equivale a `revision revisar`.
#[derive(Parser)]
#[command(
    name = "revision",
    version,
    help_template = HELP,
    override_usage = "revision [COMANDO] [opciones]",
    subcommand_help_heading = "Comandos",
    subcommand_value_name = "COMANDO",
    disable_help_subcommand = true,
    disable_help_flag = true,
    disable_version_flag = true,
    next_help_heading = "Opciones"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Muestra esta ayuda
    #[arg(short = 'h', long, action = ArgAction::Help, global = true)]
    help: Option<bool>,
    /// Muestra la versión
    #[arg(short = 'V', long, action = ArgAction::Version)]
    version: Option<bool>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inicia sesión y guarda la contraseña en el llavero del sistema.
    #[command(
        help_template = HELP,
        override_usage = "revision login [--url <URL>] [--usuario <USUARIO>]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Login {
        /// Dirección del aula virtual (p. ej. https://aulavirtual.uni.cl)
        #[arg(long)]
        url: Option<String>,
        /// Nombre de usuario
        #[arg(long)]
        usuario: Option<String>,
    },
    /// Muestra lo nuevo o actualizado y deja elegir qué archivos descargar.
    #[command(
        visible_alias = "check",
        help_template = HELP,
        override_usage = "revision revisar [--curso <CURSO>] [--no-marcar]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Revisar {
        /// Solo un curso (id, nombre corto o parte del nombre)
        #[arg(long)]
        curso: Option<String>,
        /// No marcar las novedades como vistas
        #[arg(long)]
        no_marcar: bool,
    },
    /// Explora todos los archivos de tus cursos y descarga los que elijas.
    #[command(
        help_template = HELP,
        override_usage = "revision archivos [--curso <CURSO>] [--offline]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Archivos {
        /// Solo un curso (id, nombre corto o parte del nombre)
        #[arg(long)]
        curso: Option<String>,
        /// Usar la última revisión, sin conectarse
        #[arg(long)]
        offline: bool,
    },
    /// Muestra las próximas entregas.
    #[command(
        help_template = HELP,
        override_usage = "revision tareas [--todas] [--offline]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Tareas {
        /// Incluir las ya entregadas
        #[arg(long)]
        todas: bool,
        /// Usar la última revisión, sin conectarse
        #[arg(long)]
        offline: bool,
    },
    /// Muestra los anuncios recientes completos.
    #[command(
        help_template = HELP,
        override_usage = "revision anuncios [--curso <CURSO>] [-n <CANTIDAD>] [--offline]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Anuncios {
        /// Solo un curso (id, nombre corto o parte del nombre)
        #[arg(long)]
        curso: Option<String>,
        /// Cuántos anuncios mostrar (por defecto 10)
        #[arg(short = 'n', long, default_value_t = 10, hide_default_value = true)]
        cantidad: usize,
        /// Usar la última revisión, sin conectarse
        #[arg(long)]
        offline: bool,
    },
    /// Lista tus cursos y sus profesores.
    #[command(
        help_template = HELP,
        override_usage = "revision cursos [--offline]",
        disable_help_flag = true,
        next_help_heading = "Opciones"
    )]
    Cursos {
        /// Usar la última revisión, sin conectarse
        #[arg(long)]
        offline: bool,
    },
    /// Borra el token y la contraseña del llavero.
    #[command(help_template = HELP, override_usage = "revision logout", disable_help_flag = true)]
    Logout,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Revisar { curso: None, no_marcar: false });
    if let Err(e) = commands::run(command).await {
        if let Some(InquireError::OperationCanceled | InquireError::OperationInterrupted) = e.downcast_ref() {
            std::process::exit(130);
        }
        eprintln!("{} {e:#}", style("error:").red().bold());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn definicion_de_la_cli_es_valida() {
        Cli::command().debug_assert();
    }
}
