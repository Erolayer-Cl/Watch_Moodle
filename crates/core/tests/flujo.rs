//! Flujo completo contra un Moodle simulado: login, revisión inicial, cambios,
//! segunda revisión, descargas y renovación de token.

use std::path::Path;

use revision_core::{Change, Client, Config, Error, MemorySecrets, Paths, Query, Revision, download};
use wiremock::matchers::{body_string_contains, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(server: &MockServer, name: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(file).unwrap().replace("{BASE}", &server.uri())
}

fn json(body: impl Into<Vec<u8>>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

async fn ws(server: &MockServer, function: &str, extra: Option<&str>, body: String) {
    let mut mock = Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains(format!("wsfunction={function}")));
    if let Some(extra) = extra {
        mock = mock.and(body_string_contains(extra));
    }
    mock.respond_with(json(body)).mount(server).await;
}

async fn mount_moodle(server: &MockServer, version: u8) {
    Mock::given(method("POST"))
        .and(path("/login/token.php"))
        .respond_with(json(r#"{"token": "tok123", "privatetoken": null}"#))
        .mount(server)
        .await;
    ws(server, "core_webservice_get_site_info", None, fixture(server, "site_info.json")).await;
    ws(server, "core_enrol_get_users_courses", None, fixture(server, "users_courses.json")).await;
    ws(server, "core_course_get_courses_by_field", None, fixture(server, "courses_by_field.json")).await;
    ws(server, "mod_forum_get_forums_by_courses", None, fixture(server, "forums.json")).await;
    ws(
        server,
        "mod_forum_get_forum_discussions",
        Some("forumid=900"),
        fixture(server, &format!("discussions_v{version}.json")),
    )
    .await;
    ws(
        server,
        "core_course_get_contents",
        Some("courseid=101"),
        fixture(server, &format!("contents_101_v{version}.json")),
    )
    .await;
    ws(server, "core_course_get_contents", Some("courseid=102"), fixture(server, "contents_102.json")).await;
    ws(server, "mod_assign_get_assignments", None, fixture(server, &format!("assignments_v{version}.json"))).await;
    ws(server, "mod_assign_get_submission_status", None, fixture(server, "submission_status.json")).await;
}

#[tokio::test]
async fn revision_completa_con_moodle_simulado() {
    let server = MockServer::start().await;
    mount_moodle(&server, 1).await;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(dir.path());
    let mut config = Config::new(server.uri(), "estudiante".into());
    config.download_dir = dir.path().join("Moodle").to_string_lossy().into_owned();
    config.save(&paths.config_file).unwrap();

    let secrets = MemorySecrets::default();
    let mut rev =
        Revision::login(&paths, Box::new(secrets.clone()), &server.uri(), "estudiante", "secreto").await.unwrap();
    assert_eq!(rev.config().download_dir, config.download_dir, "el login conserva la carpeta de descargas");

    // Primera revisión: todo queda como punto de partida.
    let r1 = rev.sync().await.unwrap();
    assert!(r1.baseline);
    assert!(r1.warnings.is_empty(), "{:?}", r1.warnings);
    assert_eq!(r1.unseen, 0);
    let keys: Vec<String> = rev.store().entries(&Query::default()).unwrap().into_iter().map(|e| e.item.key).collect();
    for expected in [
        "file:510:/Guía 1.pdf",
        "file:511:/semana1/ejercicios.pdf",
        "mod:514",
        "mod:600",
        "ann:7001",
        "assign:300",
        "assignfile:300:/enunciado.pdf",
    ] {
        assert!(keys.iter().any(|k| k == expected), "falta {expected} en {keys:?}");
    }
    // Foro de anuncios, etiqueta, tarea (va por su API) y recurso oculto no son actividades.
    for excluded in ["mod:500", "mod:501", "mod:512", "file:513:/oculto.pdf"] {
        assert!(!keys.iter().any(|k| k == excluded), "no debería estar {excluded}");
    }
    let tarea = rev.store().entry("assign:300").unwrap().unwrap();
    assert_eq!(tarea.item.status.as_deref(), Some("new"));
    assert_eq!(tarea.item.section.as_deref(), Some("Unidad 1"));
    assert_eq!(rev.store().find_course("calculo").unwrap().teachers, ["Ana Rojas"]);

    // Segunda revisión: Guía 1 reemplazada, Guía 2 nueva, carpeta eliminada,
    // anuncio nuevo con adjunto y fecha de entrega movida.
    server.reset().await;
    mount_moodle(&server, 2).await;
    let r2 = rev.sync().await.unwrap();
    assert!(!r2.baseline);
    assert_eq!((r2.new, r2.updated, r2.removed, r2.unseen), (3, 2, 1, 5), "{r2:?}");

    let unseen = rev.store().entries(&Query { unseen_only: true, ..Query::default() }).unwrap();
    let find = |key: &str| unseen.iter().find(|e| e.item.key == key).unwrap_or_else(|| panic!("falta {key}"));
    assert_eq!(find("file:515:/Guía 2.pdf").change, Some(Change::New));
    assert_eq!(find("ann:7002").change, Some(Change::New));
    assert_eq!(find("annfile:7002:/sala.pdf").item.parent.as_deref(), Some("ann:7002"));
    assert_eq!(find("file:510:/Guía 1.pdf").change_detail.as_deref(), Some("archivo reemplazado"));
    let tarea = find("assign:300");
    assert_eq!(tarea.change, Some(Change::Updated));
    assert!(tarea.change_detail.as_deref().unwrap().starts_with("entrega: "), "{:?}", tarea.change_detail);

    // Descarga con el token, dentro de la carpeta del curso y la sección.
    Mock::given(method("GET"))
        .and(path_regex(r"^/webservice/pluginfile\.php/12/"))
        .and(query_param("token", "tok123"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("contenido guía 2", "application/pdf"))
        .mount(&server)
        .await;
    let jobs = rev.plan_downloads(&["file:515:/Guía 2.pdf".into()]).unwrap();
    assert_eq!(jobs[0].target, dir.path().join("Moodle/Cálculo II/Unidad 1/Guía 2.pdf"));
    download::fetch(rev.client(), &jobs[0], |_, _| {}).await.unwrap();
    rev.record_download(&jobs[0]).unwrap();
    assert_eq!(std::fs::read_to_string(&jobs[0].target).unwrap(), "contenido guía 2");
    assert!(rev.store().entry("file:515:/Guía 2.pdf").unwrap().unwrap().downloaded.is_some());

    // Un archivo propio con el mismo nombre no se pisa.
    std::fs::write(dir.path().join("Moodle/Cálculo II/Unidad 1/Guía 1.pdf"), "mío").unwrap();
    let jobs = rev.plan_downloads(&["file:510:/Guía 1.pdf".into()]).unwrap();
    assert_eq!(jobs[0].target, dir.path().join("Moodle/Cálculo II/Unidad 1/Guía 1 (2).pdf"));

    let keys: Vec<String> = unseen.iter().map(|e| e.item.key.clone()).collect();
    rev.store_mut().mark_seen(&keys).unwrap();
    assert_eq!(rev.store().count_unseen().unwrap(), 0);

    // La sesión se reabre con lo guardado en el llavero.
    drop(rev);
    let rev = Revision::open(&paths, Box::new(secrets)).unwrap();
    assert_eq!(rev.client().token(), "tok123");
}

#[tokio::test]
async fn renueva_el_token_vencido() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/token.php"))
        .and(body_string_contains("username=estudiante"))
        .respond_with(json(r#"{"token": "nuevo"}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wstoken=viejo"))
        .respond_with(json(
            r#"{"exception": "moodle_exception", "errorcode": "invalidtoken", "message": "Invalid token"}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webservice/rest/server.php"))
        .and(body_string_contains("wstoken=nuevo"))
        .respond_with(json(fixture(&server, "site_info.json")))
        .mount(&server)
        .await;

    let client = Client::with_token(&server.uri(), "viejo", Some(("estudiante".into(), "secreto".into()))).unwrap();
    assert_eq!(client.site_info().await.unwrap().userid, 7);
    assert_eq!(client.token(), "nuevo");
}

#[tokio::test]
async fn explica_los_errores_de_login() {
    let cases = [
        (
            json(r#"{"error": "Invalid login, please try again", "errorcode": "invalidlogin"}"#),
            "Usuario o contraseña incorrectos.",
        ),
        (
            json(
                r#"{"error": "Web services must be enabled in Advanced features.", "errorcode": "enablewsdescription"}"#,
            ),
            "Tu Moodle no permite conectarse como la app móvil",
        ),
        (
            ResponseTemplate::new(404).set_body_raw("<html>No encontrado</html>", "text/html"),
            "no respondió como un Moodle",
        ),
    ];
    for (response, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/login/token.php")).respond_with(response).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());
        let result =
            Revision::login(&paths, Box::new(MemorySecrets::default()), &server.uri(), "estudiante", "x").await;
        let Err(err) = result else { panic!("el login debió fallar") };
        assert!(matches!(err, Error::Moodle { .. }), "{err:?}");
        assert!(err.to_string().contains(expected), "«{err}» no contiene «{expected}»");
        assert!(!paths.config_file.exists(), "no se guarda nada si el login falla");
    }
}
