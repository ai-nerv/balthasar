use super::*;

fn item(what: &str, stage: Stage, evidence: &[u64]) -> Item {
    Item {
        what: what.to_owned(),
        stage,
        evidence: evidence.to_vec(),
    }
}

fn check(ran: &str, passed: Option<bool>, said: &str) -> Check {
    Check {
        ran: ran.to_owned(),
        passed,
        said: said.to_owned(),
        evidence: vec![1],
    }
}

/// A failing check is not forgotten by leaving it out.
mod mem_failed_test_stays_failed {
    use super::*;

    fn failing() -> Working {
        Working {
            checks: vec![check("cargo test", Some(false), "2 failed")],
            ..Working::default()
        }
    }

    #[test]
    fn dropping_it_is_refused() {
        // Preparation that quietly loses a failure reads as a session where nothing is wrong.
        let next = Working::default();
        assert_eq!(
            next.may_replace(Some(&failing())),
            Err(Refused::FailureDropped {
                ran: "cargo test".to_owned()
            })
        );
    }

    #[test]
    fn running_it_again_may_resolve_it() {
        let next = Working {
            checks: vec![check("cargo test", Some(true), "ok")],
            ..Working::default()
        };
        assert_eq!(next.may_replace(Some(&failing())), Ok(()));
    }

    #[test]
    fn it_may_stay_failing() {
        assert_eq!(failing().may_replace(Some(&failing())), Ok(()));
    }

    #[test]
    fn not_run_is_never_passed() {
        // The plan's words: never turn "not tested" into "passed" during reflection. A pass has
        // to name the row that says so.
        let claimed = Working {
            checks: vec![Check {
                ran: "cargo test".to_owned(),
                passed: Some(true),
                said: "looks fine".to_owned(),
                evidence: Vec::new(),
            }],
            ..Working::default()
        };
        assert_eq!(
            claimed.may_replace(None),
            Err(Refused::Untested {
                ran: "cargo test".to_owned()
            })
        );
    }

    #[test]
    fn a_check_that_was_never_run_is_shown_as_not_run() {
        let held = Working {
            checks: vec![check("cargo test", None, "")],
            ..Working::default()
        };
        assert!(held.rendered().contains("Not run"), "{}", held.rendered());
        assert!(!held.rendered().contains("Still failing"));
    }
}

/// Work that is not finished stays on the list.
mod mem_unfinished_work_stays_open {
    use super::*;

    fn midway() -> Working {
        Working {
            work: vec![
                item("rewrite the parser", Stage::Attempted, &[3]),
                item("add the tests", Stage::Planned, &[]),
            ],
            ..Working::default()
        }
    }

    #[test]
    fn dropping_unfinished_work_is_refused() {
        let next = Working {
            work: vec![item("rewrite the parser", Stage::Attempted, &[3])],
            ..Working::default()
        };
        assert_eq!(
            next.may_replace(Some(&midway())),
            Err(Refused::WorkDropped {
                what: "add the tests".to_owned()
            })
        );
    }

    #[test]
    fn finishing_it_needs_a_row_that_says_so() {
        let next = Working {
            work: vec![
                item("rewrite the parser", Stage::Done, &[]),
                item("add the tests", Stage::Planned, &[]),
            ],
            ..Working::default()
        };
        assert_eq!(
            next.may_replace(Some(&midway())),
            Err(Refused::Unevidenced {
                what: "rewrite the parser".to_owned()
            })
        );
    }

    #[test]
    fn finished_and_evidenced_work_may_go_on_being_listed() {
        let next = Working {
            work: vec![
                item("rewrite the parser", Stage::Done, &[9]),
                item("add the tests", Stage::Planned, &[]),
            ],
            ..Working::default()
        };
        assert_eq!(next.may_replace(Some(&midway())), Ok(()));
    }

    #[test]
    fn work_already_done_need_not_be_carried_for_ever() {
        let was = Working {
            work: vec![item("rewrite the parser", Stage::Done, &[9])],
            ..Working::default()
        };
        assert_eq!(Working::default().may_replace(Some(&was)), Ok(()));
    }

    #[test]
    fn every_stage_is_shown_apart_from_the_others() {
        let text = midway().rendered();
        assert!(
            text.contains("Tried") && text.contains("rewrite the parser"),
            "{text}"
        );
        assert!(
            text.contains("Planned") && text.contains("add the tests"),
            "{text}"
        );
    }
}

/// What the person said survives being summarised.
mod mem_exact_user_constraint_survives {
    use super::*;

    const SAID: &str = "never touch the vendored directory, not even to format it";

    fn corrected() -> Working {
        Working {
            corrections: vec![Correction {
                said: SAID.to_owned(),
                cursor: 4,
            }],
            ..Working::default()
        }
    }

    #[test]
    fn losing_it_is_refused() {
        assert_eq!(
            Working::default().may_replace(Some(&corrected())),
            Err(Refused::CorrectionDropped {
                said: SAID.to_owned()
            })
        );
    }

    #[test]
    fn a_paraphrase_is_not_the_same_correction() {
        // Rewritten, a constraint loses the exception that made it one.
        let softened = Working {
            corrections: vec![Correction {
                said: "avoid the vendored directory".to_owned(),
                cursor: 4,
            }],
            ..Working::default()
        };
        assert!(softened.may_replace(Some(&corrected())).is_err());
    }

    #[test]
    fn it_reaches_the_request_word_for_word() {
        assert!(corrected().rendered().contains(SAID));
    }
}

/// One prepared state cannot grow without bound.
mod bounds {
    use super::*;

    #[test]
    fn a_list_longer_than_the_bound_is_refused() {
        let held = Working {
            open: (0..=MOST).map(|n| n.to_string()).collect(),
            ..Working::default()
        };
        assert_eq!(
            held.may_replace(None),
            Err(Refused::TooLarge { field: "open" })
        );
    }

    #[test]
    fn an_entry_wider_than_the_bound_is_refused() {
        let held = Working {
            goal: "x".repeat(WIDEST + 1),
            ..Working::default()
        };
        assert_eq!(
            held.may_replace(None),
            Err(Refused::TooLarge { field: "goal" })
        );
    }

    #[test]
    fn what_fits_is_allowed() {
        let held = Working {
            goal: "x".repeat(WIDEST),
            open: (0..MOST).map(|n| n.to_string()).collect(),
            ..Working::default()
        };
        assert_eq!(held.may_replace(None), Ok(()));
    }
}

/// What a helper answered, read back.
mod reading {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_well_formed_answer_is_read() {
        let said = json!({
            "goal": "make the parser accept trailing commas",
            "work": [{ "what": "read the grammar", "stage": "done", "evidence": [2] }],
            "checks": [{ "ran": "cargo test", "passed": false, "said": "1 failed" }],
            "corrections": [{ "said": "keep the old behaviour behind a flag", "cursor": 7 }],
            "open": ["decide the flag's name"],
            "references": ["src/parse.rs:120"],
        });
        let held = Working::read(&said).expect("a state");
        assert_eq!(held.work[0].stage, Stage::Done);
        assert_eq!(held.checks[0].passed, Some(false));
        assert_eq!(held.corrections[0].cursor, 7);
        assert_eq!(held.may_replace(None), Ok(()));
    }

    #[test]
    fn prose_where_a_state_was_asked_for_is_no_state() {
        assert!(Working::read(&json!("I rewrote the parser and it works now")).is_none());
        assert!(Working::read(&json!({ "work": "everything" })).is_none());
    }

    #[test]
    fn an_empty_answer_is_an_empty_state_rather_than_none() {
        let held = Working::read(&json!({})).expect("a state");
        assert_eq!(held, Working::default());
        assert_eq!(held.rendered(), "");
    }
}
