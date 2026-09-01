# memo

Memory for agents. Short-term and long-term, in one layer, driven over Lua.

A separate binary with its own store — not a library a harness links. It holds the four things
a harness cannot hold for itself: what is in the context window right now, what happened in this
session, what is true, and how things are done here. And the ladder between them.

```sh
memo remember "we run the tests with make test"
memo recall "tests" --explain
memo why <handle>            # the evidence, not just the number
memo decay                   # what today's forgetting would take, before it takes it
```

See [`PLAN.md`](PLAN.md) for the whole design.

## What is different about it

**Every durable memory names its witnesses.** Confidence is never assigned — it is computed
from the evidence that promoted a memory, and recomputed whenever that evidence changes. So
`memo why` prints an argument rather than a number:

```
project test_command make test
01M1CTNN7SZ613D58ZXM4JYT8Z  asserted · fact

confidence 0.82  ████████··
strength   1.00  ██████████  high

because 3 witness(es) across 2 session(s)
  correction    0.80  2 days ago in 01K9 at 412
  cost          0.50  1 week ago in 01K2 at 88
  distillation  0.28  3 weeks ago in 01J7 at 1201

since  3 weeks ago  still

related
  replaced     project test_command cargo test
```

**Two floors, and they are different floors.** A memory stops being *asserted* long before it
stops being *findable*. That gap is the answer to staleness: an agent that can say "you told me
this in March, it may be stale" instead of stating it flatly.

**Nothing is ever deleted.** Superseded, contradicted, decayed past the floor, forgotten on
purpose — every one of those is a column. `memo forget --purge` is the single exception, and it
exists so that "delete the key I pasted" can be answered with yes.

**Diversity beats volume.** One session repeating something is a person being emphatic; the
same thing surfacing in unrelated runs is a property of the world. Confidence counts sessions,
not mentions.

## Status

`PLAN.md` is the build order and says what is done. M0 through M9 are built.

```sh
memo serve                     # listen for a harness
memo ingest --source axon      # or read transcripts that already exist
memo consolidate               # carry what recurred into the project's memory
memo context "run the tests"   # exactly what a model would be told
memo eval                      # whether session k+1 is less annoying than session k
```

### Does it earn its place

```
10 session(s) in one project, one lesson

  with memory     90%  █████████·
  without memory   0%  ··········
  the ceiling     90%  the first session of a project has nothing to know from
```

A session that discovers `make test` after `cargo test` fails has the next one start knowing
it. `memo eval` runs that N times with memory and without. Synthetic and reproducible on
purpose — no clock, no model, no network, no embedder — and every one of those points is the
rule-based path, with no distiller in the loop at all.

### A project has many sessions

Durable memory is the project's and every session in it reads it. What a session holds on its
own dies with it, unless something on the ladder carries it across.

```
$ memo sessions
/home/you/work/thing
the project. every session below shares its memory.

0831-yt8z  get the test suite passing
     2 hours ago · axon · 2 kept
```

Every answer says which project and which run — by name, never by a twenty-six character id.

```sh
make build        # the binary
make test         # the suite
make gates        # the architectural gates
make verify       # all of it
```

The gates are not advisory:

| Gate | Rule |
|---|---|
| `gate-file-size` | no `.rs` over 800 lines |
| `gate-no-delete` | no `DELETE FROM` outside `purge.rs` |
| `gate-independent` | no Rust file names a harness |
| `gate-witnessed` | every asserted memory answers for itself |
| `gate-no-llm` | the suite passes with no key, no network, no embeddings |

The last one is the load-bearing one. A model makes memo better; its absence never makes memo
fail, and the only way that stays true is to prove it on every run rather than remember it.

## Requirements

`.make.lua` and `.env.lua` are read by [oslo](https://github.com/termworks/oslo), which provides
both the `make` task runner and the directory environment. Without it, `cargo` does everything
`make` does.
