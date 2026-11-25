# Commands

The user contract. Internal steps live in rustdoc, not here.

Both commands are guild-only and require **Manage Messages**.

## `/spam`

| Option | Type | Required | Range | Default |
| --- | --- | --- | --- | --- |
| `message` | text | yes | 1–2000 characters, not whitespace-only | — |
| `mention` | user | no | any member | none |
| `count` | integer | no | 1–99 | 5 |
| `interval` | number (seconds) | no | ≥ 2.0 effective | 2.0 |

**Behaviour**

- Posts `message` `count` times.
- `mention` appears on **every message**, so a default run pings the target five times.
- `interval` below 2.0 s is raised to 2.0 s and the acknowledgement says so. The floor is half the channel's sustained allowance (the pacer permits a burst of 5 then one per second), so a single run cannot consume all of it and a concurrent run still gets through.
- `count` outside 1–99 is rejected, not clamped: asking for 500 is a mistake worth reporting.
- Up to **4 runs** may be in flight in one channel. Each message in every run waits for a shared channel budget (5 burst, then 1/second) and a process-wide budget (50/second), so the channel's total output stays inside what Discord allows and a busy channel queues rather than failing or bursting. A fifth run in the same channel is refused until one finishes.
- At `count:99` a run lasts about three minutes.
- A given user may be mentioned **once per channel every 5 minutes**. A second mention inside that window is refused with the remaining time. Plain messages are not rate-limited this way; only mentions are.
- The message body is posted as written. Mentions typed into the body — `@everyone`, role mentions — do **not** ping, because the allowed-mentions set is exactly the optional `mention` user.

**Acknowledgement**

Sent **ephemerally** — only the invoker sees it.

```
posting 5 message(s)
posting 5 message(s); interval raised to 2.0s (minimum)
```

All error replies are ephemeral for the same reason.

**What is not private**

Discord renders a line in the channel saying who used the command (`kyle used /spam`). That is the client's UI for slash-command invocations, not a message the bot sends, and an application cannot suppress it. The bot's own replies are hidden; the fact that the command ran is not. The messages themselves are public by definition — they are the feature.

If the run should be fully private, this bot cannot do it: `/spam` is guild-only and gated on `Manage Messages`, so it cannot be invoked in a DM where the invocation line would not appear. Privacy here covers the replies, not the fact that the command ran.

**Errors** (exact strings the user sees)

| Cause | Message |
| --- | --- |
| Empty or whitespace message | `message must not be empty` |
| Over 2000 characters | `message is {n} characters; the limit is 2000` |
| Count 0 or above 99 | `count must be between 1 and 99` |
| NaN, infinite, or negative interval | `interval must be a finite number of seconds` |
| Channel already at capacity | `this channel already has {n} run(s) in flight; wait for one to finish or cancel it` |
| Same user mentioned too recently | `that user was mentioned here in the last few minutes; try again in about {n}s` |

**Examples**

```
/spam message:"good morning" count:3 interval:5
/spam message:"hey" mention:@someone
/spam message:"one more time" count:2
/spam message:"the long one" count:99 mention:@someone
```

## `/cancel`

| Option | Type |
| --- | --- |
| none | — |

Stops every active run **in the calling channel**. Replies with the count:

```
cancelling 2 runs
cancelling 1 run
no active run in this channel
```

`no active run in this channel` is a normal answer, not an error. A run that already finished leaves nothing to cancel, and the registry does not retain completed runs.

## Not implemented

There is no `/spam history`, no run listing, and no per-run ids. The registry holds active runs only, and entries are removed the moment a run ends.