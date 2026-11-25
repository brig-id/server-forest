-- The state several nodes must share, which a single node keeps in memory.
-- The measuring tool creates these tables again before each run.

-- What a single node keeps in memory today, and several nodes must share:
-- passkey ceremonies in progress, sign-in requests of applications, their
-- codes. One row is written once, read at most a few times, taken once.
-- `{PENDING}` is replaced by `TABLE` or `UNLOGGED TABLE`.
CREATE {PENDING} pending (
    kind       SMALLINT NOT NULL,
    id         TEXT     NOT NULL,
    value      BYTEA    NOT NULL,              -- encrypted
    expires_at BIGINT   NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE INDEX idx_pending_expires_at ON pending(expires_at);

-- The assertions of clients already presented (`private_key_jwt`): one use
-- each, whichever node receives it. Always a logged table: losing it would
-- let an assertion be presented a second time.
CREATE TABLE used_assertions (
    client_id  TEXT   NOT NULL,
    jti        TEXT   NOT NULL,
    expires_at BIGINT NOT NULL,
    PRIMARY KEY (client_id, jti)
);
CREATE INDEX idx_used_assertions_expires_at ON used_assertions(expires_at);
