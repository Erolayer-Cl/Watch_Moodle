# Revisión

Herramienta de terminal para revisar las novedades de tu **aula virtual Moodle**: archivos nuevos, anuncios y tareas que cambiaron. Te deja **elegir qué archivos descargar**.

Muchas aulas virtuales no avisan cuando un profesor sube material o mueve una fecha de entrega. Con `revision` lo ves todo de una vez, cuando tú quieras: nada corre en segundo plano.

```
$ revision
4 novedades

Cálculo II
  Anuncios
    + Cambio de sala · Ana Rojas · 27-10 14:30
      La prueba será en la sala B-12.
  Tareas
    ~ Tarea 3 · entrega: 01-11-2026 23:59 → 03-11-2026 23:59 · vence 03-11 23:59 (en 6 días) · sin entrega
  Archivos
    + Guía 2.pdf · 900 B · en «Guía 2» · Unidad 1
    ~ Guía 1.pdf · archivo reemplazado · 1,5 KB · en «Guía 1» · Unidad 1

? ¿Qué archivos descargar?
> [x] Cálculo II · Guía 2.pdf (900 B)  [nuevo]
  [ ] Cálculo II · Guía 1.pdf (1,5 KB)  [actualizado]

↓ Cálculo II/Unidad 1/Guía 2.pdf
✓ 1 archivo(s) en ~/Moodle
```

`+` es nuevo y `~` es actualizado (archivo reemplazado, anuncio editado, fecha de entrega movida…).

## Qué revisa

- **Archivos** de recursos y carpetas, y los adjuntos de anuncios y tareas.
- **Anuncios** del foro de avisos de cada curso, con su texto.
- **Tareas**: aparición, cambios de fecha de entrega o de cierre, y tu estado de entrega.
- **Otras actividades** nuevas: cuestionarios, enlaces, páginas, foros…

## Requisitos

- **Tu Moodle debe permitir la app móvil oficial.** Si puedes entrar con la app *Moodle* en el celular, esta herramienta también funciona. Usa la misma API.
- Inicio de sesión con **usuario y contraseña**. Por ahora no funciona con «Ingresar con Google/Microsoft».
- [Rust](https://rustup.rs) 1.85 o superior.
- En Linux, un **llavero del sistema** (GNOME Keyring o KWallet) y las librerías de desarrollo de D-Bus:
  - Arch / CachyOS: `sudo pacman -S --needed base-devel pkgconf dbus`
  - Debian / Ubuntu: `sudo apt install build-essential pkg-config libdbus-1-dev`
  - Fedora: `sudo dnf install gcc pkgconf-pkg-config dbus-devel`

Está probado en Linux. En macOS y Windows usa el llavero de cada sistema y debería funcionar, pero no está probado.

## Instalación

```sh
git clone <URL-de-este-repositorio> revision
cd revision
cargo install --path crates/cli
```

Esto deja el comando `revision` en `~/.cargo/bin` (asegúrate de tenerlo en el `PATH`).

## Uso

### 1. Iniciar sesión (una sola vez)

```sh
revision login
```

Te pide la dirección del aula virtual (por ejemplo `https://aulavirtual.tu-universidad.cl`; sirve pegar la URL que ves en el navegador), tu usuario y tu contraseña. Al terminar muestra tus cursos y sus profesores.

En esa primera revisión, todo lo que ya existe queda como **punto de partida**. Desde ahí solo verás lo nuevo.

### 2. Revisar novedades

```sh
revision            # igual que: revision revisar
```

Muestra lo nuevo y lo actualizado agrupado por curso. Después abre una lista para marcar qué archivos descargar:

| Tecla | Acción |
|---|---|
| `↑` `↓` | moverse |
| `espacio` | marcar o desmarcar |
| `→` / `←` | marcar todos / ninguno |
| escribir | filtrar la lista |
| `enter` | descargar lo marcado |
| `esc` | no descargar nada |

Al terminar, las novedades quedan marcadas como vistas. Con `Ctrl+C` sales sin marcar nada.

### Todos los comandos

| Comando | Qué hace |
|---|---|
| `revision revisar` | Novedades y descarga de archivos nuevos (alias: `check`) |
| `revision archivos` | Explora **todos** los archivos de tus cursos, no solo los nuevos, y descarga los que elijas |
| `revision tareas` | Próximas entregas: cuánto falta y si ya entregaste |
| `revision anuncios` | Anuncios recientes completos, con sus adjuntos |
| `revision cursos` | Tus cursos, sus profesores y cuántas novedades tienes sin ver |
| `revision login` | Inicia sesión (o la renueva) |
| `revision logout` | Borra el token y la contraseña del llavero |

Opciones útiles:

- `--curso <texto>` filtra por curso. Acepta el id, el nombre corto o parte del nombre, sin importar tildes: `--curso calculo`.
- `--offline` usa la última revisión sin conectarse (en `archivos`, `tareas`, `anuncios` y `cursos`).
- `revision revisar --no-marcar` muestra las novedades sin marcarlas como vistas.
- `revision tareas --todas` incluye las tareas ya entregadas.
- `revision anuncios -n 20` muestra más anuncios (por defecto 10).

`revision --help` y `revision <comando> --help` muestran la ayuda completa.

## Dónde se guardan las cosas

| Qué | Dónde (Linux) |
|---|---|
| Descargas | `~/Moodle/<curso>/<sección>/…` |
| Configuración | `~/.config/revision/config.toml` |
| Estado (qué viste, qué descargaste) | `~/.local/share/revision/revision.db` |
| Contraseña y token | Llavero del sistema, **nunca** en un archivo |

Detalles de las descargas:

- Los archivos de carpetas de Moodle mantienen sus subcarpetas. Los adjuntos de anuncios van a `<curso>/Anuncios/` y los de tareas a `<curso>/Tareas/<tarea>/`.
- Si en la carpeta ya existe un archivo tuyo con el mismo nombre, no se sobrescribe: el descargado queda como `Guía (2).pdf`.
- Si un profesor reemplaza un archivo que ya descargaste, se marca como `[descargado, hay versión nueva]` y al volver a descargarlo se actualiza tu copia.

### Configuración

`~/.config/revision/config.toml` se crea al iniciar sesión. Puedes editarlo:

```toml
url = "https://aulavirtual.tu-universidad.cl"
username = "tu.usuario"
download_dir = "~/Moodle"        # carpeta de descargas
courses = ["MAT1620", 1234]      # opcional: revisar solo estos cursos (nombre corto o id)
```

Si inicias sesión en otra aula virtual, el estado se reinicia y la siguiente revisión vuelve a ser el punto de partida.

## Seguridad y privacidad

- La contraseña y el token se guardan en el **llavero del sistema**, cifrados por el sistema operativo. Si el token vence, se renueva solo con la contraseña guardada.
- Solo se conecta por **HTTPS** a tu aula virtual. El token nunca se envía a otro sitio, aunque un archivo apunte a otra dirección.
- Los nombres de archivo que vienen del servidor se limpian, así que una descarga no puede escribir fuera de tu carpeta de descargas.
- Todo queda en tu computador: no hay servidores intermedios ni telemetría.

## Problemas comunes

| Mensaje | Qué significa |
|---|---|
| `Usuario o contraseña incorrectos.` | Revisa tus datos; son los mismos de la página del aula virtual. |
| `Tu Moodle no permite conectarse como la app móvil…` | Tu institución desactivó el acceso para la app móvil. Por ahora no hay alternativa. |
| `La dirección no respondió como un Moodle…` | La URL no es la del aula virtual. Prueba con la dirección base, sin `/my` ni `/login`. |
| `Llavero del sistema: …` | No hay un llavero disponible (pasa por SSH o sin sesión gráfica). Inicia sesión desde tu escritorio. |
| `aviso: …` | Una parte de un curso no se pudo leer. El resto de la revisión sigue normal. |

KWallet o GNOME Keyring pueden pedirte desbloquear el llavero la primera vez.

## Limitaciones

- Lee los últimos 25 anuncios de cada curso.
- No avisa sola: la revisión se ejecuta cuando tú la pides.
- No sube entregas; solo revisa y descarga.

## Desarrollo

```
crates/
├─ core/   revision-core: cliente de Moodle, detección de cambios (SQLite) y descargas
└─ cli/    revision: comandos, salida en terminal y selección de archivos
```

```sh
cargo test --workspace                                 # tests (con un Moodle simulado; no necesitas cuenta)
cargo clippy --workspace --all-targets -- -D warnings  # lint
cargo fmt --all                                        # formato
cargo run -p revision-cli -- revisar                   # ejecutar sin instalar
```

## Próximamente

- **App de escritorio** (Tauri + Svelte) sobre el mismo núcleo, compartiendo configuración y estado con la terminal.
- **IA local** (Ollama) para resumir los archivos descargados y ordenarlos por ramo, unidad o profesor.

---

Proyecto independiente, sin relación con Moodle ni con ninguna institución.
