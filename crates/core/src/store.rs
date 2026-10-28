//! Estado local en SQLite: qué existe en Moodle, qué es novedad y qué se descargó.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::model::{Change, Course, Download, Entry, Item, Kind, SyncReport, describe_change};
use crate::sync::Fetched;
use crate::{Error, Result};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS courses (
    id INTEGER PRIMARY KEY,
    shortname TEXT NOT NULL,
    fullname TEXT NOT NULL,
    teachers TEXT NOT NULL DEFAULT '[]',
    active INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS items (
    key TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    course_id INTEGER NOT NULL,
    data TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    change TEXT,
    change_detail TEXT,
    seen INTEGER NOT NULL DEFAULT 0,
    removed INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL,
    last_changed INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS items_course ON items (course_id, source);
CREATE TABLE IF NOT EXISTS downloads (
    key TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    downloaded_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS downloads_path ON downloads (path);
PRAGMA user_version = 1;
";

const ENTRY_COLUMNS: &str = "i.data, i.change, i.change_detail, i.seen, i.first_seen, i.last_changed,
    d.path, d.fingerprint, d.downloaded_at";

/// Filtro para [`Store::entries`].
#[derive(Debug, Default, Clone)]
pub struct Query {
    pub unseen_only: bool,
    pub kind: Option<Kind>,
    pub course_id: Option<i64>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // WAL + espera: la CLI y la app de escritorio pueden usar la BD a la vez.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        Store::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA)?;
        }
        Ok(Store { conn })
    }

    /// Borra todo (se usa al cambiar de aula virtual).
    pub fn reset(&mut self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM items; DELETE FROM courses; DELETE FROM downloads; DELETE FROM meta;")?;
        Ok(())
    }

    pub fn last_sync(&self) -> Result<Option<i64>> {
        let value: Option<String> =
            self.conn.query_row("SELECT value FROM meta WHERE key = 'last_sync'", [], |r| r.get(0)).optional()?;
        Ok(value.and_then(|v| v.parse().ok()))
    }

    /// Compara lo que hay en Moodle con lo guardado y registra novedades.
    /// La primera vez todo queda como punto de partida, marcado como visto.
    pub fn apply(&mut self, fetched: &Fetched, now: i64) -> Result<SyncReport> {
        let tx = self.conn.transaction()?;
        let baseline = tx.query_row("SELECT 1 FROM meta WHERE key = 'last_sync'", [], |_| Ok(())).optional()?.is_none();
        let mut report = SyncReport {
            baseline,
            courses: fetched.courses.len(),
            items: fetched.items.len(),
            warnings: fetched.warnings.clone(),
            ..SyncReport::default()
        };

        tx.execute("UPDATE courses SET active = 0", [])?;
        for c in &fetched.courses {
            // Si esta vez no se pudieron leer los profesores, se conservan los anteriores.
            tx.execute(
                "INSERT INTO courses (id, shortname, fullname, teachers, active, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                    shortname = excluded.shortname, fullname = excluded.fullname, active = 1,
                    teachers = CASE WHEN excluded.teachers = '[]' THEN courses.teachers ELSE excluded.teachers END,
                    updated_at = excluded.updated_at",
                params![c.id, c.shortname, c.fullname, serde_json::to_string(&c.teachers)?, now],
            )?;
        }

        let seen = baseline as i64;
        for item in &fetched.items {
            let fingerprint = item.fingerprint();
            let data = serde_json::to_string(item)?;
            let existing: Option<(String, String, Option<String>, bool)> = tx
                .query_row("SELECT fingerprint, data, change, seen FROM items WHERE key = ?1", [&item.key], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            match existing {
                None => {
                    tx.execute(
                        "INSERT INTO items (key, kind, source, course_id, data, fingerprint, change, seen, first_seen, last_changed)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'new', ?7, ?8, ?8)",
                        params![item.key, item.kind.as_str(), item.source.as_str(), item.course_id, data, fingerprint, seen, now],
                    )?;
                    if !baseline {
                        report.new += 1;
                    }
                }
                Some((old_fingerprint, old_data, old_change, old_seen)) if old_fingerprint != fingerprint => {
                    // Si seguía sin verse como nuevo, sigue siendo nuevo.
                    let still_new = old_change.as_deref() == Some("new") && !old_seen;
                    let (change, detail) = if still_new {
                        ("new", None)
                    } else {
                        let old: Option<Item> = serde_json::from_str(&old_data).ok();
                        report.updated += 1;
                        ("updated", old.map(|old| describe_change(&old, item)))
                    };
                    tx.execute(
                        "UPDATE items SET data = ?2, fingerprint = ?3, change = ?4, change_detail = ?5,
                            seen = ?6, removed = 0, last_changed = ?7 WHERE key = ?1",
                        params![item.key, data, fingerprint, change, detail, seen, now],
                    )?;
                }
                Some(_) => {
                    tx.execute("UPDATE items SET data = ?2, removed = 0 WHERE key = ?1", params![item.key, data])?;
                }
            }
        }

        let fetched_keys: HashSet<&str> = fetched.items.iter().map(|i| i.key.as_str()).collect();
        for (course_id, source) in &fetched.complete {
            let mut stmt = tx.prepare("SELECT key FROM items WHERE course_id = ?1 AND source = ?2 AND removed = 0")?;
            let keys = stmt
                .query_map(params![course_id, source.as_str()], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for key in keys.iter().filter(|k| !fetched_keys.contains(k.as_str())) {
                tx.execute("UPDATE items SET removed = 1, last_changed = ?2 WHERE key = ?1", params![key, now])?;
                report.removed += 1;
            }
        }

        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('last_sync', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [now.to_string()],
        )?;
        report.unseen = count_unseen(&tx)? as usize;
        tx.commit()?;
        Ok(report)
    }

    /// Elementos vigentes (no eliminados) de cursos activos.
    pub fn entries(&self, query: &Query) -> Result<Vec<Entry>> {
        let mut sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM items i
             JOIN courses c ON c.id = i.course_id AND c.active = 1
             LEFT JOIN downloads d ON d.key = i.key
             WHERE i.removed = 0"
        );
        let mut args: Vec<SqlValue> = Vec::new();
        if query.unseen_only {
            sql.push_str(" AND i.seen = 0");
        }
        if let Some(kind) = query.kind {
            sql.push_str(" AND i.kind = ?");
            args.push(kind.as_str().to_string().into());
        }
        if let Some(id) = query.course_id {
            sql.push_str(" AND i.course_id = ?");
            args.push(id.into());
        }
        sql.push_str(" ORDER BY c.fullname, i.kind, i.last_changed DESC, i.key");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), read_entry)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn entry(&self, key: &str) -> Result<Option<Entry>> {
        let sql =
            format!("SELECT {ENTRY_COLUMNS} FROM items i LEFT JOIN downloads d ON d.key = i.key WHERE i.key = ?1");
        self.conn.query_row(&sql, [key], read_entry).optional()?.transpose()
    }

    pub fn mark_seen(&mut self, keys: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for key in keys {
            tx.execute("UPDATE items SET seen = 1 WHERE key = ?1", [key])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_all_seen(&mut self, course_id: Option<i64>) -> Result<()> {
        match course_id {
            Some(id) => self.conn.execute("UPDATE items SET seen = 1 WHERE course_id = ?1", [id])?,
            None => self.conn.execute("UPDATE items SET seen = 1", [])?,
        };
        Ok(())
    }

    pub fn count_unseen(&self) -> Result<i64> {
        count_unseen(&self.conn)
    }

    pub fn record_download(&mut self, key: &str, path: &Path, fingerprint: &str, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO downloads (key, path, fingerprint, downloaded_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (key) DO UPDATE SET path = excluded.path, fingerprint = excluded.fingerprint,
                downloaded_at = excluded.downloaded_at",
            params![key, path.to_string_lossy(), fingerprint, now],
        )?;
        Ok(())
    }

    /// Qué elemento se descargó en esa ruta, si alguno.
    pub fn download_owner(&self, path: &Path) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT key FROM downloads WHERE path = ?1", [path.to_string_lossy()], |r| r.get(0))
            .optional()?)
    }

    pub fn courses(&self) -> Result<Vec<Course>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.shortname, c.fullname, c.teachers,
                (SELECT COUNT(*) FROM items i WHERE i.course_id = c.id AND i.seen = 0 AND i.removed = 0)
             FROM courses c WHERE c.active = 1 ORDER BY c.fullname",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Course {
                id: r.get(0)?,
                shortname: r.get(1)?,
                fullname: r.get(2)?,
                teachers: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                unseen: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Busca un curso por id, nombre corto o parte del nombre (sin importar tildes).
    pub fn find_course(&self, query: &str) -> Result<Course> {
        let courses = self.courses()?;
        let q = fold(query.trim());
        if let Some(c) =
            courses.iter().find(|c| c.id.to_string() == q || fold(&c.shortname) == q || fold(&c.fullname) == q)
        {
            return Ok(c.clone());
        }
        let matches: Vec<&Course> =
            courses.iter().filter(|c| fold(&c.fullname).contains(&q) || fold(&c.shortname).contains(&q)).collect();
        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(Error::Config(format!("No encontré un curso que coincida con «{query}»."))),
            many => Err(Error::Config(format!(
                "«{query}» coincide con varios cursos: {}. Sé más específico.",
                many.iter().map(|c| c.fullname.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }
}

fn count_unseen(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM items i JOIN courses c ON c.id = i.course_id AND c.active = 1
         WHERE i.seen = 0 AND i.removed = 0",
        [],
        |r| r.get(0),
    )?)
}

/// Lee una fila de [`ENTRY_COLUMNS`]. El error de JSON va aparte para no
/// confundirlo con un error de SQLite.
fn read_entry(r: &Row) -> rusqlite::Result<Result<Entry>> {
    let data: String = r.get(0)?;
    let change: Option<String> = r.get(1)?;
    let path: Option<String> = r.get(6)?;
    let downloaded = match path {
        Some(path) => Some(Download { path, fingerprint: r.get(7)?, at: r.get(8)? }),
        None => None,
    };
    let (change_detail, seen, first_seen, last_changed) = (r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?);
    Ok(serde_json::from_str::<Item>(&data).map_err(Error::from).map(|item| Entry {
        item,
        change: match change.as_deref() {
            Some("new") => Some(Change::New),
            Some("updated") => Some(Change::Updated),
            _ => None,
        },
        change_detail,
        seen,
        first_seen,
        last_changed,
        downloaded,
    }))
}

/// Minúsculas y sin tildes, para buscar «calculo» y encontrar «Cálculo».
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileRef, Source};
    use crate::sync::CourseInfo;

    fn course(id: i64, name: &str) -> CourseInfo {
        CourseInfo { id, shortname: format!("C{id}"), fullname: name.into(), teachers: vec!["Ana".into()] }
    }

    fn file(key: &str, modified: i64) -> Item {
        let mut item = Item::new(key.into(), Kind::File, Source::Contents, 1, "Cálculo", key.into());
        item.file = Some(FileRef {
            filename: key.into(),
            size: 10,
            url: "https://a/f".into(),
            mimetype: None,
            modified,
            subdir: vec![],
        });
        item
    }

    fn fetched(items: Vec<Item>) -> Fetched {
        Fetched {
            courses: vec![course(1, "Cálculo")],
            items,
            complete: HashSet::from([(1, Source::Contents)]),
            warnings: vec![],
        }
    }

    #[test]
    fn primera_revision_y_cambios() {
        let mut store = Store::open_in_memory().unwrap();
        let r = store.apply(&fetched(vec![file("a", 1), file("b", 1)]), 100).unwrap();
        assert!(r.baseline);
        assert_eq!((r.new, r.unseen), (0, 0));

        // a reemplazado, b eliminado, c nuevo.
        let r = store.apply(&fetched(vec![file("a", 2), file("c", 1)]), 200).unwrap();
        assert!(!r.baseline);
        assert_eq!((r.new, r.updated, r.removed, r.unseen), (1, 1, 1, 2));

        let unseen = store.entries(&Query { unseen_only: true, ..Query::default() }).unwrap();
        let a = unseen.iter().find(|e| e.item.key == "a").unwrap();
        assert_eq!(a.change, Some(Change::Updated));
        assert_eq!(a.change_detail.as_deref(), Some("archivo reemplazado"));
        assert_eq!(unseen.iter().find(|e| e.item.key == "c").unwrap().change, Some(Change::New));
        assert!(store.entry("b").unwrap().is_some());
        assert!(store.entries(&Query::default()).unwrap().iter().all(|e| e.item.key != "b"));

        // c cambia antes de verse: sigue siendo «nuevo».
        let r = store.apply(&fetched(vec![file("a", 2), file("c", 5)]), 300).unwrap();
        assert_eq!((r.new, r.updated), (0, 0));
        assert_eq!(store.entry("c").unwrap().unwrap().change, Some(Change::New));

        store.mark_seen(&["a".into(), "c".into()]).unwrap();
        assert_eq!(store.count_unseen().unwrap(), 0);
    }

    #[test]
    fn no_elimina_si_la_consulta_fallo() {
        let mut store = Store::open_in_memory().unwrap();
        store.apply(&fetched(vec![file("a", 1)]), 100).unwrap();
        let mut f = fetched(vec![]);
        f.complete.clear();
        assert_eq!(store.apply(&f, 200).unwrap().removed, 0);
    }

    #[test]
    fn descargas_y_busqueda_de_cursos() {
        let mut store = Store::open_in_memory().unwrap();
        let mut f = fetched(vec![file("a", 1)]);
        f.courses.push(course(2, "Física I"));
        f.courses.push(course(3, "Física II"));
        store.apply(&f, 100).unwrap();

        store.record_download("a", Path::new("/tmp/a"), "1:10", 150).unwrap();
        assert_eq!(store.download_owner(Path::new("/tmp/a")).unwrap().as_deref(), Some("a"));
        let a = store.entry("a").unwrap().unwrap();
        assert_eq!(a.downloaded.as_ref().unwrap().path, "/tmp/a");
        assert!(!a.download_outdated());

        assert_eq!(store.find_course("calculo").unwrap().id, 1);
        assert_eq!(store.find_course("C2").unwrap().id, 2);
        assert!(store.find_course("física").is_err());
        assert!(store.find_course("química").is_err());
        assert_eq!(store.courses().unwrap()[0].teachers, ["Ana"]);
    }
}
