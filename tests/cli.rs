use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::{fs, process::Command};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct TestApp {
    _dir: TempDir,
    db_path: std::path::PathBuf,
}

impl TestApp {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("tasks.db");
        Self { _dir: dir, db_path }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::cargo_bin("ct").expect("ct binary");
        cmd.env("CT_DB", &self.db_path);
        cmd
    }

    fn list(&self) {
        self.command().arg("list").assert().success();
    }
}

#[test]
fn binary_exposes_help() {
    let app = TestApp::new();

    app.command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Couch Tasks CLI"))
        .stdout(predicate::str::contains("Usage:"));

    app.command()
        .assert()
        .success()
        .stdout(predicate::str::contains("Couch Tasks CLI"))
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn test_harness_sets_isolated_ct_db() {
    let app = TestApp::new();

    app.command().arg("--help").assert().success();

    assert!(
        app.db_path.starts_with(app._dir.path()),
        "test database path should stay inside the temp directory"
    );
}

#[test]
fn invalid_arguments_exit_with_user_error() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "Usage: ct add <project> <task...>",
        ));
}

#[test]
fn removed_short_aliases_are_not_commands() {
    let app = TestApp::new();

    app.command()
        .args(["q", "buy", "milk"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Unknown command: q"));
    app.command()
        .args(["do", "1"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Unknown command: do"));
    app.command()
        .args(["rm", "1"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Unknown command: rm"));
}

#[test]
fn empty_database_lists_no_pending_or_completed_tasks() {
    let app = TestApp::new();

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("No pending tasks.\n");

    app.command()
        .arg("completed")
        .assert()
        .success()
        .stdout("No completed tasks.\n");
}

#[test]
fn projects_lists_active_projects_and_always_includes_inbox() {
    let app = TestApp::new();

    app.command()
        .arg("projects")
        .assert()
        .success()
        .stdout("inbox\n");

    app.command()
        .args(["add", "Work", "review", "pr"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "clean", "gutters"])
        .assert()
        .success();
    app.list();
    app.command().args(["delete", "1"]).assert().success();

    app.command()
        .arg("projects")
        .assert()
        .success()
        .stdout("inbox\nWork\n");
}

#[test]
fn projects_json_outputs_an_array_of_active_project_names() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Work", "review", "pr"])
        .assert()
        .success();

    app.command()
        .args(["projects", "--json"])
        .assert()
        .success()
        .stdout("[\n  \"inbox\",\n  \"Work\"\n]\n");
}

#[test]
fn add_quick_and_list_tasks() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success()
        .stdout("Added to Home: fix garage light\n");

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success()
        .stdout("Added to inbox: buy milk\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n\x1b[48;2;220;224;232m2  Home   fix garage light\x1b[0m\n");
}

#[test]
fn commands_accept_task_uuid_selectors() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success();

    let assert = app.command().args(["list", "--json"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let uuid = tasks[0]["uuid"].as_str().expect("task uuid");

    app.command()
        .args(["done", uuid])
        .assert()
        .success()
        .stdout(format!("Done {uuid}: fix garage light\n"));

    app.command()
        .args(["show", uuid])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{uuid}  Home  fix garage light"
        )));
}

#[test]
fn list_json_outputs_pending_tasks_with_metadata() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success();
    app.list();
    app.command()
        .args(["note", "1", "check", "breaker"])
        .assert()
        .success();
    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();

    let assert = app.command().args(["list", "--json"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");

    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0]["id"], 2);
    assert_eq!(tasks[0]["project_name"], "inbox");
    assert_eq!(tasks[0]["title"], "buy milk");
    assert_eq!(tasks[0]["status"], "pending");
    assert_eq!(tasks[0]["notes"], "");
    assert!(
        tasks[0]["uuid"]
            .as_str()
            .is_some_and(|uuid| !uuid.is_empty())
    );
    assert!(
        tasks[0]["created_at"]
            .as_str()
            .is_some_and(|time| !time.is_empty())
    );
    assert!(
        tasks[0]["updated_at"]
            .as_str()
            .is_some_and(|time| !time.is_empty())
    );
    assert_eq!(tasks[0]["completed_at"], serde_json::Value::Null);

    assert_eq!(tasks[1]["id"], 1);
    assert_eq!(tasks[1]["project_id"], 2);
    assert_eq!(tasks[1]["project_name"], "Home");
    assert_eq!(tasks[1]["title"], "fix garage light");
    assert_eq!(tasks[1]["notes"], "check breaker");

    let assert = app
        .command()
        .args(["list", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let home_tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(home_tasks.as_array().expect("array").len(), 1);
}

#[test]
fn project_shortcut_lists_one_project_case_insensitively() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success();
    app.command()
        .args(["add", "home", "replace", "air", "filter"])
        .assert()
        .success()
        .stdout("Added to Home: replace air filter\n");

    app.command().arg("home").assert().success().stdout(
        "1  Home  fix garage light\n\x1b[48;2;220;224;232m2  Home  replace air filter\x1b[0m\n",
    );
}

#[test]
fn multi_id_mutations_report_errors_on_stderr_and_do_not_partially_apply() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.command()
        .args(["quick", "cancel", "subscription"])
        .assert()
        .success();
    app.list();

    app.command()
        .args(["done", "1", "bad", "99"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("Invalid task ID: bad\nTask not found: 99\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n\x1b[48;2;220;224;232m2  inbox  cancel subscription\x1b[0m\n");

    app.command()
        .args(["delete", "2", "bad"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("Invalid task ID: bad\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n\x1b[48;2;220;224;232m2  inbox  cancel subscription\x1b[0m\n");
}

#[test]
fn commands_require_a_generated_task_list() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();

    app.command()
        .args(["done", "1"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("No task list has been generated yet; run ct list first.\n");
}

#[test]
fn visible_ids_stay_stable_until_the_next_list() {
    let app = TestApp::new();

    for title in ["first", "second", "third", "fourth"] {
        app.command().args(["quick", title]).assert().success();
    }

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  first\n\x1b[48;2;220;224;232m2  inbox  second\x1b[0m\n3  inbox  third\n\x1b[48;2;220;224;232m4  inbox  fourth\x1b[0m\n");

    app.command()
        .args(["done", "4"])
        .assert()
        .success()
        .stdout("Done 4: fourth\n");
    app.command()
        .args(["done", "3"])
        .assert()
        .success()
        .stdout("Done 3: third\n");
    app.command()
        .args(["done", "2"])
        .assert()
        .success()
        .stdout("Done 2: second\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  first\n");
}

#[test]
fn json_and_completed_lists_do_not_refresh_visible_ids() {
    let app = TestApp::new();

    app.command().args(["quick", "first"]).assert().success();
    app.command().args(["quick", "second"]).assert().success();
    app.list();
    app.command().args(["quick", "third"]).assert().success();

    app.command().args(["list", "--json"]).assert().success();
    app.command()
        .args(["done", "2"])
        .assert()
        .success()
        .stdout("Done 2: second\n");

    app.command().arg("completed").assert().success();
    app.command()
        .args(["done", "1"])
        .assert()
        .success()
        .stdout("Done 1: first\n");
}

#[test]
fn project_filtered_list_replaces_the_visible_id_mapping() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "inbox", "task"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "first"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "second"])
        .assert()
        .success();

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  first\n\x1b[48;2;220;224;232m2  Home  second\x1b[0m\n");

    app.command()
        .args(["done", "2"])
        .assert()
        .success()
        .stdout("Done 2: second\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  inbox task\n\x1b[48;2;220;224;232m2  Home   first\x1b[0m\n");
}

#[test]
fn delete_removes_task_so_later_commands_report_missing() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();
    app.command().args(["delete", "1"]).assert().success();

    app.command()
        .args(["done", "1"])
        .assert()
        .code(1)
        .stderr("Task not found: 1\n");
    app.command()
        .args(["_complete", "projects"])
        .assert()
        .success()
        .stdout("inbox\n");
}

#[test]
fn completed_lists_done_tasks_for_project_and_clear_deletes_them() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "clean", "gutters"])
        .assert()
        .success();
    app.command()
        .args(["add", "Work", "review", "pr"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1", "3"]).assert().success();
    let storage = couch_tasks::storage::Storage::open(app.db_path.clone()).unwrap();
    let home_uuid = storage.get_task(1).unwrap().unwrap().uuid;
    let work_uuid = storage.get_task(3).unwrap().unwrap().uuid;

    app.command()
        .args(["completed", "Home"])
        .assert()
        .success()
        .stdout(format!("{home_uuid}  Home  fix garage light\n"));

    app.command()
        .arg("completed")
        .assert()
        .success()
        .stdout(format!("{home_uuid}  Home  fix garage light\n\x1b[48;2;220;224;232m{work_uuid}  Work  review pr\x1b[0m\n"));

    let assert = app
        .command()
        .args(["completed", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0]["id"], 1);
    assert_eq!(tasks[0]["status"], "completed");
    assert_eq!(tasks[1]["id"], 3);
    assert_eq!(tasks[1]["status"], "completed");

    let assert = app
        .command()
        .args(["completed", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], 1);
    assert_eq!(tasks[0]["status"], "completed");
    assert!(tasks[0]["completed_at"].as_str().is_some());

    app.command()
        .args(["clear-completed", "Home"])
        .assert()
        .success()
        .stdout("Cleared 1 completed tasks from Home.\n");
    app.command()
        .args(["completed", "Home"])
        .assert()
        .success()
        .stdout("Home has no completed tasks.\n");
    app.command()
        .args(["completed", "Work"])
        .assert()
        .success()
        .stdout(format!("{work_uuid}  Work  review pr\n"));

    app.command()
        .arg("clear-completed")
        .assert()
        .success()
        .stdout("Cleared 1 completed tasks.\n");
    app.command()
        .args(["completed", "Work"])
        .assert()
        .success()
        .stdout("Work has no completed tasks.\n");
}

#[test]
fn undone_restores_completed_tasks_to_pending() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "fix", "garage", "light"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "clean", "gutters"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();

    app.command()
        .args(["undone", "1"])
        .assert()
        .success()
        .stdout("Undone 1: fix garage light\n");

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  clean gutters\n\x1b[48;2;220;224;232m2  Home  fix garage light\x1b[0m\n");

    let assert = app
        .command()
        .args(["list", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0]["id"], 2);
    assert_eq!(tasks[0]["sort_order"], 1);
    assert_eq!(tasks[1]["id"], 1);
    assert_eq!(tasks[1]["sort_order"], 2);
    assert_eq!(tasks[1]["status"], "pending");
    assert_eq!(tasks[1]["completed_at"], serde_json::Value::Null);

    let assert = app
        .command()
        .args(["completed", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(tasks.as_array().expect("json task list is array").len(), 0);
}

#[test]
fn undone_reports_errors_on_stderr_and_does_not_partially_apply() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();

    app.command()
        .args(["undone", "1", "bad", "99"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("Invalid task ID: bad\nTask not found: 99\n");

    app.command()
        .args(["completed", "inbox"])
        .assert()
        .success()
        .stdout(format!(
            "{}  inbox  buy milk\n",
            couch_tasks::storage::Storage::open(app.db_path.clone())
                .unwrap()
                .get_task(1)
                .unwrap()
                .unwrap()
                .uuid
        ));
}

#[test]
fn move_changes_project_and_creates_destination() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "fix", "garage", "light"])
        .assert()
        .success();
    app.list();

    app.command()
        .args(["move", "1", "Home"])
        .assert()
        .success()
        .stdout("Moved 1 to Home: fix garage light\n");

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  fix garage light\n");
}

#[test]
fn update_changes_task_fields_without_editor() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();

    app.command()
        .args([
            "update",
            "1",
            "--project",
            "Home",
            "--title",
            "replace air filter",
            "--notes",
            "Check size.",
        ])
        .assert()
        .success()
        .stdout("Updated 1: replace air filter\n");

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  replace air filter [note]\n");

    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout("1  Home  replace air filter\n\nNotes:\nCheck size.\n");
}

#[test]
fn update_can_clear_notes_and_rejects_missing_tasks() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();
    app.command()
        .args(["note", "1", "check", "coupon"])
        .assert()
        .success();

    app.command()
        .args(["update", "1", "--notes", ""])
        .assert()
        .success()
        .stdout("Updated 1: buy milk\n");

    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n\nNotes:\nNo notes.\n");

    app.command()
        .args(["update", "99", "--title", "missing"])
        .assert()
        .code(1)
        .stderr("Task not found: 99\n");
}

#[test]
fn reorder_changes_pending_task_order_within_project() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "first"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "second"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "third"])
        .assert()
        .success();
    app.command()
        .args(["add", "Work", "other"])
        .assert()
        .success();
    app.list();

    app.command()
        .args(["reorder", "3", "1"])
        .assert()
        .success()
        .stdout("Reordered 3 in Home to position 1: third\n");

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  third\n\x1b[48;2;220;224;232m2  Home  first\x1b[0m\n3  Home  second\n");
    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("4  Work  other"));

    app.command()
        .args(["reorder", "1", "99"])
        .assert()
        .code(1)
        .stderr("Invalid reorder position: 99; valid range is 1..=3\n");

    let assert = app
        .command()
        .args(["list", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");
    assert_eq!(tasks[0]["id"], 3);
    assert_eq!(tasks[0]["sort_order"], 1);
    assert_eq!(tasks[1]["id"], 1);
    assert_eq!(tasks[1]["sort_order"], 2);
    assert_eq!(tasks[2]["id"], 2);
    assert_eq!(tasks[2]["sort_order"], 3);
}

#[test]
fn done_and_undone_enforce_status_preconditions() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();

    app.command()
        .args(["done", "1"])
        .assert()
        .code(1)
        .stderr("Task is not pending: 1\n");

    app.command().args(["undone", "1"]).assert().success();

    app.command()
        .args(["undone", "1"])
        .assert()
        .code(1)
        .stderr("Task is not completed: 1\n");
}

#[test]
fn pending_sort_order_is_one_based_within_project_after_completion_and_add() {
    let app = TestApp::new();

    app.command()
        .args(["add", "Home", "first"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "second"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();
    app.command()
        .args(["add", "Home", "third"])
        .assert()
        .success();

    let assert = app
        .command()
        .args(["list", "--json", "Home"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let tasks: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let tasks = tasks.as_array().expect("json task list is array");

    assert_eq!(tasks[0]["id"], 2);
    assert_eq!(tasks[0]["sort_order"], 1);
    assert_eq!(tasks[1]["id"], 3);
    assert_eq!(tasks[1]["sort_order"], 2);
}

#[test]
fn free_form_task_text_can_contain_help_flags() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "read", "--help", "output"])
        .assert()
        .success()
        .stdout("Added to inbox: read --help output\n");
    app.list();

    app.command()
        .args(["note", "1", "check", "-h", "flag"])
        .assert()
        .success()
        .stdout("Noted 1: read --help output\n");

    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout("1  inbox  read --help output\n\nNotes:\ncheck -h flag\n");
}

#[test]
fn note_appends_without_changing_list_output() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();

    app.command()
        .args(["note", "1", "check", "coupon"])
        .assert()
        .success()
        .stdout("Noted 1: buy milk\n");

    app.command()
        .arg("list")
        .assert()
        .success()
        .stdout("1  inbox  buy milk [note]\n");

    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n\nNotes:\ncheck coupon\n");

    app.command()
        .args(["list", "--notes"])
        .assert()
        .success()
        .stdout("1  inbox  buy milk\n          note: check coupon\n");
}

#[cfg(unix)]
#[test]
fn edit_uses_editor_and_saves_valid_changes() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();

    let editor = app._dir.path().join("fake-editor.sh");
    fs::write(
        &editor,
        "#!/bin/sh\ncat > \"$1\" <<'EOF'\nProject: Home\nStatus: pending\n\nTitle:\nreplace air filter\n\nNotes:\nCheck size.\nEOF\n",
    )
    .expect("write fake editor");
    let mut permissions = fs::metadata(&editor).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).expect("chmod fake editor");

    app.command()
        .args(["edit", "1"])
        .env("EDITOR", &editor)
        .assert()
        .success()
        .stdout("");

    app.command()
        .arg("Home")
        .assert()
        .success()
        .stdout("1  Home  replace air filter [note]\n");
}

#[test]
fn sync_requires_configuration() {
    let app = TestApp::new();

    app.command()
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Missing sync URL: pass --url, set CT_SYNC_URL, or configure sync.url in",
        ))
        .stderr(predicate::str::contains("couch-tasks/config.json"));
}

#[test]
fn sync_rejects_malformed_client_config() {
    let app = TestApp::new();
    let config_home = app._dir.path().join("config");
    let config_dir = config_home.join("couch-tasks");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("config.json"), "not json").expect("write config");

    app.command()
        .env("XDG_CONFIG_HOME", &config_home)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid client config"));
}

#[test]
fn complete_projects_lists_active_projects_only() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.command()
        .args(["add", "Work", "review", "pr"])
        .assert()
        .success();
    app.command()
        .args(["add", "Home", "clean", "gutters"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();
    app.command().args(["delete", "3"]).assert().success();

    app.command()
        .args(["_complete", "projects"])
        .assert()
        .success()
        .stdout("inbox\nHome\n");
}

#[test]
fn list_project_reports_empty_and_missing_projects() {
    let app = TestApp::new();

    app.command()
        .args(["quick", "buy", "milk"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();

    app.command()
        .arg("inbox")
        .assert()
        .success()
        .stdout("inbox has no pending tasks.\n");

    app.command()
        .arg("Garage")
        .assert()
        .code(1)
        .stderr("Project not found: Garage\n");
}

#[test]
fn completed_uuids_select_the_right_task_without_replacing_pending_ids() {
    let app = TestApp::new();
    app.command()
        .args(["quick", "completed task"])
        .assert()
        .success();
    app.command()
        .args(["quick", "pending task"])
        .assert()
        .success();
    app.list();
    app.command().args(["done", "1"]).assert().success();
    app.list(); // Numeric ID 1 now refers to the other task.

    for args in [vec!["completed"], vec!["completed", "inbox"]] {
        let result = app.command().args(args).output().unwrap();
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout).unwrap();
        let uuid = output.split_whitespace().next().unwrap();
        assert!(uuid::Uuid::parse_str(uuid).is_ok());
        app.command()
            .args(["show", "1"])
            .assert()
            .success()
            .stdout(predicate::str::contains("pending task"));
        app.command().args(["undone", uuid]).assert().success();
        app.command().args(["done", uuid]).assert().success();
        // Both global and filtered output must expose the same safe selector.
        let storage = couch_tasks::storage::Storage::open(app.db_path.clone()).unwrap();
        assert_eq!(
            storage.get_task_by_uuid(uuid).unwrap().unwrap().title,
            "completed task"
        );
    }
    let storage = couch_tasks::storage::Storage::open(app.db_path.clone()).unwrap();
    let uuid = storage.list_completed_tasks().unwrap()[0].uuid.clone();
    app.command().args(["delete", &uuid]).assert().success();
    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pending task"));
    assert!(storage.list_completed_tasks().unwrap().is_empty());
}

#[test]
fn terminal_controls_are_escaped_but_machine_data_is_preserved() {
    use couch_tasks::output::{escape_terminal, escape_terminal_multiline};
    let app = TestApp::new();
    let project = "Home\x1b[2J";
    let title = "café\x1b]0;spoof\x07\rtitle";
    let notes = "first\x1b[2J\rline\nsecond\tline\u{009b}2J";
    app.command()
        .args(["add", project, title])
        .assert()
        .success()
        .stdout(format!(
            "Added to {}: {}\n",
            escape_terminal(project),
            escape_terminal(title)
        ));
    app.list();
    app.command()
        .args(["update", "1", "--notes", notes])
        .assert()
        .success()
        .stdout(format!("Updated 1: {}\n", escape_terminal(title)));
    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(format!(
            "1  {}  {}\n\nNotes:\n{}\n",
            escape_terminal(project),
            escape_terminal(title),
            escape_terminal_multiline(notes)
        ));
    app.command()
        .args(["list", "--notes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b").not())
        .stdout(predicate::str::contains("note: second\\tline\\u{9b}2J"));
    app.command()
        .arg("projects")
        .assert()
        .success()
        .stdout(predicate::str::contains(escape_terminal(project)));
    app.command()
        .args(["_complete", "projects"])
        .assert()
        .success()
        .stdout(predicate::str::contains(project));
    let result = app.command().args(["list", "--json"]).output().unwrap();
    let tasks: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(tasks[0]["project_name"], project);
    assert_eq!(tasks[0]["title"], title);
    assert_eq!(tasks[0]["notes"], notes);
    app.command()
        .args(["show", "bad\x1b[2J\r\nforged"])
        .assert()
        .code(1)
        .stderr("Invalid task ID: bad\\u{1b}[2J\\r\\nforged\n");
    app.command()
        .args(["done", "1"])
        .assert()
        .success()
        .stdout(format!("Done 1: {}\n", escape_terminal(title)));
    app.command()
        .args(["delete", "1"])
        .assert()
        .success()
        .stdout(format!("Removed 1: {}\n", escape_terminal(title)));
}
