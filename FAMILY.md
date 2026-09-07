# The family contract, revision 1

What every program in this family answers, so that a stranger can find out what it does without
reading its source — and so a fifth program can join by implementing a list rather than by
imitating four codebases.

This document is the canonical text. It is copied into each repository beside the code that
implements it, and `scripts/gate-family.sh` is the executable version: point it at a binary and it
reports which of this contract that binary implements.

---

## The reply shape

Every answer, on every door, in either encoding:

```json
{"ok": true,  "family": 1, "n": 2, "result": [ … ]}
{"ok": false, "family": 1, "n": 0, "result": [], "error": "…", "fault": "refused"}
```

| field | means |
|---|---|
| `ok` | whether the call was answered |
| `family` | which revision of this contract the reply is written in — **this document is 1** |
| `n` | how many values came back |
| `result` | the values, **always a list**, so one call answers with what the function did |
| `error` | why not, when `ok` is false |
| `fault` | which kind of no. Absent means `refused`. |

**A refusal is a reply, and its exit status is zero.** A real error arriving as "exited 1" cannot
be told from the binary being missing, and the two need different responses: one is a bug to
report, the other is a sibling to carry on without.

**A newer `family` is refused by name; an older one is not.** A reply with no `family` at all is
from before the field existed and is accepted.

---

## The floor — every program answers these

| verb | answers |
|---|---|
| `verbs` | every verb this program answers, each with one line saying what it does |
| `client` | this program's own client library, as source, on stdout |

`verbs` answers **in the reply shape above**, never as prose. A program's self-description is the
one thing another program must be able to parse; if it is human text then the only way to discover
a surface is to read it, which is the situation this contract exists to end.

`client` is served by the program that implements the surface, so a consumer cannot hold a stale
copy — the client comes from the server. `lua-api` is accepted as an older name for it.

**A program with no client library still answers.** casper is reached by spawn with a JSON call
and has no Lua-facing surface, so its answer is a refusal that says so. The contract requires the
question to be *answered*, not that every program have a library — and "there is none, because the
surface is reached this other way" is a parseable answer, where silence is not.

---

## Coordinated — every program a coordinator may configure

| verb | answers |
|---|---|
| `needs` | what a coordinator may tell it, as typed declarations |
| `configure` | takes that configuration on stdin, applies it, and reports per setting what was kept and what was refused |

A refused setting is **named**, with what would have been accepted:

```json
{"name": "nonesuch",
 "why": "melchior takes no setting by that name; `needs` lists what it takes"}
```

A coordinator is not required to answer these. magi coordinates and is not coordinated; the other
three are.

---

## Encodings

`--json` and `--cbor`, on every verb above. JSON is the default and is what a person reading a
pipe gets. CBOR is the same shape as bytes, for a caller that is not going to read it.

On a socket nothing is negotiated: a body says which encoding it is in its first byte — JSON's
top level is `{` or `[`, CBOR's map or array is `0x80`–`0xBF`, and the ranges do not overlap — so
a reply goes back in whichever the call arrived in.

---

## Doors

A program may answer on its command line, on a socket, or both. **Where the two differ, the
difference is written down here**, so that a deliberate asymmetry is legible and an accidental one
cannot hide among them.

| program | command line | socket | deliberate differences |
|---|---|---|---|
| magi | the floor | one per session, its own CBOR protocol for a front end | Coordinates rather than being coordinated: no `needs`, no `configure`. |
| casper | the floor, plus `tools`, `run`, `surface` | read-only verbs only | **`run` is never reachable over the socket.** casper's job is running programs, and a socket that runs commands is a remote shell wearing a friendly name. The spawn link carries the trust instead. |
| melchior | the floor, plus `models`, `ask`, `serve`, `fork`, `auth` | the session surface | — |
| balthasar | the floor, plus its own memory verbs | the same memory verbs | Some verbs are owner-only: a socket peer may propose but may not pin, write globally, or purge, and may not sign its report as the user's judgment. |

---

## Two rules that are tested, not reviewed

**Advertised equals dispatched.** Every verb `verbs` lists is one the program answers, and every
verb it answers is one `verbs` lists. A surface that advertises what it refuses is worse than one
that advertises nothing, because the second is honest.

**The client library round-trips.** What `client` prints is what the program's own surface accepts.

---

## Joining the family

Implement the floor, answer in the reply shape, and pass `gate-family.sh`. Nothing else is
required — there is no registration, no manifest, and no shared library to link, because a shared
library between these programs is the dependency the whole arrangement exists to prevent.
