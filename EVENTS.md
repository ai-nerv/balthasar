# Local operational events

The approved initial Phase11 policy is implemented in `balthasar_host::events`.
Dispatched `job_done` calls emit their observed result after native processing.
Layout decisions, transcript application and cross-store recovery also emit
separate outcomes. Other memory transitions are not yet wired.
The default path creates no disk spool, worker or network exporter.

## Explicit native forwarding

Bootstrap revision2 has a512byte cap and a strict immutable run/agent/turn/ask
binding. Run/agent must be paired; turn/ask require that root. Bound fields
override local candidates during encoding, without replacing source-owned job
IDs. The collector verifies bindings before deduplication/accounting. Event
envelopes remain revision1 with a4096byte cap. Capability replies advertise
bootstrap2; parent and descriptor admission checks are unchanged.
`lineage-native-fixture.log` verifies owner tests and fully bound selected native
interoperability. Normal long-lived servers bind run/agent only; request-scoped
turn/ask and helper ancestry remain separate required integration.

`serve --event-bootstrap` receives a bounded private parent packet containing an
event descriptor and original stdin. It validates the parent, owner, identity,
descriptor types, flags and buffers, then restores stdin before reading the native
key. Invalid packets fail before serving. The stop-signal mask and parent tie are
established before starting the joined forwarding worker, so SIGTERM remains on
the server's normal cleanup path. Invalid keys return through the forwarder guard.

Forwarding uses the existing sole bounded queue and one nonblocking send per event,
without retry, spool or another queue. Rate-limited status includes explicit loss;
missing final delivery remains unknown. Failure to activate after restoring stdin
does not prevent useful memory work. Ordinary standalone launches remain disabled.
Opted-in Magi negotiates normal serve activation using `event-capabilities`, a
fixed family-JSON reply emitted before configuration or store access.

The independent host implementation uses the workspace's existing rustix version;
only a host dependency edge and empty fs/stdio features are added. Unit coverage
includes isolated received-descriptor EOF checks in the existing permitted test
directory convention; the no-exec gate is unchanged. Magi's selected-native test
checks actual executable/input identity, key admission, observe/layout results,
event receipt, signal masking, cleanup and malformed-key startup failure.
`sibling-bootstrap-observed.log` passes those cases, owner event tests and Clippy.
Magi's normal-events-owned receipt additionally verifies normal startup/key input,
read-only collected status and actual reaping. Full containment, lineage, overhead
and family acceptance remain pending.

The strict revision1 envelope records `program: balthasar`, numeric package version,
OS-random128bit producer ID, nonzero sequence and nullable opaque correlation for
run/agent/turn/request/ask/attempt/job/tool-call. Missing measurements remain null.
Only typed facts are accepted; no memory text, paths, job answers, model labels,
error details, headers or arbitrary measurement maps exist in the schema.

## Owner facts

- `layout`: decided/failed describe decision preparation; applied/application-failed/
  ignored-repeated describe transcript confirmation; recovered/recovery-failed
  describe cross-store replay. Optional monotonic duration currently remains null.
  A decision or recovery is not a newly applied layout.
- `job_result`: accepted/retry-queued/failed/ignored-obsolete/ignored-repeated/
  rejected, with optional nonzero revision. Successful empty replies are not
  evidence of accepted work.
- `memory`: read/written/failed. Actual persistence boundaries still need source
  instrumentation; the enum itself does not establish durability.

The job-result observation follows the transcript transaction's return, including
commit. A queued retry is distinct from a completed or terminally failed job.
Rechecks inside the transaction distinguish a repeated completion from rejected
lease changes. Rejected completion does not imply that earlier, separate memory
operations were rolled back. Calls refused before dispatch are not observed here.

Layout application emits `applied` only after the transcript transaction commits;
the subsequent replay can fail without undoing that application. A repeated
confirmation emits `ignored_repeated`, but can still recover pending effects.
`recovered` means a nonempty loaded effect batch completed, not a count of new
memory writes or a claim of exclusive ownership. Empty replay emits nothing.
`recovery_failed` may follow partial cross-store progress; it never implies
rollback. Decision failures likewise do not imply zero preparatory writes.

Correlation uses opaque IDs, never raw session/job/layout identifiers. Root-only
bootstrap2 bindings enable request-context revision1 in successful verbs replies.
The outer header is limited to256encoded bytes and requires the normal key even
for otherwise unguarded reads. It travels separately through the dispatch queue;
the request body, model arguments, stored data and Lua input do not gain metadata.
Source scopes require the bootstrap parent's current PID/euid and matching
run/agent. Missing or rejected context clears turn/ask attribution without changing
otherwise permitted work. Layout and job completion capture context at their
source boundary; emission does not read dispatcher-local state. Job revision and
other uninstrumented transitions remain separate work.

The Magi selected-native fixture covers a reused socket across turns and missing
context, bad key/root/oversized headers, another same-UID process, and Lua attempts
to forge attribution. Useful layouts and graceful reaping remain required.

## Queue and loss

Emission is disabled until explicit local setup creates the process's sole
receiver; disabled calls do not evaluate payload preparation. Encoding/decoding
is capped at4096bytes and try-send capacity at256events. Queued payload is at most
1MiB, not a bound on total process or future consumer memory. No capacity wait,
automatic retry, recursive loss event or implicit export occurs.

Full/closed/encoding/exhaustion counters saturate. Closing the receiver reports
queued events not handed to the consumer as abandoned, including racing sends
after producers settle. Counter snapshots are not atomic. This best-effort
stream is not a transcript, job database, billing ledger or recovery authority.

`oslo make test-events` runs independent golden, hostile-input, ownership-state,
disabled, saturation, concurrency and receiver-shutdown contracts. A separate
integration process exercises real job completion, retries, stale replies, atomic
rechecks and transaction rollback. It checks persisted state and privacy canaries,
and proves job completion with a full queue or closed receiver. A second isolated
process verifies real layout decisions, confirmation rollback, partial recovery
across reopen, repeated acknowledgements and recovery during a new decision.
It checks durable state and completion with disabled, full or closed queues.
Other transition
accuracy, collection/export and overhead remain separate acceptance work.
