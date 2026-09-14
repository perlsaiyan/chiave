//! Groups, moving, copying and attachments. Creating and editing entries lives
//! in `editing.rs`; saving and the database-level commands in `save.rs`.

mod common;

// ----- groups --------------------------------------------------------------

#[test]
fn mkdir_rename_and_rmdir() {
    let mut h = common::harness();
    assert!(h
        .ok("mkdir /Internet/Forums")
        .contains("Created /Internet/Forums"));
    assert!(h.ok("ls /Internet").contains("Forums/"));

    let out = h.ok("rename /Internet/Forums Boards");
    assert!(
        out.contains("Renamed /Internet/Forums to /Internet/Boards"),
        "{out}"
    );

    // A non-empty group needs -r.
    h.ok("new --title Post /Internet/Boards/Post");
    let out = h.run("rmdir /Internet/Boards");
    assert!(out.contains("is not empty"), "{out}");
    assert!(out.contains("-r"), "{out}");

    let out = h.ok("rmdir -r /Internet/Boards");
    assert!(
        out.contains("Moved /Internet/Boards to /Recycle Bin/Boards"),
        "{out}"
    );
}

#[test]
fn rmdir_permanent_skips_the_recycle_bin() {
    let mut h = common::harness();
    let out = h.ok("rmdir --permanent /Empty");
    assert!(out.contains("Deleted /Empty"), "{out}");
    assert!(h.run("cd /Empty").contains("not found"));
    assert!(!h.ok("ls /'Recycle Bin'").contains("Empty"));
}

#[test]
fn mkdir_refuses_a_duplicate() {
    let mut h = common::harness();
    assert!(h.run("mkdir /Internet").contains("already exists"));
}

// ----- rm / mv / cp --------------------------------------------------------

#[test]
fn rm_asks_first_and_uses_the_recycle_bin() {
    let mut h = common::harness();
    h.script(["n"]);
    assert_eq!(h.ok("rm /Work/Servers/web01"), "Cancelled.\n");
    assert!(h
        .shell
        .vault()
        .unwrap()
        .resolve_entry("/Work/Servers/web01")
        .is_ok());

    let script = h.script(["y"]);
    let out = h.ok("rm /Work/Servers/web01");
    assert_eq!(script.remaining(), 0);
    assert!(
        out.contains("Moved /Work/Servers/web01 to /Recycle Bin/web01"),
        "{out}"
    );
}

#[test]
fn rm_permanent_and_force_skip_both_the_prompt_and_the_bin() {
    let mut h = common::harness();
    let out = h.ok("rm -f --permanent /Work/Servers/db01");
    assert!(out.contains("Deleted /Work/Servers/db01"), "{out}");
    assert!(h.run("show /Work/Servers/db01").contains("not"));
    assert!(!h.ok("ls /'Recycle Bin'").contains("db01"));
}

#[test]
fn mv_moves_entries_and_groups() {
    let mut h = common::harness();
    let out = h.ok("mv '/Sample Entry' /Work");
    assert!(
        out.contains("Moved /Sample Entry to /Work/Sample Entry"),
        "{out}"
    );

    let out = h.ok("mv /Work/Servers /Internet");
    assert!(
        out.contains("Moved /Work/Servers to /Internet/Servers"),
        "{out}"
    );
    assert!(h.ok("ls /Internet").contains("Servers/"));

    assert!(h.run("mv /Internet /Internet/Servers").contains("itself"));
}

#[test]
fn cp_copies_into_a_group_or_under_a_new_title() {
    let mut h = common::harness();
    let out = h.ok("cp '/Sample Entry' /Work");
    assert!(
        out.contains("Copied /Sample Entry to /Work/Sample Entry"),
        "{out}"
    );
    assert_eq!(h.password("/Work/Sample Entry"), "s3cret");

    let out = h.ok("copy '/Sample Entry' '/Work/Second Copy'");
    assert!(out.contains("to /Work/Second Copy"), "{out}");
    assert_eq!(h.entry("/Work/Second Copy").attachments, ["note.txt"]);
}

#[test]
fn clone_copies_then_edits_the_copy() {
    let mut h = common::harness();
    let script = h.script([
        "",     // Title: keep "Clone"
        "dave", // Username
        "",     // Password: keep
        "",     // URL
        "",     // Notes
        "",     // PIN
        "",     // Plain custom
    ]);
    let out = h.ok("clone '/Sample Entry' /Work/Clone");
    assert_eq!(script.remaining(), 0);
    assert!(out.contains("Copied /Sample Entry to /Work/Clone"), "{out}");
    assert!(out.contains("Updated /Work/Clone"), "{out}");
    assert_eq!(h.entry("/Work/Clone").username.as_deref(), Some("dave"));
    // The original is untouched.
    assert_eq!(h.entry("/Sample Entry").username.as_deref(), Some("alice"));
}

// ----- attachments ---------------------------------------------------------

#[test]
fn attach_lists_adds_exports_and_removes() {
    let mut h = common::harness();
    let out = h.ok("attach '/Sample Entry'");
    assert!(out.contains("note.txt"), "{out}");
    assert!(out.contains("5 bytes"), "{out}");

    let src = h.dir.path().join("key.pem");
    std::fs::write(&src, b"-----BEGIN-----\n").unwrap();
    let out = h.ok(&format!("attach '/Sample Entry' --add {}", src.display()));
    assert!(out.contains("Attached key.pem (16 bytes)"), "{out}");

    let out = h.ok(&format!(
        "attach '/Sample Entry' --add {} --name renamed.pem",
        src.display()
    ));
    assert!(out.contains("Attached renamed.pem"), "{out}");
    let listed = h.ok("attach '/Sample Entry'");
    for name in ["key.pem", "note.txt", "renamed.pem"] {
        assert!(listed.contains(name), "{listed}");
    }

    let dest = h.dir.path().join("out.txt");
    let out = h.ok(&format!(
        "attach '/Sample Entry' --export note.txt {}",
        dest.display()
    ));
    assert!(out.contains("5 bytes"), "{out}");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello");

    assert!(h
        .ok("attach '/Sample Entry' --rm key.pem")
        .contains("Removed attachment key.pem"));
    assert!(!h.ok("attach '/Sample Entry'").contains("key.pem"));
    assert!(h
        .run("attach '/Sample Entry' --rm nope")
        .contains("not found"));
}

#[test]
fn attach_reports_an_entry_without_attachments() {
    let mut h = common::harness();
    assert_eq!(h.ok("attach /Work/Servers/web01"), "No attachments.\n");
}

// ----- pwgen ---------------------------------------------------------------

#[test]
fn pwgen_prints_passwords() {
    let mut h = common::harness();
    let out = h.ok("pwgen --count 3 --length 12");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert_eq!(line.chars().count(), 12, "{line}");
    }
    assert_ne!(lines[0], lines[1]);

    let out = h.ok("pwgen --length 8 --no-special");
    assert!(
        out.trim().chars().all(|c| c.is_ascii_alphanumeric()),
        "{out}"
    );
    assert!(h.run("pwgen --words 4").contains("no word list configured"));
}

#[test]
fn pwgen_uses_a_configured_word_list() {
    let mut h = common::harness();
    let list = h.dir.path().join("words.txt");
    std::fs::write(&list, "11111 alpha\n11112 bravo\n11113 charlie\n").unwrap();
    h.shell.options_mut().pwwords = Some(list);
    let out = h.ok("pwgen --words 4");
    let phrase = out.trim();
    assert_eq!(phrase.split('-').count(), 4, "{phrase}");
    for word in phrase.split('-') {
        assert!(["alpha", "bravo", "charlie"].contains(&word), "{phrase}");
    }
}

// ----- help ----------------------------------------------------------------

#[test]
fn help_lists_every_write_command() {
    let mut h = common::harness();
    let out = h.ok("help");
    for name in [
        "mkdir", "rmdir", "rename", "new", "edit", "set", "rm", "mv", "cp", "clone", "attach",
        "save", "saveas", "passwd", "newdb", "upgrade", "pwgen",
    ] {
        assert!(out.contains(name), "`help` never mentions {name}:\n{out}");
    }
    assert!(h.ok("help new").contains("--password-from-stdin"));
    assert!(h.ok("help rmdir").contains("recursive"));
    assert!(
        h.ok("help copy").contains("Copy an entry"),
        "aliases resolve"
    );
}
