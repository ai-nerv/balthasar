use balthasar_model::{SessionId, scratch::Scratch};
use balthasar_store::{Change, Transcript};
use serde_json::json;

#[test]
fn legacy_notes_migrate_without_losing_content_or_history() {
    let dir = Scratch::new("note-evidence", "migration");
    let path = dir.join("transcript.db");
    {
        let old = rusqlite::Connection::open(&path).expect("old store");
        old.execute_batch("CREATE TABLE note (
            n INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, text TEXT NOT NULL,
            description TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, retired INTEGER,
            created INTEGER NOT NULL, updated INTEGER NOT NULL) STRICT;
            CREATE TABLE note_change (n INTEGER PRIMARY KEY AUTOINCREMENT, note TEXT,
            op TEXT NOT NULL, before TEXT, after TEXT, state TEXT NOT NULL,
            by TEXT NOT NULL, session TEXT, at INTEGER NOT NULL) STRICT;
            INSERT INTO note VALUES (7, 'Package manager', 'Use uv.', 'Packages', 1, NULL, 1, 2);
            INSERT INTO note_change VALUES (9, 'N-7', 'add', NULL, '{\"text\":\"Use uv.\"}', 'applied', 'old', 's', 2);")
            .expect("legacy schema and rows");
    }
    let transcript = Transcript::open(&path).expect("migrate");
    let note = transcript.note("N-7").expect("note").expect("preserved");
    assert_eq!(note.text, "Use uv.");
    assert!(note.pinned && note.evidence.is_null());
    let old = transcript
        .change("C-9")
        .expect("change")
        .expect("preserved");
    assert_eq!(old.after.expect("after")["text"], "Use uv.");
    assert!(old.evidence.is_null() && old.reason.is_none());
    let evidence = json!({"version":1,"sources":[{"cursor":0,"quote":"Use uv."}]});
    let id = transcript
        .record_change(
            &Change {
                op: "add".into(),
                state: "rejected".into(),
                by: "J-1".into(),
                evidence: evidence.clone(),
                reason: Some("unsupported source".into()),
                ..Change::default()
            },
            Some(&SessionId::new("s")),
        )
        .expect("new evidence");
    assert_eq!(id, "C-10");
    let mut fields = note.fields();
    fields["evidence"] = evidence.clone();
    transcript
        .put_note(Some("N-7"), Some(&fields), 3)
        .expect("note evidence");
    drop(transcript);
    let reopened = Transcript::open(&path).expect("idempotent migration");
    let change = reopened.change(&id).expect("change").expect("kept");
    assert_eq!(change.evidence, evidence);
    assert_eq!(change.reason.as_deref(), Some("unsupported source"));
    assert_eq!(
        reopened.note("N-7").expect("note").expect("kept").evidence,
        evidence
    );
}
