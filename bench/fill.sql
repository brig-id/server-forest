-- Fills the database to forest's tested maximum (spec `editions.md` §2):
-- 1,000,000 accounts, 10,000,000 identities. Identifiers are derived from a
-- number with MD5 so that the generator can name any row, and so that they
-- spread over the indexes like the random identifiers of a real server.
-- :accounts is given by the caller (psql -v accounts=1000000).

CREATE FUNCTION bench_id(prefix TEXT, n BIGINT) RETURNS TEXT
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    RETURN md5(prefix || n::text)::uuid::text;

CREATE FUNCTION bench_bytes(n BIGINT, len INT) RETURNS BYTEA
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    RETURN substring(decode(repeat(md5(n::text), len / 16 + 1), 'hex') FROM 1 FOR len);

INSERT INTO users (id, username, server, created_at, username_index)
SELECT bench_id('u', n), bench_bytes(n, 44), bench_bytes(n, 44),
       '2026-01-01T00:00:00Z', md5('x' || n::text) || md5('y' || n::text)
FROM generate_series(0, :accounts - 1) AS n;

-- One passkey per account; its stored form is about 400 bytes.
INSERT INTO webauthn_credentials (id, user_id, data, label)
SELECT bench_id('c', n), bench_id('u', n), bench_bytes(n, 400), bench_bytes(n, 40)
FROM generate_series(0, :accounts - 1) AS n;
CREATE INDEX idx_webauthn_credentials_user_id ON webauthn_credentials(user_id);

-- Ten identities per account: identity n belongs to account n modulo the
-- number of accounts.
INSERT INTO identities (id, account_id, name, logo, description, attributes, created_at, updated_at)
SELECT bench_id('i', n), bench_id('u', n % :accounts), bench_bytes(n, 60), 'user',
       bench_bytes(n, 100), bench_bytes(n, 200),
       '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
FROM generate_series(0, :accounts * 10 - 1) AS n;
CREATE INDEX idx_identities_account_id ON identities(account_id);

INSERT INTO clients (client_id, name, redirect_uris, created_at)
SELECT 'client-' || n, 'Application ' || n, '["https://app' || n || '.example/callback"]', 1767225600
FROM generate_series(0, 199) AS n;

-- Three applications per account, each with its identity and what was
-- accepted for it: application (n + k) modulo 200, identity n + k × accounts.
INSERT INTO client_identities (account_id, client_id, identity_id, updated_at)
SELECT bench_id('u', n), 'client-' || ((n + k) % 200), bench_id('i', n + k * :accounts), 1767225600
FROM generate_series(0, :accounts - 1) AS n, generate_series(0, 2) AS k;

INSERT INTO grants (identity_id, client_id, decisions, created_at, last_used_at)
SELECT bench_id('i', n + k * :accounts), 'client-' || ((n + k) % 200), bench_bytes(n, 120),
       1767225600, 1767225600
FROM generate_series(0, :accounts - 1) AS n, generate_series(0, 2) AS k;

-- Three accounts in ten have a session open. A token's hash is 64
-- hexadecimal characters.
INSERT INTO sessions (token_hash, account_id, created_at, idle_expires_at, expires_at)
SELECT md5('s' || n::text) || md5('t' || n::text), bench_id('u', n),
       1767225600, 4102444800, 4102444800
FROM generate_series(0, :accounts * 3 / 10 - 1) AS n;
CREATE INDEX idx_sessions_account_id ON sessions(account_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);

VACUUM ANALYZE;
