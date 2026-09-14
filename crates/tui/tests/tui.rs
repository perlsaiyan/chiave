//! End-to-end tests of the TUI against a `TestBackend` and the sample vault.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;
use tempfile::TempDir;

use chiave_clip::{ClipError, Clipboard, MemoryClipboard};
use chiave_core::{testdb, SecretString, Vault};
use chiave_tui::{App, Focus, Mode, NoPrompt, TuiOptions};

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
        ..TuiOptions::default()
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

    /// Draw once at the standard test size, so the app knows where its panes
    /// are; mouse events are hit-tested against the last frame's geometry.
    fn draw(&self) {
        let _ = self.render();
    }

    fn mouse_at(&mut self, kind: MouseEventKind, x: u16, y: u16, now: Instant) {
        self.app.handle_mouse_at(
            MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
            now,
        );
    }

    fn click_at(&mut self, x: u16, y: u16, now: Instant) {
        self.mouse_at(MouseEventKind::Down(MouseButton::Left), x, y, now);
    }

    /// Draw, then left-click at `(x, y)`.
    fn click(&mut self, x: u16, y: u16) {
        self.draw();
        self.click_at(x, y, Instant::now());
    }

    /// Two clicks on the same spot, 100 ms apart.
    fn double_click(&mut self, x: u16, y: u16) {
        self.draw();
        let now = Instant::now();
        self.click_at(x, y, now);
        self.draw();
        self.click_at(x, y, now + Duration::from_millis(100));
    }

    /// One wheel notch at `(x, y)`.
    fn wheel(&mut self, kind: MouseEventKind, x: u16, y: u16) {
        self.draw();
        self.mouse_at(kind, x, y, Instant::now());
    }

    /// Select the Internet group (tree row 2), which has four entries.
    fn select_internet(&mut self) {
        self.click(TREE_X, TREE_Y0 + 2);
        assert_eq!(
            self.app.tree().selected_row().map(|r| r.name.as_str()),
            Some("Internet")
        );
    }
}

// The 120x30 test terminal lays out as three panes over a status line: the tree
// at x 0..28, the entries at x 28..70, the detail at x 70..120, each with a
// one-cell border, and the status line on row 29.
const TREE_X: u16 = 10;
const TREE_Y0: u16 = 1;
const ENTRIES_X: u16 = 35;
const ENTRIES_Y0: u16 = 1;
const DETAIL_X: u16 = 80;
const DETAIL_Y0: u16 = 1;

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

fn buffer_at(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn render_at(app: &App, width: u16, height: u16) -> String {
    buffer_text(&buffer_at(app, width, height))
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
    assert_eq!(h.app.entries().len(), 4);
    let titles: Vec<&str> = h.app.entries().iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Comcast/Xfinity", "GitHub", "GitHub", "Legacy 2FA"]
    );
    assert_eq!(h.app.entries().iter().filter(|e| e.has_otp).count(), 2);
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
        idle_lock: Some(Duration::from_secs(300)),
        ..TuiOptions::default()
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
        read_only: true,
        ..TuiOptions::default()
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

// ----- mouse ----------------------------------------------------------------

#[test]
fn clicking_a_tree_row_selects_that_group() {
    let mut h = harness();
    assert_eq!(
        h.app.tree().selected_row().map(|r| r.name.as_str()),
        Some("Sample")
    );
    h.key(KeyCode::Tab); // focus elsewhere first, so the click has to move it
    assert_eq!(h.app.focus(), Focus::Entries);
    h.click(TREE_X, TREE_Y0 + 1);
    assert_eq!(h.app.focus(), Focus::Tree);
    assert_eq!(
        h.app.tree().selected_row().map(|r| r.name.as_str()),
        Some("Empty")
    );
    // The entry list followed the group.
    assert!(h.app.entries().is_empty());
}

#[test]
fn clicking_the_fold_arrow_expands_a_group() {
    let mut h = harness();
    // Work is tree row 3 at depth 1, so its ▸ marker sits in the two columns
    // right after the indent.
    h.click(3, TREE_Y0 + 3);
    assert_eq!(
        h.app.tree().selected_row().map(|r| r.name.as_str()),
        Some("Work")
    );
    assert!(h.render().contains("Servers"), "{}", h.render());
    h.click(3, TREE_Y0 + 3);
    assert!(!h.render().contains("Servers"), "{}", h.render());
}

#[test]
fn clicking_an_entry_row_selects_it_and_focuses_the_entries() {
    let mut h = harness();
    h.select_internet();
    assert_eq!(h.app.focus(), Focus::Tree);
    h.click(ENTRIES_X, ENTRIES_Y0 + 1);
    assert_eq!(h.app.focus(), Focus::Entries);
    assert_eq!(h.app.entry_index(), 1);
    assert_eq!(h.app.entries()[1].title, "GitHub");
}

#[test]
fn double_clicking_an_entry_opens_the_detail_pane() {
    let mut h = harness();
    h.select_internet();
    h.double_click(ENTRIES_X, ENTRIES_Y0);
    assert_eq!(h.app.focus(), Focus::Detail);
    assert_eq!(h.app.entry_index(), 0);
    // A single click on its own does not open anything.
    let mut h = harness();
    h.select_internet();
    h.click(ENTRIES_X, ENTRIES_Y0);
    assert_eq!(h.app.focus(), Focus::Entries);
}

#[test]
fn double_clicking_the_password_line_reveals_it() {
    let mut h = harness();
    assert!(!h.app.is_revealed());
    // Title, Username, Password: the third content row of the detail pane.
    h.double_click(DETAIL_X, DETAIL_Y0 + 2);
    assert_eq!(h.app.focus(), Focus::Detail);
    assert!(h.app.is_revealed());
    assert!(h.render().contains("s3cret"), "{}", h.render());
}

#[test]
fn the_wheel_moves_the_entry_selection_one_row_per_notch() {
    let mut h = harness();
    h.select_internet();
    assert_eq!(h.app.entry_index(), 0);
    h.wheel(MouseEventKind::ScrollDown, ENTRIES_X, 5);
    assert_eq!(h.app.entry_index(), 1);
    h.wheel(MouseEventKind::ScrollDown, ENTRIES_X, 5);
    assert_eq!(h.app.entry_index(), 2);
    h.wheel(MouseEventKind::ScrollUp, ENTRIES_X, 5);
    assert_eq!(h.app.entry_index(), 1);
    // The wheel over the tree moves the group selection instead.
    h.wheel(MouseEventKind::ScrollDown, TREE_X, 5);
    assert_eq!(
        h.app.tree().selected_row().map(|r| r.name.as_str()),
        Some("Work")
    );
}

#[test]
fn the_wheel_scrolls_the_detail_pane_three_lines_per_notch() {
    let mut h = harness();
    assert_eq!(h.app.detail_scroll(), 0);
    h.wheel(MouseEventKind::ScrollDown, DETAIL_X, 5);
    assert_eq!(h.app.detail_scroll(), 3);
    h.wheel(MouseEventKind::ScrollDown, DETAIL_X, 5);
    assert_eq!(h.app.detail_scroll(), 6);
    h.wheel(MouseEventKind::ScrollUp, DETAIL_X, 5);
    assert_eq!(h.app.detail_scroll(), 3);
}

#[test]
fn clicking_outside_a_dialog_leaves_it_open() {
    let mut h = harness();
    h.key(KeyCode::Tab);
    h.ch('d');
    assert_eq!(h.app.mode(), Mode::Dialog);
    // Bottom-left corner, well clear of the centred popup.
    h.click(1, 28);
    assert_eq!(h.app.mode(), Mode::Dialog);
    assert!(h.render().contains("recycle bin"), "{}", h.render());
    // The entry survived, so nothing answered the dialog.
    assert!(h.app.entries().iter().any(|e| e.title == "Sample Entry"));
}

#[test]
fn clicking_a_form_row_focuses_that_field() {
    let mut h = harness();
    h.ch('e'); // edit "Sample Entry"
    assert_eq!(h.app.mode(), Mode::Form);
    let screen = h.render();
    // The focus marker starts on Title; Username is the row below it.
    let title_row = screen
        .lines()
        .position(|l| l.contains("› Title"))
        .expect("the form's Title row") as u16;
    h.click(40, title_row + 1);
    let screen = h.render();
    assert!(screen.contains("› Username"), "{screen}");
    assert!(!screen.contains("› Title"), "{screen}");
    // And the typing really goes into that field.
    h.type_text("-moved");
    h.ctrl('s');
    let vault = h.app.vault().unwrap();
    assert!(vault
        .db()
        .root()
        .entries()
        .any(|e| e.get_username() == Some("alice-moved")));
}

#[test]
fn clicking_outside_the_form_changes_nothing() {
    let mut h = harness();
    h.ch('e');
    h.click(1, 28);
    assert_eq!(h.app.mode(), Mode::Form);
    assert!(h.render().contains("› Title"), "{}", h.render());
}

#[test]
fn mouse_activity_holds_off_the_idle_lock() {
    let mut h = harness_with(TuiOptions {
        idle_lock: Some(Duration::from_secs(300)),
        ..TuiOptions::default()
    });
    h.draw();
    let start = Instant::now();
    h.app.tick(start + Duration::from_secs(200));
    assert!(!h.app.is_locked());
    // A click counts as activity, exactly like a key.
    h.click_at(TREE_X, TREE_Y0, start + Duration::from_secs(250));
    h.app.tick(start + Duration::from_secs(500));
    assert!(!h.app.is_locked(), "the click should have reset the timer");
    h.app.tick(start + Duration::from_secs(560));
    assert!(h.app.is_locked());
}

#[test]
fn a_click_outside_every_pane_does_nothing() {
    let mut h = harness();
    h.select_internet();
    let before = (
        h.app.focus(),
        h.app.entry_index(),
        h.app.tree().selected,
        h.app.mode(),
    );
    h.click(60, 29); // the status line
    assert_eq!(
        (
            h.app.focus(),
            h.app.entry_index(),
            h.app.tree().selected,
            h.app.mode()
        ),
        before
    );
}

#[test]
fn mouse_events_before_the_first_draw_are_ignored() {
    let mut h = harness();
    // No draw yet, so there is no geometry to hit-test against.
    h.click_at(TREE_X, TREE_Y0 + 1, Instant::now());
    assert_eq!(h.app.tree().selected, 0);
}

#[test]
fn the_help_overlay_mentions_the_mouse() {
    let mut h = harness();
    h.ch('?');
    let screen = h.render();
    assert!(screen.contains("mouse"), "{screen}");
    assert!(screen.contains("click to select"), "{screen}");
    assert!(screen.contains("double-click to open"), "{screen}");
    assert!(screen.contains("wheel"), "{screen}");
    assert!(screen.contains("Shift+drag"), "{screen}");
}

// ----- theming --------------------------------------------------------------

/// The UI must stay on the terminal's own palette: named ANSI colours and
/// `Reset` only, so light and dark themes both work.
fn assert_palette_only(buf: &Buffer, what: &str) {
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = buf.cell((x, y)).expect("cell in bounds");
            for (which, color) in [("fg", cell.fg), ("bg", cell.bg)] {
                assert!(
                    !matches!(color, Color::Rgb(..) | Color::Indexed(_)),
                    "{what}: {which} at {x},{y} is {color:?}, not a terminal palette colour"
                );
            }
        }
    }
}

#[test]
fn no_rgb_or_indexed_colours_reach_the_screen() {
    let mut h = harness();
    assert_palette_only(&buffer_at(&h.app, 120, 30), "browse");
    h.select_internet();
    h.key(KeyCode::Tab);
    assert_palette_only(&buffer_at(&h.app, 120, 30), "entries selected");
    h.ch('?');
    assert_palette_only(&buffer_at(&h.app, 120, 30), "help");
    h.key(KeyCode::Esc);
    h.ch('e');
    assert_palette_only(&buffer_at(&h.app, 120, 30), "form");
    h.key(KeyCode::Esc);
    h.ch('d');
    assert_palette_only(&buffer_at(&h.app, 120, 30), "dialog");
    h.ch('n');
    h.ch('L');
    assert_palette_only(&buffer_at(&h.app, 120, 30), "locked");
    // And at the narrow layouts too.
    assert_palette_only(&buffer_at(&h.app, 50, 15), "narrow locked");
}

#[test]
fn status_messages_expire_and_hints_return() {
    let mut h = harness_with(TuiOptions {
        clip_timeout: Some(Duration::from_secs(10)),
        ..TuiOptions::default()
    });
    h.ch('j');
    h.ch('j'); // Internet
    h.key(KeyCode::Enter);
    h.ch('y');
    assert!(h.app.message().contains("copied"), "{}", h.app.message());
    let start = Instant::now();
    h.app.tick(start + Duration::from_secs(11));
    assert_eq!(h.app.message(), "Clipboard cleared");
    h.app.tick(start + Duration::from_secs(13));
    assert_eq!(
        h.app.message(),
        "Clipboard cleared",
        "still within the message TTL"
    );
    h.app
        .tick(start + Duration::from_secs(11) + chiave_tui::App::MESSAGE_TTL);
    assert!(h.app.message().is_empty(), "message should have expired");
    let screen = h.render();
    assert!(!screen.contains("Clipboard cleared"), "{screen}");
}

#[test]
fn error_messages_stay_until_the_next_key() {
    let mut h = harness_with(TuiOptions {
        read_only: true,
        ..TuiOptions::default()
    });
    h.ch('j');
    h.ch('j');
    h.key(KeyCode::Enter);
    h.ch('e'); // refused: read-only
    let msg = h.app.message();
    assert!(!msg.is_empty(), "expected a read-only message");
    h.app.tick(Instant::now() + Duration::from_secs(60));
    assert_eq!(h.app.message(), msg, "errors do not time out");
    h.ch('j');
    assert!(h.app.message().is_empty(), "next key clears the error");
}
