use super::{id_problem, join};

#[test]
fn rule_ids_are_lowercase_words_joined_by_dashes() {
    for good in ["a", "no-print", "no-direct-db-access"] {
        assert!(id_problem(good).is_none(), "{good}");
    }
    for bad in ["", "web/no-print", "No-Print", "no_print", "-a", "a-", "a--b", "no-md5", "a b"] {
        assert!(id_problem(bad).is_some(), "{bad}");
    }
}

#[test]
fn join_resolves_paths_against_the_config_directory() {
    assert_eq!(join("web", "src/"), "web/src/");
    assert_eq!(join("a/b", "./codes.json"), "a/b/codes.json");
    assert_eq!(join("a/b", "../shared/codes.json"), "a/shared/codes.json");
}
