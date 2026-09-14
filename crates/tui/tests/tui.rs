//! End-to-end tests of the TUI against a `TestBackend` and the sample vault.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use tempfile::TempDir;

use chiave_clip::{ClipError, Clipboard, MemoryClipboard};
use chiave_core::{testdb, SecretString, Vault};
use chiave_tui::{App, NoPrompt, TuiOptions};

/// A `MemoryClipboard` the test keeps a handle on.
struct SharedClip(Arc<MemoryClipboard>);

impl Clipboard for SharedClip {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn copy_secret(
        &self,
        secret: &SecretString,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipError> {
        self.0.copy_secret(secret, clear_after)
    }
    fn copy_text(&self, text: &str) -> Result<(), ClipError> {
        self.0.copy_text(text)
    }
    fn clear(&self) -> Result<(), ClipError> {
        self.0.clear()
    }
}

struct Harness {
    app: App,
    clip: Arc<MemoryClipboard>,
    #[allow(dead_code)]
    dir: TempDir,
}

fn harness_with(opts: TuiOptions) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let (path, creds) = testdb::sample_file(dir.path());
    let vault = Vault::open(&path, &creds, opts.read_only).unwrap();
    let clip = Arc::new(MemoryClipboard::new());
    let app = App::new(
        vault,
        Box::new(SharedClip(Arc::clone(&clip))),
        opts,
        Box::new(NoPrompt),
    );
    Harness { app, clip, dir }
}

fn harness() -> Harness {
    harness_with(TuiOptions {
        clip_timeout: Some(Duration::from_secs(10)),
        idle_lock: None,
        read_only: false,
    })
}

impl Harness {
    fn key(&mut self, code: KeyCode) {
        self.app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ch(&mut self, c: char) {
        let mods = if c.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        self.app.handle_key(KeyEvent::new(KeyCode::Char(c), mods));
    }

    fn ctrl(&mut self, c: char) {
        self.app
            .handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.ch(c);
        }
    }

    fn render(&self) -> String {
        render_at(&self.app, 120, 30)
    }
}

fn buffer_text(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
        }
        out.push('\n');
    }
    out
}

fn render_at(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    buffer_text(terminal.backend().buffer())
}

#[test]
fn initial_render_shows_the_tree_and_the_root_entries() {
    let h = harness();
    let screen = h.render();
    for name in ["Sample", "Empty", "Internet", "Work", "Recycle Bin"] {
        assert!(screen.contains(name), "{name} missing from\n{screen}");
    }
    assert!(screen.contains("Sample Entry"), "{screen}");
    // The recycle bin is last in the tree.
    let bin = screen.find("Recycle Bin").unwrap();
    let work = screen.find("Work").unwrap();
    assert!(bin > work, "recycle bin should sort last\n{screen}");
}

#[test]
fn internet_group_lists_three_entries_and_marks_the_otp_one() {
    let mut h = harness();
    h.ch('j');
    h.ch('j'); // Sample -> Empty -> Internet
    assert_eq!(h.app.entries().len(), 3);
    let titles: Vec<&str> = h.app.entries().iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Comcast/Xfinity", "GitHub", "GitHub"]);
    assert_eq!(h.app.entries().iter().filter(|e| e.has_otp).count(), 1);
    let screen = h.render();
    assert!(screen.contains("[otp]"), "{screen}");
}

#[test]
fn detail_masks_the_password_until_v_reveals_it() {
    let mut h = harness();
    let screen = h.render();
    assert!(screen.contains("••••••••"), "{screen}");
    assert!(!screen.contains("s3cret"), "{screen}");
    h.ch('v');
    let screen = h.render();
    assert!(screen.contains("s3cret"), "{screen}");
    // Reveal is per entry: moving the selection hides it again.
    h.key(KeyCode::Tab);
    h.ch('j');
    h.ch('k');
    let screen = h.render();
    assert!(!screen.contains("s3cret"), "{screen}");
}

#[test]
fn expiry_and_totp_are_shown() {
    let mut h = harness();
    h.ch('j');
    h.ch('j'); // Internet
    h.key(KeyCode::Tab);
    h.ch('j'); // first GitHub
    let screen = h.render();
    assert!(screen.contains("TOTP"), "{screen}");
    // The countdown bar is drawn next to the code.
    assert!(screen.contains('█') || screen.contains('░'), "{screen}");
}

#[test]
fn y_copies_the_password_with_the_timeout() {
    let mut h = harness();
    h.ch('y');
    let state = h.clip.state();
    assert_eq!(state.content.as_deref(), Some("s3cret"));
    assert!(state.sensitive);
    assert_eq!(state.clear_after, Some(Duration::from_secs(10)));
    assert!(
        h.app.message().contains("clears in 10s"),
        "{}",
        h.app.message()
    );
    assert!(h.render().contains("clears in 10s"));
}

#[test]
fn u_copies_the_username_as_plain_text_and_x_clears() {
    let mut h = harness();
    h.ch('u');
    let state = h.clip.state();
    assert_eq!(state.content.as_deref(), Some("alice"));
    assert!(!state.sensitive);
    assert_eq!(state.clear_after, None);
    h.ch('x');
    assert_eq!(h.clip.state().content, None);
    assert_eq!(h.clip.state().clears, 1);
}

#[test]
fn slash_git_filters_to_two_results() {
    let mut h = harness();
    h.ch('/');
    h.type_text("git");
    assert_eq!(h.app.search().hits.len(), 2);
    for hit in &h.app.search().hits {
        assert_eq!(hit.title, "GitHub");
    }
    let screen = h.render();
    assert!(screen.contains("/Internet/GitHub"), "{screen}");
    h.key(KeyCode::Esc);
    assert!(!h.app.search().active);
    assert!(h.render().contains("Sample Entry"));
}

#[test]
fn new_entry_form_creates_an_entry() {
    let mut h = harness();
    h.ch('n');
    h.type_text("Demo");
    h.key(KeyCode::Tab);
    h.type_text("demo-user");
    h.ctrl('g'); // generate a password
    h.ctrl('s');
    let vault = h.app.vault().unwrap();
    let root = vault.db().root();
    let created = root
        .entries()
        .find(|e| e.get_title() == Some("Demo"))
        .expect("entry was created");
    assert_eq!(created.get_username(), Some("demo-user"));
    assert!(created.get_password().map(|p| p.len()).unwrap_or(0) >= 8);
    assert!(h.app.has_unsaved_changes());
    assert!(h.app.entries().iter().any(|e| e.title == "Demo"));
}

#[test]
fn edit_form_updates_an_entry() {
    let mut h = harness();
    h.ch('e');
    // Title is focused; clear it and type a new one.
    for _ in 0.."Sample Entry".len() {
        h.key(KeyCode::Backspace);
    }
    h.type_text("Renamed");
    h.ctrl('s');
    let vault = h.app.vault().unwrap();
    assert!(vault
        .db()
        .root()
        .entries()
        .any(|e| e.get_title() == Some("Renamed")));
}

#[test]
fn delete_moves_the_entry_to_the_recycle_bin() {
    let mut h = harness();
    h.key(KeyCode::Tab); // focus the entry list
    h.ch('d');
    let screen = h.render();
    assert!(screen.contains("recycle bin"), "{screen}");
    h.ch('y');
    let vault = h.app.vault().unwrap();
    let bin = vault.recycle_bin_id().unwrap();
    let bin_titles: Vec<String> = vault
        .children(bin)
        .unwrap()
        .entries
        .iter()
        .map(|e| e.title.clone())
        .collect();
    assert!(
        bin_titles.contains(&"Sample Entry".to_string()),
        "{bin_titles:?}"
    );
    assert!(h.app.entries().iter().all(|e| e.title != "Sample Entry"));
}

#[test]
fn save_clears_the_unsaved_marker() {
    let mut h = harness();
    h.ch('n');
    h.type_text("Temp");
    h.ctrl('s');
    assert!(h.app.has_unsaved_changes());
    assert!(h.render().contains('*'));
    h.ch('s');
    assert!(!h.app.has_unsaved_changes());
    let screen = h.render();
    assert!(screen.contains("Saved"), "{screen}");
}

#[test]
fn lock_drops_the_vault_and_hides_its_contents() {
    let mut h = harness();
    h.ch('L');
    assert!(h.app.is_locked());
    assert!(h.app.vault().is_none());
    let screen = h.render();
    assert!(screen.contains("Locked"), "{screen}");
    assert!(screen.contains("Password"), "{screen}");
    for secret in ["Sample Entry", "Internet", "alice", "s3cret", "GitHub"] {
        assert!(!screen.contains(secret), "{secret} leaked into\n{screen}");
    }
    // The typed password is masked, and the right one unlocks.
    h.type_text(testdb::PASSWORD);
    assert!(!h.render().contains(testdb::PASSWORD));
    h.key(KeyCode::Enter);
    assert!(!h.app.is_locked());
    assert!(h.render().contains("Sample Entry"));
}

#[test]
fn a_wrong_password_says_so_and_stays_locked() {
    let mut h = harness();
    h.ch('L');
    h.type_text("not-the-password");
    h.key(KeyCode::Enter);
    assert!(h.app.is_locked());
    assert!(h.render().contains("Wrong password"));
}

#[test]
fn locking_with_unsaved_changes_asks_first() {
    let mut h = harness();
    h.ch('n');
    h.type_text("Pending");
    h.ctrl('s');
    h.ch('L');
    assert!(!h.app.is_locked(), "should ask before locking");
    let screen = h.render();
    assert!(screen.contains("Save and lock"), "{screen}");
    h.ch('d'); // lock and discard
    assert!(h.app.is_locked());
}

#[test]
fn idle_lock_triggers_after_the_configured_duration() {
    let mut h = harness_with(TuiOptions {
        clip_timeout: None,
        idle_lock: Some(Duration::from_secs(300)),
        read_only: false,
    });
    let start = Instant::now();
    h.app.tick(start + Duration::from_secs(120));
    assert!(!h.app.is_locked());
    h.app.tick(start + Duration::from_secs(301));
    assert!(h.app.is_locked());
    assert!(h.app.vault().is_none());
}

#[test]
fn read_only_vaults_refuse_mutations() {
    let mut h = harness_with(TuiOptions {
        clip_timeout: None,
        idle_lock: None,
        read_only: true,
    });
    let screen = h.render();
    assert!(screen.contains("[RO]"), "{screen}");
    h.ch('n');
    assert!(h.app.message().contains("read-only"), "{}", h.app.message());
    assert!(!h.app.has_unsaved_changes());
}

#[test]
fn quitting_with_unsaved_changes_asks_first() {
    let mut h = harness();
    h.ch('n');
    h.type_text("Later");
    h.ctrl('s');
    h.ch('q');
    assert!(!h.app.should_quit());
    let screen = h.render();
    assert!(screen.contains("Save and quit"), "{screen}");
    h.ch('c'); // cancel
    assert!(!h.app.should_quit());
    h.ch('q');
    h.ch('d'); // discard
    assert!(h.app.should_quit());
}

#[test]
fn new_group_and_rename_group() {
    let mut h = harness();
    h.ch('N');
    h.type_text("Cloud");
    h.key(KeyCode::Enter);
    assert!(h.render().contains("Cloud"));
    // Select it and rename.
    h.ch('j');
    assert_eq!(
        h.app.tree().selected_row().map(|r| r.name.as_str()),
        Some("Cloud")
    );
    h.ch('r');
    for _ in 0.."Cloud".len() {
        h.key(KeyCode::Backspace);
    }
    h.type_text("Servers");
    h.key(KeyCode::Enter);
    let screen = h.render();
    assert!(screen.contains("Servers"), "{screen}");
    assert!(!screen.contains("Cloud"), "{screen}");
}

#[test]
fn move_picker_moves_an_entry() {
    let mut h = harness();
    h.key(KeyCode::Tab); // entries
    h.ch('m');
    let screen = h.render();
    assert!(screen.contains("Move Sample Entry"), "{screen}");
    h.ch('j'); // first child group: Empty
    h.key(KeyCode::Enter);
    let vault = h.app.vault().unwrap();
    let empty = vault
        .children(vault.root())
        .unwrap()
        .groups
        .iter()
        .find(|g| g.name == "Empty")
        .unwrap()
        .id;
    assert!(vault
        .children(empty)
        .unwrap()
        .entries
        .iter()
        .any(|e| e.title == "Sample Entry"));
}

#[test]
fn help_overlay_lists_the_keys() {
    let mut h = harness();
    h.ch('?');
    let screen = h.render();
    assert!(screen.contains("Keys"), "{screen}");
    assert!(screen.contains("fuzzy search"), "{screen}");
    h.key(KeyCode::Esc);
    assert!(!h.render().contains("fuzzy search"));
}

#[test]
fn generator_options_popup_sets_the_length() {
    let mut h = harness();
    h.ch('n');
    h.app
        .handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::ALT));
    let screen = h.render();
    assert!(screen.contains("Generate password"), "{screen}");
    for _ in 0..4 {
        h.ch('l'); // longer
    }
    h.ch('s'); // no specials
    h.key(KeyCode::Enter);
    h.type_text("Gen");
    h.ctrl('s');
    let vault = h.app.vault().unwrap();
    let root = vault.db().root();
    let e = root
        .entries()
        .find(|e| e.get_title() == Some("Gen"))
        .unwrap();
    let password = e.get_password().unwrap();
    assert_eq!(password.chars().count(), 24);
    assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn narrow_and_wide_layouts_render_without_panicking() {
    let mut h = harness();
    for (w, hgt) in [(50u16, 15u16), (60, 20), (99, 24), (200, 50)] {
        let screen = render_at(&h.app, w, hgt);
        assert!(!screen.is_empty());
    }
    // At 50 columns only the focused pane is drawn.
    let screen = render_at(&h.app, 50, 15);
    assert!(screen.contains("Groups"), "{screen}");
    assert!(!screen.contains("Details"), "{screen}");
    h.key(KeyCode::Tab);
    let screen = render_at(&h.app, 50, 15);
    assert!(screen.contains("Entries"), "{screen}");
    assert!(!screen.contains("Groups"), "{screen}");
    // Between 60 and 100 the tree only shows when it has the focus.
    let screen = render_at(&h.app, 90, 20);
    assert!(!screen.contains("Groups"), "{screen}");
    assert!(screen.contains("Details"), "{screen}");
}
