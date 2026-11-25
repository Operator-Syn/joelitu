# joelitu

A Discord bot that posts a message a bounded number of times, with an optional mention that repeats with every message, and a `/cancel` to stop a run in flight.

Rust, one process, one container, no database.

## Commands

| Command | Effect |
| --- | --- |
| `/spam message:<text> [mention:@user] [count:1-99] [interval:seconds]` | Posts the message `count` times, `interval` seconds apart. Defaults: 5 messages, 1.2 s. The mention is attached to every message. |
| `/cancel` | Stops every active run in the current channel. |

Both require **Manage Messages** and work in guilds only. Full contract: [`docs/commands.md`](docs/commands.md).

## Run it

Development (no container):

```sh
export DISCORD_TOKEN=...
export TEST_GUILD_ID=...        # optional; makes commands appear instantly
cargo run
```

Deployment (on the VPS; the image is built by CI, never on the box):

```sh
cp deploy/joelitu.env.example deploy/joelitu.env   # fill in the token
docker compose pull
docker compose up -d
docker compose logs -f
```

Updating is `git pull && docker compose pull && docker compose up -d`. There is no local build step on the server.

## Docs

- [`docs/architecture.md`](docs/architecture.md) — module map, dependency edges, run lifecycle
- [`docs/commands.md`](docs/commands.md) — user contract, limits, error strings
- [`docs/operations.md`](docs/operations.md) — runbook: it is down, it is stuck, logs, token rotation
- [`docs/development.md`](docs/development.md) — build, test, lint, doc policy
- [`docs/decisions.md`](docs/decisions.md) — why the design is the way it is

## Known behaviour

A restart drops in-flight runs. A run lasts at most about twelve seconds, so the worst case is that the last few messages never arrive. This is deliberate: there is no journal to resume from, and resuming a burst minutes later would surprise the channel.
