# Transcript writer reservations

The native socket supports revision 1 of a connection-owned writer extension.
This is an optional memory-role extension, not a new transcript format. Magi's
startup/resume integration reserves before replay and preserves the connection
through host handoff. Persistent ACP load acceptance remains pending.

## Calls

`writer_status(transcript, { revision: 1 })` returns one object:

```json
{"revision":1,"supported":true,"owned":false,"lifetime":"connection","scope":"store-transcript"}
```

`owned` describes this connection, not another client's reservation. `supported`
is false on a one-shot dispatcher or an ephemeral store. This read requires no
key. Invalid revisions, unknown option fields, and extra arguments are refused.

`writer(transcript, { revision: 1, operation: "reserve" })` requires the socket's
write key. It acquires an exclusive, nonblocking store/transcript reservation and
returns `{"revision":1,"owned":true}`. A busy transcript is refused. The key is
1–1024 bytes, hashed before use as a lock filename. No caller chooses a lock path.

One connection holds at most one writer. Reserving the same key again checks the
existing reservation without releasing it. A different key or store is refused;
the previous reservation remains held. Callers must reserve before reading history.

`writer(transcript, { revision: 1, operation: "release" })` requires the same key
and store and returns `{"revision":1,"owned":false}`. Invalid release attempts
do not drop the existing reservation. A one-shot API cannot acquire a reservation.

## Lifetime and fencing

The serialized dispatcher owns the guard, indexed by a private connection ID,
not PID or request data. Disconnect releases that connection's guard after its
current call settles. Dispatcher shutdown releases all guards. Process death
releases the kernel lock. Guard drop explicitly unlocks before closing its
descriptor, including when another descriptor references the same open file.
Lock files are never unlinked on release.

A retained writer prevents idle expiry; frame, partial-frame and write deadlines
still apply. A client that loses the connection has lost ownership. Reconnecting
does not restore it, and must not trigger an automatic replay of a failed write.

Calls to `observe`, `amend`, `model`, `layout`, `applied`, `overflowed`, `jobs` and
`job_done` use the connection's matching writer or a short-lived shared mutation
guard. Shared guards preserve existing concurrent idempotent acknowledgements but
exclude an exclusive reservation. An exclusive writer excludes legacy socket and
one-shot writes too. Another store or transcript is checked independently.
An excluded mutation answers with `fault: "failed"`: it was not recorded and a
harness must retain its pending entry, not discard it as an unsupported feature.

Owned `replay`, `resume` and `scroll` calls validate the held store/key identity.
Other readers may still inspect history without claiming a writer. The reservation
does not turn a reader into a controller or authorize tools.

Retention skips reserved transcripts. Maintenance holds one exclusive store
admission guard through commit and probes each transcript one at a time; it does
not retain a descriptor per transcript. New writer and shared mutation acquisition
hold shared admission only while acquiring their transcript guard. Existing
owners retain their transcript guards and are not displaced by maintenance.
Admission is nonblocking; contention fails explicitly. Admission lock files are
private, identity-checked and never unlinked, like transcript lock files.
Run purge refuses if any member transcript has a writer. Its transcript refusal is
checked before deleting project memories. These are cooperative filesystem locks,
not a defense against a malicious same-UID process modifying database files directly.

## Verification

`oslo make test-writer-reservation` runs store and IPC unit tests, host tests, and
native writer protocol tests. The native tests assert the running server's
`/proc/<pid>/exe` against `BALTHASAR_TEST_BINARY` when set, or Cargo's standalone
binary otherwise. They cover authentication, same-PID connection separation,
independent daemons sharing a store, disconnect, normal exit, crash, and one-shot
API exclusion. Full-family and Magi/ACP integration remain separate acceptance.
