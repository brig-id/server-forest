//! What the tests of forest share: a PostgreSQL schema per test, the
//! `forest` executable run as a command or as a node, and a person with a
//! software passkey who talks to the nodes in turn.
//!
//! The tests need a PostgreSQL database, named by `BRIGID_TEST_POSTGRES`
//! (`postgres://user:password@host:5432/database`, a role that may create
//! schemas and roles). Without it each test says so and passes: CI sets it.

// Each test file includes this module and uses part of it.
#![allow(dead_code)]

use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use webauthn_authenticator_rs::{WebauthnAuthenticator as _, softpasskey::SoftPasskey};
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse};

/// Fixed, valid-looking master key (32 bytes in hex).
pub const KEY: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

// RFC 7636 appendix B.
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
pub const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
pub const REDIRECT: &str = "https://shop.example/cb";

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A database of the test's own: a new schema in the PostgreSQL database
/// `BRIGID_TEST_POSTGRES` names.
pub struct Database {
    /// The address without credentials: `host:port/database?…search_path…`.
    rest: String,
    /// `user:password` of the role that owns the schema.
    owner: String,
    pub schema: String,
    admin: sqlx::PgPool,
}

impl Database {
    /// `None`, and a word on standard error, without `BRIGID_TEST_POSTGRES`.
    pub async fn new() -> Option<Self> {
        let Ok(base) = std::env::var("BRIGID_TEST_POSTGRES") else {
            eprintln!("BRIGID_TEST_POSTGRES is not set: no PostgreSQL to test forest on");
            return None;
        };
        let admin = sqlx::PgPool::connect(&base)
            .await
            .expect("BRIGID_TEST_POSTGRES names a database that answers");
        let schema = format!("t{:032x}", rand_u128());
        // The schema's name is drawn here, never input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("a schema for the test");
        let (_, rest) = base.split_once("://").expect("a URL");
        let (owner, host) = rest.split_once('@').expect("a role in the address");
        Some(Self {
            // No TLS on the test database: the operator's written choice.
            rest: format!("{host}?options=-c%20search_path%3D{schema}&sslmode=disable"),
            owner: owner.to_owned(),
            schema,
            admin,
        })
    }

    /// The address as the role that owns the schema: what `init` runs as.
    pub fn as_owner(&self) -> String {
        format!("postgres://{}@{}", self.owner, self.rest)
    }

    /// A new role that can log in and owns nothing: what the nodes run as,
    /// once `init --grant` let it read and write rows. Returns its name and
    /// the address as that role.
    pub async fn node_role(&self) -> (String, String) {
        let role = format!("node_{}", &self.schema[1..13]);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN PASSWORD 'node'"
        )))
        .execute(&self.admin)
        .await
        .expect("a role for the nodes");
        let address = format!("postgres://{role}:node@{}", self.rest);
        (role, address)
    }

    /// Runs a statement as the owner, in the test's schema.
    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {}.{table}",
            self.schema
        )))
        .fetch_one(&self.admin)
        .await
        .unwrap()
    }
}

fn rand_u128() -> u128 {
    use std::hash::{BuildHasher, Hasher};
    let a = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let b = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (u128::from(a) << 64) | u128::from(b)
}

/// How a `forest` process is configured: the database, and the address
/// people reach the server at (the same for every node).
#[derive(Clone)]
pub struct Settings {
    pub database: String,
    pub public_url: String,
}

impl Settings {
    fn apply(&self, command: &mut Command) {
        command
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("BRIGID_MASTER_KEY", KEY)
            .env("BRIGID_SERVER__DOMAIN", "localhost")
            .env("BRIGID_SERVER__PUBLIC_URL", &self.public_url)
            .env("BRIGID_DATABASE__URL", &self.database)
            .env("BRIGID_DATABASE__ALLOW_UNVERIFIED", "true")
            .env("BRIGID_DATABASE__MAX_CONNECTIONS", "4");
    }

    /// Runs `forest <args…>`: whether it succeeded, its standard output and
    /// its standard error.
    pub fn forest(&self, args: &[&str]) -> (bool, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_forest"));
        self.apply(&mut command);
        let out = command.args(args).output().expect("run forest");
        (
            out.status.success(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    /// `forest <args…>`, which must succeed; its standard output.
    pub fn ok(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.forest(args);
        assert!(ok, "forest {}: {out}{err}", args.join(" "));
        out
    }

    /// Starts a node on a free port. It may not be ready yet.
    pub fn node(&self) -> Node {
        let port = free_port();
        let mut command = Command::new(env!("CARGO_BIN_EXE_forest"));
        self.apply(&mut command);
        let child = command
            .env("BRIGID_SERVER__HOST", "127.0.0.1")
            .env("BRIGID_SERVER__PORT", port.to_string())
            .env("BRIGID_SERVER__SHUTDOWN_DELAY_SECONDS", "2")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start a node");
        Node { child, port }
    }
}

/// One node: a `forest` process, stopped when the value is dropped.
pub struct Node {
    child: Child,
    pub port: u16,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Node {
    /// Something that already listens on `port`, and is not this value's to
    /// stop: a server started in the test's own process.
    pub fn listening_on(port: u16) -> Self {
        // A process that ends at once, so that dropping the value stops
        // nothing.
        let child = Command::new("true").spawn().expect("run true");
        Self { child, port }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// The status of `GET path`, or `None` when the node does not answer.
    pub async fn status(&self, path: &str) -> Option<u16> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        client
            .get(self.url(path))
            .send()
            .await
            .ok()
            .map(|resp| resp.status().as_u16())
    }

    /// Waits until `GET /ready` says 200.
    pub async fn ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.status("/ready").await != Some(200) {
            assert!(Instant::now() < deadline, "a node never became ready");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Tells the node to stop, as a platform does (SIGTERM).
    pub fn stop(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("kill");
        assert!(status.success());
    }

    /// Whether the process ended, and how.
    pub fn ended(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }
}

/// What stands in front of the nodes in a test: each request goes to the
/// next node. No node answers two requests in a row.
pub struct Spread<'a> {
    nodes: Vec<&'a Node>,
    turn: AtomicUsize,
    /// A browser on the server's own pages: it says where it comes from
    /// (`Origin`), without which the server changes nothing for a session.
    pub client: reqwest::Client,
    /// The server of a site: no cookie, no `Origin`.
    pub site: reqwest::Client,
}

impl<'a> Spread<'a> {
    pub fn over(nodes: &[&'a Node], public_url: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ORIGIN,
            public_url.trim_end_matches('/').parse().unwrap(),
        );
        let client = || {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
        };
        Self {
            nodes: nodes.to_vec(),
            turn: AtomicUsize::new(0),
            client: client().default_headers(headers).build().unwrap(),
            site: client().build().unwrap(),
        }
    }

    /// The address of `path` on the next node.
    pub fn next(&self, path: &str) -> String {
        let n = self.turn.fetch_add(1, Ordering::Relaxed) % self.nodes.len();
        self.nodes[n].url(path)
    }

    pub async fn get(&self, path: &str, cookie: Option<&str>) -> reqwest::Response {
        let mut request = self.client.get(self.next(path));
        if let Some(cookie) = cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        request.send().await.unwrap()
    }

    pub async fn post(&self, path: &str, cookie: Option<&str>, body: Value) -> reqwest::Response {
        let mut request = self.client.post(self.next(path)).json(&body);
        if let Some(cookie) = cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        request.send().await.unwrap()
    }
}

pub async fn json_of(resp: reqwest::Response) -> Value {
    let status = resp.status();
    let text = resp.text().await.unwrap();
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("{status}: not JSON: {text}"))
}

/// The session cookie a response sets, as `name=value`.
pub fn session_cookie(resp: &reqwest::Response) -> String {
    let set = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .unwrap();
    set.split(';').next().unwrap().to_owned()
}

/// A person: a software passkey, and what the server said of it when it was
/// registered (needed to answer a sign-in, since `SoftPasskey` keeps no
/// discoverable credential).
pub struct Person {
    passkey: SoftPasskey,
    origin: url::Url,
    credential_id: String,
    account: Vec<u8>,
}

impl Person {
    /// Creates an account through `nodes` and returns the person with their
    /// session cookie. The server must take sign-ups (`forest signup open`).
    pub async fn sign_up(nodes: &Spread<'_>, public_url: &str) -> (Self, String) {
        Self::sign_up_with(nodes, public_url, None).await
    }

    /// [`Self::sign_up`], with the link that allows it on a server that
    /// asks for one: the setup link of the first administrator, an
    /// invitation.
    pub async fn sign_up_with(
        nodes: &Spread<'_>,
        public_url: &str,
        link: Option<&str>,
    ) -> (Self, String) {
        let origin = url::Url::parse(public_url).unwrap();
        let mut passkey = SoftPasskey::new(true);
        let body = match link {
            Some(link) => json!({ "link": link }),
            None => json!({}),
        };
        let resp = nodes.post("/auth/register/begin", None, body).await;
        assert_eq!(resp.status(), 200, "{}", resp.text().await.unwrap());
        let begin = json_of(resp).await;
        let mut challenge: CreationChallengeResponse =
            serde_json::from_value(begin["challenge"].clone()).unwrap();
        let account: Vec<u8> = challenge.public_key.user.id.clone();
        // The server asks for a discoverable passkey, which the software
        // one refuses: check the request, then relax it.
        let selection = challenge
            .public_key
            .authenticator_selection
            .as_mut()
            .expect("an authenticator selection");
        assert!(selection.require_resident_key);
        selection.resident_key = Some(webauthn_rs_proto::ResidentKeyRequirement::Discouraged);
        selection.require_resident_key = false;
        let credential = passkey
            .do_registration(origin.clone(), challenge)
            .expect("registration ceremony");
        let credential_id = serde_json::to_value(&credential).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let resp = nodes
            .post(
                "/auth/register/finish",
                None,
                json!({ "session_id": begin["session_id"], "credential": credential }),
            )
            .await;
        if resp.status() != 204 {
            panic!("sign-up: {} {}", resp.status(), resp.text().await.unwrap());
        }
        let cookie = session_cookie(&resp);
        (
            Self {
                passkey,
                origin,
                credential_id,
                account,
            },
            cookie,
        )
    }

    /// Signs in with the passkey through `nodes`: a new session cookie.
    pub async fn sign_in(&mut self, nodes: &Spread<'_>) -> String {
        let resp = nodes.post("/auth/login/begin", None, json!({})).await;
        assert_eq!(resp.status(), 200);
        let begin = json_of(resp).await;
        let mut challenge: RequestChallengeResponse =
            serde_json::from_value(begin["challenge"].clone()).unwrap();
        assert!(challenge.public_key.allow_credentials.is_empty());
        challenge.public_key.allow_credentials =
            serde_json::from_value(json!([{ "type": "public-key", "id": self.credential_id }]))
                .unwrap();
        challenge.mediation = None;
        let mut answer = self
            .passkey
            .do_authentication(self.origin.clone(), challenge)
            .expect("authentication ceremony");
        answer.response.user_handle = Some(self.account.clone());
        let resp = nodes
            .post(
                "/auth/login/finish",
                None,
                json!({ "session_id": begin["session_id"], "credential": answer }),
            )
            .await;
        assert_eq!(resp.status(), 204, "sign-in");
        session_cookie(&resp)
    }
}

/// The end of the onboarding: the recovery code, then one identity, whose
/// id is returned.
pub async fn onboard(nodes: &Spread<'_>, cookie: &str) -> String {
    let resp = nodes
        .post("/account/recovery-code", Some(cookie), json!({}))
        .await;
    assert_eq!(resp.status(), 200);
    let created = json_of(
        nodes
            .post(
                "/account/identities",
                Some(cookie),
                json!({ "name": "Personal", "logo": "icon:ph:user" }),
            )
            .await,
    )
    .await;
    created["id"].as_str().expect("an identity").to_owned()
}

/// `key=value` printed by a command.
pub fn output_value(stdout: &str, key: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(str::to_owned)
}

/// Registers a site with the command line; its `client_id` and its secret.
pub fn add_site(settings: &Settings) -> (String, String) {
    let out = settings.ok(&[
        "client",
        "add",
        "--name",
        "Shop",
        "--redirect-uri",
        REDIRECT,
    ]);
    (
        output_value(&out, "client_id").expect("client_id is printed"),
        output_value(&out, "client_secret").expect("a secret is printed once"),
    )
}

/// A whole sign-in of `identity` to the site, each request on the next
/// node: the claims of the ID token, and the form that got it (to present
/// the code again).
pub async fn sign_in_to_site(
    nodes: &Spread<'_>,
    cookie: &str,
    identity: &str,
    client_id: &str,
    secret: &str,
) -> (Value, String) {
    use base64ct::{Base64UrlUnpadded, Encoding};
    let resp = nodes
        .get(
            &format!(
                "/authorize?response_type=code&scope=openid&client_id={client_id}\
                 &redirect_uri=https%3A%2F%2Fshop.example%2Fcb&state=xyz&nonce=n-0S6\
                 &code_challenge={CHALLENGE}&code_challenge_method=S256"
            ),
            None,
        )
        .await;
    assert_eq!(resp.status(), 303);
    let location = resp.headers()["location"].to_str().unwrap().to_owned();
    let request = location
        .strip_prefix("/continue/?request=")
        .unwrap_or_else(|| panic!("unexpected redirect: {location}"));

    let view = json_of(
        nodes
            .get(&format!("/auth/authorize/{request}"), Some(cookie))
            .await,
    )
    .await;
    assert_eq!(view["client"]["name"], "Shop");
    let resp = nodes
        .post(
            &format!("/auth/authorize/{request}/approve"),
            Some(cookie),
            json!({ "identity_id": identity }),
        )
        .await;
    assert_eq!(resp.status(), 200);
    let answer = json_of(resp).await;
    let back = url::Url::parse(answer["redirect_to"].as_str().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = back.query_pairs().into_owned().collect();

    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("grant_type", "authorization_code"),
            ("code", query["code"].as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .finish();
    let resp = exchange(nodes, &form, client_id, secret).await;
    assert_eq!(resp.status(), 200);
    let tokens = json_of(resp).await;
    let payload = tokens["id_token"]
        .as_str()
        .expect("an ID token")
        .split('.')
        .nth(1)
        .expect("a JWT")
        .to_owned();
    let claims = serde_json::from_slice(&Base64UrlUnpadded::decode_vec(&payload).unwrap()).unwrap();
    (claims, form)
}

/// `POST /token` on the next node, as the site's server does.
pub async fn exchange(
    nodes: &Spread<'_>,
    form: &str,
    client_id: &str,
    secret: &str,
) -> reqwest::Response {
    nodes
        .site
        .post(nodes.next("/token"))
        .basic_auth(client_id, Some(secret))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form.to_owned())
        .send()
        .await
        .unwrap()
}

/// The `kid` of every key a node publishes.
pub async fn published_keys(node: &Node) -> Vec<String> {
    let keys = json_of(
        reqwest::get(node.url("/.well-known/jwks.json"))
            .await
            .unwrap(),
    )
    .await;
    keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key["kid"].as_str().unwrap().to_owned())
        .collect()
}

/// Whether `path` exists.
pub fn exists(path: &Path) -> bool {
    path.exists()
}
