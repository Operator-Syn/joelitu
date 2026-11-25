# Development

## Prerequisites

- Rust 1.85 or newer. The `Dockerfile` pins `rust:1.85-bookworm`, CI pins `1.85.0`, and `rust-toolchain.toml` pins the same minor for anyone using rustup.
- Build tools: `cc` (bundled TLS and any C dependencies).
- Docker, only if you want to build the image locally. Normal deployment pulls it from GHCR.

## Commands

```sh
cargo check --all-targets     # type check, including tests
cargo test                    # unit tests; no token, no network, no Discord
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run                     # needs DISCORD_TOKEN
```

`cargo test` needs no credentials. The runner tests use `tokio::time::pause` and a recording closure; the registry tests use a `std::panic::catch_unwind` probe; the config tests inject a lookup closure instead of touching the environment.

## Continuous integration

`.github/workflows/ci.yml` runs fmt, clippy (warnings are errors) and the tests on every push and pull request. On `main` and on `v*` tags it then builds the image and pushes it to `ghcr.io/operator-syn/joelitu`, tagged `latest`, the branch, the version, and the short commit sha. The image job depends on the check job, so a red test suite publishes nothing.

Local image build, to reproduce what CI produces:

```sh
docker build -t joelitu:local .
```

## Running against Discord

```sh
export DISCORD_TOKEN=...      # from the developer portal
export TEST_GUILD_ID=...      # optional but recommended
cargo run
```

With `TEST_GUILD_ID` set, commands register against that guild and appear within seconds. Without it they register globally and can take up to an hour — which makes the edit/verify loop unusable. Invite the bot with both the `bot` and `applications.commands` scopes.

## Documentation policy

Code documentation requirements come from the vault's documentation standard, applied with one threshold so reviewers are consistent:

- The full interface block — `# Arguments`, `# Returns`, `# Errors`, `# Panics`, `# Processing` — is required for every module, type, and function with **three or more steps**.
- Below that, a one-line purpose on the item is enough. Demanding a numbered walkthrough for `fn is_cancelled(&self) -> bool` is how a policy gets ignored wholesale.
- Numbered steps in the interface block must correspond to numbered comments in the body. When they drift, the code changed and the documentation did not.
- Comments explain position and intent, never syntax. A comment that restates the line beneath it is noise.
- Behaviour changes and documentation changes are one edit, never a follow-up.

Mechanical checks, enabled in `Cargo.toml`: `missing_docs`, `clippy::missing_docs_in_private_items`, `missing_errors_doc`, `missing_panics_doc`. These cover presence. Section shape stays a review rule, because the compiler can only see whether something is documented, not whether it says anything.

## Where each fact lives

- **rustdoc** — a function's behaviour. Authoritative.
- **`docs/commands.md`** — the user contract, including exact reply strings.
- **`docs/architecture.md`** — the module shape and lifecycle.
- **`docs/operations.md`** — what to do when it breaks.
- **`docs/decisions.md`** — why. Superseded, never rewritten.

If a fact appears in two of these, one of them is wrong.

## Conventions

- No logging in `runner`: it is pure control flow and returns an outcome.
- No numeric limits in `commands/`: they belong in `plan.rs`.
- No serenity types in `plan.rs`: mention handling is expressed as a user id.
- Prefer `sleep` over `interval` for pacing; see `docs/architecture.md` for why.
