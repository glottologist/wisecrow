# Configuration

Wisecrow reads its configuration from environment variables prefixed with
`WISECROW__` (note the **double underscore**: `config-rs` uses it as the
key separator). Variables can also be placed in a `.env` file in the working
directory; `dotenvy::dotenv()` is called at start-up.

## Database

You can supply the connection in either of two equivalent forms.

### Option A — full URL

```sh
export WISECROW__DB_URL=postgres://user:password@localhost/wisecrow
```

When `db_url` is set it takes precedence over the component fields.

### Option B — component fields

```sh
export WISECROW__DB_ADDRESS=localhost:5432
export WISECROW__DB_NAME=wisecrow
export WISECROW__DB_USER=wisecrow
export WISECROW__DB_PASSWORD=secret
```

> **Warning:** `WISECROW__DB_PASSWORD`, `WISECROW__LLM_API_KEY`,
> `WISECROW__UNSPLASH_API_KEY`, `WISECROW__REMOTE_API_KEY`, and
> `WISECROW__SYNC_API_KEY` are wrapped in a `SecureString` that zeroes its
> backing buffer on drop. Even so, keep them out of shell history and
> committed `.env` files.

## Optional integrations

| Variable | Purpose | Used by |
|----------|---------|---------|
| `WISECROW__LLM_PROVIDER` | `anthropic` or `openai`. | `seed-grammar`, `generate-exercises` |
| `WISECROW__LLM_API_KEY`  | API key for the chosen provider. | `seed-grammar`, `generate-exercises` |
| `WISECROW__UNSPLASH_API_KEY` | Unsplash access key for card imagery. | `learn`, `prefetch-media` (when built with `images`) |
| `WISECROW__REMOTE_URL`   | Remote Wisecrow base URL. | reserved for future remote-fetch flows |
| `WISECROW__REMOTE_API_KEY` | API key for the remote URL. | reserved |
| `WISECROW__SYNC_API_KEY` | API key sent as `x-api-key` to a sync remote. | `sync` |

The defaults file in the repo is `.env.example`; copy it to `.env` and edit:

```sh
cp .env.example .env
$EDITOR .env
```

## Logging

Wisecrow uses the `tracing` crate. Verbosity is set by `RUST_LOG`:

```sh
export RUST_LOG=wisecrow=debug,info
export RUST_BACKTRACE=1
```

Use `wisecrow=trace` for the very chatty download/parse messages.

## Database setup

Create the database once. Migrations run automatically on the first command
that opens a pool:

```sh
createdb wisecrow
wisecrow ingest -n en -f es --corpus open_subtitles
```

The migrations live in `wisecrow-core/migrations/` and are embedded into the
binary by `sqlx::migrate!`. See the [Database schema reference](../reference/database-schema.md)
for the resulting tables.

## Next step

Run your [first ingest](./first-ingest.md) to populate vocabulary.
