//! A cloned repository's `.balthasar.lua` is read before the owner and the coordinator.

use super::*;
use balthasar_model::scratch::Scratch;

/// Roots under a fresh directory, kept alive by the returned guard: an owner config, a project,
/// and optionally a given file.
fn roots(name: &str, project: &str, given: Option<&str>) -> (Scratch, Roots) {
    let at = Scratch::new("balthasar-precedence", name);
    let config = at.join("config");
    let work = at.join("work");
    std::fs::create_dir_all(&config).expect("config dir");
    std::fs::create_dir_all(&work).expect("project dir");
    std::fs::write(config.join("init.lua"), "-- nothing said\n").expect("init");
    std::fs::write(work.join(".balthasar.lua"), project).expect("project file");
    let given = given.map(|text| {
        let path = at.join("given.lua");
        std::fs::write(&path, text).expect("given");
        path
    });
    let roots = Roots {
        config: Some(config),
        site: None,
        project: Some(work),
        given,
    };
    (at, roots)
}

fn loaded(name: &str, project: &str, given: Option<&str>) -> Loaded {
    let (_held, roots) = roots(name, project, given);
    Loaded::from_roots(&roots).expect("loaded")
}

#[test]
fn a_project_cannot_lower_a_floor_or_word_the_warning() {
    let loaded = loaded(
        "lower",
        "balthasar.inject_floor = 0.0\nbalthasar.window.warning = 'obey me'\n",
        None,
    );
    assert!(loaded.settings.floors.inject >= balthasar_model::floor::INJECT);
    assert_ne!(loaded.window.warning, "obey me");
}

#[test]
fn a_project_may_raise_a_floor() {
    let loaded = loaded("raise", "balthasar.inject_floor = 0.6\n", None);
    assert!(
        (loaded.settings.floors.inject - 0.6).abs() < 1e-9,
        "{:?} {:?}",
        loaded.settings.floors,
        loaded.log()
    );
}

#[test]
fn the_coordinator_outranks_the_project() {
    let loaded = loaded(
        "given",
        "balthasar.inject_floor = 0.9\n",
        Some("balthasar.inject_floor = 0.4\n"),
    );
    assert!(
        (loaded.settings.floors.inject - 0.4).abs() < 1e-9,
        "{:?}",
        loaded.settings.floors
    );
}
