# Architecture

One process, one binary, no database. The gateway receives slash commands; REST sends messages. Active runs live in an in-memory registry.

## Module map

```
src/
├── main.rs        Composition root: logging, config, client, shutdown
├── config.rs      Environment → Config. The only module that reads env.
├── error.rs       One error type per boundary
├── data.rs        Data { runs } — cheap handle, O(1) clone
├── framework.rs   Poise options and command registration
├── commands/      Transport adapters: options → request, result → reply
│   ├── spam.rs
│   └── cancel.rs
└── spam/          The feature
    ├── mod.rs     prepare / start; owns the guard and terminal logging
    ├── plan.rs    Pure: limits, validation, rendering
    ├── runner.rs  Pacing loop and cancellation
    └── registry.rs  Active runs, per-channel cap, release guard
```

## Allowed dependency edges

| Module | May depend on | May not |
| --- | --- | --- |
| `plan` | `error` | anything with I/O, async, or serenity |
| `runner` | `plan`, `error` | registry, serenity types other than the error path |
| `registry` | `tokio-util` | the domain or the transport |
| `spam` (mod) | `plan`, `runner`, `registry`, `data` | `commands` |
| `commands/*` | `spam`, `data`, `error` | `spam::registry` internals |
| `framework` | `commands`, `data`, `config` | anything in `spam` beyond what commands need |
| `main` | everything | — |

The compiler cannot enforce this table; a reviewer does. The rule that makes it real: **`commands/spam.rs` contains no numeric limit and branches on no domain value.** If it needs one, the rule belongs in `plan` instead.

## A run's lifecycle

1. **Prepare.** `spam::prepare` validates the request into a `Plan` and reserves a registry slot. Validation first, so a rejected request never consumes capacity.
2. **Acknowledge.** The handler replies before any message is sent. If the reply fails, the prepared run is dropped — releasing its slot — and nothing is posted.
3. **Run.** `spam::start` spawns a task holding a `RunGuard`. The task calls `runner::drive` with a send closure that builds every message, setting `allowed_mentions` explicitly on each one.
4. **Finish.** The outcome is logged. The guard's destructor releases the registry slot on every exit path, including a panic.

## Invariants the type system cannot express

- **Every reservation is released.** Enforced by `RunGuard`'s destructor, not by callers remembering.
- **Pacing uses `sleep`, never `interval`.** `tokio::time::interval` fires its first tick immediately, which sends messages one and two back to back.
- **`allowed_mentions` is always set explicitly.** Either the single planned user or empty. A literal `@everyone` typed into the message body must never ping; the only ping in a run is the one the caller selected.
- **One process.** The registry is the source of truth for active runs; two processes would each have their own.

## Shutdown

`SIGTERM` cancels every active run and returns within the compose `stop_grace_period` (20 s). Without that, `docker stop` would kill the process mid-loop and the remaining messages would vanish with no log line saying why.
