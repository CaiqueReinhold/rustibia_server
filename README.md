# Rustibia Game Site

The website and REST API for Rustibia, a Tibia-like game server.

**This project owns the database schema.** `migrations/` here is the only place any
table is defined; the game server connects to an already-migrated database and never
runs migrations of its own.

## Layout

This is a cargo workspace:

| Crate | What it is |
|---|---|
| `crates/server` | The game server. Binds `127.0.0.1:5555`, binary protocol, actor-based. |
| `crates/site` | This website and its REST API. Owns the schema. |

Run each from its own directory — asset and config paths are relative to the working
directory:

```bash
cd crates/site   && cargo run    # http://127.0.0.1:8080
cd crates/server && cargo run    # 127.0.0.1:5555
```

`cargo test` at the workspace root runs both suites; `cargo test -p rustibia-site`
runs one.

## Running

```bash
docker compose up -d db   # from the workspace root
cd crates/site && cargo run
```

Serves on `127.0.0.1:8080`. `DATABASE_URL` and `BIND_ADDRESS` come from the
environment; everything else lives in `config.yaml`.

## Tests

```bash
DATABASE_URL=postgres://rustibia:rustibia@localhost:5432/rustibia cargo test
```

`#[sqlx::test]` creates a throwaway database per test from `migrations/`, so the
`rustibia` role needs `CREATEDB`.

## Granting administrator access

There is no user interface for this, and no handler anywhere sets `is_admin` — that is
deliberate. Promote an account directly:

```bash
docker exec rustibia-db-1 psql -U rustibia -d rustibia \
  -c "UPDATE accounts SET is_admin = TRUE WHERE lower(email) = lower('you@example.com');"
```

Admin status is read from the database on every request rather than stored in the
session, so revoking it takes effect immediately rather than at the next login.

`/admin/news` answers **404** to a signed-in non-admin, not 403 — someone without
access should not learn the page exists.

## The API the game client calls

| Endpoint | Auth | Returns |
|---|---|---|
| `POST /api/auth` | none | `{session_token, expires_at}` |
| `GET /api/characters` | `Authorization: Bearer <session_token>` | `[{id, name, level, vocation}]` |
| `POST /api/characters/{id}/token` | same | `{auth_token, expires_at}` |

The third writes a row into `game_tokens`. That token is short-lived **and single-use** —
it is deleted the moment the game server redeems it.

**No bearer token is stored in the clear.** `sessions` and `game_tokens` hold
`token_hash`, the hex SHA-256 of the token; the token itself exists only in the response
above and in the client's cookie. SHA-256 rather than Argon2 because these are 32 bytes of
OS randomness — there is no dictionary to defend against, and a per-row salt would make
lookup-by-token a table scan instead of one index probe.

## The API the game server calls

| Endpoint | Auth | Returns |
|---|---|---|
| `POST /internal/game-tokens/redeem` | client certificate (mTLS) | `CharacterRecord`, or 404 |

Served on its own listener (`INTERNAL_BIND_ADDRESS`, default `127.0.0.1:8443`), never on
the public one — requiring client certificates on port 8080 would make every browser
prompt for one, and a separate port makes the internal router unreachable from the public
side by construction. The 404 is identical for an unknown token and for a character on
another account, so a caller cannot use it to enumerate character ids.

Generate the certificates before starting either process:

```sh
cargo run -p rustibia-certgen     # writes certs/ (git-ignored)
```

Both processes **refuse to start** without them. That is deliberate: serving
`/internal/*` unprotected would expose every player's stored character, and a silent
downgrade would look like success in the logs.

## The contract with the game server

Login is HTTP: the server no longer reads `game_tokens` or `players` to authenticate.
The request and response types live in `crates/contract`, which both crates link and
neither can avoid — a field added on one side and missing on the other is a deserialize
error, not a silent zero.

Saving is still SQL. The server writes `players`, `player_skills` and `online_players`
with its own statements, so `crates/site/tests/schema_contract.rs` still asserts the
columns those depend on. The server's tests build their schema from
`crates/site/migrations` directly, so there is no vendored copy to drift.

## Adding a starting skill

`config.yaml`'s `new_character.starting_skills` seeds `player_skills` at character
creation. **Add the skill to the game server first.** Its loader drops `skill_type`
values it does not recognise, and its save then deletes and re-inserts only what it
loaded — so a skill seeded here that the server does not know survives until the
player's first logout and then vanishes, with no error and no log line. Today the
server knows `0 = Level` and `1 = Speed`.

## Known limitations

- Public pages always render the logged-out sidebar; the Admin link appears only on
  `/admin/news` itself. Both cosmetic, both avoid a query per page render.
- Session tokens and game auth tokens are stored unhashed.
- No CSRF tokens.
- No pagination: highscores caps at 100, news at 10.
- Download / Rules / Support carry placeholder prose.
