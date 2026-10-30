use super::super::*;
use serde_json::json;

#[test]
fn optional_context_requires_a_valid_key_even_for_reads_and_never_enters_arguments() {
    for wire in [crate::Wire::Json, crate::Wire::Cbor] {
        for revision in [None, Some(1)] {
            let guard = guard_for_tests().with_request_context(revision);
            for key in [None, Some("wrong"), Some(for_tests().1)] {
                let value = json!({"call":"verbs", "key":key, "event_context":{"schema":1},
                    "args":[{"event_context":"model-data"}]});
                let (request, context) = admitted(&guard, wire, &wire.write(&value).expect("wire"))
                    .expect("read stays allowed");
                assert_eq!(request.args, vec![json!({"event_context":"model-data"})]);
                assert!(
                    serde_json::to_value(&request)
                        .expect("request")
                        .get("event_context")
                        .is_none()
                );
                assert_eq!(
                    context,
                    (revision == Some(1) && key == Some(for_tests().1))
                        .then(|| json!({"schema":1}))
                );
            }
            for key in [None, Some("wrong")] {
                let value = json!({"call":"observe", "key":key,"event_context":{"schema":1}});
                assert!(admitted(&guard, wire, &wire.write(&value).expect("wire")).is_err());
            }
        }
    }
}
