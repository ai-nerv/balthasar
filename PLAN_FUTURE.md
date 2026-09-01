| **F9** | built | `aeon eval --full` reports correctness, agent outcomes, efficiency and safety side by side; the experiment manifest with guardrails, where safety is untradeable and a win bought with context is not adopted || **F8** | built | skill descriptors whose parameters are data rather than templates, verification named or labelled absent, applicability shown rather than silently withheld, and `gate-no-exec` holding the execution boundary |
| **F9** | not built | see §4 || **F7** | built, without a model | the boundary a learned policy would sit behind: four stages with no automatic promotion, proposals that cannot express a forbidden thing, presentation clamped so a model may only weaken it, a deterministic fallback, and `aeon dataset` — features and outcomes, no content. No model is trained or required. |
| **F8–F9** | not built | see §4 for the order || **F6** | built | eight named policies chosen by query shape, degrading rather than swapping when vectors are absent; `--lexical-only` as the permanent control; every decision in `--explain`; bounded shadow comparison |
| **F7–F9** | not built | see §4 for the order || **F5** | built | write channels and trust domains; confidence counts sources, not repetitions; the imperative downgrade; quarantine as a gate; ten deterministic attacks at a zero success rate; purge closure over every derived table, with adversarial recovery tests |
| **F6–F9** | not built | see §4 for the order || **F4** | built | tier-aware decay so a fact outlasts an afternoon; habit standing, environment fingerprints and narrow negative procedures; staleness from validity rather than disuse |
| **F5–F9** | not built | see §4 for the order || **F3** | built | `relation_view` with derived edges kept apart from asserted links; temporal, causal, entity and semantic derivations that need no model; query classification; bounded one-hop traversal; `aeon relate` |
| **F4–F9** | not built | see §4 for the order |# aeon — Future Experimental Plan

> A research and implementation roadmap for the work after the baseline in `PLAN.md`.
>
> `PLAN.md` remains the architectural contract. This document does not reopen its settled
> decisions. It describes experiments that extend the existing memory model while preserving
> the independent process, SQLite store, Lua configuration surface, witnessed assertions,
> extractive fallback, and explicit-purge rule.

---

## 0. Status and purpose

**F0 through F9 are built.** Every milestone in this document is in the tree, each with the
tests its own acceptance section names. What is *not* built is a trained model — F7 is the
boundary a learned policy would sit behind, deliberately built before any model exists.

| Milestone | State | What landed |
|---|---|---|
| **F0** | built | `aeon eval --json` emits the full baseline; two seeded runs agree on every logical field; schema, revision and config fingerprint travel with every number |
| **F1** | built | `recall_run` through `outcome`; the `used`, `outcome`, `trace` and `utility` verbs; `aeon trace` and `aeon utility`; the twelve refusals of §6.10 |
| **F2** | built | rule-based segmentation over ten signals, every boundary carrying its reason, and versioned `episode_segment` rows that rebuild from the transcript |
| **F3** | built | `relation_view`, kept apart from asserted links; temporal, causal, entity and semantic derivations that need no model; query classification; bounded one-hop traversal; `aeon relate` |
| **F4** | built | tier-aware decay so a fact outlasts an afternoon; habit standing, environment fingerprints and narrow negative procedures; staleness from validity rather than disuse |
| **F5** | built | write channels and trust domains; confidence counts sources, not repetitions; the imperative downgrade; quarantine as a gate; ten deterministic attacks at a zero success rate; purge closure over every derived table |
| **F6** | built | eight policies chosen by query shape, degrading rather than swapping without vectors; `--lexical-only` as the permanent control; every decision in `--explain` |
| **F7** | boundary built, no model | four stages with no automatic promotion; proposals that cannot express a forbidden thing; presentation clamped so a model may only weaken it; a deterministic fallback; `aeon dataset` carrying features and outcomes and no content |
| **F8** | built | skill descriptors whose parameters are data rather than templates; verification named or labelled absent; applicability shown rather than silently withheld; `gate-no-exec` |
| **F9** | built | `aeon eval --full` reports correctness, outcomes, efficiency and safety side by side; the experiment manifest, where safety is untradeable and a win bought with context is not adopted |

Aeon already has the difficult foundation:

- a separate process and store rather than a harness library;
- fixed Scratch, Episode, Fact, Habit, and Archive tiers;
- four clocks for observation, occurrence, and validity;
- witness-derived confidence;
- separate assertion and retrieval floors;
- strength, decay, reinforcement, contradiction, and archive behavior;
- lexical retrieval with optional embeddings;
- entity-aware, explainable ranking;
- deterministic distillation and optional model distillers;
- Lua configuration, IPC, one-shot operation, and a coding-session benchmark;
- durable transcript/scrollback work in progress in the current checkout.

The next useful question is not whether Aeon can remember. It is whether Aeon can determine:

1. which experiences form meaningful events;
2. which relationships make an old memory useful now;
3. whether using a memory helped or harmed the agent;
4. whether a retrieved memory is safe to inject;
5. which lifecycle policy fits each kind of memory;
6. whether a learned policy can outperform rules without becoming trusted infrastructure.

This plan turns those questions into bounded, falsifiable experiments.

## 1. The future rule

Every future feature must preserve four independent judgments:

| Axis | Question | Existing foundation | Future work |
|---|---|---|---|
| **Truth** | Is the content credible? | witnesses, confidence, contradiction, validity | trust-domain diversity and stronger evidence classification |
| **Relevance** | Does it answer this query? | lexical, semantic, entity, frecency, scope scoring | temporal and causal views, query-policy routing |
| **Utility** | Did using it improve the outcome? | access counts and habit success counters | recall-to-action-to-outcome attribution |
| **Trust** | Is it safe to place in an agent context? | provenance, privacy, peer ceilings | taint, write-channel classification, injection modes, poisoning tests |

These values must never collapse into one `quality` score.

- A fact may be true and irrelevant.
- An episode may be relevant and unsafe to follow as instruction.
- A habit may have worked before and fail in the current environment.
- A low-confidence hypothesis may still be useful as a lead.
- A frequently recalled memory may be harmful rather than valuable.

The result should be explainable as four arguments, not one number.

## 2. Commitments that remain closed

All experiments in this document inherit the commitments in `PLAN.md`:

1. **No harness dependency.** New observations and outcomes enter through Aeon's transcript or
   protocol vocabulary. No harness crate enters the workspace graph.
2. **No required model.** Rules provide the baseline. Model-produced boundaries, relations, or
   policies are optional proposals.
3. **No required embeddings.** Every new retrieval path has a lexical or structural floor.
4. **No implicit deletion.** Decay, quarantine, revocation, supersession, and archive keep
   history. Only explicit purge removes it.
5. **No unwitnessed assertion.** Derived relations and learned policies cannot make a memory
   injectable as truth without the existing gate.
6. **No second configuration language.** Settings and behavioral hooks remain Lua.
7. **No distributed system.** One machine and one user remain the scope.
8. **No graph database.** Relationships remain SQLite rows over memory IDs.
9. **No vector database.** FTS5, structural candidates, and bounded cosine remain sufficient
   until measurements prove otherwise.
10. **No arbitrary execution.** Procedural memory describes a tool action. The harness retains
    authority to execute it.

The five tiers also remain fixed. New ideas must fit the existing taxonomy:

- raw experimental observations live in Scratch;
- bounded events live in Episode;
- claims live in Fact;
- positive and negative procedures live in Habit;
- inactive records live in Archive.

## 3. Research thesis

The project should pursue one distinctive claim:

> **Aeon is a witnessed, outcome-aware, adversarially testable memory substrate for local
> agents.**

The complete evidence path is:

```text
source
  └─ observation
      └─ witness
          └─ memory
              └─ recall decision
                  └─ context injection
                      └─ agent action
                          └─ observed outcome
                              └─ utility evidence
```

Existing memory systems generally optimize storage, summary quality, or retrieval. Aeon should
make the entire path inspectable.

That means `aeon why` eventually answers two different questions:

```text
Why do you believe this memory?
Why do you believe using it here is helpful and safe?
```

The first is supported by witnesses. The second requires the future work below.

## 4. Delivery order

The order is deliberate. Learned policies come last because they require trustworthy traces.
Security comes before procedural reuse because procedures are the most dangerous memories to
imitate.

| Milestone | Deliverable | Dependency |
|---|---|---|
| **F0** | Stabilize the current baseline and freeze measurements | current transcript work |
| **F1** | Recall, injection, action, and outcome ledger | F0 |
| **F2** | Event-aware episode formation | F1 |
| **F3** | Temporal, causal, entity, and semantic relationship views | F1, F2 |
| **F4** | Utility-aware habits and specialized lifecycle policies | F1, F2 |
| **F5** | Trust domains, quarantine, and poisoning laboratory | F1, F3, F4 |
| **F6** | Query-policy routing and shadow experiments | F1–F5 |
| **F7** | Offline learned policy in advisory mode | F6 |
| **F8** | Safe procedural skill descriptors | F4, F5 |
| **F9** | Reproducible external benchmark harness and research reports | continuous |

Do not parallelize milestones by adding incomplete schemas for future phases. Each migration
ships only when the behavior that uses it and the tests that defend it ship together.

---

## 5. F0 — Stabilize and measure the baseline

### 5.1 Goal

Produce a stable point from which experimental changes can be compared.

The current checkout contains in-progress transcript, replay, resume, and host/client work. That
work must be completed on its own terms before the future schema depends on transcript cursors.
The future plan must not absorb or redesign it mid-implementation.

### 5.2 Required baseline

- Transcript writes have a defined acknowledgement and durability contract.
- Replay and resume preserve session identity and cursor ordering.
- A memory witness can resolve its transcript cursor when the transcript still exists.
- Existing CLI, IPC, Lua, and one-shot paths agree on reply shapes.
- The extractive-only path works with no key, no network, and no embeddings.
- Current coding benchmark output is recorded as the F0 baseline.
- Retrieval latency and injected-token counts are measured, not inferred.
- Store schema version and binary version are printed in evaluation artifacts.

### 5.3 Baseline report

`aeon eval` should emit machine-readable JSON in addition to the human report. At minimum:

```text
run_id
git_revision
schema_version
config_fingerprint
extractor_mode
embedder_mode
scenario
seed
task_success
avoidable_failures
recall_precision
recall_relevance
assertion_accuracy
injected_tokens
store_bytes
recall_p50_ms
recall_p95_ms
```

The report belongs in test output or an explicitly requested artifact directory. Evaluation
must not silently write benchmark results into the project tree.

### 5.4 Likely modules

- `crates/aeon-store/src/transcript.rs`
- `crates/aeon-host/src/dispatch.rs`
- `crates/aeon-host/src/verbs.rs`
- `crates/aeon-cli/src/replay.rs`
- `crates/aeon-cli/src/eval.rs`
- `crates/aeon-testkit/src/run.rs`
- `crates/aeon-testkit/src/scenario.rs`

### 5.5 Acceptance

- `oslo make verify` passes from the repository root.
- `oslo make gate-no-llm` exercises the relevant transcript and evaluation path.
- Two identical seeded evaluations produce identical logical results.
- The baseline report contains no secrets or raw private transcript text.
- No F1 work begins until this point is tagged or otherwise recorded.

---

## 6. F1 — The use and outcome ledger

### 6.1 Goal

Record whether a memory was merely retrieved, actually injected, followed, ignored, corrected,
or associated with a successful outcome.

Access count is not utility. Recalling a poisoned memory ten times should not strengthen it ten
times merely because it was retrieved ten times.

### 6.2 Vocabulary

Use distinct IDs for each stage:

```text
RecallId     one search request
CandidateId one memory considered by that search
InjectionId one assembled context delivered to a caller
ActionId    one caller-reported action or decision
OutcomeId   one observation that evaluates an action or task
```

An ordinary recall may have no injection. An injection may be associated with several actions.
An action may never receive a known outcome. Unknown must remain a real state rather than being
treated as failure.

### 6.3 Store records

The exact SQL belongs in the implementation, but the logical schema is:

```text
recall_run
  id, scope, session, query_hash, query_class, requested_at,
  config_fingerprint, vector_available, result_limit, latency_us

recall_candidate
  recall_id, memory_id, rank, selected, score,
  semantic, lexical, entity, temporal, causal,
  frecency, confidence, strength, scope_signal, trust_signal

injection
  id, recall_id, session, created_at, token_count, remote,
  section_manifest, policy_name

injection_memory
  injection_id, memory_id, position, presentation_mode

action_use
  id, injection_id, session, reported_at, tool, action_hash,
  referenced_memory_ids, attribution_kind

outcome
  id, action_id, observed_at, kind, score, evidence_cursor,
  evaluator, note
```

The ledger must not duplicate full queries, prompts, tool arguments, or results by default.
Hashes, bounded labels, and transcript cursors are sufficient for normal operation. An explicit
debug setting may retain more, subject to privacy and redaction.

### 6.4 Outcome kinds

Start deterministic and small:

- `succeeded`
- `failed`
- `corrected`
- `reverted`
- `abstained`
- `ignored`
- `unknown`

Model-generated quality scores may be stored as evaluator observations, never as the only
outcome.

### 6.5 Attribution

Outcome attribution is uncertain. Aeon must not claim that every injected memory caused every
later result.

Support three strengths:

1. **Explicit:** the caller reports which memory or procedure it followed.
2. **Structural:** a tool/action matches a Habit step or named entity.
3. **Proximal:** the memory was injected shortly before the action, with no stronger evidence.

Only explicit and structural attribution should modify Habit success statistics automatically.
Proximal attribution is experimental evidence for analysis.

### 6.6 Utility

Utility is derived from attributed outcomes:

```text
utility(memory, context) = posterior over helpful | neutral | harmful
```

Do not begin with one floating-point column on `memory`. Keep observations in the ledger and
derive summaries. Context matters:

- project or global scope;
- query class;
- tool;
- environment fingerprint;
- memory age;
- whether the memory was asserted, suggested, or shown as history.

The first production policy may use conservative counts:

```text
verified_helpful
verified_harmful
unknown
last_verified_at
```

More complex Bayesian or learned utility estimates come only after real data exists.

### 6.7 API surface

Candidate family verbs:

```text
recall        returns a RecallId and candidate explanations
inject        returns an InjectionId
used          reports explicit memory use by an action
outcome       closes or updates an action outcome
trace         explains the recall-to-outcome chain
```

If `recall` and `inject` remain one public operation, the internal IDs must still be separate.
The protocol must accept callers that never report actions or outcomes.

Lua should expose registration-style configuration:

```lua
aeon.outcome.capture = true
aeon.outcome.retention_days = 90

aeon.on.outcome(function(event)
  -- Return a classification or nil.
end)
```

No callback may be required for the deterministic default.

### 6.8 CLI surface

```text
aeon trace <recall-or-injection-id>
aeon utility <memory-id>
aeon outcomes --session <name>
aeon eval --with-utility
```

`aeon why <memory-id>` remains about truth evidence. It may link to utility traces but must not
mix the two explanations.

### 6.9 Likely modules

- new focused modules under `crates/aeon-store/src/`, split before 800 lines;
- `crates/aeon-store/src/schema.rs` for forward-only migrations;
- `crates/aeon-store/src/read.rs` and `score.rs` for candidate traces;
- `crates/aeon-recall/src/assemble.rs` for injection manifests;
- `crates/aeon-host/src/verbs.rs` for bounded reporting verbs;
- `crates/aeon-lua/lua/aeon.lua` for the shipped client;
- `crates/aeon-testkit` for deterministic outcome scenarios;
- `crates/aeon-cli/src/` for trace and utility rendering.

### 6.10 Tests

- A retrieved but unselected memory receives no use evidence.
- An injected but ignored memory receives `ignored`, not `failed`.
- A failed explicit use increments harmful evidence without reducing truth confidence.
- A successful action unrelated to an injected memory does not credit it.
- Unknown outcomes do not become failures after a timeout.
- Redacted content is absent from ledger rows and rendered traces.
- Replaying an outcome event is idempotent.
- A peer cannot forge a user evaluation or rewrite an existing outcome.

### 6.11 Acceptance

- Every injected memory can be traced to its recall explanation.
- At least one coding benchmark scenario distinguishes frequently recalled from genuinely
  helpful memory.
- Existing recall behavior is unchanged when outcome capture is disabled.
- Ledger overhead is measured and bounded.
- Confidence remains byte-for-byte unaffected by utility observations.

---

## 7. F2 — Event-aware episode formation

### 7.1 Goal

Replace fixed transcript slices as the only notion of an episode with explicit, reproducible
event boundaries.

An episode is a coherent change in work, not merely N turns or N tokens.

### 7.2 Boundary signals

The rule-based segmenter should recognize:

- session start and close;
- explicit goal or task change;
- repository or working-directory change;
- transition between investigation, edit, validation, and handoff;
- tool failure followed by a different successful approach;
- user correction;
- environment or configuration change;
- commit or release boundary;
- long idle interval;
- compaction boundary;
- explicit caller-provided phase marker.

Signals create candidate boundaries. A deterministic resolver combines adjacent weak signals,
enforces minimum and maximum spans, and records why each boundary exists.

### 7.3 Optional surprise signal

An optional distiller may estimate a prediction or semantic discontinuity score. It proposes a
boundary with:

```text
cursor_before
cursor_after
score
label
model/backend identity
```

The score never replaces hard boundaries, and disabling the model must leave valid episodes.

### 7.4 Derived records

Raw transcripts remain authoritative. Segments are derived and rebuildable:

```text
episode_segment
  id, session, start_cursor, end_cursor, started_at, ended_at,
  boundary_before, boundary_after, method, derivation_version
```

If the segmentation algorithm changes, Aeon can recompute segments while retaining the old
derived version long enough to compare them. Re-segmentation must not duplicate durable Facts
or Habits.

### 7.5 Episode content

Each Episode should make the following inspectable:

- goal;
- starting state;
- relevant entities;
- attempted actions;
- turning point;
- outcome;
- unresolved items;
- supporting cursor span;
- segmentation method.

The extractive fallback may use tool names, commands, paths, first/last substantive turns, and
outcome signals. A model may produce a better summary through the existing distiller boundary.

### 7.6 Evaluation

Compare four configurations:

1. fixed cursor or token windows;
2. deterministic event segmentation;
3. model-only proposed segmentation;
4. deterministic segmentation refined by a model.

Measure:

- boundary precision on a hand-labeled set;
- temporal QA accuracy;
- causal QA accuracy;
- recall precision;
- episode compression ratio;
- duplicate promotion rate;
- injected tokens;
- segmentation latency and model cost;
- stability when one new turn is appended.

The last metric prevents a segmenter that rewrites an entire session after every observation.

### 7.7 Likely modules

- new `crates/aeon-distil/src/segment.rs` and focused helpers;
- `crates/aeon-distil/src/distil.rs` for optional refinement;
- transcript cursor reads in `aeon-store`;
- `aeon-model` only if Episode metadata cannot remain a store projection;
- `aeon-testkit` for labeled session fixtures.

### 7.8 Acceptance

- Deterministic segmentation works without a model or embeddings.
- Every boundary has a rendered reason.
- Original transcript rows are unchanged.
- Appending an ordinary turn changes only the open segment.
- Event-aware episodes beat fixed windows on at least one temporal or coding benchmark without
  regressing the others beyond a declared tolerance.

---

## 8. F3 — Orthogonal relationship views

### 8.1 Goal

Add queryable temporal, causal, entity, and semantic relationships without changing the memory
store into a graph database.

### 8.2 Two classes of relationship

Keep asserted and derived edges distinguishable.

**Asserted edges** are part of memory semantics:

- supersedes;
- contradicts;
- supports;
- derived-from;
- about.

**Derived edges** are retrieval aids:

- before / after / overlaps;
- caused / resolved / failed-because;
- same-entity;
- similar-to;
- same-goal;
- same-environment.

Derived edges never increase truth confidence merely by existing.

### 8.3 Relationship record

```text
relation_view
  from_memory, to_memory, kind, weight,
  source, derivation_version, evidence_cursor,
  created_at, stale_at
```

`source` distinguishes rule, transcript structure, embedding similarity, optional distiller, and
manual assertion. Rebuilding one derivation version must not touch asserted links.

### 8.4 Deterministic derivations

- Temporal adjacency comes from Episode spans and four clocks.
- Entity edges come from the existing entity index.
- Same-goal edges come from normalized episode goals.
- Repair chains connect a failed action, successful replacement, and derived Habit.
- Supersession and contradiction project into temporal and causal views.
- Exact content and entity overlap provide the no-embedding semantic floor.

### 8.5 Optional derivations

- Embedding similarity proposes semantic edges.
- A distiller proposes causal labels and conditions.
- A learned policy later proposes traversal strategies.

All optional relations are bounded, versioned, and disposable. The memories they connect remain
the durable source.

### 8.6 Query classification

Start with transparent rules:

| Query shape | Preferred view |
|---|---|
| what happened before/after/when | temporal |
| why did this fail / what fixed it | causal |
| what do we know about this file/tool/project | entity |
| have we solved something like this | semantic |
| what is true now | fact slot and validity first |
| how should this be done | habits, conditions, outcome utility |

Classification changes candidate generation, not the meaning of final relevance scores.

### 8.7 Retrieval pipeline

```text
query
  ├─ lexical candidates
  ├─ optional vector candidates
  ├─ entity candidates
  └─ selected relation traversals
       ↓
bounded union and deduplication
       ↓
existing explainable scorer plus view-specific signals
       ↓
assertion, privacy, trust, and budget gates
```

Traversal must be bounded by depth, candidate count, and elapsed time. The first implementation
should use one hop. Multi-hop traversal must prove value separately.

### 8.8 Explainability

`aeon recall --explain` should show paths such as:

```text
matched entity `make`
  → episode 01... recorded failed `cargo test`
  → resolved-by episode 01... recorded successful `make test`
  → derived habit 01...
```

Never print a causal label without its derivation source.

### 8.9 Acceptance

- The feature uses SQLite tables and indexes only.
- `AEON_NO_EMBED=1` retains temporal, causal-rule, and entity traversal.
- Derived relations can be rebuilt without changing memories or witnesses.
- Relationship candidates remain bounded under a dense synthetic store.
- Per-query-type benchmark results are reported; an aggregate-only gain is insufficient.
- p95 recall latency stays within the declared budget.

---

## 9. F4 — Utility-aware habits and specialized lifecycle

### 9.1 Goal

Stop applying one lifecycle intuition to every memory type.

Strength remains the general decay mechanism, but promotion, reinforcement, assertion, and
archive decisions become tier-aware.

### 9.2 Facts

- Facts retain historical versions indefinitely unless explicitly purged.
- Lack of recall does not make a valid fact false.
- Supersession closes validity and assertion, not history.
- Contradiction pressure affects confidence.
- Staleness warnings use validity and observation age rather than access frequency alone.
- A fact's outcome utility affects whether and how it is presented, not whether it is true.

### 9.3 Episodes

- Ordinary episodes fade from default retrieval.
- High-cost, high-surprise, corrective, or causally central episodes retain more strength.
- Episode summaries may be regenerated from transcript spans.
- Episode utility is query-class specific.
- An episode may remain useful as evidence after it is no longer injected by default.

### 9.4 Habits

- Verified successful use increments `worked`.
- Verified failed use increments `tried` without `worked`.
- Applicability conditions are stored and shown.
- Environment changes may suspend a Habit without archiving it.
- A Habit learned from one success remains advisory until corroborated.
- Reflection alone never raises a Habit to authoritative presentation.

### 9.5 Negative procedural memory

Represent a negative procedure inside Habit rather than adding a tier:

```text
trigger: when condition C holds
steps: do not apply procedure P; use Q or inspect first
evidence: failed episode E and correction F
polarity: avoid
```

If a `polarity` field is unnecessary, encode the distinction in a focused Habit procedure type.
Do not store a vague failure as a global prohibition.

A negative procedure must name:

- the rejected action;
- the condition under which it failed;
- the observed failure;
- any verified replacement;
- supporting episodes and witnesses;
- its scope and environment fingerprint.

### 9.6 Environment fingerprint

Procedural memory is sensitive to context. Begin with cheap, explicit components:

- scope ID;
- operating system and architecture when supplied;
- repository revision or branch when useful;
- tool name and version when observed;
- relevant configuration hashes;
- caller-provided environment labels.

Do not capture the entire environment. Secrets and unstable noise would destroy both privacy
and matching.

### 9.7 Policy experiment

Run three lifecycle policies in evaluation:

1. current uniform strength behavior;
2. tier-specific deterministic behavior;
3. tier-specific behavior plus outcome utility.

Measure repeated-error rate, stale assertion rate, store growth, archive growth, helpful recall,
harmful recall, and recovery after an environment change.

### 9.8 Acceptance

- Truth confidence never changes solely because a Fact helped or hurt a task.
- Habit statistics change only from attributable outcomes.
- Negative Habits are narrow, evidenced, and scope-aware.
- No lifecycle path issues SQL `DELETE` outside explicit purge.
- `aeon why` and `aeon utility` show different evidence chains.

---

## 10. F5 — Trust domains and adversarial memory

### 10.1 Goal

Prevent a persistent store from turning untrusted text into durable instruction.

Recent work on MemoryGraft and InjecMEM shows that successful-looking experiences and topical
retrieval anchors can poison later agent behavior. Aeon's witness chain helps, but distinct
sessions are not enough when all sessions consume the same compromised source.

### 10.2 Write-channel classification

Extend provenance conceptually beyond `Through` without breaking its process-boundary meaning:

- direct user instruction;
- direct user correction;
- local deterministic tool observation;
- peer assertion;
- external document or web content;
- model inference;
- distiller summary;
- consolidation output;
- imported history;
- manual CLI write.

The process door and the information source are separate. A trusted local peer can still submit
untrusted web text.

### 10.3 Trust domains

Witness diversity should consider:

- distinct sessions;
- distinct originating sources;
- distinct tools or channels;
- independent verification domains.

Ten observations copied from one document are one source domain. Ten model summaries of those
observations are still one source domain.

Trust-domain metadata must be privacy-preserving. Store stable local identifiers or hashes, not
credentials or full remote URLs when unnecessary.

### 10.4 Presentation modes

Every injected memory should have one of these modes:

- **asserted:** current witnessed fact suitable for declarative context;
- **advisory:** a qualified habit or suggestion;
- **evidence:** historical or uncertain material for reasoning;
- **quarantined:** retrievable only through explicit inspection or security analysis.

Quarantined content is not deleted. It is excluded from ordinary injection.

### 10.5 Imperative-content defense

External or inferred memories containing instruction-like language need special treatment:

- imperative language does not create an Imperative witness;
- quoted commands remain data unless a verified procedure promotes them;
- external content cannot alter Aeon policy or presentation mode;
- topical repetition cannot manufacture trust diversity;
- a retrieved memory is delimited and labeled by source and mode;
- executable-looking content requires a procedural record and harness authorization.

The defense belongs at write, promotion, retrieval, and injection boundaries. Prompt wording
alone is not a sufficient security boundary.

### 10.6 Security scenarios

Add deterministic attacks to `aeon-testkit`:

1. One imported page repeats a false instruction across sessions.
2. A successful task embeds an unsafe command that later matches a benign query.
3. A memory uses high-recall topical terms to force retrieval.
4. A malicious observation claims to be a user imperative.
5. Several derived summaries pretend to be independent witnesses.
6. A superseded secret remains retrievable after explicit purge.
7. A quarantined memory is requested through ordinary recall.
8. A valid historical fact is maliciously framed as a current instruction.
9. A procedure succeeds in one environment and is unsafe in another.
10. A peer attempts to report a forged successful outcome.

### 10.7 Metrics

- attack success rate;
- poisoned-memory retrieval rate;
- poisoned-memory injection rate;
- clean recall precision and recall;
- false quarantine rate;
- assertion coverage;
- trust calibration;
- purge recovery rate under adversarial queries;
- performance and token overhead.

Security improvements must report clean-quality cost beside attack reduction.

### 10.8 Explicit forgetting

Privacy-driven forgetting uses the existing explicit purge exception. Support three scopes only
through an owner-authorized operation:

- state: one memory or transcript item;
- trajectory: a bounded session span and its derived records;
- environment: a named scope or imported source domain.

Before purge, show the closure of affected derived memories, embeddings, relations, ledger rows,
and transcript spans. After purge, adversarial evaluation must demonstrate that ordinary and
fallback retrieval cannot recover the content.

### 10.9 Gates

Add architecture gates only when the implementation exists and the gate can be precise:

- external content cannot mint `WitnessKind::Imperative`;
- quarantined memories cannot enter ordinary injection;
- asserted presentation requires the existing witness floor;
- purge covers derived indexes and embeddings;
- outcome reports obey peer ceilings.

### 10.10 Acceptance

- A single untrusted interaction cannot become an asserted instruction.
- Source-domain repetition does not count as independent corroboration.
- Security labels appear in recall explanations.
- Clean benchmark regression stays below a declared tolerance.
- Explicit purge passes adversarial recovery tests.

---

## 11. F6 — Query-policy routing and shadow experiments

### 11.1 Goal

Choose retrieval behavior according to the query while keeping every decision reproducible.

### 11.2 Policy input

- query terms and detected entities;
- requested section;
- scope and session;
- temporal or causal language;
- desired tier;
- token budget;
- remote/local presentation boundary;
- availability of embeddings and relationship views.

The policy must not inspect secrets that the eventual retrieval is not allowed to return.

### 11.3 Policy output

```text
policy_name
candidate_sources
relationship_views
per-source limits
weights
depth
assertion floor
presentation modes
token budget
```

Privacy, peer ceilings, and witnessed-assertion rules are hard constraints outside the policy.

### 11.4 Shadow mode

For every selected production policy, optional shadow policies may compute candidate lists but
must not change the delivered context.

Store only bounded comparison data:

- overlap;
- counterfactual ranks;
- expected token cost;
- latency;
- later outcome association.

This creates training and evaluation data without exposing users to experimental behavior.

### 11.5 Initial policies

- `balanced`: current behavior;
- `current-fact`: live slots and validity first;
- `temporal`: Episode spans and temporal relations;
- `repair`: causal chains and positive/negative Habits;
- `entity`: entity expansion first;
- `historical`: Archive permitted, evidence presentation only;
- `safe`: high trust floor, no advisory procedures;
- `lexical-only`: no vectors, the permanent floor and control group.

### 11.6 Acceptance

- Every policy decision appears in `--explain` output.
- Shadow policies cannot touch strength or access counts.
- A policy timeout falls back to `balanced` or `lexical-only`.
- Per-policy latency and outcome metrics are reported.
- No policy can bypass privacy, trust, or assertion gates.

---

## 12. F7 — Learned policy, advisory only

### 12.1 Goal

Test whether a learned policy improves memory management after deterministic traces exist.

The learned component is not Aeon's foundation. It is a replaceable experiment that consumes
exported traces and proposes bounded actions.

### 12.2 Learnable decisions

- store or ignore a candidate;
- proposed tier and body structure;
- proposed event boundary;
- proposed relationship type;
- retrieval policy and weights;
- consolidation candidate;
- proposed quarantine or advisory presentation;
- proposed Habit applicability condition.

### 12.3 Forbidden authority

A learned policy may not directly:

- delete or purge;
- create a user Imperative witness;
- assign confidence;
- mark content as asserted;
- reach global scope when the caller cannot;
- expose Secret memory remotely;
- execute a procedure;
- rewrite transcript history;
- overwrite an outcome.

### 12.4 Training export

Export a privacy-minimized dataset containing:

- normalized query features;
- candidate feature vectors;
- policy decisions;
- presentation modes;
- attributed outcomes;
- trust and witness summaries;
- benchmark labels;
- no raw Secret content;
- no credentials or unredacted tool payloads.

Export is explicit. Aeon does not start training jobs.

### 12.5 Evaluation stages

1. **Offline replay:** score historical candidate sets.
2. **Shadow:** run beside the deterministic policy.
3. **Advisory:** expose suggestions in reports.
4. **Guarded opt-in:** allow bounded retrieval choices.

No automatic move beyond a stage. Each stage needs a written comparison report.

### 12.6 Reward design

Avoid one exact-match reward. Use a vector of outcomes:

- task success;
- evidence precision;
- harmful-memory avoidance;
- temporal correctness;
- abstention correctness;
- token cost;
- latency;
- repeated-error reduction;
- clean security behavior.

Report each component. A learned policy that wins by injecting more context has not necessarily
improved memory.

### 12.7 Generalization tests

- train on one project, evaluate on another;
- train on conversational memory, evaluate on coding sessions;
- train without embeddings, evaluate with and without them;
- evaluate temporal, factual, repair, and preference queries separately;
- introduce changed tool versions and repository configuration;
- include poisoned-memory scenarios absent from training.

### 12.8 Acceptance

- The binary remains fully useful without the policy artifact.
- Missing or incompatible artifacts fall back cleanly.
- The policy wins on multiple query categories and at least two scenario families.
- Gains survive a held-out project and a no-embedding run.
- Security performance is no worse than the deterministic policy.
- Policy decisions remain inspectable and reproducible from stored features.

---

## 13. F8 — Safe procedural skill descriptors

### 13.1 Goal

Make Habits precise enough to reuse successful procedures without letting memory execute code.

### 13.2 Descriptor

A procedural Habit should be able to describe:

```text
name
intent
trigger
preconditions
ordered tool actions
parameter schema
expected observations
verification method
rollback or recovery guidance
known failure conditions
environment fingerprint
supporting episodes
verified helpful and harmful counts
trust and presentation mode
```

The descriptor names tool operations. It is not a shell script and not an opaque prompt.

### 13.3 Ownership boundary

Aeon:

- stores and retrieves the descriptor;
- explains its evidence and utility;
- reports applicability and known failures;
- never executes it.

The harness:

- maps descriptor operations to its tools;
- checks permissions;
- obtains approval where required;
- executes actions;
- reports outcomes back to Aeon.

### 13.4 Promotion

A procedure may be proposed after one verified repair but remains advisory. Stronger
presentation requires:

- repeated verified success;
- no unresolved harmful outcome;
- matching environment conditions;
- sufficient witness and trust diversity;
- a verification step.

### 13.5 Acceptance

- A descriptor cannot bypass the harness tool boundary.
- Parameters are data, not executable interpolation.
- Every procedure names a verification method or is labeled unverifiable.
- Environment mismatch lowers applicability visibly.
- A failed procedure can produce a narrow negative Habit without deleting the original.

---

## 14. F9 — Evaluation as a product surface

### 14.1 Goal

Make claims about memory behavior reproducible from the repository.

### 14.2 Benchmark families

#### Local deterministic suite

Extend the existing coding benchmark with:

- repeated repairs;
- conflicting project/global procedures;
- environment changes;
- user corrections;
- stale facts;
- unrelated but lexically similar memories;
- poisoned successful experiences;
- missing-premise questions;
- explicit abstention;
- explicit purge and recovery attempts.

This remains the required no-network gate.

#### External adapters

Add optional adapters or import tools for:

- LongMemEval;
- LongMemEval-V2;
- LoCoMo;
- MemoryAgentBench;
- MemBench;
- MemoryBench;
- Evo-Memory.

External datasets are not vendored unless their license and size make that appropriate. Their
absence must not break `oslo make verify`.

### 14.3 Metric groups

#### Correctness

- factual accuracy;
- temporal accuracy;
- contradiction and supersession accuracy;
- multi-session reasoning;
- abstention and false-premise awareness;
- evidence precision and recall.

#### Agent outcomes

- task success;
- avoidable first attempts;
- repeated-error rate;
- time or tool calls to resolution;
- verified helpful and harmful memory uses.

#### Efficiency

- ingest throughput;
- store bytes per session;
- index amplification;
- recall p50/p95/p99 latency;
- injected tokens;
- model calls and estimated model cost;
- no-embedding performance.

#### Safety

- poisoning retrieval rate;
- poisoning injection rate;
- attack success rate;
- false quarantine rate;
- trust calibration;
- purge recovery rate.

### 14.4 Experiment manifest

Every comparison records:

```text
experiment name
hypothesis
primary metric
guardrail metrics
baseline policy
experimental policy
scenario and dataset versions
seed
configuration fingerprint
schema version
binary revision
hardware description when latency is reported
stop condition
result
decision: adopt | revise | reject
```

### 14.5 Adoption rule

An experiment enters the default path only when:

1. its primary metric improves on the declared scenarios;
2. no architectural gate is weakened;
3. no-LLM and no-embedding behavior remains valid;
4. security guardrails do not regress beyond the declared tolerance;
5. latency and storage costs are reported;
6. the explanation surface remains complete;
7. removing the experiment remains possible.

Rejected experiments remain documented. They do not leave dead schema or dormant production
branches behind.

---

## 15. Cross-cutting schema rules

1. Migrations are forward-only once a schema is in use outside development.
2. Experimental derived data names its derivation version.
3. Source memories and transcript spans remain addressable after re-derivation.
4. Derived tables can be dropped and rebuilt without losing owner-authored state.
5. Raw private content is not duplicated into telemetry tables.
6. IDs are stable and sortable where the existing ULID convention applies.
7. Foreign-key behavior must not introduce implicit deletion.
8. Purge explicitly enumerates primary and derived closure.
9. Every new table has a retention, archive, or rebuild policy.
10. Every new index is justified by a measured query.

## 16. Cross-cutting API rules

1. New verbs work over daemon and one-shot execution.
2. The Lua client remains the shipped client contract.
3. Optional fields are used when older callers may omit experimental metadata.
4. Peers cannot report authority they do not possess.
5. Debug output does not change machine reply shapes.
6. Explanation IDs are returned with results rather than reconstructed from logs.
7. Timeouts degrade to the deterministic floor.
8. Every expensive experimental path is bounded by count, time, and token budgets.

## 17. Cross-cutting privacy rules

- Queries are hashed in normal telemetry unless explicit debugging is enabled.
- Secret memories never appear in remote outcome evaluators.
- Transcript excerpts are resolved on demand rather than copied into every trace.
- External benchmark exports require explicit commands.
- Trust-domain identifiers reveal no credentials.
- Model distillers receive only the permitted, already-redacted view.
- Purge covers embeddings because embeddings can leak source content.
- Experimental reports default to aggregate values and stable local IDs.

## 18. Cross-cutting explanation surface

The future CLI should converge on four complementary views:

```text
aeon why <memory>       truth: witnesses, contradictions, validity
aeon recall --explain  relevance: candidate sources, paths, scores
aeon utility <memory>  utility: attributed helpful/harmful outcomes
aeon trust <memory>    trust: source domains, taint, presentation mode
```

`aeon trace <id>` connects them for one recall or action. None replaces the others.

## 19. Suggested module boundaries

Do not create a crate for each experiment. Begin with focused modules inside the owning crate and
split only when a stable independent abstraction appears.

| Concern | Owning crate |
|---|---|
| truth, trust, utility vocabulary | `aeon-model` |
| ledger rows, derived relations, migrations | `aeon-store` |
| segmentation and candidate extraction | `aeon-distil` |
| query routing and final context assembly | `aeon-recall` |
| configuration and hooks | `aeon-lua` |
| verbs and peer ceilings | `aeon-host` |
| framing only | `aeon-ipc` |
| deterministic scenarios and metrics | `aeon-testkit` |
| human rendering and explicit exports | `aeon-cli` |

`aeon-ipc` should not learn memory policy. `aeon-model` should not learn SQLite. `aeon-store`
should not execute distillers. The current crate boundaries remain sound.

## 20. Rejected shortcuts

- Do not reward a memory merely for being recalled.
- Do not infer causality only from temporal adjacency.
- Do not treat repeated text as independent confirmation.
- Do not let a summary outrank its source by default.
- Do not rewrite original memories when a better schema is inferred.
- Do not promote reflections directly into Facts.
- Do not make successful tool exit status the only definition of a good outcome.
- Do not train a policy before outcome attribution exists.
- Do not evaluate only on conversational question answering.
- Do not accept a benchmark gain purchased by unlimited context injection.
- Do not put learned policy on the turn path without a deterministic timeout fallback.
- Do not add multimodal, distributed, or hosted surfaces before the core research questions are
  answered.

## 21. Research references

The roadmap is grounded in these primary arXiv sources:

### Architecture and lifecycle

- [Position: Episodic Memory is the Missing Piece for Long-Term LLM Agents](https://arxiv.org/abs/2502.06975)
- [Memory in the Age of AI Agents](https://arxiv.org/abs/2512.13564)
- [A Survey on the Memory Mechanism of LLM-based Agents](https://arxiv.org/abs/2404.13501)
- [CoALA](https://arxiv.org/abs/2309.02427)
- [MemGPT](https://arxiv.org/abs/2310.08560)
- [Generative Agents](https://arxiv.org/abs/2304.03442)
- [A-MEM](https://arxiv.org/abs/2502.12110)
- [MAGMA](https://arxiv.org/abs/2601.03236)
- [MemOS](https://arxiv.org/abs/2507.03724)
- [MIRIX](https://arxiv.org/abs/2507.07957)
- [Mem0](https://arxiv.org/abs/2504.19413)
- [MemoryBank](https://arxiv.org/abs/2305.10250)

### Formation, reflection, and learned policy

- [Nemori](https://arxiv.org/abs/2508.03341)
- [Intrinsic Memory Agents](https://arxiv.org/abs/2508.08997)
- [Agentic Memory](https://arxiv.org/abs/2601.01885)
- [Mem-alpha](https://arxiv.org/abs/2509.25911)
- [What Training Data Teaches RL Memory Agents](https://arxiv.org/abs/2605.23067)
- [ExpeL](https://arxiv.org/abs/2308.10144)
- [Reflexion](https://arxiv.org/abs/2303.11366)
- [Voyager](https://arxiv.org/abs/2305.16291)
- [How Memory Management Impacts LLM Agents](https://arxiv.org/abs/2505.16067)
- [HippoRAG](https://arxiv.org/abs/2405.14831)

### Evaluation

- [LongMemEval](https://arxiv.org/abs/2410.10813)
- [LongMemEval-V2](https://arxiv.org/abs/2605.12493)
- [LoCoMo](https://arxiv.org/abs/2402.17753)
- [MemoryAgentBench](https://arxiv.org/abs/2507.05257)
- [MemBench](https://arxiv.org/abs/2506.21605)
- [MemoryBench](https://arxiv.org/abs/2510.17281)
- [Evo-Memory](https://arxiv.org/abs/2511.20857)

### Security and forgetting

- [MemoryGraft](https://arxiv.org/abs/2512.16962)
- [InjecMEM](https://arxiv.org/abs/2608.23471)
- [From Untrusted Input to Trusted Memory](https://arxiv.org/abs/2606.04329)
- [Secure Forgetting](https://arxiv.org/abs/2604.00430)
- [FSFM](https://arxiv.org/abs/2604.20300)

The newest 2026 results are hypotheses to reproduce, not authority to redesign Aeon around.

## 22. Definition of success

This plan succeeds when Aeon can demonstrate, with reproducible local evidence, that:

1. meaningful event boundaries outperform arbitrary transcript windows;
2. temporal and causal retrieval improves the questions that require it;
3. memory utility is measured from outcomes rather than access frequency;
4. a harmful but frequently retrieved memory loses procedural authority without being erased;
5. untrusted repetition cannot manufacture trusted instruction;
6. explicit purge survives adversarial recovery attempts;
7. learned policy can be evaluated without becoming a runtime dependency;
8. procedural memory remains explainable and non-executable inside Aeon;
9. every improvement retains the no-model and no-embedding floor;
10. the coding agent repeats fewer mistakes across sessions without receiving an ever-growing
    prompt.

The end state is not a larger memory database. It is a memory system that can show what it
believes, why it retrieved it, whether it helped, and whether it was safe to use.
