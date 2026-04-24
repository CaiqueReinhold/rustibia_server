CREATE TABLE players (
    id          INTEGER  PRIMARY KEY,
    account_id  INTEGER  NOT NULL,
    name        TEXT     NOT NULL,
    pos_x       INTEGER  NOT NULL,
    pos_y       INTEGER  NOT NULL,
    pos_z       SMALLINT NOT NULL,
    origin_x    INTEGER  NOT NULL,
    origin_y    INTEGER  NOT NULL,
    origin_z    SMALLINT NOT NULL,
    facing      SMALLINT NOT NULL,
    life_cur    INTEGER  NOT NULL,
    life_max    INTEGER  NOT NULL,
    mana_cur    INTEGER  NOT NULL,
    mana_max    INTEGER  NOT NULL,
    cap_cur     INTEGER  NOT NULL,
    cap_max     INTEGER  NOT NULL,
    outfit_id   SMALLINT NOT NULL,
    outfit_head SMALLINT NOT NULL,
    outfit_body SMALLINT NOT NULL,
    outfit_legs SMALLINT NOT NULL,
    outfit_feet SMALLINT NOT NULL,
    inventory   JSONB    NOT NULL DEFAULT '{}'
);

CREATE TABLE player_skills (
    player_id     INTEGER  NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    skill_type    SMALLINT NOT NULL,
    value         SMALLINT NOT NULL,
    current_ticks BIGINT   NOT NULL,
    max_ticks     BIGINT   NOT NULL,
    PRIMARY KEY (player_id, skill_type)
);
