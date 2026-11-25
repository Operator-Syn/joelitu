# Commands

The user contract. Internal steps live in rustdoc, not here.

Both commands are guild-only and require **Manage Messages**.

## `/spam`

| Option | Type | Required | Range | Default |
| --- | --- | --- | --- | --- |
| `message` | text | yes | 1–2000 characters, not whitespace-only | — |
| `mention` | user | no | any member | none |
| `count` | integer | no | 1–10 | 5 |
| `interval` | number (seconds) | no | ≥ 1.2 effective | 1.2 |

**Behaviour**

- Posts `message` `count` times.
- `mention` appears on the **first message only**. A run never becomes a ping burst.
- `interval` below 1.2 s is raised to 1.2 s and the acknowledgement says so.
- `count` outside 1–10 is rejected, not clamped: asking for 50 is a mistake worth reporting.
- Up to 2 runs may be active in one channel at a time.
- The message body is posted as written. Mentions typed into the body — `@everyone`, role mentions — do **not** ping, because the allowed-mentions set is exactly the optional `mention` user.

**Acknowledgement**

```
posting 5 message(s)
posting 5 message(s); interval raised to 1.2s (minimum)
```

**Errors** (exact strings the user sees)

| Cause | Message |
| --- | --- |
| Empty or whitespace message | `message must not be empty` |
| Over 2000 characters | `message is {n} characters; the limit is 2000` |
| Count 0 or above 10 | `count must be between 1 and 10` |
| NaN, infinite, or negative interval | `interval must be a finite number of seconds` |
| Channel already at capacity | `this channel already has {n} run(s) in flight; wait for one to finish or cancel it` |

**Examples**

```
/spam message:"good morning" count:3 interval:2
/spam message:"hey" mention:@someone
/spam message:"one more time" count:2
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
