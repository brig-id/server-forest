//! Several nodes of forest on one PostgreSQL database: real processes, real
//! HTTP, a whole passkey ceremony with a software passkey. What `forest.md`
//! (brig-id/spec) promises of the nodes is tried here on the executable.

mod common;

use std::time::{Duration, Instant};

use common::{Database, Person, Settings, Spread, add_site, exchange, onboard, published_keys};

/// A node holds nothing another node needs: of a sign-up, the onboarding, a
/// sign-in to a site and the exchange of its code, no two requests in a row
/// are answered by the same node. The nodes connect as a database role that
/// cannot change the schema.
#[tokio::test]
async fn any_node_answers_any_request_of_a_sign_in() {
    let Some(database) = Database::new().await else {
        return;
    };
    let (role, as_node) = database.node_role().await;
    let public_url = format!("http://localhost:{}", common::free_port());
    let owner = Settings {
        database: database.as_owner(),
        public_url: public_url.clone(),
    };
    let nodes_settings = Settings {
        database: as_node,
        public_url: public_url.clone(),
    };

    // The initialisation, once, as the role that owns the schema.
    let (ok, out, err) = owner.forest(&["init", "--grant", &role]);
    assert!(ok, "forest init: {out}{err}");
    assert!(
        out.trim()
            .starts_with(&format!("{public_url}/setup/#link=")),
        "the setup link of the first administrator: {out}"
    );
    // The commands of the operator work as the nodes' role.
    assert_eq!(nodes_settings.ok(&["signup", "open"]).trim(), "open");
    let (client_id, secret) = add_site(&nodes_settings);

    let (first, second) = (nodes_settings.node(), nodes_settings.node());
    first.ready().await;
    second.ready().await;
    for node in [&first, &second] {
        let about = common::json_of(reqwest::get(node.url("/auth/server")).await.unwrap()).await;
        assert_eq!(about["edition"], "forest", "{about}");
        assert_eq!(about["version"], env!("CARGO_PKG_VERSION"));
    }
    // Both read the key `init` made, and created none.
    assert_eq!(published_keys(&first).await, published_keys(&second).await);
    assert_eq!(database.count("oidc_keys").await, 1);

    let spread = Spread::over(&[&first, &second], &public_url);
    let (mut person, cookie) = Person::sign_up(&spread, &public_url).await;
    let identity = onboard(&spread, &cookie).await;
    let (claims, form) =
        common::sign_in_to_site(&spread, &cookie, &identity, &client_id, &secret).await;
    assert_eq!(claims["iss"], public_url);
    assert_eq!(claims["aud"], client_id);
    let subject = claims["sub"].as_str().unwrap().to_owned();

    // The code was taken by one node: neither gives a token for it again.
    for _ in 0..2 {
        assert_eq!(
            exchange(&spread, &form, &client_id, &secret).await.status(),
            400
        );
    }

    // A passkey ceremony begun on one node is finished on the other, and
    // the identifier the site receives does not depend on the node.
    let cookie = person.sign_in(&spread).await;
    let (claims, _) =
        common::sign_in_to_site(&spread, &cookie, &identity, &client_id, &secret).await;
    assert_eq!(claims["sub"], subject.as_str());

    // What lived minutes is in the database, and nothing of it readable:
    // rows of `pending` are swept once expired, none is left by a code taken.
    assert_eq!(database.count("pending").await, 0);

    // A node told to stop is not ready first, then ends; the person's
    // session goes on with the other one.
    let mut first = first;
    first.stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut said_not_ready = false;
    while first.ended().is_none() {
        assert!(Instant::now() < deadline, "the node did not stop");
        if first.status("/ready").await == Some(503) {
            said_not_ready = true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(said_not_ready, "a stopping node says it is not ready");
    assert!(first.ended().unwrap().success());
    let alone = Spread::over(&[&second], &public_url);
    let resp = alone.get("/account/identities", Some(&cookie)).await;
    assert_eq!(resp.status(), 200);
    let (claims, _) =
        common::sign_in_to_site(&alone, &cookie, &identity, &client_id, &secret).await;
    assert_eq!(claims["sub"], subject.as_str());
}

/// A node creates nothing: started on a database nobody initialised it
/// waits, leaves the database empty, and serves once `forest init` ran.
#[tokio::test]
async fn a_node_waits_for_the_initialisation_and_creates_nothing() {
    let Some(database) = Database::new().await else {
        return;
    };
    let settings = Settings {
        database: database.as_owner(),
        public_url: format!("http://localhost:{}", common::free_port()),
    };
    let mut node = settings.node();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(node.ended().is_none(), "the node waits");
    assert_eq!(
        node.status("/health").await,
        None,
        "it listens to nothing yet"
    );
    // Nothing in the schema: no table at all.
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = $1",
    )
    .bind(&database.schema)
    .fetch_one(
        &sqlx::PgPool::connect(&std::env::var("BRIGID_TEST_POSTGRES").unwrap())
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(tables, 0);

    // Commands refuse too, and say what to do.
    let (ok, _, err) = settings.forest(&["signup", "open"]);
    assert!(!ok && err.contains("run `init`"), "{err}");

    settings.ok(&["init"]);
    node.ready().await;
    // Two initialisations at once: one waits for the other, both succeed,
    // and the server still has one secret and one key.
    let (a, b) = (settings.clone(), settings.clone());
    let (a, b) = tokio::join!(
        tokio::task::spawn_blocking(move || a.forest(&["init"])),
        tokio::task::spawn_blocking(move || b.forest(&["init"])),
    );
    assert!(a.unwrap().0 && b.unwrap().0);
    assert_eq!(database.count("oidc_keys").await, 1);
    assert_eq!(database.count("server_secrets").await, 1);
    assert_eq!(node.status("/ready").await, Some(200));
}

/// A rotation of the signing key needs no restart: within the minute every
/// node publishes the new key, and goes on signing with the one before
/// until the new one has been published long enough.
#[tokio::test]
async fn a_new_signing_key_reaches_the_nodes_without_a_restart() {
    let Some(database) = Database::new().await else {
        return;
    };
    let settings = Settings {
        database: database.as_owner(),
        public_url: format!("http://localhost:{}", common::free_port()),
    };
    settings.ok(&["init"]);
    let (first, second) = (settings.node(), settings.node());
    first.ready().await;
    second.ready().await;
    let before = published_keys(&first).await;
    assert_eq!(before.len(), 1);

    let (ok, _, err) = settings.forest(&["rotate-oidc-key"]);
    assert!(ok && err.contains("No restart"), "{err}");

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let (a, b) = (published_keys(&first).await, published_keys(&second).await);
        if a.len() == 2 && b.len() == 2 {
            // The key that signs comes first: still the one from before.
            assert_eq!(a[0], before[0]);
            assert_eq!(b[0], before[0]);
            assert_eq!(a, b);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the nodes never published the new key: {a:?} {b:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
