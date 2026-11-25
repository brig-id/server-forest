//! Plays, against a filled database, the statements that requests to a
//! brig·id server cost, at forest's peak rate, and reports how long each
//! request waited for the database.
//!
//! It answers one question of the design (spec `forest.md` §3): where does
//! the state live that a single node keeps in memory (passkey ceremonies in
//! progress, sign-in requests of applications, their codes)? In PostgreSQL,
//! in a table or an unlogged table, or in a cache next to it.
//!
//! It measures the database only. What a node computes (the passkey
//! signature check, the token signature, encryption) is not here: it is the
//! same whatever the answer, and it scales with the number of nodes.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, ValueEnum};
use hdrhistogram::Histogram;
use md5::{Digest, Md5};
use rand::Rng;
use redis::aio::MultiplexedConnection;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::task::JoinSet;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Ephemeral {
    /// A table like the others: written to the log, replicated.
    Logged,
    /// An unlogged table: emptied by a crash or a failover.
    Unlogged,
    /// A cache next to the database (Valkey or Redis).
    Cache,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Touch {
    /// Every request pushes the session's idle deadline back: what the
    /// store of `core` does today.
    Always,
    /// The deadline is written only when it moves by a minute or more.
    Coarse,
}

#[derive(Parser)]
#[command(about = "Measures what forest's peak costs a database")]
struct Args {
    /// PostgreSQL, filled by `fill.sql`.
    #[arg(long, env = "BENCH_DATABASE_URL")]
    database_url: String,
    /// The cache, for `--ephemeral cache`.
    #[arg(long, env = "BENCH_CACHE_URL")]
    cache_url: Option<String>,
    #[arg(long, value_enum, default_value = "logged")]
    ephemeral: Ephemeral,
    #[arg(long, value_enum, default_value = "always")]
    touch: Touch,
    /// Accounts the database was filled with.
    #[arg(long, default_value_t = 1_000_000)]
    accounts: u64,
    /// Multiplies the peak: 1 is 2,900 requests per second.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// Seconds measured, after the warm-up.
    #[arg(long, default_value_t = 180)]
    seconds: u64,
    /// Seconds played before measuring.
    #[arg(long, default_value_t = 20)]
    warmup: u64,
    /// Connections to the database: what all the nodes hold together.
    #[arg(long, default_value_t = 64)]
    connections: u32,
}

// What a second of peak is made of (spec `editions.md` §2: 2,900 operations
// per second for a million accounts).
/// Sign-ins to an application with a passkey ceremony: 6 requests each.
const FULL_PER_SECOND: f64 = 150.0;
/// Sign-ins to an application from an open session: 4 requests each.
const SESSION_PER_SECOND: f64 = 350.0;
/// Visits to the account's pages: 5 requests each, on one session.
const VISITS_PER_SECOND: f64 = 120.0;
const REQUESTS_PER_VISIT: usize = 5;

const CEREMONY: i16 = 1;
const AUTHORIZATION: i16 = 2;
const CODE: i16 = 3;

const CEREMONY_TTL: i64 = 300;
const AUTHORIZATION_TTL: i64 = 600;
const CODE_TTL: i64 = 60;
const ASSERTION_TTL: i64 = 60;
const SESSION_IDLE: i64 = 3600;

/// One sign-in in ten is abandoned before the person answers: its rows stay
/// until they expire.
const ABANDONED: f64 = 0.1;

const REQUESTS: [&str; 9] = [
    "authorize",
    "ceremony begin",
    "ceremony finish",
    "linking page",
    "approve",
    "token",
    "account page",
    "sign-in, passkey (6)",
    "sign-in, session (4)",
];
const AUTHORIZE: usize = 0;
const BEGIN: usize = 1;
const FINISH: usize = 2;
const VIEW: usize = 3;
const APPROVE: usize = 4;
const TOKEN: usize = 5;
const ACCOUNT: usize = 6;
const FLOW_FULL: usize = 7;
const FLOW_SESSION: usize = 8;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The identifier `fill.sql` gave row `n`: MD5 of the prefix and the number,
/// written as a UUID.
fn bench_id(prefix: &str, n: u64) -> String {
    let hex = md5_hex(&format!("{prefix}{n}"));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn md5_hex(input: &str) -> String {
    Md5::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The hash of the session `fill.sql` opened for account `n`.
fn filled_session(n: u64) -> String {
    format!("{}{}", md5_hex(&format!("s{n}")), md5_hex(&format!("t{n}")))
}

/// 256 random bits in hexadecimal: a request id, a code, a token's hash.
fn random_token() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where the short-lived state lives.
enum Pending {
    Table,
    Cache(MultiplexedConnection),
}

struct Ctx {
    pool: PgPool,
    pending: Pending,
    touch: Touch,
    accounts: u64,
    sessions: u64,
    measure_from: Instant,
    histograms: Vec<Mutex<Histogram<u64>>>,
    errors: AtomicU64,
    lost: AtomicU64,
}

impl Ctx {
    fn record(&self, request: usize, arrived: Instant) -> Instant {
        let done = Instant::now();
        if arrived >= self.measure_from {
            let micros = done.duration_since(arrived).as_micros() as u64;
            self.histograms[request]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .saturating_record(micros.max(1));
        }
        done
    }

    async fn put(&self, kind: i16, id: &str, len: usize, ttl: i64) -> Result<()> {
        let value = vec![0x5a_u8; len];
        match &self.pending {
            Pending::Table => {
                sqlx::query(
                    "INSERT INTO pending (kind, id, value, expires_at) VALUES ($1, $2, $3, $4)",
                )
                .bind(kind)
                .bind(id)
                .bind(value)
                .bind(now() + ttl)
                .execute(&self.pool)
                .await?;
            }
            Pending::Cache(cache) => {
                let _: Option<String> = redis::cmd("SET")
                    .arg(format!("p:{kind}:{id}"))
                    .arg(value)
                    .arg("NX")
                    .arg("EX")
                    .arg(ttl)
                    .query_async(&mut cache.clone())
                    .await?;
            }
        }
        Ok(())
    }

    /// Reads without taking: the linking page shows a request more than once.
    async fn get(&self, kind: i16, id: &str) -> Result<bool> {
        Ok(match &self.pending {
            Pending::Table => sqlx::query(
                "SELECT value FROM pending WHERE kind = $1 AND id = $2 AND expires_at > $3",
            )
            .bind(kind)
            .bind(id)
            .bind(now())
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
            Pending::Cache(cache) => redis::cmd("GET")
                .arg(format!("p:{kind}:{id}"))
                .query_async::<Option<Vec<u8>>>(&mut cache.clone())
                .await?
                .is_some(),
        })
    }

    /// Takes out, once: of two nodes that present the same id, one gets it.
    async fn take(&self, kind: i16, id: &str) -> Result<bool> {
        Ok(match &self.pending {
            Pending::Table => sqlx::query(
                "DELETE FROM pending WHERE kind = $1 AND id = $2 AND expires_at > $3 \
                 RETURNING value",
            )
            .bind(kind)
            .bind(id)
            .bind(now())
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
            Pending::Cache(cache) => redis::cmd("GETDEL")
                .arg(format!("p:{kind}:{id}"))
                .query_async::<Option<Vec<u8>>>(&mut cache.clone())
                .await?
                .is_some(),
        })
    }

    /// Whether this assertion of a client is presented for the first time.
    async fn first_use(&self, client: &str, jti: &str) -> Result<bool> {
        Ok(match &self.pending {
            Pending::Table => {
                sqlx::query(
                    "INSERT INTO used_assertions (client_id, jti, expires_at) \
                     VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                )
                .bind(client)
                .bind(jti)
                .bind(now() + ASSERTION_TTL)
                .execute(&self.pool)
                .await?
                .rows_affected()
                    == 1
            }
            Pending::Cache(cache) => redis::cmd("SET")
                .arg(format!("a:{client}:{jti}"))
                .arg(1)
                .arg("NX")
                .arg("EX")
                .arg(ASSERTION_TTL)
                .query_async::<Option<String>>(&mut cache.clone())
                .await?
                .is_some(),
        })
    }

    /// The session behind a token's hash, its idle deadline pushed back.
    async fn touch(&self, token_hash: &str) -> Result<bool> {
        let now = now();
        match self.touch {
            Touch::Always => Ok(sqlx::query(
                "UPDATE sessions SET idle_expires_at = LEAST($1 + $2, expires_at) \
                 WHERE token_hash = $3 AND idle_expires_at > $1 AND expires_at > $1 \
                 RETURNING account_id, created_at, idle_expires_at, expires_at, confirmed_at",
            )
            .bind(now)
            .bind(SESSION_IDLE)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?
            .is_some()),
            Touch::Coarse => {
                let Some(row) = sqlx::query(
                    "SELECT account_id, created_at, idle_expires_at, expires_at, confirmed_at \
                     FROM sessions \
                     WHERE token_hash = $1 AND idle_expires_at > $2 AND expires_at > $2",
                )
                .bind(token_hash)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?
                else {
                    return Ok(false);
                };
                let idle: i64 = row.try_get("idle_expires_at")?;
                let end: i64 = row.try_get("expires_at")?;
                let wanted = (now + SESSION_IDLE).min(end);
                if (wanted - idle).abs() >= 60 {
                    sqlx::query(
                        "UPDATE sessions SET idle_expires_at = $1 \
                         WHERE token_hash = $2 AND idle_expires_at > $3",
                    )
                    .bind(wanted)
                    .bind(token_hash)
                    .bind(now)
                    .execute(&self.pool)
                    .await?;
                }
                Ok(true)
            }
        }
    }

    async fn client(&self, client: &str) -> Result<()> {
        sqlx::query(
            "SELECT client_id, name, redirect_uris, secret_hash, attributes, document, created_at \
             FROM clients WHERE client_id = $1",
        )
        .bind(client)
        .fetch_one(&self.pool)
        .await?;
        Ok(())
    }

    /// `GET /authorize`: the application's request is checked and kept.
    async fn authorize(&self, client: &str) -> Result<String> {
        self.client(client).await?;
        let request = random_token();
        self.put(AUTHORIZATION, &request, 512, AUTHORIZATION_TTL)
            .await?;
        Ok(request)
    }

    /// `POST /auth/login/begin`: a challenge is drawn and kept.
    async fn begin(&self) -> Result<String> {
        let ceremony = random_token();
        self.put(CEREMONY, &ceremony, 1024, CEREMONY_TTL).await?;
        Ok(ceremony)
    }

    /// `POST /auth/login/finish`: the challenge is taken, the passkey read,
    /// a session opened. Returns the hash of the session's token.
    async fn finish(&self, ceremony: &str, account: u64) -> Result<String> {
        if !self.take(CEREMONY, ceremony).await? {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        let account_id = bench_id("u", account);
        sqlx::query("SELECT user_id, data FROM webauthn_credentials WHERE id = $1")
            .bind(bench_id("c", account))
            .fetch_one(&self.pool)
            .await?;
        sqlx::query("SELECT id, admin, suspended_at FROM users WHERE id = $1")
            .bind(&account_id)
            .fetch_one(&self.pool)
            .await?;
        let token_hash = random_token();
        let now = now();
        sqlx::query(
            "INSERT INTO sessions \
             (token_hash, account_id, created_at, idle_expires_at, expires_at, confirmed_at) \
             VALUES ($1, $2, $3, $4, $5, $3)",
        )
        .bind(&token_hash)
        .bind(&account_id)
        .bind(now)
        .bind(now + SESSION_IDLE)
        .bind(now + 12 * 3600)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE users SET last_sign_in_at = $1 WHERE id = $2")
            .bind(now)
            .bind(&account_id)
            .execute(&self.pool)
            .await?;
        Ok(token_hash)
    }

    /// `GET /auth/authorize/{request}`: what the linking page shows.
    async fn view(&self, session: &str, request: &str, account: u64, client: &str) -> Result<()> {
        self.touch(session).await?;
        if !self.get(AUTHORIZATION, request).await? {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        let account_id = bench_id("u", account);
        self.client(client).await?;
        sqlx::query(
            "SELECT id, name, logo, description, attributes FROM identities WHERE account_id = $1",
        )
        .bind(&account_id)
        .fetch_all(&self.pool)
        .await?;
        let chosen = sqlx::query(
            "SELECT identity_id FROM client_identities WHERE account_id = $1 AND client_id = $2",
        )
        .bind(&account_id)
        .bind(client)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = chosen {
            let identity: String = row.try_get("identity_id")?;
            sqlx::query("SELECT decisions FROM grants WHERE identity_id = $1 AND client_id = $2")
                .bind(identity)
                .bind(client)
                .fetch_optional(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// `POST /auth/authorize/{request}/approve`: the request is taken, the
    /// choice and what was accepted are kept, a code is issued.
    async fn approve(
        &self,
        session: &str,
        request: &str,
        account: u64,
        client: &str,
    ) -> Result<String> {
        self.touch(session).await?;
        if !self.take(AUTHORIZATION, request).await? {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        let account_id = bench_id("u", account);
        // One of the account's ten identities.
        let pick = rand::rng().random_range(0..10_u64);
        let identity = bench_id("i", account + pick * self.accounts);
        sqlx::query("SELECT id, account_id, attributes FROM identities WHERE id = $1")
            .bind(&identity)
            .fetch_one(&self.pool)
            .await?;
        let now = now();
        sqlx::query(
            "INSERT INTO client_identities (account_id, client_id, identity_id, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (account_id, client_id) DO UPDATE SET \
             identity_id = excluded.identity_id, updated_at = excluded.updated_at",
        )
        .bind(&account_id)
        .bind(client)
        .bind(&identity)
        .bind(now)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO grants (identity_id, client_id, decisions, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $4) \
             ON CONFLICT (identity_id, client_id) DO UPDATE SET \
             decisions = excluded.decisions, last_used_at = excluded.last_used_at",
        )
        .bind(&identity)
        .bind(client)
        .bind(vec![0x5a_u8; 120])
        .bind(now)
        .execute(&self.pool)
        .await?;
        let code = random_token();
        self.put(CODE, &code, 512, CODE_TTL).await?;
        Ok(code)
    }

    /// `POST /token`: the code is taken, the application authenticated.
    async fn token(&self, code: &str, client: &str) -> Result<()> {
        if !self.take(CODE, code).await? {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        self.client(client).await?;
        if !self.first_use(client, &random_token()).await? {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// One of the requests a page of the account sends.
    async fn account_page(&self, session: &str, account: u64) -> Result<()> {
        self.touch(session).await?;
        sqlx::query(
            "SELECT id, name, logo, description, attributes FROM identities WHERE account_id = $1",
        )
        .bind(bench_id("u", account))
        .fetch_all(&self.pool)
        .await?;
        Ok(())
    }
}

/// A sign-in to an application. `with_passkey`: the person has no session
/// and goes through a ceremony first.
async fn sign_in(ctx: Arc<Ctx>, arrived: Instant, with_passkey: bool) -> Result<()> {
    let (account, client, abandoned) = {
        let mut rng = rand::rng();
        let account = if with_passkey {
            rng.random_range(0..ctx.accounts)
        } else {
            rng.random_range(0..ctx.sessions)
        };
        (
            account,
            format!("client-{}", (account + rng.random_range(0..3)) % 200),
            rng.random_bool(ABANDONED),
        )
    };
    let request = ctx.authorize(&client).await?;
    let mut at = ctx.record(AUTHORIZE, arrived);
    let session = if with_passkey {
        let ceremony = ctx.begin().await?;
        at = ctx.record(BEGIN, at);
        if abandoned {
            return Ok(());
        }
        let session = ctx.finish(&ceremony, account).await?;
        at = ctx.record(FINISH, at);
        session
    } else {
        filled_session(account)
    };
    ctx.view(&session, &request, account, &client).await?;
    at = ctx.record(VIEW, at);
    if abandoned {
        return Ok(());
    }
    let code = ctx.approve(&session, &request, account, &client).await?;
    at = ctx.record(APPROVE, at);
    ctx.token(&code, &client).await?;
    ctx.record(TOKEN, at);
    ctx.record(
        if with_passkey {
            FLOW_FULL
        } else {
            FLOW_SESSION
        },
        arrived,
    );
    Ok(())
}

/// A visit to the account's pages: a few requests on one session.
async fn visit(ctx: Arc<Ctx>, arrived: Instant) -> Result<()> {
    let account = rand::rng().random_range(0..ctx.sessions);
    let session = filled_session(account);
    let mut at = arrived;
    for _ in 0..REQUESTS_PER_VISIT {
        ctx.account_page(&session, account).await?;
        at = ctx.record(ACCOUNT, at);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Flow {
    Passkey,
    Session,
    Visit,
}

/// Starts `per_second` flows a second until `until`, on time whether or not
/// the earlier ones are answered: a slow database is waited for, and the
/// wait is counted.
async fn drive(ctx: Arc<Ctx>, flow: Flow, per_second: f64, until: Instant) {
    let period = Duration::from_secs_f64(1.0 / per_second);
    let mut next = Instant::now();
    let mut flows = JoinSet::new();
    while next < until {
        tokio::time::sleep_until(next.into()).await;
        let ctx = ctx.clone();
        let arrived = next;
        flows.spawn(async move {
            let done = match flow {
                Flow::Passkey => sign_in(ctx.clone(), arrived, true).await,
                Flow::Session => sign_in(ctx.clone(), arrived, false).await,
                Flow::Visit => visit(ctx.clone(), arrived).await,
            };
            if let Err(e) = done
                && ctx.errors.fetch_add(1, Ordering::Relaxed) < 5
            {
                eprintln!("error: {e}");
            }
        });
        next += period;
        // Collect what is finished, so that the set does not grow.
        while flows.try_join_next().is_some() {}
    }
    while flows.join_next().await.is_some() {}
}

/// What each node does every few seconds: remove what expired.
async fn sweep(ctx: Arc<Ctx>) {
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        if matches!(ctx.pending, Pending::Table) {
            for table in ["pending", "used_assertions"] {
                let swept = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DELETE FROM {table} WHERE expires_at <= $1"
                )))
                .bind(now())
                .execute(&ctx.pool)
                .await;
                if let Err(e) = swept {
                    eprintln!("sweep of {table}: {e}");
                }
            }
        }
    }
}

async fn wal_bytes(pool: &PgPool) -> Result<f64> {
    Ok(
        sqlx::query("SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')::float8 AS bytes")
            .fetch_one(pool)
            .await?
            .try_get("bytes")?,
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let pool = PgPoolOptions::new()
        .max_connections(args.connections)
        .min_connections(args.connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&args.database_url)
        .await?;

    // The shared state starts empty at each run.
    sqlx::query("DROP TABLE IF EXISTS pending, used_assertions")
        .execute(&pool)
        .await?;
    let kind = if args.ephemeral == Ephemeral::Unlogged {
        "UNLOGGED TABLE"
    } else {
        "TABLE"
    };
    for statement in include_str!("../pending.sql")
        .replace("{PENDING}", kind)
        .split(';')
    {
        let statement: String = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        if !statement.trim().is_empty() {
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&pool)
                .await?;
        }
    }

    let pending = match args.ephemeral {
        Ephemeral::Cache => {
            let url = args
                .cache_url
                .as_deref()
                .ok_or("--ephemeral cache needs --cache-url")?;
            let mut cache = redis::Client::open(url)?
                .get_multiplexed_async_connection()
                .await?;
            let _: String = redis::cmd("FLUSHALL").query_async(&mut cache).await?;
            Pending::Cache(cache)
        }
        _ => Pending::Table,
    };

    let measure_from = Instant::now() + Duration::from_secs(args.warmup);
    let until = measure_from + Duration::from_secs(args.seconds);
    let ctx = Arc::new(Ctx {
        pool: pool.clone(),
        pending,
        touch: args.touch,
        accounts: args.accounts,
        sessions: args.accounts * 3 / 10,
        measure_from,
        histograms: REQUESTS
            .iter()
            .map(|_| Mutex::new(Histogram::new_with_bounds(1, 600_000_000, 3).expect("bounds")))
            .collect(),
        errors: AtomicU64::new(0),
        lost: AtomicU64::new(0),
    });

    let sweeper = tokio::spawn(sweep(ctx.clone()));
    // The warm-up is played like the rest, and not recorded.
    let drivers = [
        (Flow::Passkey, FULL_PER_SECOND),
        (Flow::Session, SESSION_PER_SECOND),
        (Flow::Visit, VISITS_PER_SECOND),
    ]
    .map(|(flow, rate)| tokio::spawn(drive(ctx.clone(), flow, rate * args.scale, until)));
    tokio::time::sleep_until(measure_from.into()).await;
    let wal_before = wal_bytes(&pool).await?;
    for driver in drivers {
        driver.await?;
    }
    let elapsed = Instant::now().duration_since(measure_from).as_secs_f64();
    let wal = wal_bytes(&pool).await? - wal_before;
    sweeper.abort();

    let requests: u64 = (AUTHORIZE..=ACCOUNT)
        .map(|i| ctx.histograms[i].lock().unwrap().len())
        .sum();
    println!(
        "ephemeral: {}   session writes: {}   asked: {:.0} requests/s   answered: {:.0} requests/s over {:.0} s",
        match args.ephemeral {
            Ephemeral::Logged => "table",
            Ephemeral::Unlogged => "unlogged table",
            Ephemeral::Cache => "cache",
        },
        match args.touch {
            Touch::Always => "every request",
            Touch::Coarse => "by the minute",
        },
        2900.0 * args.scale,
        requests as f64 / elapsed,
        elapsed,
    );
    println!(
        "{:<22} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "request", "count", "p50 ms", "p95 ms", "p99 ms", "max ms"
    );
    for (i, name) in REQUESTS.iter().enumerate() {
        let h = ctx.histograms[i].lock().unwrap();
        let ms = |q: f64| h.value_at_quantile(q) as f64 / 1000.0;
        println!(
            "{:<22} {:>9} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            name,
            h.len(),
            ms(0.5),
            ms(0.95),
            ms(0.99),
            h.max() as f64 / 1000.0
        );
    }
    println!(
        "errors: {}   state not found when asked: {}   log written: {:.1} MB ({:.0} kB/s)",
        ctx.errors.load(Ordering::Relaxed),
        ctx.lost.load(Ordering::Relaxed),
        wal / 1e6,
        wal / 1e3 / elapsed,
    );
    if matches!(ctx.pending, Pending::Table) {
        let row = sqlx::query(
            "SELECT (SELECT count(*) FROM pending) AS live, \
             pg_total_relation_size('pending')::float8 AS bytes",
        )
        .fetch_one(&pool)
        .await?;
        let live: i64 = row.try_get("live")?;
        let bytes: f64 = row.try_get("bytes")?;
        println!(
            "pending at the end: {live} rows, {:.1} MB on disk",
            bytes / 1e6
        );
    }
    // A run that lost a request or a piece of state is not a measurement.
    if ctx.errors.load(Ordering::Relaxed) > 0 || ctx.lost.load(Ordering::Relaxed) > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generator must name the rows `fill.sql` wrote:
    /// `md5('u0')::uuid::text` in PostgreSQL.
    #[test]
    fn an_identifier_is_the_one_the_fill_script_wrote() {
        assert_eq!(bench_id("u", 0), "3e334e85-9879-af25-6d38-27d651b7804a");
    }

    #[test]
    fn a_session_hash_has_the_size_of_a_real_one() {
        assert_eq!(filled_session(7).len(), 64);
        assert_eq!(random_token().len(), 64);
    }
}
