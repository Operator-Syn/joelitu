# Operations

Runbook. Answers first, explanation second.

The image is built by CI and pulled from GHCR. The VPS never compiles anything: no Rust toolchain, no `cargo`, no build cache to maintain.

## First deploy on a fresh VPS

```sh
git clone git@github.com:Operator-Syn/joelitu.git && cd joelitu
cp deploy/joelitu.env.example deploy/joelitu.env
$EDITOR deploy/joelitu.env          # DISCORD_TOKEN, optionally TEST_GUILD_ID
docker compose pull
docker compose up -d
```

The GHCR package is public, so no login is needed on the VPS: an anonymous pull returns the manifest. Verified 2026-11-25 — `HTTP/2 200` for `ghcr.io/v2/operator-syn/joelitu/manifests/latest`.

If you ever make the package private, the VPS needs a token with `read:packages`:

```sh
echo "$GHCR_TOKEN" | docker login ghcr.io -u Operator-Syn --password-stdin
```

## Deploying a new version

```sh
git pull
docker compose pull
docker compose up -d
```

`pull_policy: always` means `up -d` recreates the container when the tag moved. To pin a version instead of tracking `latest`, edit the image tag in `compose.yaml` to a version or a short sha — every build is tagged with both, so a deployed container is traceable to a commit.

## Is it up?

```sh
docker compose ps
docker compose logs --tail=100 joelitu
```

A healthy start prints, in order:

```
INFO joelitu: commands registered for the test guild     (or: registered globally)
```

Anything before that line that says `required but not set` is a configuration failure; the process exits non-zero and Docker restarts it in a loop. Fix the env file, not the container.

## It is down

1. `docker compose ps` — is the container `Up` or restarting?
2. `docker compose logs --tail=200 joelitu` — look for the last error before the exit.
3. `joelitu failed to start: ...` on stderr means startup validation refused. The message names the variable.
4. If the logs are empty and the container is gone, `docker compose up -d` and watch again.

## It says the commands do not exist

- Global registration can take **up to an hour** to propagate. If `TEST_GUILD_ID` is empty, that delay is expected, not a failure.
- Set `TEST_GUILD_ID` to a development server's id and restart: commands register there immediately.
- If the commands were registered before and are missing, check that the bot was invited with the `applications.commands` scope.

## It is posting faster than expected

It cannot. The pacing floor is 1.2 s and is not configurable — it is a constant in `src/spam/plan.rs`, not an environment variable. If messages look bunched:

- The interval was raised to the floor and the acknowledgement said so.
- A second run in the same channel is active; up to two are allowed, and they pace independently of each other. `/cancel` stops both.

## It will not stop

```sh
docker compose restart joelitu
```

A restart drops in-flight runs. A run lasts at most about twelve seconds, so at most a few messages are lost, and nothing is left in a broken state.

## Logs

Everything goes to stdout and is collected by the Docker log driver. Nothing is written to disk inside the container.

Per-run lines:

```
INFO joelitu: run completed run_id=3 sent=5
INFO joelitu: run cancelled run_id=3 sent=1
WARN joelitu: run failed run_id=3 sent=2 error=discord rejected the message: ...
```

A repeating `WARN` with a 429 inside is rate limiting, not an outage — serenity retries those. Sustained 429s mean something else is posting into the same channel.

## Rotating the token

```sh
$EDITOR deploy/joelitu.env      # replace DISCORD_TOKEN
docker compose up -d            # recreates the container with the new env
```

No image rebuild is needed; the token is not baked into the image. If the old token was leaked, reset it in the Discord developer portal first — an old token in a running container is not revoked by changing the file.

## Backups

None, and none are needed. There is no database and no persistent state. The only thing worth keeping is `deploy/joelitu.env`, which holds the token — treat it as a secret, not as data.

## Known behaviour, not a bug

| Symptom | Why |
| --- | --- |
| A run stops after a restart with no notice | No journal; runs are in-memory by design |
| `/cancel` says `no active run` right after a run finished | Entries are removed when a run ends |
| `no active run in this channel` while a run is visibly posting | That run is being served by a different process — only happens if you run two containers on the same guilds, which is unsupported |
