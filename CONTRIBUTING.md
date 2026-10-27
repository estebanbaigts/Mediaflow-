# Contributing

Thank you for helping. MediaFlow handles people's only copy of their work, so
the bar for correctness is high.

## Ground rules

1. **No feature may delete, move or overwrite a user file.** Not behind a
   flag, not "with confirmation" in v0.x.
2. Heuristics never become proof. Only equal SHA-256 digests mean identical.
3. Every state change goes through `state.rs`. Never mark an operation `done`
   without verification.
4. The engine re-validates paths and grants at execution time. The UI never
   sends paths.
5. Errors are shown to users through `MfError::user_message()` — write them
   for someone who is not technical.

## Workflow

```sh
cargo fmt --all && cargo clippy --all-targets -- -D warnings
cargo test --workspace --exclude mediaflow-app
cd app && npm ci && npx tsc --noEmit && npm test
```

- Tests that create or damage files must use `tempfile` directories and
  generated data (`tests/common`). Never point tests at real folders.
- A bug fix comes with a test that fails before the fix.
- Database changes = a new numbered migration in `db.rs`; never edit an
  existing one.
- New dependencies: explain why in the PR, check licence (MIT/Apache/BSD
  compatible) and maintenance.
- New Tauri command: add it to `app/src-tauri/build.rs`, the capability file,
  and take ids, not paths.

## Commit messages

Imperative mood, one topic per commit. Mention user-visible changes in
`CHANGELOG.md`.
