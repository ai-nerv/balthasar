# Changelog

## [0.6.1] - 2026-10-14

### <!-- 0 -->⛰️  Features

- Rehearse and apply retention
- Prune quiet sessions once a day
- Declare retain_days, unset keeps all
- Prune every copy of a quiet session

### <!-- 3 -->📚 Documentation

- The model role's spend cap
- Retain_days in init.lua and the README

## [0.6.0] - 2026-10-14

### <!-- 0 -->⛰️  Features

- Require the key handed on stdin
- Handle:call frames family calls

### <!-- 1 -->🐛 Bug Fixes

- Bound Lua-to-JSON by nodes, bytes, cycles
- Refuse precompiled chunks
- Hold every VM under a memory ceiling
- Refuse __gc metatables
- Build every VM from an allowlist
- Key writes only; publish a minted key
- Page the raw of a row over the cap
- Keep serve.rs within its size gate
- Keep run.rs in its comment budget
- Uid from the host; check .tool owner
- A failed claim leaves no lock file
- Bad reply closes the line; wire corpus
- Create .tmp in the make prelude
- A moved project store takes its new scope
- Write the prompt's first page at once
- Create .tmp before naming it TMPDIR
- Cap FTS terms, cache mask stubs
- Ledger ids hash the real query
- Page and count scrollback reads in SQL
- Submodule and HOME scopes
- Supersede in one transaction
- Reindex the scrollback in one txn
- Prepare the scrollback only on change
- Migrate step by step in IMMEDIATE txns
- Prune applied layouts after 30 days
- Purge every session-keyed table
- Runtime dirs safe, 0700, owner-checked
- Given config only from NERV_GIVEN, no links
- Store dirs 0700, databases and logs 0600
- Read, idle and write deadlines; busy
- Deadline, capped reads, kill and reap
- A panicking verb answers failed
- Slice text on character boundaries
- Bind and sweep under flock, corpses only
- Pin peer identity at accept, strip deleted
- Scrollback first, insert-only, traces
- Insert-only turns, kept revisions, traces
- One 16 MiB frame cap, answered not dropped
- Client checks frame length against cap
- Project files are data, VMs run on fuel

### <!-- 2 -->🚜 Refactor

- Crowd tests in a file of their own
- Store failures are errors
- No panics on input
- No panics on input
- No panics on input
- No panics on input
- No panics on input
- No panics on input
- No panics on input

### <!-- 3 -->📚 Documentation

- Getmetatable in all three VMs
- Name the new sandbox refusals
- The memory key and socket families
- One contract in all four
- Install through Nix
- A peer may forget only scratch
- What is deleted; every gate named
- One text across the family
- Doc links that resolve
- Identity is attribution, not kernel
- Observe is insert-only, traces apart
- Name hexe and oslo sockets
- Frames, sockets, deadlines, callers

### <!-- 6 -->🧪 Testing

- One gate-family in all four
- Wire gate checks the cap and dir rule
- Probe a live serve for refusals
- No shared directory in client tests
- Bind in private dirs; drop only our lock
- Race migrations in WAL, as stores open
- No shared directory named in a model test
- Keep the harness name out of Rust

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Harden lua vms
- Publish tagged builds to ai-nerv
- Pin actions by commit

### Build

- Enable flakes for the dev shell
- Cargo deny and audit in verify and CI
- Read the version from Cargo.toml
- Nix-build, nix-check, download check
- A static package with its config
- Deny expect and indexing outside tests
- Overflow checks; audit and deny in verify

## [0.5.5] - 2026-09-29

## [0.5.4] - 2026-09-29

## [0.5.3] - 2026-09-29

## [0.5.2] - 2026-09-23

## [0.5.1] - 2026-09-23

## [0.5.0] - 2026-09-23

## [0.4.0] - 2026-09-23

### <!-- 0 -->⛰️  Features

- Budgeted layouts and prepared working state

## [0.3.0] - 2026-09-21

### <!-- 0 -->⛰️  Features

- A model may retitle, a person names
- A run can be put away, and listed
- Let a person settle a disagreement
- Link claims that cannot both be true
- A role for summaries, notes and curating
- Provenance, coverage and configs
- Read a prompt as it arrives
- Every verb and step in the family log
- Take window and memory from magi
- Notes extracted, reviewed, undone
- Notes and their change log
- Summarise and curate jobs
- Jobs handed to a helper model
- Layout, applied and overflowed verbs
- Keep layouts, factors, summaries
- Layout rules for a request
- Tool stubs, and plan records nothing
- Turns carry group, stub, handle, keep

### <!-- 1 -->🐛 Bug Fixes

- Extraction needs room to think
- A denial is not the claim it denies
- Gate each job on the role it asks for
- A summary carries the calls that failed
- A rule in plain words is a rule
- Believe the window a refusal names
- What was taken back is not recalled
- A rewrite does not lose the doubt
- A session's own rule is not shown back
- A quote may come with its row label
- Say the cursor is the number in brackets
- History carries what was said, not raw
- Runs opened in one second list newest first
- More verbs that open an instruction
- A note is never the request said back
- No rule from a span with none laid
- A prompt keeps its reads until last
- A request about rules lays none down
- A plain request keeps a rule its words
- A tidy leaves a pinned rule its words
- A rule reworded in the middle is an echo
- A replay comes a page at a time
- A plain request has no say over pinning
- An empty answer to a stated rule is asked again
- A summary is told today's date
- A plain request never unpins a rule
- A rule restated is not kept twice
- Pin only what a person laid down as a rule
- Old tool results never block a summary
- Even a long reply fits a summary
- Pin rules, not what the task asks
- A summary reads whole replies
- A second pinned rule gets tidied at once
- A model named before its run is kept
- Learn the factor only from sane counts
- A checkout is never taken for a store
- A store above a repository stays out
- A session keeps its own given config
- A blocking job is never retried
- An example that cannot be copied
- Take rules only from the person
- Log only the stubs a hook wrote
- Record stated rules pinned and tight
- Pin stated rules, drop run-bound detail

### <!-- 2 -->🚜 Refactor

- One gate for what an extraction keeps
- Tests in their own file

### <!-- 3 -->📚 Documentation

- Say why the sweep gets no reasoning
- Repair a mangled module comment
- Plan as the layout fallback
- Layout, jobs and notes verbs
- Layout, jobs and notes

### <!-- 6 -->🧪 Testing

- Pin what the revision rule costs
- A task detail is no note at all

## [0.2.0] - 2026-09-15

### <!-- 0 -->⛰️  Features

- Bind the role's name and the old one
- What this program is for, as a contract
- One reply shape, and both flags on every verb
- One Lua VM, and it is sandboxed
- Key scratch by agent, with lazy layout 2 migration
- Acknowledge packages and version the surface
- Implement the family contract
- Answer the family contract on the command line
- One-shot replies take --cbor and name the family
- Die with the caller that named itself

### <!-- 1 -->🐛 Bug Fixes

- Never move a checkout named balthasar
- Keep memory in a hidden .balthasar
- Bound the stores one daemon opens
- Observe and plan reach the wire
- Redial when a call gave up its handle
- A failed read drops the connection
- Honour a short TMPDIR, name a full disk
- --json means the family's shape, always
- A listing is the rows, not one row
- The corpse sweep leaves bind
- A part-moved run goes back whole
- Leaks and swallowed errors the gates missed
- A stop signal takes the socket with it
- A held connection no longer shuts out the next caller
- Fall back to this process's own name for a peer
- The gate skip list was not valid sh
- A stray .git must not scope everything under it

### <!-- 3 -->📚 Documentation

- Casper's socket door, and its jail
- How a tools program is configured
- Sessions is core to the memory role
- Both client shapes are correct
- The send half closes too
- Abandoning a call abandons it
- The tool door, and the export corpus
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Cut the argument from the comments
- Comments describe, they do not argue
- Casper's declarations are an installed file
- A spawn-per-call sibling is told on every spawn
- A lua tool declares its transport
- Link the wiring walkthrough

### <!-- 4 -->⚡ Performance

- A read costs what it returns

### <!-- 6 -->🧪 Testing

- A leak under either name is a leak
- A dropped reply cannot answer twice
- Recall is judged by shape not by clock
- Prove the store-home ceiling

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Ignore hash-named test store dirs
- Merge develop to main
- A run can be started by hand

### Build

- Comments describe, they do not argue

## [0.1.0] - 2026-09-06

### <!-- 1 -->🐛 Bug Fixes

- The dev shell evaluates on aarch64

