-- The schema of `core` (crates/brigid-store/migrations, 0001 to 0004) as
-- PostgreSQL would hold it, with the same shapes: identifiers as text, times
-- as Unix seconds, encrypted values as bytes. Only what a sign-in reads or
-- writes is here; the rest of the schema does not change the measurement.

CREATE TABLE users (
    id              TEXT   NOT NULL PRIMARY KEY,
    username        BYTEA  NOT NULL,           -- encrypted
    server          BYTEA  NOT NULL,           -- encrypted
    created_at      TEXT   NOT NULL,
    username_index  TEXT   NOT NULL UNIQUE,
    admin           BIGINT NOT NULL DEFAULT 0,
    suspended_at    BIGINT,
    last_sign_in_at BIGINT
);

CREATE TABLE webauthn_credentials (
    id      TEXT  NOT NULL PRIMARY KEY,
    user_id TEXT  NOT NULL,
    data    BYTEA NOT NULL,                    -- encrypted
    label   BYTEA                              -- encrypted
);

CREATE TABLE sessions (
    token_hash      TEXT   NOT NULL PRIMARY KEY,
    account_id      TEXT   NOT NULL,
    created_at      BIGINT NOT NULL,
    idle_expires_at BIGINT NOT NULL,
    expires_at      BIGINT NOT NULL,
    confirmed_at    BIGINT
);

CREATE TABLE identities (
    id          TEXT  NOT NULL PRIMARY KEY,
    account_id  TEXT  NOT NULL,
    name        BYTEA NOT NULL,                -- encrypted
    logo        TEXT  NOT NULL,
    description BYTEA,                         -- encrypted
    attributes  BYTEA,                         -- encrypted
    created_at  TEXT  NOT NULL,
    updated_at  TEXT  NOT NULL
);

CREATE TABLE clients (
    client_id     TEXT   NOT NULL PRIMARY KEY,
    name          TEXT   NOT NULL,
    redirect_uris TEXT   NOT NULL,
    secret_hash   TEXT,
    attributes    TEXT   NOT NULL DEFAULT '{}',
    document      TEXT,
    created_at    BIGINT NOT NULL
);

CREATE TABLE client_identities (
    account_id  TEXT   NOT NULL,
    client_id   TEXT   NOT NULL,
    identity_id TEXT   NOT NULL,
    updated_at  BIGINT NOT NULL,
    PRIMARY KEY (account_id, client_id)
);

CREATE TABLE grants (
    identity_id  TEXT   NOT NULL,
    client_id    TEXT   NOT NULL,
    decisions    BYTEA  NOT NULL,              -- encrypted
    created_at   BIGINT NOT NULL,
    last_used_at BIGINT NOT NULL,
    PRIMARY KEY (identity_id, client_id)
);
