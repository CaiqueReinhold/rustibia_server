CREATE TABLE auth_tokens (
    token       VARCHAR     PRIMARY KEY,
    account_id  INTEGER     NOT NULL,
    valid_until TIMESTAMPTZ NOT NULL
);
