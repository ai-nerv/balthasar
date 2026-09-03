# balthasar

Memory for agents. Short-term and long-term, in one layer, driven over Lua.

A separate binary with its own store — not a library a harness links. It holds the four things
a harness cannot hold for itself: what is in the context window right now, what happened in this
session, what is true, and how things are done here. And the ladder between them.

```sh
balthasar remember "we run the tests with make test"
balthasar recall "tests" --explain
balthasar why <handle>            # the evidence, not just the number
balthasar decay                   # what today's forgetting would take, before it takes it
```

```
                          a harness
                              │
                      observe │  SO_PEERCRED names it, so the kernel says who
                              ▼
                     ┌────────────────────┐
                     │  api@default.sock  │
                     └────────┬───────────┘
              ┌───────────────┴───────────────┐
              ▼                               ▼
    <run>/transcript.db                <run>/memory.db
    that run's turns, verbatim         this run's scratch
    the only copy of what was said     dies with the run
              │                               │
       ┌──────┴───────┐                       │  the ladder — eight kinds of
       ▼              ▼                       │  evidence, two floors
   turn_fts      plan · scroll                ▼
   searchable    mask · summarise      project.db      global.db
       │              │                facts, habits   true everywhere
       │              │                       │
       │ spans        │ the window            │ memories
       │ evidence,    │                       │ asserted above 0.35
       │ not truth    │                       │
       └──────────────┴───────────┬───────────┘
                                  ▼
                             the prompt
```

Three sources, one budget. The window is what just happened; the memories are what crossed the
ladder; the spans are what nobody ever wrote down — offered as evidence that somebody said a
thing, which is a weaker claim than the thing being true.

The design lives in the code's own doc comments — `confidence.rs` for the witness model,
`claim.rs` for when two claims are one claim, `akin.rs` for why the embedder was measured
for that job and rejected. That is where to read *why* something works the way it does.

## What is different about it

**Every durable memory names its witnesses.** Confidence is never assigned — it is computed
from the evidence that promoted a memory, and recomputed whenever that evidence changes. So
`balthasar why` prints an argument rather than a number:

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

**Evidence has weights, and the weights are the design.** Eight kinds, and what one of each is
worth. The top three cross alone; the rest have to find company.

```
  SAID      ████████████████████  1.00  the person said to remember it — and it pins
  FIX       ████████████████      0.80  the person corrected something
  SCAR      ██████████            0.50  it was expensive to learn
  ─────────────────────────────── 0.50  promote · crosses alone above this line
  MANUAL    ████████              0.40  a peer proposed it over the socket
  INFERRED  ███████               0.35  a model read the prose and suggested it
  TIDE      ██████                0.30  it fell out of the context window
  ─────────────────────────────── 0.30  hold · waits in scratch above this line
  CALLUS    █████                 0.25  it recurred in unrelated runs
  SLEEP     ████                  0.20  a consolidation pass produced it
```

TIDE 0.30 plus CALLUS 0.25 is 0.55. Something that scrolled out of a window once and then
recurred in an unrelated run becomes a fact without anyone having said so.

**Two floors, and they are different floors.** A memory stops being *asserted* long before it
stops being *findable*. That gap is the answer to staleness: an agent that can say "you told me
this in March, it may be stale" instead of stating it flatly.

**Nothing is ever deleted.** Superseded, contradicted, decayed past the floor, forgotten on
purpose — every one of those is a column. `balthasar forget --purge` is the single exception, and it
exists so that "delete the key I pasted" can be answered with yes.

**Diversity beats volume.** One session repeating something is a person being emphatic; the
same thing surfacing in unrelated runs is a property of the world. Confidence counts sessions,
not mentions.

**A contradiction is a correction.** A turn that disagrees with what the project already holds
is read as a correction whatever words it used — so "we moved off Heroku last month" lands,
though it carries no marker any word list would catch.

**What was said is searchable.** A claim stated once, never repeated and never extracted lives
in the transcript and nowhere else. `recall` answers from it too, as evidence — never as truth.

## Status

Built. M0–M9, F0–F9, and the three things a survey of the field turned up afterwards —
searching the spans, the control arm that can lose, and an erasure that reaches what was
derived from what. 927 tests, seven gates.

```sh
balthasar serve                     # listen for a harness
balthasar distil                    # read what this project's own runs said
balthasar ingest --source magi      # or read transcripts that already exist
balthasar consolidate               # carry what recurred into the project's memory
balthasar context "run the tests"   # exactly what a model would be told
balthasar forget <handle> --purge   # gone, with everything derived from it
balthasar eval --full --long        # measured against the arm that can beat it
```

### Does it earn its place

A session that discovers `make test` after `cargo test` fails should leave the next one already
knowing. `balthasar eval` runs that N times, three ways.

```
$ balthasar eval --full --long --sessions 40

agent outcomes
      57%  task success against a 57% ceiling, 0% without memory
      41%  the same history in the window, no memory at all — balthasar is ahead by 16 points
       2   sessions before memory catches the window
```

The third arm is the one that matters, and it is the one most memory systems do not run: the
same history simply carried forward in the window, with no memory layer at all. It can win, and
on a short history it does —

```
$ balthasar eval --full --sessions 30        # one lesson, repeated

      97%  task success against a 97% ceiling, 0% without memory
      97%  the same history in the window, no memory at all — balthasar is level
```

**Level.** Carrying the text forward does exactly as well until the history outruns the window.
That is the whole defensible claim, and printing it is not optional — a benchmark shaped so that
losing is impossible measures nothing. Synthetic and reproducible on purpose: no clock, no model,
no network, no embedder, and every point is the rule-based path with no distiller in the loop.

### A project has many sessions

Durable memory is the project's and every session in it reads it. What a session holds on its
own dies with it, unless something on the ladder carries it across.

```
$ balthasar sessions
/home/you/work/thing
the project. every session below shares its memory.

0831-yt8z  get the test suite passing
     2 hours ago · magi · 2 kept
```

Every answer says which project and which run — by name, never by a twenty-six character id.

```sh
make build        # the binary — static, musl, ~6.9 MB, no runtime dependencies
make test         # the suite
make gates        # the architectural gates
make verify       # all of it
```

A transformer is available and off by default: `cargo build --features dense` links a local
`bge-small-en-v1.5` for retrieval, costing 12.5 MB of binary and 127 MB of weights on disk. It
makes *finding* better and is deliberately not allowed near *judging* — measured on real weights,
a rewording scores 0.813 and a claim beside its own replacement scores 0.801, twelve thousandths
apart, so no threshold on that signal can tell them apart. Deciding two claims are one claim uses
content words instead, and always will.

The gates are not advisory:

| Gate | Rule |
|---|---|
| `gate-file-size` | no `.rs` over 800 lines |
| `gate-no-delete` | no `DELETE FROM` outside `purge.rs` |
| `gate-independent` | no Rust file names a harness |
| `gate-witnessed` | every asserted memory answers for itself |
| `gate-untrusted` | untrusted content cannot become durable instruction |
| `gate-no-exec` | balthasar describes procedures and never runs them |
| `gate-no-llm` | the suite passes with no key, no network, no embeddings |

The last one is the load-bearing one. A model makes balthasar better; its absence never makes balthasar
fail, and the only way that stays true is to prove it on every run rather than remember it.

## Requirements

`.make.lua` and `.env.lua` are read by [oslo](https://github.com/termworks/oslo), which provides
both the `make` task runner and the directory environment. Without it, `cargo` does everything
`make` does.
