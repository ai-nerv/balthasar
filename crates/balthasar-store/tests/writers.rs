//! Several openers writing one scrollback at once: every turn lands, its search entry with it.

use balthasar_model::SessionId;
use balthasar_model::scratch::Scratch;
use balthasar_store::{Transcript, Turn};

fn turn(cursor: u64, text: &str) -> Turn {
    Turn {
        cursor,
        at: 1_756_000_000,
        role: "user".into(),
        kind: "prose".into(),
        text: text.to_owned(),
        raw: Some(format!("{{\"cursor\":{cursor}}}")),
        ..Turn::default()
    }
}

#[test]
fn writers_on_one_scrollback_each_land_whole() {
    let dir = Scratch::new("balthasar-writers", "many");
    let path = dir.join("transcript.db");
    drop(Transcript::open(&path).expect("created"));
    let session = SessionId::new("shared");

    let writers: Vec<_> = (0..6_u64)
        .map(|writer| {
            let (path, session) = (path.clone(), session.clone());
            std::thread::spawn(move || {
                let mut held = Transcript::open(&path).expect("open");
                for n in 0..25 {
                    let cursor = writer * 100 + n;
                    held.insert(&session, &turn(cursor, &format!("said {cursor}")))
                        .expect("insert");
                    held.write(&session, &turn(cursor, &format!("revised {cursor}")))
                        .expect("amend");
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("a writer finished");
    }

    let held = Transcript::open(&path).expect("open");
    assert_eq!(held.count(&session).expect("count"), 150);
    let found = held.spans_matching("revised", 1_000).expect("search");
    assert_eq!(
        found.len(),
        150,
        "every turn's search entry is its latest text"
    );
    assert!(
        held.spans_matching("said", 1_000)
            .expect("search")
            .is_empty()
    );
    assert_eq!(held.revisions_of(&session, 105).expect("kept").len(), 1);
    assert_eq!(held.reindex().expect("reindex"), 150);
}
