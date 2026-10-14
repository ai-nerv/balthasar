//! A project's store moved with its checkout answers for the new place, once.

use balthasar_model::scratch::Scratch;
use balthasar_model::{
    Body, Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{SCOPED, SCOPED_SCROLLBACK, Store, Transcript, mint};
use rusqlite::Connection;

const THEN: Timestamp = 1_710_000_000;
const NOW: Timestamp = 1_756_000_000;

fn fact(scope: &str, object: &str, at: Timestamp) -> Memory {
    Memory::new(
        mint(at),
        Tier::Fact,
        ScopeId::new(scope),
        Body::fact("project", "test_command", object),
        at,
    )
}

fn saw(scope: &str, at: Timestamp) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{scope}-{at}")),
        WitnessKind::Correction,
        SessionId::new("s"),
        ScopeId::new(scope),
        at,
    )
}

fn scoped(db: &Connection) -> Vec<String> {
    let mut tables = db
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE '%fts%'",
        )
        .expect("tables");
    let names: Vec<String> = tables
        .query_map([], |r| r.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("names");
    let mut found: Vec<String> = names
        .into_iter()
        .filter(|table| {
            db.query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = 'scope'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .expect("columns")
                > 0
        })
        .collect();
    found.sort();
    found
}

#[test]
fn the_tables_rewritten_are_every_table_that_names_a_scope() {
    let dir = Scratch::new("balthasar-placed", "tables");
    drop(Store::open(&dir.join("memory.db")).expect("store"));
    drop(Transcript::open(&dir.join("transcript.db")).expect("scrollback"));
    let mut memory: Vec<String> = SCOPED.iter().map(|t| (*t).to_owned()).collect();
    memory.sort();
    let mut scrollback: Vec<String> = SCOPED_SCROLLBACK.iter().map(|t| (*t).to_owned()).collect();
    scrollback.sort();
    assert_eq!(
        scoped(&Connection::open(dir.join("memory.db")).expect("open")),
        memory
    );
    assert_eq!(
        scoped(&Connection::open(dir.join("transcript.db")).expect("open")),
        scrollback
    );
}

#[test]
fn a_moved_store_holds_one_live_answer_per_slot_under_its_new_place() {
    let dir = Scratch::new("balthasar-placed", "moved");
    let path = dir.join("memory.db");
    let mut store = Store::open(&path).expect("store");
    store
        .remember(
            fact("/old/place", "cargo test", THEN),
            saw("/old/place", THEN),
            THEN,
        )
        .expect("before the move");
    store
        .remember(
            fact("/new/place", "make test", NOW),
            saw("/new/place", NOW),
            NOW,
        )
        .expect("after the move");

    assert_eq!(
        store
            .settle_scope(&ScopeId::new("/new/place"), NOW)
            .expect("settled"),
        1
    );

    let live = store
        .live_slot("/new/place", "project", "test_command")
        .expect("slot")
        .expect("one answer");
    assert_eq!(live.body.object(), Some("make test"));
    let raw = Connection::open(&path).expect("raw");
    for table in SCOPED {
        let left: i64 = raw
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE scope = '/old/place'"),
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(left, 0, "{table} still names the old place");
    }
    assert_eq!(
        store
            .settle_scope(&ScopeId::new("/new/place"), NOW)
            .expect("again"),
        0
    );
}

#[test]
fn an_unmoved_store_is_settled_without_a_write_lock() {
    let dir = Scratch::new("balthasar-placed", "unmoved");
    let path = dir.join("memory.db");
    let mut store = Store::open(&path).expect("store");
    store
        .remember(fact("/here", "make test", NOW), saw("/here", NOW), NOW)
        .expect("remembered");
    let writer = Connection::open(&path).expect("writer");
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold the lock");
    let started = std::time::Instant::now();
    assert_eq!(
        store
            .settle_scope(&ScopeId::new("/here"), NOW)
            .expect("settled"),
        0
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    writer.execute_batch("ROLLBACK").expect("release");
}
