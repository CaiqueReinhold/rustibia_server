CREATE TABLE accounts (
    id            INTEGER     GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email         TEXT        NOT NULL,
    password_hash TEXT        NOT NULL,
    is_admin      BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX accounts_email_lower_idx ON accounts (lower(email));

CREATE TABLE sessions (
    token       TEXT        PRIMARY KEY,
    account_id  INTEGER     NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    valid_until TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX sessions_account_idx ON sessions (account_id);

CREATE TABLE auth_tokens (
    token       TEXT        PRIMARY KEY,
    account_id  INTEGER     NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    valid_until TIMESTAMPTZ NOT NULL
);

CREATE TABLE players (
    id          INTEGER     GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id  INTEGER     NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT        NOT NULL,
    vocation    SMALLINT    NOT NULL,
    sex         SMALLINT    NOT NULL,
    pos_x       INTEGER     NOT NULL,
    pos_y       INTEGER     NOT NULL,
    pos_z       SMALLINT    NOT NULL,
    origin_x    INTEGER     NOT NULL,
    origin_y    INTEGER     NOT NULL,
    origin_z    SMALLINT    NOT NULL,
    facing      SMALLINT    NOT NULL,
    life_cur    INTEGER     NOT NULL,
    life_max    INTEGER     NOT NULL,
    mana_cur    INTEGER     NOT NULL,
    mana_max    INTEGER     NOT NULL,
    cap_cur     INTEGER     NOT NULL,
    cap_max     INTEGER     NOT NULL,
    outfit_id   SMALLINT    NOT NULL,
    outfit_head SMALLINT    NOT NULL,
    outfit_body SMALLINT    NOT NULL,
    outfit_legs SMALLINT    NOT NULL,
    outfit_feet SMALLINT    NOT NULL,
    inventory   JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at  TIMESTAMPTZ
);
CREATE UNIQUE INDEX players_name_lower_idx ON players (lower(name));
CREATE INDEX players_account_idx ON players (account_id);

CREATE TABLE player_skills (
    player_id     INTEGER  NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    skill_type    SMALLINT NOT NULL,
    value         SMALLINT NOT NULL,
    current_ticks BIGINT   NOT NULL,
    max_ticks     BIGINT   NOT NULL,
    PRIMARY KEY (player_id, skill_type)
);
CREATE INDEX player_skills_level_idx ON player_skills (skill_type, value DESC);

CREATE TABLE online_players (
    character_id INTEGER     PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
    since        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE news_posts (
    id        INTEGER     GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title     TEXT        NOT NULL,
    body      TEXT        NOT NULL,
    author_id INTEGER     NOT NULL REFERENCES accounts(id),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
