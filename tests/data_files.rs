//! End-to-end tests of the CLI on small projects written to a temp directory.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Project {
    root: PathBuf,
}

impl Project {
    fn new(name: &str, files: &[(&str, &str)]) -> Project {
        let root = std::env::temp_dir().join(format!("tvl-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project { root };
        for (path, text) in files {
            project.write(path, text);
        }
        project
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }

    fn tvl(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tvl")).args(args).current_dir(&self.root).output().unwrap()
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn exit(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn violations(output: &Output) -> Vec<String> {
    stdout(output).lines().filter(|l| l.contains(": error[")).map(|l| l.split(": error[").next().unwrap().to_string()).collect()
}

const WORKFLOW_RULES: &str = "rules:
  timeouts:
    language: yaml
    require_property: [\"jobs.*.timeout-minutes\"]
    rationale: r
    fix: f
scopes:
  - rules: [timeouts]
    include: [\"**/*.yml\"]
";

#[test]
fn hidden_directories_are_read_but_git_is_not() {
    let p = Project::new(
        "hidden",
        &[
            ("tvl.yaml", WORKFLOW_RULES),
            (".github/workflows/ci.yml", "jobs:\n  a:\n    runs-on: x\n"),
            (".git/odd.yml", "jobs:\n  a:\n    runs-on: x\n"),
        ],
    );
    let out = p.tvl(&["check"]);
    assert_eq!(exit(&out), 1, "{}", stdout(&out));
    assert_eq!(violations(&out), [".github/workflows/ci.yml:2:3"]);
}

const REGISTRY_RULES: &str = "rules:
  codes:
    language: python
    literal_argument:
      calls: [\"*Exception\"]
      registry: { file: codes.json, path: \"codes.*\" }
      report_unused: true
    rationale: r
    fix: f
scopes:
  - rules: [codes]
    include: [\"src/**\"]
";

const CODES: &str = "{\n  \"codes\": [\n    \"used\",\n    \"dead\"\n  ]\n}\n";
const RAISES: &str = "raise AppException('used')\nraise AppException('typo')\n";

#[test]
fn a_missing_registry_file_stops_check_with_exit_two() {
    let p = Project::new("noregistry", &[("tvl.yaml", REGISTRY_RULES), ("src/a.py", RAISES)]);
    let out = p.tvl(&["check"]);
    assert_eq!(exit(&out), 2);
    assert!(stderr(&out).contains("rule `codes` reads codes.json"), "{}", stderr(&out));
}

#[test]
fn validate_checks_registry_files_against_the_project() {
    let p = Project::new("validate", &[("tvl.yaml", REGISTRY_RULES), ("src/a.py", RAISES)]);
    let out = p.tvl(&["validate"]);
    assert_eq!(exit(&out), 1);
    assert!(stdout(&out).contains("`codes.json` is not a JSON, YAML or TOML file in the project"), "{}", stdout(&out));

    p.write("codes.json", "{\"other\": [\"x\"]}");
    let out = p.tvl(&["validate"]);
    assert_eq!(exit(&out), 0, "{}", stdout(&out));
    assert!(stdout(&out).contains("`codes.*` selects no entries in codes.json"), "{}", stdout(&out));

    p.write("codes.json", CODES);
    let out = p.tvl(&["validate"]);
    assert_eq!(exit(&out), 0);
    assert!(!stdout(&out).contains("selects no entries"), "{}", stdout(&out));
}

#[test]
fn unused_entries_are_reported_in_the_registry_and_only_when_it_is_selected() {
    let p = Project::new("unused", &[("tvl.yaml", REGISTRY_RULES), ("codes.json", CODES), ("src/a.py", RAISES)]);
    let all = p.tvl(&["check"]);
    assert_eq!(violations(&all), ["codes.json:4:5", "src/a.py:2:20"]);

    // Selecting the code file reports the typo but not the dead entry.
    let code_only = p.tvl(&["check", "src/a.py"]);
    assert_eq!(violations(&code_only), ["src/a.py:2:20"]);

    // Selecting the registry still counts usage from the whole scope: `used` is not reported.
    let registry_only = p.tvl(&["check", "codes.json"]);
    assert_eq!(violations(&registry_only), ["codes.json:4:5"]);
}

#[test]
fn staged_registry_counts_usage_from_files_that_are_not_staged() {
    let p = Project::new("staged", &[("tvl.yaml", REGISTRY_RULES), ("codes.json", "{\"codes\": [\"used\"]}"), ("src/a.py", RAISES)]);
    p.git(&["init", "-q"]);
    p.git(&["add", "."]);
    p.git(&["commit", "-q", "-m", "init"]);
    p.write("codes.json", CODES);
    p.git(&["add", "codes.json"]);

    let out = p.tvl(&["check", "--staged"]);
    assert_eq!(violations(&out), ["codes.json:4:5"], "{}", stdout(&out));
}

#[test]
fn suppression_comments_work_in_yaml_toml_and_json() {
    let rules = "rules:
  no-debug:
    language: [yaml, toml, json]
    forbid_property: [debug]
    rationale: r
    fix: f
scopes:
  - rules: [no-debug]
    include: [\"cfg/**\"]
";
    let p = Project::new(
        "suppress",
        &[
            ("tvl.yaml", rules),
            ("cfg/a.yaml", "# tvl-ignore-next-line: legacy\ndebug: true\n"),
            ("cfg/b.toml", "# tvl-ignore-next-line: legacy\ndebug = true\n"),
            ("cfg/c.json", "{\n  // tvl-ignore-next-line: legacy\n  \"debug\": true\n}\n"),
            ("cfg/d.yaml", "# tvl-ignore-file\ndebug: true\n"),
            ("cfg/e.yaml", "debug: true\n"),
        ],
    );
    let out = p.tvl(&["check"]);
    assert_eq!(violations(&out), ["cfg/e.yaml:1:1"], "{}", stdout(&out));
    assert!(stdout(&out).contains("3 hidden by tvl-ignore-next-line"), "{}", stdout(&out));
    assert!(stdout(&out).contains("1 file(s) skipped by tvl-ignore-file"), "{}", stdout(&out));
}


#[test]
fn init_scaffolds_an_empty_valid_config_and_never_overwrites() {
    let project = Project::new("init", &[("app/models.py", "print(1)\n")]);

    let created = project.tvl(&["init"]);
    assert_eq!(created.status.code(), Some(0), "{}", stdout(&created));
    let config = std::fs::read_to_string(project.root.join("tvl.yaml")).unwrap();
    assert!(config.contains("rules: {}"));

    // Valid, with a warning that nothing is enforced yet, and nothing is flagged.
    let validated = project.tvl(&["validate"]);
    assert_eq!(validated.status.code(), Some(0), "{}", stdout(&validated));
    assert!(stdout(&validated).contains("no rules are defined yet"));
    assert_eq!(project.tvl(&["check"]).status.code(), Some(0));

    let again = project.tvl(&["init"]);
    assert_eq!(again.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&again.stderr).contains("--force"));
    assert_eq!(project.tvl(&["init", "--force"]).status.code(), Some(0));
}

#[test]
fn json_and_sarif_output_are_documents_on_stdout() {
    let project = Project::new(
        "formats",
        &[
            ("tvl.yaml", "rules:\n  no-print:\n    language: python\n    forbid_call: [print]\n    rationale: Use the logger.\n    fix: Call logging.info.\nscopes:\n  - rules: [no-print]\n    include: ['**']\n"),
            ("a.py", "print(1)\n"),
        ],
    );
    let json = project.tvl(&["check", "--format", "json"]);
    assert_eq!(json.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(doc["violations"][0]["rule"], "no-print");
    assert_eq!(doc["violations"][0]["fix"], "Call logging.info.");
    assert_eq!(doc["summary"]["violations"], 1);

    let sarif = project.tvl(&["check", "--format", "sarif"]);
    assert_eq!(sarif.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&sarif)).unwrap();
    assert_eq!(doc["version"], "2.1.0");
    assert_eq!(doc["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"], "a.py");

    // A clean project still gives a valid, empty document.
    project.write("a.py", "x = 1\n");
    let clean = project.tvl(&["check", "--format", "json"]);
    assert_eq!(clean.status.code(), Some(0));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&clean)).unwrap();
    assert_eq!(doc["violations"].as_array().unwrap().len(), 0);
}
