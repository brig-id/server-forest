//! A single-node server becomes a forest (`forest.md` §9): its database, as
//! its `backup` command copies it, imported into PostgreSQL. The person's
//! passkey, the site's registration and the identifier the site knows the
//! person by are the same after the move.

mod common;

use std::time::{Duration, Instant};

use common::{Database, KEY, Person, REDIRECT, Settings, Spread, onboard, published_keys};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_ui.rs"));
}

/// What seedling and grove run: `brigid_server::run`, one node on a SQLite
/// file. Started in this process, on its own thread, until the process ends.
fn start_single_node(dir: &std::path::Path, port: u16, public_url: &str) {
    let key_file = dir.join("master.key");
    std::fs::write(&key_file, KEY).unwrap();
    let config = dir.join("single.toml");
    std::fs::write(
        &config,
        format!(
            "[server]\ndomain = \"localhost\"\nhost = \"127.0.0.1\"\nport = {port}\n\
             public_url = \"{public_url}\"\n\n[database]\npath = \"{}\"\n",
            dir.join("single.db").display()
        ),
    )
    .unwrap();
    let edition: &'static brigid_server::Edition = Box::leak(Box::new(brigid_server::Edition {
        name: "grove",
        version: "0.0.0",
        ui: brigid_server::EmbeddedUi {
            files: embedded::FILES,
            csp_hashes: embedded::CSP_HASHES,
        },
        config_path: None,
        key_path: Some(Box::leak(
            key_file.to_str().unwrap().to_owned().into_boxed_str(),
        )),
        sealed_key_path: None,
        key_hint: "",
        root_hint: "",
    }));
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(brigid_server::run(edition, Some(config), None));
    });
}

#[tokio::test]
async fn a_single_node_server_becomes_a_forest_and_nothing_changes_for_people_and_sites() {
    let Some(database) = Database::new().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let public_url = format!("http://localhost:{port}");

    // -- The server as it is today: one node, a SQLite file -----------------
    // The operator's commands, done through the store they are made of.
    let single_db = dir.path().join("single.db");
    let master = || brigid_store::MasterKey::from_hex(KEY).unwrap();
    let store =
        brigid_store::EncryptedStore::new(&format!("sqlite:{}", single_db.display()), master())
            .await
            .unwrap();
    store
        .set_signup_policy(brigid_store::SignupPolicy::Open)
        .await
        .unwrap();
    let (site, secret) = store
        .create_client("Shop", &[REDIRECT.to_owned()], true)
        .await
        .unwrap();
    let secret = secret.unwrap();
    store.close().await;

    start_single_node(dir.path(), port, &public_url);
    let single = common::Node::listening_on(port);
    let deadline = Instant::now() + Duration::from_secs(30);
    while single.status("/health").await != Some(200) {
        assert!(Instant::now() < deadline, "the single node did not start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let on_single = Spread::over(&[&single], &public_url);
    let (mut person, cookie) = Person::sign_up(&on_single, &public_url).await;
    let identity = onboard(&on_single, &cookie).await;
    let (claims, _) =
        common::sign_in_to_site(&on_single, &cookie, &identity, &site.client_id, &secret).await;
    let subject = claims["sub"].as_str().unwrap().to_owned();
    let single_keys = published_keys(&single).await;

    // -- Its backup, taken while it runs -------------------------------------
    let backup = dir.path().join("backup.db");
    brigid_store::backup::copy_database(&single_db, &backup)
        .await
        .unwrap();

    // -- The forest: initialised, then filled with the backup ---------------
    let settings = Settings {
        database: database.as_owner(),
        public_url: public_url.clone(),
    };
    let backup_arg = backup.to_str().unwrap();
    // Not before `init`.
    let (ok, _, err) = settings.forest(&["import", backup_arg]);
    assert!(!ok && err.contains("run `init`"), "{err}");
    settings.ok(&["init"]);
    // Not from another domain, unless the operator says so.
    let elsewhere = Settings {
        public_url: "https://id.example.org".into(),
        ..settings.clone()
    };
    let (ok, _, err) = elsewhere.forest(&["import", backup_arg]);
    assert!(!ok && err.contains("bound to that domain"), "{err}");
    assert_eq!(database.count("users").await, 0);

    let (ok, out, err) = settings.forest(&["import", backup_arg]);
    assert!(ok, "forest import: {out}{err}");
    assert!(out.contains("users\t1"), "{out}");
    assert!(err.contains("1 accounts, 1 identities"), "{err}");
    // An import fills an empty server, once.
    let (ok, _, err) = settings.forest(&["import", backup_arg]);
    assert!(!ok && err.contains("already has accounts"), "{err}");

    // -- Nothing changed for the person, nor for the site -------------------
    let (first, second) = (settings.node(), settings.node());
    first.ready().await;
    second.ready().await;
    assert_eq!(published_keys(&first).await, single_keys);
    let listed = settings.ok(&["client", "list"]);
    assert!(listed.contains(&site.client_id), "{listed}");

    let spread = Spread::over(&[&first, &second], &public_url);
    // The session opened on the single node is still open.
    let resp = spread.get("/account/identities", Some(&cookie)).await;
    assert_eq!(resp.status(), 200);
    // The passkey signs in, and the site sees the same person.
    let cookie = person.sign_in(&spread).await;
    let (claims, _) =
        common::sign_in_to_site(&spread, &cookie, &identity, &site.client_id, &secret).await;
    assert_eq!(claims["sub"], subject.as_str());
    assert_eq!(claims["iss"], public_url);
}
