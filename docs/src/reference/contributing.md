# Contributing

Wisecrow is a small project with strong conventions. This page distils the
ones you need to follow before submitting a patch.

## Tooling

| Tool | Why |
|------|-----|
| `cargo build` / `cargo check` | Verify the workspace compiles. |
| `cargo nextest run` | Run the test suite (fall back to `cargo test` if you don't have nextest). |
| `cargo clippy --workspace --all-targets` | The codebase is required to be clippy-clean — no `#[allow]` annotations are acceptable. |
| `cargo fmt --all` | The repository is `rustfmt`-formatted. |

A pre-commit hook is wired through `pre-commit-config.yaml`; install
[`pre-commit`](https://pre-commit.com/) and run `pre-commit install` to get
the hooks locally.

## Workspace conventions

These rules live in the project's `CLAUDE.md` and are enforced in review:

- **No `as` numeric casts.** Use `try_from`, `into`, or `From`. Compile-time
  literals are the only exception.
- **No silent error discards.** `let _ = expr_returning_Result` is a bug.
  Either propagate with `?` or log with `tracing::warn!`.
- **No `unreachable!()` outside tests.** "Should never happen" branches still
  return `Err`.
- **Justify every `.clone()`.** Production clones must carry an inline
  `// clone: <reason>` comment. Tests are exempt.
- **Use `Url` for URL construction.** Never `format!()` URLs — that is how
  double-slash bugs are born.
- **`unwrap`/`expect`/`panic!` are forbidden** in async tasks and HTTP
  handlers. Convert to `?`.

## Test hierarchy

Pick the test type *before* writing it:

1. **Property-based (`proptest`)** — pure functions, parsers, validators,
   serializers. The first choice for anything with an invariant.
2. **Parameterised (`rstest`)** — bounded enum exhaustion or fewer than ~10
   specific cases.
3. **Standalone** — only when you genuinely need bespoke setup or mocking.

Do not write a standalone test "to get it working" with the intent of
converting it later. The conversion never happens.

Tests live alongside the code in `#[cfg(test)] mod tests` blocks. Use
`tests/` directories only for crate-spanning integration tests.

## Database changes

- New migrations go in `wisecrow-core/migrations/` numbered sequentially
  (`0NN_short_name.sql`).
- Migrations run automatically on first connection, so the workspace's tests
  must be runnable against a fresh database.
- Always include the matching `DROP` semantics by relying on `ON DELETE
  CASCADE` rather than ad-hoc cleanup.

## Documentation changes

- Public types should have rustdoc comments (`///`). Document `# Errors`
  and `# Panics` sections where they apply.
- This mdBook (`docs/`) is not auto-generated; if you change a CLI surface
  or a library type, update the relevant page in `docs/src/`.
- Code examples in this book are not currently doctested. Keep them small
  and obviously correct; copy them from real source where possible.

## Commit messages

Recent history is the source of truth. Run `git log --oneline -20` and
follow the prevailing style — short subject, optional body, no
`Co-Authored-By:` for human contributors.

## Adding a new command

A typical CLI addition has the following touch-points:

1. New `Args` struct + `Command` variant in `wisecrow-core/src/cli.rs`.
2. New handler function in `wisecrow-core/src/bin/wisecrow.rs`.
3. Library code in the appropriate module under `wisecrow-core/src/`.
4. Updated rstest cases in `cli.rs` covering both the long and short alias.
5. New page in `docs/src/reference/cli-reference.md` and (if user-facing) a
   guide in `docs/src/guides/`.
6. Updated `docs/src/SUMMARY.md`.

## Reporting issues

The repository is at <https://github.com/glottologist/wisecrow>. Please
include:

- The exact command you ran.
- The output of `wisecrow --version`.
- A minimal reproduction (a small ingest script, a redacted SQL snapshot, …).
- Logs at `RUST_LOG=wisecrow=debug,info`.

## Dependency policy

Wisecrow leans on a curated set of crates. Adding new dependencies is a
review-able event:

- Prefer `tokio`-native crates for async work.
- Database access goes through `sqlx`. Adding a different driver is unlikely
  to be accepted.
- Audio/image dependencies must stay behind their respective feature flags.
- HTTP clients should be `reqwest` with `rustls-tls` to keep cross-platform
  builds simple.
