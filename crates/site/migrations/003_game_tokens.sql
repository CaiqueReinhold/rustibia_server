-- The game token stops being an account credential and becomes a character ticket.
--
-- `auth_tokens` was minted per character -- POST /api/characters/{id}/token checked
-- ownership -- but stored only the account. So the character the mint call had already
-- established was thrown away, re-supplied by the client, carried across two hops, and
-- re-checked against ownership at redemption. Storing what the mint call already knew
-- takes the client out of the decision and collapses the two ownership answers into one.
--
-- Renamed because the table now sits beside `sessions` meaning something quite
-- different: `sessions` is an account's web credential, good for seven days, while
-- `game_tokens` is a single-use 60-second ticket admitting one character to the world.
-- One name should not suggest they are variations of the same thing.
--
-- UNIQUE on character_id is the enforcement mechanism, not decoration. It makes
-- "one live ticket per character" something the schema will not let a caller violate,
-- and it supplies the index the mint upsert's conflict target requires -- which is why
-- no separate index is declared here.
--
-- ON DELETE CASCADE now runs through players rather than accounts. Deleting an account
-- still reaps its tokens, transitively. It fires only on a hard delete; characters are
-- soft-deleted via deleted_at, so a soft-deleted character keeps any outstanding token
-- and redemption must still refuse it.
--
-- Existing rows are dropped rather than migrated, which is the opposite of what 002 did.
-- That is deliberate: 002 migrated seven-day sessions because logging everyone out was
-- a real cost and the plaintext was right there to hash. These rows live 60 seconds and
-- carry no character at all, so `character_id NOT NULL` has nothing to backfill from.

DROP TABLE auth_tokens;

CREATE TABLE game_tokens (
    token_hash   TEXT        PRIMARY KEY,
    character_id INTEGER     NOT NULL UNIQUE REFERENCES players(id) ON DELETE CASCADE,
    valid_until  TIMESTAMPTZ NOT NULL
);
