//! Idle lock, explicit lock/close/open and batch mode.

mod common;

use std::time::Duration;

use chiave_clip::NullClipboard;
use chiave_core::testdb;
use chiave_shell::{run_commands, FixedPrompt, Flow, Shell, ShellOptions};

#[test]
fn the_idle_lock_reprompts_for_the_master_password() {
    let mut h = common::harness();
    h.ok("cd /Internet");
    assert_eq!(h.prompt.calls(), 0);
    h.shell.options_mut().timeout = Some(Duration::ZERO);
    let out = h.ok("pwd");
    assert!(out.contains("Idle for too long"), "{out}");
    assert!(out.contains("Unlocked"), "{out}");
    assert_eq!(h.prompt.calls(), 1);
    // The current group survives the round trip.
    assert!(out.trim_end().ends_with("/Internet"), "{out}");
}

#[test]
fn exempt_commands_do_not_trigger_the_idle_lock() {
    let mut h = common::harness();
    h.shell.options_mut().timeout = Some(Duration::ZERO);
    h.ok("ver");
    h.ok("help");
    assert_eq!(h.prompt.calls(), 0);
    assert!(!h.shell.is_locked());
}

#[test]
fn lock_then_a_command_unlocks_again() {
    let mut h = common::harness();
    let out = h.ok("lock");
    assert!(out.contains("Locked"), "{out}");
    assert!(h.shell.is_locked());
    assert_eq!(h.shell.prompt_string(), "chiave:[locked]> ");
    let out = h.ok("pwd");
    assert!(out.contains("Unlocked"), "{out}");
    assert_eq!(h.prompt.calls(), 1);
    assert!(!h.shell.is_locked());
}

#[test]
fn close_forgets_the_database() {
    let mut h = common::harness();
    assert!(h.ok("close").contains("Closed"));
    assert!(!h.shell.is_locked());
    assert_eq!(h.shell.prompt_string(), "chiave> ");
    let out = h.run("ls");
    assert!(out.contains("no database is open"), "{out}");
}

#[test]
fn open_loads_a_database_in_the_repl() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, _creds) = testdb::sample_file(dir.path());
    let mut shell = Shell::new(Box::new(NullClipboard), ShellOptions::default());
    shell.set_prompt(Box::new(FixedPrompt::new(testdb::PASSWORD)));
    let mut out = Vec::new();
    let line = format!("open {}", shell_words::quote(&path.to_string_lossy()));
    shell.run_line(&line, &mut out).expect("open");
    let text = String::from_utf8(out).expect("utf-8");
    assert!(text.contains("Opened"), "{text}");
    assert!(shell.vault().is_some());
    assert_eq!(shell.prompt_string(), "chiave:/> ");
}

#[test]
fn open_with_a_wrong_password_reports_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, _creds) = testdb::sample_file(dir.path());
    let mut shell = Shell::new(Box::new(NullClipboard), ShellOptions::default());
    shell.set_prompt(Box::new(FixedPrompt::new("nope")));
    let mut out = Vec::new();
    let line = format!("open {}", shell_words::quote(&path.to_string_lossy()));
    shell.run_line(&line, &mut out).expect("open");
    let text = String::from_utf8(out).expect("utf-8");
    assert!(text.contains("error:"), "{text}");
    assert!(shell.vault().is_none());
}

#[test]
fn batch_mode_runs_a_list_of_commands() {
    let mut h = common::harness();
    let cmds: Vec<String> = ["cd /Internet", "ls", "get 3 username"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::new();
    let flow = run_commands(&mut h.shell, &cmds, &mut out).expect("batch");
    let text = String::from_utf8(out).expect("utf-8");
    assert_eq!(flow, Flow::Continue);
    assert!(text.contains("3. GitHub"), "{text}");
    assert!(text.trim_end().ends_with("someone-else"), "{text}");
}

#[test]
fn batch_mode_stops_at_quit() {
    let mut h = common::harness();
    let cmds: Vec<String> = ["pwd", "quit", "stats"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::new();
    let flow = run_commands(&mut h.shell, &cmds, &mut out).expect("batch");
    let text = String::from_utf8(out).expect("utf-8");
    assert_eq!(flow, Flow::Quit);
    assert!(!text.contains("Entries:"), "{text}");
}

#[test]
fn one_shot_exec_propagates_errors() {
    let mut h = common::harness();
    let mut out = Vec::new();
    let err = h
        .shell
        .exec(
            chiave_shell::Command::Cd {
                path: Some("/Nowhere".into()),
            },
            &mut out,
        )
        .expect_err("missing group");
    assert!(err.to_string().contains("not found"), "{err}");
}
