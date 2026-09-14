//! Tab completion.

mod common;

use chiave_shell::complete;

fn at_end(h: &common::Harness, line: &str) -> (usize, Vec<String>) {
    complete(&h.shell, line, line.len())
}

#[test]
fn completes_command_names_at_word_zero() {
    let h = common::harness();
    let (start, names) = at_end(&h, "l");
    assert_eq!(start, 0);
    assert_eq!(names, ["lock", "ls"]);
    let (start, names) = at_end(&h, "");
    assert_eq!(start, 0);
    assert!(names.contains(&"show".to_string()));
    assert!(names.contains(&"dir".to_string()));
}

#[test]
fn completes_groups_and_entries_for_show() {
    let h = common::harness();
    let (start, items) = at_end(&h, "show /Internet/Git");
    assert_eq!(start, 5);
    assert_eq!(items, ["/Internet/GitHub"]);
}

#[test]
fn cd_and_ls_complete_groups_only() {
    let h = common::harness();
    let (start, items) = at_end(&h, "cd Inte");
    assert_eq!(start, 3);
    assert_eq!(items, ["Internet/"]);
    let (_, items) = at_end(&h, "ls ");
    assert_eq!(items, ["Empty/", "Internet/", "'Recycle Bin/'", "Work/"]);
    // No entries offered even though the root group has one.
    assert!(!items.iter().any(|i| i.contains("Sample")));
}

#[test]
fn entry_names_are_escaped_and_quoted() {
    let h = common::harness();
    let (_, items) = at_end(&h, "show /Internet/Com");
    assert_eq!(items, [r"'/Internet/Comcast\/Xfinity'"]);
    let (_, items) = at_end(&h, "show /Sam");
    assert_eq!(items, ["'/Sample Entry'"]);
}

#[test]
fn completion_is_relative_to_the_current_group() {
    let mut h = common::harness();
    h.ok("cd /Work");
    let (_, items) = at_end(&h, "ls Serv");
    assert_eq!(items, ["Servers/"]);
    let (_, items) = at_end(&h, "show Servers/db");
    assert_eq!(items, ["Servers/db01"]);
}

#[test]
fn a_partial_word_already_quoted_still_completes() {
    let h = common::harness();
    let (start, items) = at_end(&h, "show '/Sample En");
    assert_eq!(start, 5);
    assert_eq!(items, ["'/Sample Entry'"]);
    let (_, items) = at_end(&h, r"show /Sample\ En");
    assert_eq!(items, ["'/Sample Entry'"]);
}

#[test]
fn an_unknown_parent_group_yields_nothing() {
    let h = common::harness();
    let (_, items) = at_end(&h, "show /Nowhere/x");
    assert!(items.is_empty());
}
