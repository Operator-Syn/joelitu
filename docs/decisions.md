# Decisions

Dated entries, newest last. Each records **why**, never what the code does — the code and its rustdoc are the source of truth for behaviour. When a decision changes, the entry is superseded by a newer one, not edited.

---

## 2026-11-25 — Rust, serenity, poise

Single binary, `serenity` 0.12 for gateway and REST, `poise` 0.6 for slash commands.

Alternatives considered: `discord.js`/TypeScript (fastest to write, but the operator wanted Rust), Rust on Cloudflare Workers (works for slash-command-only bots, but timer-driven work needs Durable Objects and the Rust binding's alarm examples are thin), `twilight` (more modular, more assembly).

Consequence: a long-lived process to supervise, which Docker handles with a restart policy.

---

## 2026-11-25 — No database

Postgres was planned, then dropped.

Reason: a run lasts at most ~12 seconds. Persistence would exist only to survive a restart mid-burst; the worst case without it is that the last few messages never arrive, silently. That does not justify a pool, migrations, a compose service, and a reconcile-on-startup path.

Consequences, all accepted:

- A restart drops in-flight runs. Documented in `operations.md` as behaviour.
- `/cancel` reaches only runs in the serving process.
- Adding a second process on the same guilds would need shared state reintroduced. The seam is the per-channel cap in `registry`; nothing is added speculatively now.

---

## 2026-11-25 — Limits are constants, not configuration

`DEFAULT_COUNT = 5`, `MAX_COUNT = 10`, `MIN_INTERVAL = 1200 ms` live in `src/spam/plan.rs`.

These are safety properties. An environment variable that can set the pacing floor to zero is a way to get the bot rate-limited or its application flagged, so config holds only what genuinely differs per deployment: the token and the optional test guild id.

---

## 2026-11-25 — The interval floor is clamped and disclosed

A request below 1.2 s is raised to 1.2 s and the acknowledgement says so, rather than the command failing.

A silent clamp would contradict "loud failure"; a hard rejection would make the command annoying for an obviously small mistake. Disclosing the adjustment is the middle that keeps the command usable and the user informed.

Count, by contrast, is rejected when out of range: asking for 50 expresses an intent that clamping would quietly ignore.

---

## 2026-11-25 — The mention repeats on every message

Supersedes the entry below, which attached the mention to the first message only.

Reason: the operator's intended behaviour is a repeat, and the earlier restriction was my own judgement rather than a requirement — the same call I made when refusing the unbounded version of this command. It was too broad. What makes a repeat defensible is not sending the mention once; it is that the count is capped in code (10), the pacing floor is enforced (1.2 s), the command is gated on `Manage Messages`, and any run can be stopped with `/cancel`. Those four are unchanged.

`Plan::content()` now renders the same body for every send, so it takes no index. `allowed_mentions` is still set explicitly on every send, still to exactly the selected user — the guard against `@everyone` typed into the body is untouched.

Accepted consequence: a default run pings one person five times. Ten mentions in twelve seconds is the pattern Discord's anti-spam heuristics watch, so the realistic failure is rate limiting or mention suppression, not a ban. Documented in `docs/commands.md` so it is not discovered as a surprise.

---

## 2026-11-25 — Mention on the first message only, with `allowed_mentions` set explicitly

The message body is arbitrary user input. If `allowed_mentions` were left unset, `@everyone` or a role mention typed into the body would ping on every message.

Every send sets `allowed_mentions` explicitly — the single planned user, or empty. The rendering rule (mention on index 0 only) lives in `plan.rs`, so it is testable without Discord.

---

## 2026-11-25 — The runner does not touch the registry or the network

`runner::drive` is pure control flow over a `Plan`, a `CancellationToken`, and an injected send closure.

The alternative — letting the pacing loop own sends and registry bookkeeping — would make the one behaviour that needs deterministic tests depend on a live database and a live gateway, and would put terminal-state decisions in two places instead of one.

---

## 2026-11-25 — `sleep`, not `interval`

`tokio::time::interval` fires its first tick immediately. Used naively for pacing, it sends messages one and two back to back on every run.

`sleep` inside the loop has no such trap and needs no `MissedTickBehavior` reasoning. The final message is not followed by a sleep either, so a run never outlives its own pacing by one interval.

---

## 2026-11-25 — `RunGuard` instead of explicit release

Every reservation must be released. A spawned task can return normally, return early, or unwind, and a chain of explicit releases covers only the first two.

A destructor covers all three. This is why the registry exposes `RunGuard` rather than a public `release`.

---

## 2026-11-25 — Channel-scoped `/cancel`

No run ids are exposed to users. Run ids are process-local and meaningless outside it, so `/cancel` operates on the channel the caller is in.

Consequence: `/cancel` cannot stop another channel's run, and reports "no active run" for anything already finished, since completed runs are not retained.

---

## 2026-11-25 — Commands require Manage Messages

Without a gate, any member of any guild the bot has joined could make it post ten messages at a time in any channel it can see.

This is the one line in `commands/spam.rs` and `commands/cancel.rs` that is a deliberate policy choice rather than plumbing: `required_permissions = "MANAGE_MESSAGES"`. Deleting it makes both commands open to every member.

---

## 2026-11-25 — The image is built in CI and pulled from GHCR

`ghcr.io/operator-syn/joelitu`, published by `.github/workflows/ci.yml`.

Reason: the VPS should run a container, not compile one. A release build of the serenity dependency graph is several hundred crates and minutes of CPU; doing that on the box means carrying a Rust toolchain, a `~/.cargo` cache, and enough RAM headroom for eight rustc processes — all of it duplicated from the machine that already builds and tests the code.

The image job `needs: check`, so a red test suite publishes nothing. Every build is tagged `latest`, the branch, the version (on `v*` tags), and the short commit sha; the sha tag is what makes a running container traceable to a diff.

`compose.yaml` carries `image:` and `pull_policy: always` with no `build:` stanza. A compose file with both would silently build locally and hide the fact that the registry copy is stale.

Consequence to know: this package is public, so the VPS pulls it without credentials. If you set it private, the VPS needs `docker login ghcr.io` with a `read:packages` token — documented in `operations.md`.

---

## 2026-11-25 — Docker, no systemd unit

The Docker daemon starts at boot and `restart: unless-stopped` brings the container back after a crash or reboot.

A systemd unit wrapping `docker compose up -d` would report the wrapper's health rather than the container's, and adds a layer that hides what the operator actually wants to inspect. If ordering ever matters, the unit is four lines with `After=docker.service` and `RemainAfterExit=yes`.

`init: true` reaps zombies because the bot is PID 1. `stop_grace_period: 20s` gives SIGTERM time to cancel runs instead of being SIGKILLed mid-loop.

---

## 2026-11-25 — Slim glibc runtime, not musl or scratch

The GNU target needs glibc, so the runtime image is `debian:bookworm-slim`.

A musl static binary would allow `scratch` and a smaller image, at the cost of extra build tooling for the TLS stack. The payoff is a few megabytes on a VPS with disk to spare, so the simpler build wins.

The release profile keeps `debug = "line-tables-only"` and `panic = "unwind"`, and does not strip symbols: a backtrace is the only diagnostic available after a container restart.

---

## 2026-11-25 — `MAX_COUNT` is 99, and one run per channel

Raising the ceiling from 10 to 99 is a parameter change. It exposed a separate defect that had to be fixed with it.

`DEFAULT_MAX_RUNS_PER_CHANNEL` was 2, justified by the claim that "two runs pacing at the floor still land under five messages per five seconds". That arithmetic was wrong when it was written: two runs at 1.2 s is 2 messages per 1.2 s, about 8.3 per five seconds, against an allowance of roughly 5. At five messages each the overlap lasted six seconds and produced a handful of 429s nobody noticed. At 99 messages each the overlap would last two minutes of retries, so the bug had to go before the ceiling moved.

The cap is now 1. A second `/spam` in a channel is refused rather than run unreliably, which keeps "the interval is the interval" true.

Rejected: making the pacing floor adaptive so concurrent runs share a budget. It preserves concurrency at the cost of a second pacing story the tests would have to model. Serialising is one constant and one honest error message.

Also rejected: anything that disguises the traffic to avoid platform detection — spreading sends across webhooks or tokens, jittering timing to look human, or splitting a run across channels. The rate limits are enforced server-side per route, and evasion is what turns rate limiting into application termination. The command stays inside the documented allowance instead.

---

## 2026-11-25 — Pacing is derived from a channel budget, and mentions have a cooldown

Two guards replace "pick an interval that feels slow enough".

**Budget.** `CHANNEL_BUDGET_PER_MINUTE = 30`, and `MIN_INTERVAL` is computed from it (`60_000 / 30` = 2 s) instead of being a separate constant. Two numbers that must agree will eventually disagree; deriving one from the other makes that impossible. Because only one run may be active per channel, per-run pacing is channel pacing, so the budget holds at every `count` — a 99-message run simply takes about three minutes.

**Cooldown.** A user may be mentioned once per channel every five minutes (`MENTION_COOLDOWN`), enforced in the registry, checked before the run reserves and recorded only after. That ordering matters: recording first would put a user on cooldown for a run that was then refused for a busy channel. The cooldown covers mentions only — plain messages are limited by the budget alone — because repeated pings at one person are the abuse shape, and repeated text is not.

Timing uses `tokio::time::Instant`, so the cooldown is exercised under `start_paused` in tests rather than by waiting five real minutes.

---

## 2026-11-25 — Throughput comes from shared budgets, not from going faster

The previous design conflated two things: how fast a run should send, and how much a channel may receive. A per-run `sleep` answered the first, and a one-run-per-channel cap was the only thing answering the second — which meant a second user in the same channel was refused rather than served.

`spam::pacer` now owns both budgets: five tokens per channel refilling at one per second, and fifty per second for the process. A send waits for a token instead of being refused. The ceiling on concurrent runs rises to four because it is no longer a rate control, only a fairness and memory bound: four runs share one channel allowance, so the channel's output is bounded no matter how many are active, and each user gets latency rather than an error.

The per-run interval stays, floored at 2 s, as a fairness floor: one run cannot consume the whole channel allowance by itself, leaving half available for a concurrent run.

The runner changed with it: the send is now raced against cancellation, because waiting for a permit is unbounded and `/cancel` must be able to interrupt a queued send. Without the race the command would appear frozen behind a queue.

Rejected again: any scheme that disguises the traffic — spreading sends across webhooks, jittering to look human, or splitting a run across channels. The budgets match Discord's published allowances; the goal is to stay inside them by construction, not to hide from them.

---

## 2026-11-25 — Replies are ephemeral; the invocation line is not ours to hide

`/spam` and `/cancel` now reply with `CreateReply::ephemeral(true)`, and a custom `on_error` handler does the same for every command error. The reason is attribution: the count, the clamped interval, and which validation failed are the invoker's business, and broadcasting them makes the command feel like a public accusation.

What this does **not** achieve: Discord renders "user used /spam" in the channel itself. That line is client UI for slash-command invocations, produced by Discord rather than by the bot, and an application cannot suppress it. Any claim that the command is invisible would be false, so `docs/commands.md` states the limit explicitly.

A DM invocation would avoid the channel line, but it is not available here: the command is `guild_only` and gated on `Manage Messages`, neither of which can be evaluated in a DM. Making it DM-capable would mean dropping the permission gate, which is a worse trade than the visual attribution.

The channel messages remain public. Making them ephemeral would remove the feature: an ephemeral message is a reply to the invoker, not something the channel can read.

---

## 2026-11-25 — The mention cooldown is removed

`MENTION_COOLDOWN` is gone, along with its registry state, its error variant and its test. Yashindo's instruction: it felt restrictive with no freedom, and the constraint was not earning its cost.

Recorded honestly, because the entry that introduced it claimed otherwise: it never prevented the behaviour it was named for. A single run at `count:99` pings the target ninety-nine times regardless, since the cooldown is checked once per run. All it prevented was a second run inside the window — which meant the command was blocked for minutes after any ordinary use, while the ninety-nine-ping case remained available.

What actually bounds a run is `MAX_COUNT`, the pacer's channel budget, the `Manage Messages` gate, and `/cancel`. Those are unchanged. The gap this leaves, stated plainly: several people can now ping the same target back-to-back, and the only remedy is cancelling the runs.

Timing state for mentions is gone with it, so the registry is back to one map and one counter.

---

## 2026-11-25 — `/cancel` is scoped to the caller's own runs

The registry records `started_by` again, and `cancel_owned_in_channel` matches on both channel and caller. Previously `/cancel` stopped every run in the channel, which meant one member could kill another member's burst — including a member with `Manage Messages` stopping someone who had done nothing wrong.

Hierarchy was considered and rejected: letting administrators cancel others would be defensible in a moderation tool, but this command posts messages on the caller's behalf, so the run is theirs. Anyone who needs a hostile run stopped has Discord's own moderation tools, which leave an audit trail; a slash command should not be a quieter version of the same power.

`started_by` was removed once before as dead state. It is load-bearing now, which is why the field carries a doc comment explaining what would break without it.

The acknowledgement text changed with the behaviour: "cancelling your 2 runs", and "you have no active run in this channel" when the caller owns none — which is no longer the same as "nothing is running here".






