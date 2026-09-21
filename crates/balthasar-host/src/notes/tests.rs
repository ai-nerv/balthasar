use super::*;

#[test]
fn a_day_is_written_as_a_date() {
    assert_eq!(day(0), "1970-01-01");
    assert_eq!(day(1_756_000_000), "2025-08-24");
    assert_eq!(day(951_782_400), "2000-02-29");
}

#[test]
fn memory_settings_fall_back_to_the_shipped_ones() {
    assert_eq!(Keeping::read(None), Keeping::default());
    let said = json!({ "review": true, "extract_every": 3, "tidy_every": 0 });
    let keeping = Keeping::read(Some(&said));
    assert!(keeping.review);
    assert_eq!(keeping.extract_every, 3);
    assert_eq!(
        keeping.tidy_every, 10,
        "zero would tidy after every extraction"
    );
    assert_eq!(keeping.contradict_every, 20);
    assert_eq!(
        Keeping::read(Some(&json!({ "contradict_every": 3 }))).contradict_every,
        3
    );
}
