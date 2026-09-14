//! ls / cd / pwd / cl.

mod common;

#[test]
fn ls_lists_groups_then_numbered_entries() {
    let mut h = common::harness();
    let out = h.ok("ls");
    let groups: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "=== Groups ===")
        .skip(1)
        .take_while(|l| *l != "=== Entries ===")
        .collect();
    assert_eq!(groups, ["Empty/", "Internet/", "Recycle Bin/", "Work/"]);
    assert!(out.contains("1. Sample Entry"), "{out}");
    assert!(out.contains("alice"), "{out}");
}

#[test]
fn ls_numbers_entries_alphabetically_and_marks_them() {
    let mut h = common::harness();
    let out = h.ok("ls /Internet");
    let entries: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "=== Entries ===")
        .skip(1)
        .collect();
    assert_eq!(entries.len(), 4, "{out}");
    assert!(entries[0].starts_with(r"1. Comcast\/Xfinity"), "{out}");
    assert!(entries[1].starts_with("2. GitHub [otp]"), "{out}");
    assert!(entries[1].ends_with("perlsaiyan"), "{out}");
    assert!(entries[2].starts_with("3. GitHub"), "{out}");
    assert!(entries[2].ends_with("someone-else"), "{out}");
}

#[test]
fn expired_entries_are_marked() {
    let mut h = common::harness();
    let out = h.ok("ls /Work/Servers");
    assert!(out.contains("1. db01 *EXPIRED*"), "{out}");
    assert!(out.contains("2. web01"), "{out}");
}

#[test]
fn the_last_listed_path_sets_the_numbered_listing() {
    let mut h = common::harness();
    h.ok("ls / /Internet");
    let out = h.ok("show 1");
    assert!(out.contains("Title: Comcast/Xfinity"), "{out}");
}

#[test]
fn cd_pwd_and_relative_paths() {
    let mut h = common::harness();
    assert_eq!(h.ok("pwd"), "/\n");
    h.ok("cd Work");
    assert_eq!(h.ok("pwd"), "/Work\n");
    h.ok("cd Servers");
    assert_eq!(h.ok("pwd"), "/Work/Servers\n");
    h.ok("cd ..");
    assert_eq!(h.ok("pwd"), "/Work\n");
    h.ok("cd /Internet");
    assert_eq!(h.ok("pwd"), "/Internet\n");
    h.ok("chdir /");
    assert_eq!(h.ok("pwd"), "/\n");
}

#[test]
fn cl_changes_group_and_lists_it() {
    let mut h = common::harness();
    let out = h.ok("cl /Work/Servers");
    assert!(out.contains("1. db01"), "{out}");
    assert_eq!(h.ok("pwd"), "/Work/Servers\n");
}

#[test]
fn dir_is_an_alias_for_ls() {
    let mut h = common::harness();
    assert_eq!(h.ok("dir"), h.ok("ls"));
}

#[test]
fn cd_into_a_missing_group_reports_an_error() {
    let mut h = common::harness();
    let out = h.run("cd /Nowhere");
    assert!(out.starts_with("error:"), "{out}");
    assert_eq!(h.ok("pwd"), "/\n");
}
