use super::*;
use serde_json::json;

fn spec(instruction: &str, input: &str, schema: Value) -> Value {
    json!({ "instruction": instruction, "input": input, "schema": schema })
}

/// A helper request is bounded on the whole of it.
mod mem_helper_input_is_bounded {
    use super::*;

    #[test]
    fn the_instruction_and_the_schema_count_against_the_budget() {
        // Budgeting the input alone let a long instruction and a large schema ride along free,
        // and all three reach the helper's window.
        let held = spec(&"i".repeat(100), &"x".repeat(100), json!({}));
        assert_eq!(measured(&held), 100 + 100 + 2);
        assert!(fits(&held, 202));
        assert!(!fits(&held, 201));
    }

    #[test]
    fn what_is_left_for_the_input_is_what_the_rest_did_not_take() {
        assert_eq!(room(&"i".repeat(40), &Value::Null, 100), 60);
        assert_eq!(room(&"i".repeat(40), &json!({}), 100), 58);
    }

    #[test]
    fn a_request_larger_than_the_budget_leaves_no_room_rather_than_wrapping() {
        assert_eq!(room(&"i".repeat(500), &Value::Null, 100), 0);
    }

    #[test]
    fn appending_stops_at_the_bound_instead_of_running_past_it() {
        // The rule that was wrong: a per-row check that keeps appending after it trips bounds
        // nothing. `add` reports what it did, and what does not fit is not written.
        let mut input = String::new();
        assert!(add(&mut input, "0123456789", 20));
        assert!(add(&mut input, "0123456789", 20));
        assert!(!add(&mut input, "x", 20));
        assert_eq!(input.len(), 20);
    }

    #[test]
    fn a_missing_schema_costs_nothing() {
        assert_eq!(measured(&spec("i", "x", Value::Null)), 2);
    }

    #[test]
    fn a_spec_with_no_parts_at_all_measures_zero() {
        assert_eq!(measured(&json!({})), 0);
        assert!(fits(&json!({}), 0));
    }
}
