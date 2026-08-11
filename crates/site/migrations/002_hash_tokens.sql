-- Store only the SHA-256 of every bearer token, never the token itself.
--
-- Both `sessions.token` and `auth_tokens.token` were credentials held in plaintext: a
-- read of either table — a leaked backup, an errant log of a query result, a SELECT by
-- anyone with database access — handed over working sessions and working logins. Storing
-- the digest means a reader of the table learns nothing usable, because the value the
-- client presents cannot be derived from it.
--
-- SHA-256 and not Argon2, deliberately. Argon2 exists to make guessing *low-entropy*
-- secrets slow; these tokens are 32 bytes of OS randomness, so there is nothing to guess
-- and nothing to slow down. A salted password hash would also be unusable here: lookup is
-- by token, and a per-row salt would turn every authenticated request into a full table
-- scan instead of one index probe.
--
-- Existing rows are migrated rather than deleted. Sessions last seven days, and truncating
-- would log out every account for no security gain — the plaintext is right here to hash.

ALTER TABLE sessions ADD COLUMN token_hash TEXT;
UPDATE sessions SET token_hash = encode(sha256(token::bytea), 'hex');
ALTER TABLE sessions ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE sessions DROP CONSTRAINT sessions_pkey;
ALTER TABLE sessions ADD PRIMARY KEY (token_hash);
ALTER TABLE sessions DROP COLUMN token;

ALTER TABLE auth_tokens ADD COLUMN token_hash TEXT;
UPDATE auth_tokens SET token_hash = encode(sha256(token::bytea), 'hex');
ALTER TABLE auth_tokens ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE auth_tokens DROP CONSTRAINT auth_tokens_pkey;
ALTER TABLE auth_tokens ADD PRIMARY KEY (token_hash);
ALTER TABLE auth_tokens DROP COLUMN token;
