use super::*;
use balthasar_model::scratch::Scratch;

/// A scratch directory nothing else is using, named after the test rather than shared.
fn scratch(name: &str) -> Scratch {
    Scratch::new("balthasar-paths", name)
}

#[test]
fn a_tool_name_that_would_escape_its_directory_is_refused() {
    assert!(Tool::new("..").is_none());
    assert!(Tool::new(".").is_none());
    assert!(Tool::new("a/b").is_none());
    assert!(Tool::new("").is_none());
    assert!(Tool::new("-lead").is_none());
    assert!(Tool::new("Magi").is_none(), "case is not silently folded");
    assert_eq!(Tool::new("harness").expect("valid").as_str(), "harness");
}

#[test]
fn a_program_name_is_made_usable_rather_than_refused() {
    assert_eq!(
        Tool::from_program("My Harness").expect("slug").as_str(),
        "my-harness"
    );
    assert_eq!(
        Tool::from_program("/usr/bin/thing").expect("slug").as_str(),
        "usr-bin-thing"
    );
    assert!(Tool::from_program("///").is_none(), "nothing survives");
    assert!(Tool::from_program("").is_none());
}

#[test]
fn a_session_name_cannot_climb_out_of_the_store() {
    for hostile in ["..", ".", "../../etc", "a/b"] {
        let stem = path_stem(hostile);
        assert!(!stem.contains('/'), "{hostile} -> {stem}");
        assert_ne!(stem, "..", "{hostile}");
        assert_ne!(stem, ".", "{hostile}");
    }
}

#[test]
fn an_agent_name_is_held_to_the_same_boundary_as_a_session_name() {
    let home = Path::new("/w/p/balthasar/harness");
    let run = SessionId::new("01K5X8ZQ");
    let hostile = session_dir_in(home, &run, &AgentId::new("../../etc"));

    assert_eq!(
        hostile.parent(),
        Some(run_dir_in(home, &run).as_path()),
        "it stayed inside its run: {hostile:?}"
    );
    let leaf = hostile.file_name().expect("a name").to_string_lossy();
    assert!(leaf != ".." && leaf != "." && !leaf.contains('/'), "{leaf}");
}

#[test]
fn two_mangled_session_names_do_not_share_a_directory() {
    assert_ne!(path_stem("a/b"), path_stem("a:b"));
}

#[test]
fn an_ordinary_session_name_is_left_alone() {
    assert_eq!(path_stem("01K5X8ZQ"), "01K5X8ZQ");
}

#[test]
fn a_directory_named_balthasar_is_not_mistaken_for_a_store() {
    // This checkout is called `balthasar`; without the marker its parent resolves as a root.
    let root = scratch("lookalike");
    std::fs::create_dir_all(root.join(HOME)).expect("mkdir");
    assert!(!is_home(&root.join(HOME)));
    assert!(nearest_home(&root).is_none());

    make_home(&root.join(HOME)).expect("make");
    assert!(is_home(&root.join(HOME)));
}

#[test]
fn the_shared_directories_are_the_ones_nobody_owns() {
    assert!(is_shared(&std::env::temp_dir()));
    assert!(is_shared(std::path::Path::new("/")));
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Some(above) = home.as_deref().and_then(Path::parent) {
        assert!(is_shared(above), "{}", above.display());
    }
}

#[test]
fn a_store_home_in_a_shared_directory_scopes_nothing() {
    // A leftover `.store` in the temporary directory otherwise resolves every path under it
    // to one scope. The ceiling is passed in because no test may write into the real `/tmp`.
    let root = scratch("ceiling");
    let shared = root.join("shared");
    let under = shared.join("project");
    std::fs::create_dir_all(&under).expect("mkdir");
    make_home(&shared.join(HOME)).expect("make");

    let stops = |dir: &Path| dir == shared;
    assert_eq!(
        home_below(&under, stops),
        None,
        "the walk passed the ceiling"
    );
    assert_eq!(home_below(&shared, stops), None);

    assert_eq!(home_below(&under, |_| false), Some(shared.clone()));
}

#[test]
fn a_store_above_a_repository_does_not_reach_into_it() {
    let root = scratch("reach");
    let top = root.join("top");
    let project = top.join("down/project");
    std::fs::create_dir_all(project.join(".git")).expect("mkdir");
    std::fs::create_dir_all(project.join("src")).expect("mkdir");
    make_home(&top.join(HOME)).expect("make");

    assert_eq!(home_below(&project.join("src"), |_| false), None);
    assert_eq!(
        scope_of(&project.join("src")).as_str(),
        project.to_string_lossy()
    );
    assert_eq!(
        home_below(&top.join("down"), |_| false),
        Some(top.clone()),
        "outside any checkout the store still covers what is under it"
    );
}

#[test]
fn a_store_home_scopes_the_subtree_it_sits_in() {
    let root = scratch("subtree");
    let package = root.join("crates/thing");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    std::fs::create_dir_all(package.join("src")).expect("mkdir");
    make_home(&package.join(HOME)).expect("make");

    assert_eq!(
        scope_of(&package.join("src")).as_str(),
        package.to_string_lossy()
    );
    assert_eq!(scope_of(&root).as_str(), root.to_string_lossy());
}

#[test]
fn a_repository_keeps_its_memory_inside_itself() {
    let root = scratch("in-project");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    let scope = scope_of(&root);
    let path = scope_path(&scope, &Tool::default());

    assert!(path.starts_with(&root), "{}", path.display());
    assert_eq!(path, root.join(HOME).join("balthasar/project.db"));
}

#[test]
fn renaming_a_project_keeps_its_memory() {
    let root = scratch("rename");
    let before = root.join("before");
    std::fs::create_dir_all(before.join(".git")).expect("mkdir");
    make_home(&before.join(HOME)).expect("make");
    let was = scope_path(&scope_of(&before), &Tool::default());
    let relative = was.strip_prefix(&before).expect("under the project");

    let after = root.join("after");
    std::fs::rename(&before, &after).expect("mv");
    let now = scope_path(&scope_of(&after), &Tool::default());

    assert_eq!(
        now,
        after.join(relative),
        "the store moved with the project"
    );
}

#[test]
fn everything_under_a_repository_shares_its_scope() {
    let root = scratch("in-repo");
    let deep = root.join("crates/thing/src");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    std::fs::create_dir_all(&deep).expect("mkdir");

    assert_eq!(scope_of(&deep), scope_of(&root));
    assert_eq!(scope_of(&deep).as_str(), root.to_string_lossy());
}

#[test]
fn every_worktree_of_a_repository_shares_one_scope() {
    // A worktree's `.git` is a file pointing at the real one.
    let root = scratch("worktree");
    let work = root.join("checkout");
    let tree = root.join("tree");
    std::fs::create_dir_all(work.join(".git/worktrees/tree")).expect("mkdir");
    std::fs::create_dir_all(&tree).expect("mkdir");
    std::fs::write(
        tree.join(".git"),
        format!("gitdir: {}/.git/worktrees/tree\n", work.display()),
    )
    .expect("write");

    assert_eq!(scope_of(&tree), scope_of(&work));
}

#[test]
fn a_directory_in_no_repository_is_its_own_scope() {
    // Deliberately a path that does not exist: a real temporary directory may sit under
    // somebody else's checkout.
    let nowhere = Path::new("/balthasar-no-such-root-9f3a/deep/inside");
    assert_eq!(scope_of(nowhere).as_str(), nowhere.to_string_lossy());
}

#[test]
fn a_scope_with_no_project_keeps_its_memory_in_the_data_directory() {
    let nowhere = ScopeId::new("/balthasar-no-such-root-9f3a/deep");
    let path = scope_path(&nowhere, &Tool::default());
    assert!(path.starts_with(data_dir()), "{}", path.display());
}

#[test]
fn the_global_store_is_per_tool_and_not_per_project() {
    let tool = Tool::new("oslo").expect("valid");
    let path = scope_path(&ScopeId::global(), &tool);
    assert_eq!(path, data_dir().join("oslo/global.db"));
}

#[test]
fn two_tools_in_one_project_do_not_share_a_store() {
    let root = scratch("two-tools");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    let scope = scope_of(&root);
    let one = scope_path(&scope, &Tool::new("harness").expect("valid"));
    let two = scope_path(&scope, &Tool::new("oslo").expect("valid"));

    assert_ne!(one, two);
    assert_eq!(
        one.parent().and_then(Path::parent),
        two.parent().and_then(Path::parent)
    );
}

#[test]
fn every_agent_of_a_run_keeps_its_scratch_under_that_run() {
    let scope = ScopeId::new("/balthasar-no-such-root-9f3a/p");
    let tool = Tool::default();
    let run = SessionId::new("01K5X8ZQ");
    let one = session_dir(&scope, &tool, &run, &AgentId::main());
    let two = session_dir(&scope, &tool, &run, &AgentId::new("reviewer"));

    assert_ne!(one, two, "two agents, two directories");
    assert_eq!(one.parent(), two.parent(), "and one run above them");
    assert_eq!(
        session_path(&scope, &tool, &run, &AgentId::main()),
        one.join("memory.db")
    );
}

#[test]
fn a_run_directory_sits_beside_the_project_store_it_promotes_into() {
    let scope = ScopeId::new("/balthasar-no-such-root-9f3a/p");
    let tool = Tool::default();
    let run = SessionId::new("01K5X8ZQ");
    assert_eq!(
        session_dir(&scope, &tool, &run, &AgentId::main())
            .parent()
            .and_then(Path::parent),
        scope_path(&scope, &tool).parent()
    );
}

#[test]
fn listing_tools_finds_only_the_ones_with_memory() {
    let root = scratch("tools-in");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    let home = root.join(HOME);
    make_home(&home).expect("make");
    std::fs::create_dir_all(home.join("harness")).expect("mkdir");
    std::fs::create_dir_all(home.join("oslo")).expect("mkdir");
    std::fs::create_dir_all(home.join("empty")).expect("mkdir");
    std::fs::write(home.join("harness/project.db"), "").expect("write");
    std::fs::write(home.join("oslo/project.db"), "").expect("write");

    let found = tools_in(&scope_of(&root));
    let names: Vec<&str> = found.iter().map(Tool::as_str).collect();
    assert_eq!(
        names,
        vec!["harness", "oslo"],
        "a directory with no store is not a tool"
    );
}

#[test]
fn a_store_home_keeps_sessions_out_of_the_checkout() {
    let root = scratch("ignore");
    let home = root.join(HOME);
    make_home(&home).expect("make");
    let body = std::fs::read_to_string(home.join(".gitignore")).expect("read");
    assert!(
        body.lines().any(|line| line == "*"),
        "none of it shows in git"
    );
    assert!(
        body.contains("# !*/project.db"),
        "committing is offered, not chosen"
    );

    std::fs::write(home.join(".gitignore"), "mine\n").expect("write");
    make_home(&home).expect("again");
    assert_eq!(
        std::fs::read_to_string(home.join(".gitignore")).expect("read"),
        "mine\n"
    );
}

#[test]
fn a_store_under_the_old_name_is_moved_and_kept() {
    // It used to be a visible `balthasar/` in every checkout, showing in `git status`.
    let root = scratch("legacy");
    std::fs::create_dir_all(root.join(".git")).expect("git");
    let old = root.join(LEGACY_HOME);
    make_home(&old).expect("make");
    std::fs::write(old.join(".gitignore"), OLD_IGNORE_BODY).expect("the old ignore");
    std::fs::create_dir_all(old.join("tool-a")).expect("tool");
    std::fs::write(old.join("tool-a/project.db"), "memory").expect("db");

    let scope = ScopeId::new(root.to_string_lossy().into_owned());
    let home = project_home(&scope).expect("a home");
    assert_eq!(home, root.join(HOME));
    assert!(!old.exists(), "moved, not copied");
    assert_eq!(
        std::fs::read_to_string(home.join("tool-a/project.db")).expect("kept"),
        "memory"
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".gitignore")).expect("ignore"),
        IGNORE_BODY,
        "the old default is brought up to date"
    );
}

#[test]
fn a_checkout_carrying_a_stray_marker_is_not_a_store() {
    let root = scratch("stray");
    let checkout = root.join(LEGACY_HOME);
    std::fs::create_dir_all(checkout.join("crates")).expect("mkdir");
    std::fs::write(checkout.join("Cargo.toml"), "[workspace]").expect("write");
    make_home(&checkout).expect("make");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");

    let _ = project_home(&ScopeId::new(root.to_string_lossy().into_owned()));
    assert!(
        checkout.join("Cargo.toml").is_file(),
        "the checkout was moved"
    );
    assert_eq!(home_below(&checkout.join("crates"), |_| false), None);
}

#[test]
fn a_checkout_under_the_old_name_is_never_moved() {
    // A superproject whose `balthasar/` is the code, with a store once written into it.
    let root = scratch("legacy-checkout");
    std::fs::create_dir_all(root.join(".git")).expect("git");
    let code = root.join(LEGACY_HOME);
    make_home(&code).expect("make");
    std::fs::write(code.join(".git"), "gitdir: ../.git/modules/balthasar").expect("gitfile");
    std::fs::write(code.join("Cargo.toml"), "[workspace]").expect("code");

    let scope = ScopeId::new(root.to_string_lossy().into_owned());
    assert_eq!(project_home(&scope), Some(root.join(HOME)));
    assert!(
        code.join("Cargo.toml").exists(),
        "the checkout is where it was"
    );
}

#[test]
fn a_repository_in_a_shared_directory_scopes_nothing() {
    // A `.git` at the top of the temporary directory, or in a home directory, made every
    // directory below it resolve to that one root.
    let shared = std::env::temp_dir();
    assert!(too_broad(&shared), "{}", shared.display());

    let under = balthasar_model::scratch::Scratch::new("balthasar-git-ceiling", "probe");
    assert_eq!(git_common_dir(&under), None);
    assert_eq!(scope_of(&under).as_str(), under.to_string_lossy());
}

#[test]
fn a_home_directory_is_too_broad_to_be_a_project() {
    // A repository at `$HOME` is a dotfiles repository, not the project an unrelated
    // subdirectory belongs to; `is_shared` still lets a deliberate store here count.
    let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
        return;
    };
    let home = std::path::Path::new(&home);
    assert!(too_broad(home), "{}", home.display());
    assert!(!is_shared(home), "an explicit store here would still count");
}

#[test]
fn a_real_checkout_is_still_found() {
    let root = scratch("checkout");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir");
    let deep = root.join("crates/thing/src");
    std::fs::create_dir_all(&deep).expect("mkdir");
    assert_eq!(git_common_dir(&deep).as_deref(), Some(&*root));
}
