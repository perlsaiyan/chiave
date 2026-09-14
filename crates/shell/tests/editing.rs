//! `new`, `edit` and `set`: the interactive entry flows, driven by a scripted
//! prompt so every question and answer is spelled out here.

mod common;

use chiave_core::FieldValue;

// ----- new -----------------------------------------------------------------

#[test]
fn new_asks_for_every_field_and_can_generate_the_password() {
    let mut h = common::harness();
    let script = h.script([
        "",                    // Title: keep the default from the path
        "tom",                 // Username
        "",                    // Password: empty offers generation
        "g",                   // generate a random one
        "",                    // Accept? [Y/n/show]
        "https://lobste.rs",   // URL
        r"first line\nsecond", // Notes
        "PIN",                 // Add another field?
        "y",                   // protected?
        "4242",                // PIN value
        "",                    // no more fields
    ]);
    let out = h.ok("new /Internet/Lobsters");
    assert_eq!(script.remaining(), 0, "the whole script was consumed");
    assert!(out.contains("Created /Internet/Lobsters"), "{out}");
    assert!(out.contains("Generated 20 characters"), "{out}");
    assert!(out.contains("bits of entropy"), "{out}");

    let password = h.password("/Internet/Lobsters");
    assert_eq!(password.chars().count(), 20, "the default length");
    assert!(
        !out.contains(&password),
        "the generated password leaked:\n{out}"
    );

    let view = h.entry("/Internet/Lobsters");
    assert_eq!(view.title, "Lobsters");
    assert_eq!(view.username.as_deref(), Some("tom"));
    assert_eq!(view.url.as_deref(), Some("https://lobste.rs"));
    assert_eq!(view.notes.as_deref(), Some("first line\nsecond"));
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].0, "PIN");
    assert!(matches!(view.custom[0].1, FieldValue::Protected(_)));
}

#[test]
fn new_shows_a_generated_password_only_when_asked() {
    let mut h = common::harness();
    h.script(["", "", "", "g", "show", "y", "", "", ""]);
    let out = h.ok("new /Shown");
    let password = h.password("/Shown");
    assert!(out.contains(&password), "`show` should print it:\n{out}");
}

#[test]
fn new_with_flags_asks_nothing() {
    let mut h = common::harness();
    // No prompt is set up, so any question would fail the test.
    let out = h.ok("new --title Router --user admin --url http://10.0.0.1 --notes 'a\\nb' --generate --length 32 --no-special /Work/Router");
    assert!(out.contains("Created /Work/Router"), "{out}");
    assert!(out.contains("Generated 32 characters"), "{out}");

    let password = h.password("/Work/Router");
    assert_eq!(password.chars().count(), 32);
    assert!(
        password.chars().all(|c| c.is_ascii_alphanumeric()),
        "{password}"
    );
    assert!(!out.contains(&password), "{out}");

    let view = h.entry("/Work/Router");
    assert_eq!(view.username.as_deref(), Some("admin"));
    assert_eq!(view.notes.as_deref(), Some("a\nb"));
}

#[test]
fn new_needs_a_title() {
    let mut h = common::harness();
    assert!(h.run("new --user nobody").contains("needs a title"));
}

// ----- edit ----------------------------------------------------------------

#[test]
fn edit_changes_the_fields_the_user_answers() {
    let mut h = common::harness();
    let script = h.script([
        "",    // Title: keep
        "bob", // Username: change
        "",    // Password: keep
        "",    // URL: keep
        "",    // Notes: keep
        "",    // PIN (protected): keep
        "d",   // Plain custom: delete
    ]);
    let out = h.ok("edit '/Sample Entry'");
    assert_eq!(script.remaining(), 0);
    assert!(out.contains("Updated /Sample Entry"), "{out}");

    let view = h.entry("/Sample Entry");
    assert_eq!(view.title, "Sample Entry");
    assert_eq!(view.username.as_deref(), Some("bob"));
    assert_eq!(h.password("/Sample Entry"), "s3cret");
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].0, "PIN");
    // One history entry: the edit, not one per field.
    assert_eq!(h.entry("/Sample Entry").history_count, 1);
}

#[test]
fn edit_with_no_answers_reports_no_changes() {
    let mut h = common::harness();
    h.script(["", "", "", "", "", "", ""]);
    assert_eq!(h.ok("edit '/Sample Entry'"), "No changes.\n");
    assert!(!h.shell.is_dirty());
}

#[test]
fn edit_can_generate_a_new_password() {
    let mut h = common::harness();
    h.script(["", "", "g", "", "", "", "", ""]);
    let out = h.ok("edit '/Sample Entry'");
    assert!(out.contains("Generated 20 characters"), "{out}");
    let password = h.password("/Sample Entry");
    assert_ne!(password, "s3cret");
    assert_eq!(password.chars().count(), 20);
    assert!(!out.contains(&password), "{out}");
}

// ----- set -----------------------------------------------------------------

#[test]
fn set_writes_one_field() {
    let mut h = common::harness();
    assert!(h
        .ok("set '/Sample Entry' url https://new.example")
        .contains("Updated"));
    assert_eq!(h.ok("get '/Sample Entry' url"), "https://new.example\n");
    // The same value again changes nothing.
    assert_eq!(
        h.ok("set '/Sample Entry' url https://new.example"),
        "No changes.\n"
    );
    // Aliases go through fields::canonical.
    h.ok("set '/Sample Entry' uname carol");
    assert_eq!(h.ok("get '/Sample Entry' username"), "carol\n");
    // A new custom field.
    h.ok("set '/Sample Entry' Locker 42");
    assert_eq!(h.ok("get '/Sample Entry' Locker"), "42\n");
}

#[test]
fn set_asks_for_a_missing_value_and_masks_secrets() {
    let mut h = common::harness();
    let script = h.script(["hunter2"]);
    h.ok("set '/Sample Entry' password");
    assert_eq!(script.remaining(), 0);
    assert_eq!(h.password("/Sample Entry"), "hunter2");
    assert!(
        script.asked()[0].starts_with("Password"),
        "{:?}",
        script.asked()
    );
}

#[test]
fn set_expires_takes_a_date_or_never() {
    let mut h = common::harness();
    h.ok("set '/Sample Entry' expires 2030-01-02");
    assert!(h
        .ok("show -a '/Sample Entry'")
        .contains("Expires: 2030-01-02"));
    h.ok("set '/Sample Entry' expires never");
    assert!(h.ok("show -a '/Sample Entry'").contains("Expires: never"));
    assert!(h
        .run("set '/Sample Entry' expires soon")
        .contains("not a date"));
}

#[test]
fn set_delete_removes_a_custom_field_only() {
    let mut h = common::harness();
    assert!(h.ok("set '/Sample Entry' PIN --delete").contains("Updated"));
    assert!(!h.ok("show '/Sample Entry'").contains("PIN:"));
    assert!(h
        .run("set '/Sample Entry' PIN --delete")
        .contains("no such field"));
    let out = h.run("set '/Sample Entry' password --delete");
    assert!(out.contains("standard field"), "{out}");
}
