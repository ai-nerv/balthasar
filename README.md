<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="misc/balthasar-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="misc/balthasar.svg">
    <img src="misc/balthasar.svg" alt="balthasar" width="180">
  </picture>
</p>

<p align="center"><em>Memory for agents. Short-term and long-term, in one layer, driven over Lua.</em></p>

<p align="center">
  <a href="https://claude.ai/code/artifact/cf3ff7f0-1c1d-472a-b01e-d08a854178b1"><strong>How the four fit together</strong></a> —
  a turn end to end, writing a tool, memory both directions, what may run
</p>

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
    <run>/transcript.db                <run>/<agent>/memory.db
    that run's turns, verbatim         this agent's scratch
    the only copy of what was said     dies with the run
              │                               │
       ┌──────┴───────┐                       │  the ladder — eight kinds of
       ▼              ▼                       │  evidence, two floors
   turn_fts     layout · scroll               ▼
   searchable   stub · summarise       project.db      global.db
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
| `gate-cycles` | no two top-level modules depend on each other |
| `gate-hermetic` | the suite leaves nothing behind in `$TMPDIR` |
| `gate-wire` | one way of saying a thing crosses a boundary |
| `gate-no-llm` | the suite passes with no key, no network, no embeddings |

The last one is the load-bearing one. A model makes balthasar better; its absence never makes balthasar
fail, and the only way that stays true is to prove it on every run rather than remember it.

## Talking to it

Four-byte big-endian length, then a JSON body — the same shape the rest of the family speaks.
Every reply carries `family`, which says which revision of the wire it is written in: a reader
refuses a number it does not know and tolerates one it predates.

```
->  {"call":"recall","args":["deploy",{"limit":5}]}
<-  {"ok":true,"family":1,"n":1,"result":[[{"id":"…","text":"…","asserted":true,…}]]}
```

`asserted` is the distinction the whole design turns on, handed over rather than left for a
caller to work out from a number and a threshold it would have to be told. Above the floor a
memory is current truth; below it, it is still there, still searchable, still explained by `why`,
and no longer stated as fact — which is what lets a harness say "you told me this in March, it
may be stale" instead of repeating it flatly.

### The context, laid out

balthasar decides what each request holds; the harness only renders it. The harness `observe`s
every item as it is committed (with `tokens`, `group`, and a tool's `stub`, `handle`, `keep`,
`error`), then asks `layout` before every request:

```
->  {"call":"layout","args":["<session>",{"round":0,"window":200000,"reply":32000,
       "fixed":{"system":4100,"tools":6900},"live":[0,1,2,3],"query":"…","idle_s":12,
       "helpers":["memory"]}]}
<-  {"id":"L-1","budget":{…,"room":157000,"factor":1.0},
     "slots":[{"kind":"item","cursor":0},…,{"kind":"memory","text":"…","ids":["…"]},
              {"kind":"item","cursor":3}],"jobs":[],"fits":true,"why":"…"}
```

Slots come in the order the prompt cache wants — pinned notes, the conversation (word for word,
stubbed, or a stored summary where its span was), then the warning note and memory just before
the latest prompt. Nothing is recorded by answering: `applied {id, usage}` marks what was sent
and corrects the model's token estimate from the provider's count, and `overflowed {id, said}`
answers a tighter layout. The rules are `balthasar.window` in `init.lua`.

balthasar never calls a model. When a summary is due, or memory could be curated, or notes
extracted from a finished turn, the layout (or `jobs` between turns) hands the harness a job —
an instruction, an input, a schema — and `job_done` brings the answer back. Notes are pinned or
deferred, every change is in `changes` and can be `undo`ne, and with `balthasar.memory.review`
on they wait for `approve`.

Each extracted rule is checked against typed original transcript rows, not the labels printed
in the helper's input. Helpers can provide `evidence: [{cursor, quote}]`; the store validates
those references against the extraction's immutable source fingerprints. A new pinned rule
must match a complete unquoted user directive, preserving negations, conditions, and names.
Without explicit references, a rule is accepted only if this same literal match can be found.
Quoted examples, code, other agents, helper summaries, and truncated source lines grant no rule
authority. This deliberately rejects paraphrased rules rather than guessing at their meaning.
An example/header ending with a colon, a block quote, a prose quotation mark, or tag-like
markup makes the rest of that message ineligible for automatic rule adoption; blank lines
do not end that restriction. Put an intended standing rule in a separate plain message, or
use a same-line form such as `A firm rule: Always use uv.`. Markdown code fences have explicit
closing boundaries, so a standalone directive after a closed fence can qualify. This is a
conservative syntax policy, not an inference of intent from arbitrary prose.
Literal rules retain their paths; project-root normalization applies only to unpinned
observations. Explicit quotes keep their original spelling, including Markdown list markers.

Changing a pinned rule requires a user line naming it, such as `Package manager: Always use uv.`
Unpinning and retirement use `Unpin note N-1.` and `Retire note N-1.`. A tidy may retire an
identical pinned duplicate with `duplicate_of: "N-retained"`, but cannot change the retained
copy in the same batch or invent a replacement rule. Helper titles and descriptions are not
included in pinned rule bodies. Unpinned observations remain lower-trust helper distillations,
not verified user instructions or independently verified facts.

Layouts expose verified rules in a `rules` slot and the deferred-note index in an
`observations` slot. Both use the existing `pinned` project-note quota; rules get space first.
Observation fields are JSON-encoded, so embedded newlines and role-looking labels remain
inside their fields. A pinned note without current internal user-rule evidence is retained
as an unverified observation and remains available through `note_open`, not promoted to a rule.
Every layout rebuilds this projection from current sources and the current quota. Unchanged
notes produce identical bytes; legacy cached mixed bodies are never reused as authority.
The retained prompt snapshot has version 2 and is for inspection, not authorization.

Magi renders `rules` on the user-instruction channel and `observations` as assistant/helper
context. Summaries, recalled memory and old mixed `pinned` slots are also lower-trust assistant
context; legacy journal compaction uses the same boundary even without a memory service. Older consumers
that do not know the new slot kinds may skip them; the exact current family is the tested
combination. This separation prevents note metadata from becoming a user instruction; it is
not a claim that any model is immune to prompt injection or that helper observations are facts.

`note_open` includes the accepted evidence; `changes` includes evidence and rejection reasons.
Review checks source fingerprints, literal rule support and the target again before applying a
staged change. Saved evidence is reloaded from original rows, with role/kind/tool eligibility
recomputed rather than taken from old labels. Projection also rechecks the note's actual text
against its standing-rule or named-change source; an unchanged fingerprint alone does not
grant authority. Historical unsupported rules remain inspectable lower-trust observations,
and historical unsupported staged changes are rejected without applying them. Undo
restores the previous note and evidence. Each proposal batch, review decision batch, or undo
checks and changes notes and their audit records inside one immediate transcript transaction.
An error rolls back the whole batch; readers cannot see a note without its matching audit.
Invalid explicit references are audited even on otherwise unchanged or duplicate proposals.
Existing databases gain nullable/empty evidence
without losing notes or history; legacy notes with no evidence are not newly verified by this
migration.

Helper completion commits its transcript effects, note audit, counters, follow-up jobs, and
completion result together. The issued job is checked again under the writer lock; concurrent
or repeated completions cannot apply its effects twice. Curation and configuration hooks run
before the transaction, and curation can update only the matching current prompt.

Applying a layout commits its row states, calibration, confirmation marker, and pending
distillation intents together. Each intent captures the source at confirmation and its
transcript/run/agent/project identities. Cross-store replay attaches a stable witness to the
matching memory, completes rescoring, then acknowledges the intent. Before attaching, it
durably binds the first selected memory ID or records that no target exists. Promotion keeps
that target; purge does not redirect the effect to replacement memory with identical text.
Competing resolutions return the same recorded target. Repeating `applied`
or requesting the next layout resumes pending work without duplicating witnesses or calibration.
Completed intents discard their source text; purging a run removes its intents too. This is
replay across independent databases, not a transaction spanning both files. Legacy applied
layouts without recorded intents are retained, but earlier partial effects cannot be recovered
automatically. For older unbound intents, an existing witness identifies the original target;
without one, their first new resolution cannot reconstruct previously erased target history.

Extraction queues whole original transcript rows, never truncated row prefixes or helper stubs.
`memory.extract_bytes` bounds the input string in UTF-8 bytes (default 100,000, clamped to
4,096–1,000,000). Instructions and the output allowance are separate. Each job records only
its represented cursor range. Queueing does not acknowledge progress: accepted completion
commits note effects, audit, job result and acknowledgment together. Acknowledgments advance
only through contiguous ranges; malformed answers, rejected operations and changed source
snapshots leave coverage pending. Supported operations in a partly rejected batch remain
audited and idempotent on retry. A valid empty result acknowledges its represented range.

Automatic failure recovery retries a job once, then leaves it visibly failed. A row too large
for the input budget stays blocked rather than skipped. `jobs(session, {inspect=true})` returns
one status object with `extraction` progress and job states/results without issuing work.
`extraction.blocked` includes the oversized cursor, required bytes and current limit. After
adjusting the limit, or fixing a failed helper, `jobs(session, {retry_extraction=true})` explicitly
queues a fresh source snapshot and returns runnable work. Increasing the limit alone does not
retry a blocked row. Rows above the maximum remain visibly unprocessed; there is no arbitrary
row splitting. Old counters remain an uncertain `legacy_through` baseline in this status:
historical omissions are not silently declared recovered. Run purge removes this progress
and its acknowledgment records, including child transcripts.

The `client` verb hands over the Lua library that speaks all this, as source. A consumer keeping
its own copy is a consumer whose copy goes stale — and one did, silently, for a whole machine.

```lua
local balthasar = load(source)(transport)
local mem = balthasar.connect()
for _, m in ipairs(mem.recall("build command")) do print(m.text, m.confidence) end
```

## How this family talks

Three transports, two shapes, one encoding — written out because it was written out nowhere, and
five wires had grown five ways to say the same thing.

| Transport | When | Framing |
|---|---|---|
| **argv** | a question with an answer and nothing to hold open | one JSON object on stdout |
| **pipe** | a parent and the child it started | newline-delimited JSON, both directions |
| **socket** | anything may knock | four bytes of big-endian length, then JSON |

JSON is on all three. It is the *encoding*, not a transport.

A **call** is answered; an **event** is not:

```
->  {"call":"status","args":[]}
<-  {"ok":true,"family":1,"n":1,"result":[{"busy":false}]}

    {"event":"listening","at":"…"}
```

`result` is a **list** and `n` says how long it is: a sibling that unpacked a bare value would
read an answer as nothing at all. `family` says which revision the reply is written in — a reader
refuses a number it does not know and tolerates one it predates. A refused call is a *reply*, not
a dropped connection.

**The tag key is `event`, everywhere, in both directions**, and `gate-wire` refuses any other.
The failure it prevents is silent: two of these wires exist as byte-identical copies in two
repositories, so when two spellings drift nothing fails and no test goes red — the surface simply
stops being answered.

## Requirements

`.make.lua` and `.env.lua` are read by [oslo](https://github.com/termworks/oslo), which provides
both the `make` task runner and the directory environment. Without it, `cargo` does everything
`make` does.
