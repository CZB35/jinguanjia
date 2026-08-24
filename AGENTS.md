## Repo Shape
- This is a single-binary Rust backend. The root `Cargo.toml` is the only manifest, and `src/main.rs` only calls `app::bootstrap()`.
- `src/routes/mod.rs` is the executable source of truth for the HTTP surface. `src/app.rs` mounts it under `/api`; uploads are served separately from `/uploads/*`.
- `docs/*.md` are helpful product docs, but they can lag code. Trust `src/routes/mod.rs` and the handler files first.

## Run And Verify
- Runtime config is loaded from `.env` via `dotenvy`; start from `.env.example`. `.env` is gitignored and overrides defaults in `src/config.rs`.
- The main env values that usually matter locally are `DATABASE_URL` and `JWT_SECRET`. Defaults bind to `0.0.0.0:3001`.
- Start the service with `cargo run --release`.
- Fast manual check: `GET /api/health`.
- Repo-level verification is `cargo test`, but it currently runs `0` tests, so treat it as a compile smoke test. A cold Windows build is slow.
- There is no checked-in CI workflow, task runner, `rustfmt.toml`, or `clippy.toml`.

## Database And State
- There is no migration runner. Apply `migrations/*.sql` manually in filename order; the repo currently goes through `0019_wechat_notice_delivery.sql`.
- `migrations/0002_seed_data.sql` inserts the default admin account and baseline demo data.
- `src/state.rs` keeps login verification codes in memory only. They are not persisted across restarts.
- `uploads/` is gitignored. `storage::resolve_upload_root()` resolves relative upload paths back to the project root when running from `target/debug` or `target/release`, so `cargo run` still writes to the repo-level upload directory.

## Code Conventions
- `src/models/*.rs` are thin `*Entity` structs with `rbatis::crud!`; most business logic lives directly in `src/routes/*.rs`.
- Prefer `src/db/mod.rs` helpers such as `select_by_map_compat` / `select_one_by_map_compat` over ad hoc RBatis decoding; they normalize MySQL result shapes plus JSON and `DateTime` fields.
- Protected routes do not use a global auth middleware. Handlers explicitly accept `HeaderMap` and call `auth::authenticate(&state, &headers)`.
- Role checks are also manual via `CurrentAuthUser::require_any_role(...)`.
- Success responses should use `response::ApiResponse<T>`. Return `AppError` for failures so the standard `{ code, message, data, request_id }` envelope stays intact.
