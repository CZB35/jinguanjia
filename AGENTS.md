## Quick Facts
- This repo is a single-binary Rust backend: the root `Cargo.toml` is the only manifest, and `src/main.rs` only calls `app::bootstrap()`.
- The HTTP surface is defined in `src/routes/mod.rs`. `src/app.rs` mounts that router under `/api` and separately serves `UPLOAD_DIR` as `/uploads/*`.
- `docs/*.md` are useful product docs, but if they disagree with code, trust `src/routes/mod.rs` and the handler implementations.

## Run And Verify
- Local config comes from `.env`, which is loaded automatically by `dotenvy`; start from `.env.example`.
- `.env` is gitignored and overrides the hardcoded defaults in `src/config.rs`; inspect it before assuming which database or JWT secret the app is using.
- The only env values you usually need to change are `DATABASE_URL` and `JWT_SECRET`. Defaults bind to `0.0.0.0:3001`.
- The documented startup command is `cargo run --release`.
- Fast manual check: `GET /api/health`.
- Verified repo-level check: `cargo test`. It currently succeeds but runs `0` tests, so use it as a compile smoke test. A cold Windows build is slow.
- There is no checked-in CI workflow, task runner, `rustfmt.toml`, or `clippy.toml`; do not assume extra repo-specific commands exist.

## Database And Seed Data
- There is no migration runner in the repo. Apply `migrations/0001_*.sql` through `0008_*.sql` manually in filename order.
- `migrations/0002_seed_data.sql` inserts the default admin/demo data described in `README.md`; use that when you need a loginable local database quickly.

## Code Structure
- `src/app.rs` wires tracing, env config, the RBatis connection, permissive CORS, request tracing, and the JSON UTF-8 content-type patch.
- `src/state.rs` is the shared state: config, `Arc<RBatis>`, and an in-memory verification-code store. Login codes are not persisted.
- `src/models/*.rs` are thin `*Entity` structs with `rbatis::crud!`; most business logic lives directly in `src/routes/*.rs`.
- `src/db/mod.rs` contains compatibility helpers such as `select_by_map_compat` that normalize RBatis/MySQL result shapes, including JSON and `DateTime` fields. Prefer those helpers over ad hoc raw decoding.

## Handler Conventions
- Protected routes do not use a global auth middleware. Handlers explicitly take `HeaderMap` and call `auth::authenticate(&state, &headers)`.
- Role gates are manual via `CurrentAuthUser::require_any_role`; add the auth call and role check yourself when introducing a protected endpoint.
- Success responses should use `response::ApiResponse<T>`. Errors should return `AppError` so the standard `{ code, message, data, request_id }` envelope stays consistent.
