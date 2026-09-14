//! Application state: everything the TUI knows, driven only by keys and a clock.

use std::cell::Cell;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use chiave_clip::Clipboard;
use chiave_core::generate::{self, CharOptions};
use chiave_core::{
    path, DatabaseVersion, EntryId, EntryPatch, EntryRow, ExposeSecret, FieldValue, LockedVault,
    NewEntry, NodeId, OtpCode, SaveError, SaveOptions, SecretString, Vault,
};

use crate::dialog::{Dialog, DialogAction, GenOptions, Picker, Prompt, PromptKind};
use crate::form::{
    FormAction, FormState, TextInput, F_EXPIRY, F_NOTES, F_PASSWORD, F_TITLE, F_URL, F_USERNAME,
};
use crate::search::Search;
use crate::tree::Tree;
use crate::ui::{self, Panes};
use crate::{PasswordPrompt, TuiOptions};

/// Two clicks on the same row inside this window are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How far one wheel notch scrolls the detail pane.
const DETAIL_WHEEL_LINES: u16 = 3;

/// Which pane has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Entries,
    Detail,
}

/// What the keyboard is currently doing. Overlays stack over `Browse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Search,
    Form,
    Prompt,
    Dialog,
    Picker,
    GenOptions,
    Help,
    Locked,
}

/// The unlock screen state. Holds the typed password and nothing else.
#[derive(Debug, Default)]
pub struct Unlock {
    pub input: TextInput,
    pub error: Option<String>,
}

/// A copy that is counting down to its automatic clear.
#[derive(Debug, Clone)]
pub struct ClipState {
    pub label: String,
    pub deadline: Instant,
    pub remaining: u64,
}

pub struct App {
    vault: Option<Vault>,
    locked: Option<LockedVault>,
    clip: Box<dyn Clipboard>,
    prompt: Box<dyn PasswordPrompt>,
    pub(crate) opts: TuiOptions,

    pub(crate) focus: Focus,
    pub(crate) mode: Mode,
    mode_stack: Vec<Mode>,

    pub(crate) tree: Tree,
    pub(crate) entries: Vec<EntryRow>,
    pub(crate) entry_sel: usize,
    pub(crate) detail_scroll: u16,
    pub(crate) reveal: bool,
    pub(crate) otp: Option<OtpCode>,
    pub(crate) search: Search,

    pub(crate) form: Option<FormState>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) prompt_box: Option<Prompt>,
    pub(crate) picker: Option<Picker>,
    pub(crate) gen: Option<GenOptions>,
    pub(crate) unlock: Unlock,

    pub(crate) message: String,
    pub(crate) message_is_error: bool,
    pub(crate) clip_state: Option<ClipState>,
    pub(crate) db_label: String,
    pub(crate) file_label: String,

    last_input: Instant,
    quit: bool,
    /// Set when the user asked for the out-of-band password prompt.
    external_prompt: bool,

    /// Where the panes were the last time [`App::draw`] ran, so a mouse event
    /// can be hit-tested without a frame. Written through a `Cell` because
    /// drawing only borrows the app.
    panes: Cell<Panes>,
    /// The scroll offsets the two lists ended up with in that same frame.
    tree_offset: Cell<usize>,
    entry_offset: Cell<usize>,
    /// The last left click, for double-click detection.
    last_click: Option<LastClick>,
}

/// What a left click landed on, for comparing one click against the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickTarget {
    Tree(usize),
    Entries(usize),
    /// A row of the detail pane's content, scroll included.
    Detail(u16),
}

#[derive(Debug, Clone, Copy)]
struct LastClick {
    target: ClickTarget,
    at: Instant,
}

impl App {
    pub fn new(
        vault: Vault,
        clip: Box<dyn Clipboard>,
        opts: TuiOptions,
        prompt: Box<dyn PasswordPrompt>,
    ) -> Self {
        let db_label = vault
            .db()
            .meta
            .database_name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| vault.db().root().name.clone());
        let file_label = vault
            .path()
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut app = App {
            vault: Some(vault),
            locked: None,
            clip,
            prompt,
            opts,
            focus: Focus::Tree,
            mode: Mode::Browse,
            mode_stack: Vec::new(),
            tree: Tree::new(),
            entries: Vec::new(),
            entry_sel: 0,
            detail_scroll: 0,
            reveal: false,
            otp: None,
            search: Search::default(),
            form: None,
            dialog: None,
            prompt_box: None,
            picker: None,
            gen: None,
            unlock: Unlock::default(),
            message: String::new(),
            message_is_error: false,
            clip_state: None,
            db_label,
            file_label,
            last_input: Instant::now(),
            quit: false,
            external_prompt: false,
            panes: Cell::new(Panes::default()),
            tree_offset: Cell::new(0),
            entry_offset: Cell::new(0),
            last_click: None,
        };
        app.refresh();
        app.sync_from_cwd();
        app
    }

    // ----- accessors --------------------------------------------------------

    /// The open vault, or `None` while locked.
    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    pub fn is_locked(&self) -> bool {
        self.vault.is_none()
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The message currently shown in the status line.
    pub fn message(&self) -> String {
        if let Some(c) = &self.clip_state {
            return match self.opts.clip_timeout {
                Some(_) => format!("{} copied, clears in {}s", c.label, c.remaining),
                None => format!("{} copied", c.label),
            };
        }
        self.message.clone()
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.vault
            .as_ref()
            .map(|v| v.has_unsaved_changes())
            .unwrap_or(false)
    }

    /// The entry the detail pane is showing.
    pub fn selected_entry_id(&self) -> Option<EntryId> {
        if self.search.active {
            self.search.selected_item().map(|i| i.id)
        } else {
            self.entries.get(self.entry_sel).map(|e| e.id)
        }
    }

    /// The entries of the selected group (not the search results).
    pub fn entries(&self) -> &[EntryRow] {
        &self.entries
    }

    /// The `/` search state, including its hits.
    pub fn search(&self) -> &Search {
        &self.search
    }

    /// The flattened group tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The group the tree has selected.
    pub fn selected_group(&self) -> Option<chiave_core::GroupId> {
        self.tree.selected_id()
    }

    pub fn last_input(&self) -> Instant {
        self.last_input
    }

    /// How far the detail pane is scrolled, in lines.
    pub fn detail_scroll(&self) -> u16 {
        self.detail_scroll
    }

    /// Whether the selected entry's secrets are currently revealed.
    pub fn is_revealed(&self) -> bool {
        self.reveal
    }

    /// The selected row of the middle pane: an entry, or a search hit.
    pub fn entry_index(&self) -> usize {
        self.list_sel()
    }

    /// Where the panes were the last time [`App::draw`] ran.
    ///
    /// Empty until the first draw; [`App::handle_mouse`] hit-tests against it.
    pub fn panes(&self) -> Panes {
        self.panes.get()
    }

    pub(crate) fn remember_panes(&self, panes: Panes) {
        self.panes.set(panes);
    }

    pub(crate) fn remember_tree_offset(&self, offset: usize) {
        self.tree_offset.set(offset);
    }

    pub(crate) fn remember_entry_offset(&self, offset: usize) {
        self.entry_offset.set(offset);
    }

    /// Whether the run loop should drop out of the alternate screen and use the
    /// external [`PasswordPrompt`].
    pub fn wants_external_prompt(&self) -> bool {
        self.external_prompt
    }

    /// Ask for the master password with the out-of-band prompt and try it.
    /// The caller must have left the alternate screen first.
    pub fn run_external_prompt(&mut self) -> std::io::Result<()> {
        self.external_prompt = false;
        let password = self.prompt.prompt("Master password: ")?;
        self.try_unlock(password);
        Ok(())
    }

    /// Record that the out-of-band prompt failed.
    pub fn note_prompt_error(&mut self, msg: &str) {
        self.unlock.error = Some(msg.to_string());
    }

    pub fn draw(&self, frame: &mut Frame) {
        crate::ui::draw(self, frame);
    }

    // ----- state upkeep -----------------------------------------------------

    fn set_msg(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
        self.message_is_error = false;
    }

    fn set_err(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
        self.message_is_error = true;
    }

    fn push_mode(&mut self, mode: Mode) {
        self.mode_stack.push(self.mode);
        self.mode = mode;
    }

    fn pop_mode(&mut self) {
        self.mode = self.mode_stack.pop().unwrap_or(Mode::Browse);
    }

    /// Rebuild the tree and the entry list from the vault.
    pub(crate) fn refresh(&mut self) {
        let label = self.db_label.clone();
        let App { vault, tree, .. } = self;
        if let Some(v) = vault.as_ref() {
            tree.rebuild(v, &label);
        } else {
            tree.rows.clear();
        }
        self.reload_entries();
        if self.search.active {
            let App { vault, search, .. } = self;
            if let Some(v) = vault.as_ref() {
                search.run(v);
            }
        }
        self.refresh_otp();
    }

    fn reload_entries(&mut self) {
        let gid = self.tree.selected_id();
        self.entries = match (self.vault.as_ref(), gid) {
            (Some(v), Some(g)) => v.children(g).map(|l| l.entries).unwrap_or_default(),
            _ => Vec::new(),
        };
        if self.entry_sel >= self.entries.len() {
            self.entry_sel = self.entries.len().saturating_sub(1);
        }
    }

    /// Make the current group of the vault the selected tree row.
    fn sync_from_cwd(&mut self) {
        let Some(cwd) = self.vault.as_ref().map(|v| v.cwd()) else {
            return;
        };
        let label = self.db_label.clone();
        let App { vault, tree, .. } = self;
        if let Some(v) = vault.as_ref() {
            tree.reveal(v, cwd);
            tree.rebuild(v, &label);
        }
        self.tree.select_id(cwd);
        self.reload_entries();
        self.refresh_otp();
    }

    fn refresh_otp(&mut self) {
        self.otp = match (self.vault.as_ref(), self.selected_entry_id()) {
            (Some(v), Some(id)) => v.totp(id).ok(),
            _ => None,
        };
    }

    fn on_selection_changed(&mut self) {
        self.reveal = false;
        self.detail_scroll = 0;
        self.refresh_otp();
    }

    fn on_group_changed(&mut self) {
        if let (Some(v), Some(g)) = (self.vault.as_mut(), self.tree.selected_id()) {
            let _ = v.set_cwd(g);
        }
        self.entry_sel = 0;
        self.reload_entries();
        self.on_selection_changed();
    }

    /// The number of rows in the middle pane.
    fn list_len(&self) -> usize {
        if self.search.active {
            self.search.hits.len()
        } else {
            self.entries.len()
        }
    }

    fn list_select(&mut self, index: usize) {
        if self.search.active {
            self.search.selected = index;
        } else {
            self.entry_sel = index;
        }
        self.on_selection_changed();
    }

    fn list_sel(&self) -> usize {
        if self.search.active {
            self.search.selected
        } else {
            self.entry_sel
        }
    }

    // ----- clock ------------------------------------------------------------

    /// Advance timers: the clipboard countdown, the TOTP code and the idle lock.
    pub fn tick(&mut self, now: Instant) {
        if let Some(clip) = self.clip_state.as_mut() {
            if now >= clip.deadline {
                self.clip_state = None;
                self.set_msg("Clipboard cleared");
            } else {
                clip.remaining = clip.deadline.saturating_duration_since(now).as_secs() + 1;
            }
        }
        if self.vault.is_some() {
            self.refresh_otp();
            if let Some(idle) = self.opts.idle_lock {
                if now.saturating_duration_since(self.last_input) >= idle {
                    self.idle_lock();
                }
            }
        }
    }

    // ----- locking ----------------------------------------------------------

    fn idle_lock(&mut self) {
        let mut note = String::from("Locked after idle timeout");
        if self.has_unsaved_changes() {
            // Nobody is at the keyboard to answer a dialog, so save what we can
            // rather than lose it; if saving is impossible, say so plainly.
            match self.try_save(false) {
                Ok(_) => note = "Saved and locked after idle timeout".into(),
                Err(e) => note = format!("Locked after idle timeout; unsaved changes lost ({e})"),
            }
        }
        self.lock_now();
        self.unlock.error = Some(note);
    }

    /// Drop the vault, keeping only what is needed to reopen it.
    pub fn lock_now(&mut self) {
        if let Some(v) = self.vault.take() {
            self.locked = Some(v.lock());
        }
        self.form = None;
        self.dialog = None;
        self.prompt_box = None;
        self.picker = None;
        self.gen = None;
        self.reveal = false;
        self.otp = None;
        self.entries.clear();
        self.entry_sel = 0;
        self.tree.rows.clear();
        self.search.clear();
        self.message.clear();
        self.unlock = Unlock::default();
        self.mode_stack.clear();
        self.mode = Mode::Locked;
        self.focus = Focus::Tree;
    }

    /// Try the typed password against the locked vault.
    pub fn try_unlock(&mut self, password: SecretString) {
        let Some(locked) = self.locked.as_ref() else {
            return;
        };
        match locked.unlock(Some(password)) {
            Ok(v) => {
                self.vault = Some(v);
                self.locked = None;
                self.unlock = Unlock::default();
                self.mode = Mode::Browse;
                self.refresh();
                self.sync_from_cwd();
                self.set_msg("Unlocked");
            }
            Err(_) => {
                self.unlock.input = TextInput::default();
                self.unlock.error = Some("Wrong password".into());
            }
        }
    }

    // ----- saving -----------------------------------------------------------

    fn try_save(&mut self, force: bool) -> Result<String, String> {
        let Some(v) = self.vault.as_mut() else {
            return Err("vault is locked".into());
        };
        if !v.can_save() {
            return Err(
                read_only_reason(v, self.opts.read_only).unwrap_or_else(|| "read-only".into())
            );
        }
        let opts = SaveOptions {
            force,
            ..SaveOptions::default()
        };
        match v.save(opts) {
            Ok(r) => {
                let backup = r
                    .backup
                    .as_ref()
                    .and_then(|b| b.file_name())
                    .map(|b| format!(", backup {}", b.to_string_lossy()))
                    .unwrap_or_default();
                Ok(format!(
                    "Saved {} ({} bytes{}{})",
                    r.path.file_name().unwrap_or_default().to_string_lossy(),
                    r.bytes,
                    if r.verified { ", verified" } else { "" },
                    backup
                ))
            }
            Err(SaveError::ChangedOnDisk(p)) => Err(format!("__conflict__{}", p.display())),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&mut self, force: bool) {
        match self.try_save(force) {
            Ok(msg) => self.set_msg(msg),
            Err(e) if e.starts_with("__conflict__") => {
                self.dialog = Some(Dialog::new(
                    "File changed on disk",
                    vec![
                        "The vault file changed since it was opened.".into(),
                        "Saving now overwrites those changes.".into(),
                    ],
                    &[('f', "Force overwrite"), ('c', "Cancel")],
                    DialogAction::SaveConflict,
                ));
                self.push_mode(Mode::Dialog);
            }
            Err(e) => self.set_err(e),
        }
    }

    /// True when mutations are allowed; otherwise sets an explanatory message.
    fn writable(&mut self) -> bool {
        let reason = match self.vault.as_ref() {
            None => Some("vault is locked".to_string()),
            Some(v) => read_only_reason(v, self.opts.read_only),
        };
        match reason {
            Some(r) => {
                self.set_err(r);
                false
            }
            None => true,
        }
    }

    // ----- clipboard --------------------------------------------------------

    fn note_copy(&mut self, label: &str, result: Result<(), chiave_clip::ClipError>) {
        match result {
            Ok(()) => match self.opts.clip_timeout {
                Some(d) => {
                    self.clip_state = Some(ClipState {
                        label: label.to_string(),
                        deadline: Instant::now() + d,
                        remaining: d.as_secs(),
                    });
                    self.message.clear();
                    self.message_is_error = false;
                }
                None => {
                    self.clip_state = None;
                    self.set_msg(format!("{label} copied"));
                }
            },
            Err(e) => self.set_err(format!("clipboard: {e}")),
        }
    }

    fn copy_secret_field(&mut self, what: Secret) {
        let Some(id) = self.selected_entry_id() else {
            self.set_err("no entry selected");
            return;
        };
        let timeout = self.opts.clip_timeout;
        let (label, value) = {
            let Some(v) = self.vault.as_ref() else { return };
            match what {
                Secret::Password => match v.entry(id) {
                    Ok(view) => ("Password", view.password),
                    Err(e) => {
                        self.set_err(e.to_string());
                        return;
                    }
                },
                Secret::Otp => match v.totp(id) {
                    Ok(code) => ("TOTP", Some(SecretString::from(code.code))),
                    Err(e) => {
                        self.set_err(e.to_string());
                        return;
                    }
                },
            }
        };
        match value {
            Some(secret) => {
                let r = self.clip.copy_secret(&secret, timeout);
                self.note_copy(label, r);
            }
            None => self.set_err(format!("entry has no {}", label.to_lowercase())),
        }
    }

    fn copy_plain_field(
        &mut self,
        label: &str,
        pick: fn(&chiave_core::EntryView) -> Option<String>,
    ) {
        let Some(id) = self.selected_entry_id() else {
            self.set_err("no entry selected");
            return;
        };
        let value = {
            let Some(v) = self.vault.as_ref() else { return };
            match v.entry(id) {
                Ok(view) => pick(&view),
                Err(e) => {
                    self.set_err(e.to_string());
                    return;
                }
            }
        };
        match value {
            Some(text) => {
                let r = self.clip.copy_text(&text);
                match r {
                    Ok(()) => {
                        self.clip_state = None;
                        self.set_msg(format!("{label} copied"));
                    }
                    Err(e) => self.set_err(format!("clipboard: {e}")),
                }
            }
            None => self.set_err(format!("entry has no {}", label.to_lowercase())),
        }
    }

    fn clear_clipboard(&mut self) {
        match self.clip.clear() {
            Ok(()) => {
                self.clip_state = None;
                self.set_msg("Clipboard cleared");
            }
            Err(e) => self.set_err(format!("clipboard: {e}")),
        }
    }

    // ----- key entry point --------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.last_input = Instant::now();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c')) && self.mode != Mode::Dialog {
            self.dialog = Some(Dialog::yes_no(
                "Quit",
                vec!["Quit without saving?".into()],
                DialogAction::QuitForce,
            ));
            self.push_mode(Mode::Dialog);
            return;
        }
        match self.mode {
            Mode::Locked => self.key_locked(key),
            Mode::Dialog => self.key_dialog(key),
            Mode::Prompt => self.key_prompt(key),
            Mode::Picker => self.key_picker(key),
            Mode::GenOptions => self.key_gen(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.pop_mode();
                }
            }
            Mode::Form => self.key_form(key),
            Mode::Search => self.key_search(key),
            Mode::Browse => self.key_browse(key),
        }
    }

    // ----- mouse entry point ------------------------------------------------

    /// Apply a mouse event, timestamped now.
    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        self.handle_mouse_at(ev, Instant::now());
    }

    /// Apply a mouse event against an injected clock, the way [`App::tick`]
    /// takes its `now`, so double-clicks can be tested without sleeping.
    ///
    /// Hit-testing uses the geometry the last [`App::draw`] left in
    /// [`App::panes`]; before the first draw every click is a no-op.
    pub fn handle_mouse_at(&mut self, ev: MouseEvent, now: Instant) {
        // Anything the mouse does counts as activity, exactly like a key.
        self.last_input = now;
        let at = Position::new(ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_click(at, now),
            MouseEventKind::ScrollUp => self.mouse_scroll(at, -1),
            MouseEventKind::ScrollDown => self.mouse_scroll(at, 1),
            // Right and middle buttons, releases, drags and bare motion: nothing.
            _ => {}
        }
    }

    /// The overlay on top and where it sits, or `None` when none is open.
    fn modal_rect(&self) -> Option<Rect> {
        let body = self.panes.get().body;
        match self.mode {
            Mode::Help => Some(ui::help_rect(body)),
            Mode::Dialog => self.dialog.as_ref().map(|d| ui::dialog_rect(d, body)),
            Mode::GenOptions => Some(ui::gen_rect(body)),
            Mode::Prompt => Some(ui::prompt_rect(body)),
            Mode::Picker => self.picker.as_ref().map(|p| ui::picker_rect(p, body)),
            Mode::Form => self.form.as_ref().map(|f| ui::form_rect(f, body)),
            Mode::Locked => Some(ui::unlock_rect(body)),
            Mode::Browse | Mode::Search => None,
        }
    }

    /// True when this click pairs up with the previous one.
    fn double_click(&mut self, target: ClickTarget, now: Instant) -> bool {
        let double = self.last_click.is_some_and(|c| {
            c.target == target && now.saturating_duration_since(c.at) <= DOUBLE_CLICK
        });
        // Forget the pair, so a third click does not double again.
        self.last_click = (!double).then_some(LastClick { target, at: now });
        double
    }

    fn mouse_click(&mut self, at: Position, now: Instant) {
        if let Some(rect) = self.modal_rect() {
            // Deliberately no dismiss-on-click: a stray click outside a
            // confirmation must never answer it. Only the form reads clicks,
            // and only inside itself.
            if self.mode == Mode::Form && rect.contains(at) {
                self.form_click(rect, at);
            }
            return;
        }
        let panes = self.panes.get();
        if let Some(r) = panes.tree.filter(|r| r.contains(at)) {
            self.tree_click(r, at, now);
        } else if let Some(r) = panes.entries.filter(|r| r.contains(at)) {
            self.entries_click(r, at, now);
        } else if let Some(r) = panes.detail.filter(|r| r.contains(at)) {
            self.detail_click(r, at, now);
        }
        // A click anywhere else -- the status line, a hidden pane's gap -- does nothing.
    }

    fn tree_click(&mut self, area: Rect, at: Position, now: Instant) {
        self.focus = Focus::Tree;
        let inner = ui::inner(area);
        let Some(offset) = row_in(area, at) else {
            return;
        };
        let row = self.tree_offset.get() + offset as usize;
        let Some((depth, has_children)) =
            self.tree.rows.get(row).map(|r| (r.depth, r.has_children))
        else {
            return;
        };
        if self.search.active {
            self.search.clear();
            self.reload_entries();
        }
        if self.tree.selected != row {
            self.tree.selected = row;
            self.on_group_changed();
        }
        // The two columns that hold the ▸/▾ marker fold on a single click.
        let arrow = inner.x.saturating_add(depth as u16 * 2);
        let on_arrow = has_children && at.x >= arrow && at.x < arrow.saturating_add(2);
        if self.double_click(ClickTarget::Tree(row), now) || on_arrow {
            self.toggle_tree_fold();
        }
    }

    /// Fold or unfold the selected group, without the keyboard's habit of
    /// stepping into the first child when it is already open.
    fn toggle_tree_fold(&mut self) {
        let Some((has_children, expanded)) = self
            .tree
            .selected_row()
            .map(|r| (r.has_children, r.expanded))
        else {
            return;
        };
        if !has_children {
            return;
        }
        let changed = if expanded {
            self.tree.collapse_selected()
        } else {
            self.tree.expand_selected()
        };
        if changed {
            self.refresh();
        }
        self.on_group_changed();
    }

    fn entries_click(&mut self, area: Rect, at: Position, now: Instant) {
        self.focus = Focus::Entries;
        let Some(offset) = row_in(area, at) else {
            return;
        };
        let row = self.entry_offset.get() + offset as usize;
        if row >= self.list_len() {
            return;
        }
        if row != self.list_sel() {
            self.list_select(row);
        }
        if self.double_click(ClickTarget::Entries(row), now) {
            self.activate();
        }
    }

    fn detail_click(&mut self, area: Rect, at: Position, now: Instant) {
        self.focus = Focus::Detail;
        let Some(offset) = row_in(area, at) else {
            return;
        };
        let line = offset.saturating_add(self.detail_scroll);
        if self.double_click(ClickTarget::Detail(line), now)
            && line == ui::DETAIL_PASSWORD_LINE
            && self.selected_entry_id().is_some()
        {
            self.reveal = !self.reveal;
        }
    }

    /// A click inside the edit form moves the focus to the field it landed on.
    fn form_click(&mut self, rect: Rect, at: Position) {
        let inner = ui::inner(rect);
        if !inner.contains(at) {
            return;
        }
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let mut row = at.y - inner.y;
        let mut hit = None;
        for (i, height) in ui::form_field_rows(form).into_iter().enumerate() {
            let height = height as u16;
            if row < height {
                hit = Some(i);
                break;
            }
            row -= height;
        }
        if let (Some(i), Some(form)) = (hit, self.form.as_mut()) {
            form.focus = i;
        }
    }

    fn mouse_scroll(&mut self, at: Position, delta: i32) {
        if self.modal_rect().is_some() {
            // The group picker is the only scrollable overlay; the others are
            // sized to their contents.
            if let (Mode::Picker, Some(p)) = (self.mode, self.picker.as_mut()) {
                if delta > 0 {
                    p.next();
                } else {
                    p.prev();
                }
            }
            return;
        }
        let panes = self.panes.get();
        if panes.tree.is_some_and(|r| r.contains(at)) {
            if self.search.active {
                self.search.clear();
                self.reload_entries();
            }
            if delta > 0 {
                self.tree.select_next();
            } else {
                self.tree.select_prev();
            }
            self.on_group_changed();
        } else if panes.entries.is_some_and(|r| r.contains(at)) {
            let len = self.list_len();
            if len == 0 {
                return;
            }
            let next = (self.list_sel() as i32 + delta).clamp(0, len as i32 - 1) as usize;
            if next != self.list_sel() {
                self.list_select(next);
            }
        } else if panes.detail.is_some_and(|r| r.contains(at)) {
            self.detail_scroll = if delta > 0 {
                self.detail_scroll.saturating_add(DETAIL_WHEEL_LINES)
            } else {
                self.detail_scroll.saturating_sub(DETAIL_WHEEL_LINES)
            };
        }
    }

    // ----- unlock screen ----------------------------------------------------

    fn key_locked(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => {
                let pw = SecretString::from(std::mem::take(&mut self.unlock.input.value));
                self.unlock.input = TextInput::default();
                self.try_unlock(pw);
            }
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('p') if ctrl => self.external_prompt = true,
            _ => {
                self.unlock.handle(key);
            }
        }
    }

    // ----- dialogs ----------------------------------------------------------

    fn key_dialog(&mut self, key: KeyEvent) {
        let choice = match key.code {
            KeyCode::Esc => None,
            KeyCode::Char(c) => {
                let Some(d) = self.dialog.as_ref() else {
                    return;
                };
                if d.accepts(c) {
                    Some(c.to_ascii_lowercase())
                } else {
                    return;
                }
            }
            _ => return,
        };
        let Some(d) = self.dialog.take() else { return };
        self.pop_mode();
        let Some(choice) = choice else { return };
        match d.action {
            DialogAction::DeleteEntry { id, permanent } => {
                if choice == 'y' {
                    let res = self
                        .vault
                        .as_mut()
                        .map(|v| v.rm_entry(id, permanent))
                        .unwrap_or(Ok(()));
                    match res {
                        Ok(()) => {
                            self.refresh();
                            self.on_selection_changed();
                            self.set_msg(if permanent {
                                "Entry deleted"
                            } else {
                                "Entry moved to the recycle bin"
                            });
                        }
                        Err(e) => self.set_err(e.to_string()),
                    }
                }
            }
            DialogAction::DeleteGroup { id, permanent } => {
                if choice == 'y' {
                    let res = self
                        .vault
                        .as_mut()
                        .map(|v| v.rmdir(id, true, permanent))
                        .unwrap_or(Ok(()));
                    match res {
                        Ok(()) => {
                            self.tree.select_prev();
                            self.refresh();
                            self.on_group_changed();
                            self.set_msg(if permanent {
                                "Group deleted"
                            } else {
                                "Group moved to the recycle bin"
                            });
                        }
                        Err(e) => self.set_err(e.to_string()),
                    }
                }
            }
            DialogAction::QuitUnsaved => match choice {
                's' => match self.try_save(false) {
                    Ok(_) => self.quit = true,
                    Err(e) => self.set_err(e),
                },
                'd' => self.quit = true,
                _ => {}
            },
            DialogAction::QuitForce => {
                if choice == 'y' {
                    self.quit = true;
                }
            }
            DialogAction::LockUnsaved => match choice {
                's' => match self.try_save(false) {
                    Ok(msg) => {
                        self.lock_now();
                        self.unlock.error = Some(msg);
                    }
                    Err(e) => self.set_err(e),
                },
                'd' => self.lock_now(),
                _ => {}
            },
            DialogAction::DiscardForm => {
                if choice == 'y' {
                    self.form = None;
                    self.pop_mode();
                }
            }
            DialogAction::SaveConflict => {
                if choice == 'f' {
                    self.save(true);
                }
            }
        }
    }

    // ----- text prompt ------------------------------------------------------

    fn key_prompt(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.prompt_box = None;
                self.pop_mode();
            }
            KeyCode::Enter => {
                let Some(p) = self.prompt_box.take() else {
                    return;
                };
                self.pop_mode();
                let name = p.input.value.trim().to_string();
                if name.is_empty() {
                    self.set_err("name cannot be empty");
                    return;
                }
                let res = match p.kind {
                    PromptKind::RenameGroup(id) => self
                        .vault
                        .as_mut()
                        .map(|v| v.rename_group(id, &name).map(|_| "Group renamed")),
                    PromptKind::NewGroup(parent) => self.vault.as_mut().map(|v| {
                        v.set_cwd(parent)
                            .map_err(chiave_core::WriteError::from)
                            .and_then(|_| v.mkdir(&path::escape(&name)))
                            .map(|_| "Group created")
                    }),
                };
                match res {
                    Some(Ok(msg)) => {
                        if let PromptKind::NewGroup(parent) = p.kind {
                            self.tree.select_id(parent);
                        }
                        self.refresh();
                        self.set_msg(msg);
                    }
                    Some(Err(e)) => self.set_err(e.to_string()),
                    None => {}
                }
            }
            _ => {
                if let Some(p) = self.prompt_box.as_mut() {
                    p.input.handle_key(key);
                }
            }
        }
    }

    // ----- move picker ------------------------------------------------------

    fn key_picker(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.picker = None;
                self.pop_mode();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(p) = self.picker.as_mut() {
                    p.next();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(p) = self.picker.as_mut() {
                    p.prev();
                }
            }
            KeyCode::Enter => {
                let Some(p) = self.picker.take() else { return };
                self.pop_mode();
                let Some(dest) = p.selected_group() else {
                    return;
                };
                let res = self.vault.as_mut().map(|v| v.mv(p.what, dest));
                match res {
                    Some(Ok(())) => {
                        self.refresh();
                        self.on_group_changed();
                        self.set_msg(format!("Moved {}", p.label));
                    }
                    Some(Err(e)) => self.set_err(e.to_string()),
                    None => {}
                }
            }
            _ => {}
        }
    }

    // ----- generator options ------------------------------------------------

    fn key_gen(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.gen = None;
                self.pop_mode();
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Char('-') => {
                if let Some(g) = self.gen.as_mut() {
                    g.length = g.length.saturating_sub(1).max(4);
                }
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char('+') => {
                if let Some(g) = self.gen.as_mut() {
                    g.length = (g.length + 1).min(128);
                }
            }
            KeyCode::Char('s') => {
                if let Some(g) = self.gen.as_mut() {
                    g.special = !g.special;
                }
            }
            KeyCode::Char('a') => {
                if let Some(g) = self.gen.as_mut() {
                    g.exclude_ambiguous = !g.exclude_ambiguous;
                }
            }
            KeyCode::Enter => {
                let Some(g) = self.gen.take() else { return };
                self.pop_mode();
                self.generate_password(g.to_char_options());
            }
            _ => {}
        }
    }

    fn generate_password(&mut self, opts: CharOptions) {
        match generate::password(&opts) {
            Ok(secret) => {
                if let Some(f) = self.form.as_mut() {
                    f.set_password(secret.expose_secret());
                }
                self.set_msg(format!("Generated a {}-character password", opts.length));
            }
            Err(e) => self.set_err(e.to_string()),
        }
    }

    // ----- edit form --------------------------------------------------------

    fn key_form(&mut self, key: KeyEvent) {
        let action = match self.form.as_mut() {
            Some(f) => f.handle_key(key),
            None => {
                self.pop_mode();
                return;
            }
        };
        match action {
            FormAction::None => {}
            FormAction::Generate => self.generate_password(CharOptions::default()),
            FormAction::GenerateOptions => {
                self.gen = Some(GenOptions::default());
                self.push_mode(Mode::GenOptions);
            }
            FormAction::Cancel => {
                let dirty = self.form.as_ref().map(|f| f.dirty).unwrap_or(false);
                if dirty {
                    self.dialog = Some(Dialog::yes_no(
                        "Discard changes",
                        vec!["This entry has unsaved edits. Discard them?".into()],
                        DialogAction::DiscardForm,
                    ));
                    self.push_mode(Mode::Dialog);
                } else {
                    self.form = None;
                    self.pop_mode();
                }
            }
            FormAction::Save => self.submit_form(),
        }
    }

    fn submit_form(&mut self) {
        let Some(form) = self.form.take() else { return };
        let res = match self.vault.as_mut() {
            Some(v) => apply_form(v, &form),
            None => Err("vault is locked".into()),
        };
        match res {
            Ok((msg, id)) => {
                self.pop_mode();
                self.refresh();
                if let Some(id) = id {
                    if let Some(i) = self.entries.iter().position(|e| e.id == id) {
                        self.entry_sel = i;
                    }
                }
                self.focus = Focus::Entries;
                self.on_selection_changed();
                self.set_msg(msg);
            }
            Err(e) => {
                self.form = Some(form);
                self.set_err(e);
            }
        }
    }

    // ----- search -----------------------------------------------------------

    fn key_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search.clear();
                self.pop_mode();
                self.reload_entries();
                self.on_selection_changed();
            }
            KeyCode::Enter => {
                self.pop_mode();
                self.focus = Focus::Entries;
            }
            KeyCode::Down => {
                if self.search.selected + 1 < self.search.hits.len() {
                    self.search.selected += 1;
                    self.on_selection_changed();
                }
            }
            KeyCode::Up => {
                self.search.selected = self.search.selected.saturating_sub(1);
                self.on_selection_changed();
            }
            _ => {
                let mut input = TextInput {
                    value: std::mem::take(&mut self.search.query),
                    cursor: self.search.cursor,
                };
                let changed = input.handle_key(key);
                self.search.query = input.value;
                self.search.cursor = input.cursor;
                if changed {
                    self.search.selected = 0;
                    let App { vault, search, .. } = self;
                    if let Some(v) = vault.as_ref() {
                        search.run(v);
                    }
                    self.on_selection_changed();
                }
            }
        }
    }

    // ----- browsing ---------------------------------------------------------

    fn key_browse(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char('?') => self.push_mode(Mode::Help),
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('/') => {
                self.search.query.clear();
                self.search.cursor = 0;
                self.search.selected = 0;
                let App { vault, search, .. } = self;
                if let Some(v) = vault.as_ref() {
                    search.run(v);
                }
                self.focus = Focus::Entries;
                self.push_mode(Mode::Search);
                self.on_selection_changed();
            }
            KeyCode::Char('L') => self.request_lock(),
            KeyCode::Char('s') => self.save(false),
            KeyCode::Char('x') => self.clear_clipboard(),
            KeyCode::Char('y') | KeyCode::Char('p') => self.copy_secret_field(Secret::Password),
            KeyCode::Char('o') => self.copy_secret_field(Secret::Otp),
            KeyCode::Char('u') => self.copy_plain_field("Username", |v| v.username.clone()),
            KeyCode::Char('U') => self.copy_plain_field("URL", |v| v.url.clone()),
            KeyCode::Char('v') => {
                if self.selected_entry_id().is_some() {
                    self.reveal = !self.reveal;
                }
            }
            KeyCode::Char('n') => self.open_new_entry(),
            KeyCode::Char('e') => self.open_edit_entry(),
            KeyCode::Char('d') => self.confirm_delete(false),
            KeyCode::Char('D') => self.confirm_delete(true),
            KeyCode::Char('m') => self.open_move_picker(),
            KeyCode::Char('r') => self.open_rename_group(),
            KeyCode::Char('N') => self.open_new_group(),
            KeyCode::Char('g') | KeyCode::Char('G') => {
                self.set_msg("password generation lives in the edit form: Ctrl-g / Alt-g")
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Home => self.move_to_edge(true),
            KeyCode::End => self.move_to_edge(false),
            KeyCode::Char('h') | KeyCode::Left => self.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.move_right(),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => {
                if self.search.active {
                    self.search.clear();
                    self.reload_entries();
                    self.on_selection_changed();
                } else {
                    self.cycle_focus(false);
                }
            }
            _ => {}
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        self.focus = match (self.focus, forward) {
            (Focus::Tree, true) => Focus::Entries,
            (Focus::Entries, true) => Focus::Detail,
            (Focus::Detail, true) => Focus::Tree,
            (Focus::Tree, false) => Focus::Detail,
            (Focus::Entries, false) => Focus::Tree,
            (Focus::Detail, false) => Focus::Entries,
        };
    }

    fn move_selection(&mut self, delta: i32) {
        match self.focus {
            Focus::Tree => {
                if self.search.active {
                    self.search.clear();
                    self.reload_entries();
                }
                if delta > 0 {
                    self.tree.select_next();
                } else {
                    self.tree.select_prev();
                }
                self.on_group_changed();
            }
            Focus::Entries => {
                let len = self.list_len();
                if len == 0 {
                    return;
                }
                let cur = self.list_sel() as i32;
                let next = (cur + delta).clamp(0, len as i32 - 1) as usize;
                self.list_select(next);
            }
            Focus::Detail => {
                self.detail_scroll = if delta > 0 {
                    self.detail_scroll.saturating_add(1)
                } else {
                    self.detail_scroll.saturating_sub(1)
                };
            }
        }
    }

    fn move_to_edge(&mut self, first: bool) {
        match self.focus {
            Focus::Tree => {
                self.tree.selected = if first {
                    0
                } else {
                    self.tree.rows.len().saturating_sub(1)
                };
                self.on_group_changed();
            }
            Focus::Entries => {
                let len = self.list_len();
                if len > 0 {
                    self.list_select(if first { 0 } else { len - 1 });
                }
            }
            Focus::Detail => self.detail_scroll = 0,
        }
    }

    fn move_left(&mut self) {
        match self.focus {
            Focus::Tree => {
                if self.tree.collapse_selected() {
                    self.refresh();
                }
                self.on_group_changed();
            }
            Focus::Entries => self.focus = Focus::Tree,
            Focus::Detail => self.focus = Focus::Entries,
        }
    }

    fn move_right(&mut self) {
        match self.focus {
            Focus::Tree => {
                if self.tree.expand_selected() {
                    self.refresh();
                }
                self.on_group_changed();
            }
            Focus::Entries => self.focus = Focus::Detail,
            Focus::Detail => {}
        }
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Tree => self.focus = Focus::Entries,
            Focus::Entries => {
                if self.search.active {
                    if let Some(item) = self.search.selected_item() {
                        let (group, id) = (item.group, item.id);
                        self.search.clear();
                        let App { vault, tree, .. } = self;
                        if let Some(v) = vault.as_ref() {
                            tree.reveal(v, group);
                        }
                        self.refresh();
                        self.tree.select_id(group);
                        self.on_group_changed();
                        if let Some(i) = self.entries.iter().position(|e| e.id == id) {
                            self.entry_sel = i;
                        }
                        self.on_selection_changed();
                    }
                }
                self.focus = Focus::Detail;
            }
            Focus::Detail => {}
        }
    }

    fn request_quit(&mut self) {
        if self.has_unsaved_changes() {
            self.dialog = Some(Dialog::new(
                "Unsaved changes",
                vec!["The vault has unsaved changes.".into()],
                &[('s', "Save and quit"), ('d', "Discard"), ('c', "Cancel")],
                DialogAction::QuitUnsaved,
            ));
            self.push_mode(Mode::Dialog);
        } else {
            self.quit = true;
        }
    }

    fn request_lock(&mut self) {
        if self.has_unsaved_changes() {
            self.dialog = Some(Dialog::new(
                "Unsaved changes",
                vec!["The vault has unsaved changes.".into()],
                &[
                    ('s', "Save and lock"),
                    ('d', "Lock and discard"),
                    ('c', "Cancel"),
                ],
                DialogAction::LockUnsaved,
            ));
            self.push_mode(Mode::Dialog);
        } else {
            self.lock_now();
        }
    }

    fn open_new_entry(&mut self) {
        if !self.writable() {
            return;
        }
        let Some(group) = self.tree.selected_id() else {
            return;
        };
        self.form = Some(FormState::new(group));
        self.push_mode(Mode::Form);
    }

    fn open_edit_entry(&mut self) {
        if !self.writable() {
            return;
        }
        let Some(id) = self.selected_entry_id() else {
            self.set_err("no entry selected");
            return;
        };
        let (view, group) = {
            let Some(v) = self.vault.as_ref() else { return };
            match v.entry(id) {
                Ok(view) => {
                    let group = v
                        .db()
                        .entry(id)
                        .map(|e| e.parent().id())
                        .unwrap_or_else(|| v.cwd());
                    (view, group)
                }
                Err(e) => {
                    self.set_err(e.to_string());
                    return;
                }
            }
        };
        self.form = Some(FormState::edit(&view, group));
        self.push_mode(Mode::Form);
    }

    fn confirm_delete(&mut self, permanent: bool) {
        if !self.writable() {
            return;
        }
        if self.focus == Focus::Tree {
            let Some(id) = self.tree.selected_id() else {
                return;
            };
            let root = self.vault.as_ref().map(|v| v.root());
            if Some(id) == root {
                self.set_err("cannot delete the root group");
                return;
            }
            let name = self
                .tree
                .selected_row()
                .map(|r| r.name.clone())
                .unwrap_or_default();
            let body = if permanent {
                vec![
                    format!("Permanently delete the group \"{name}\" and everything in it?"),
                    "This cannot be undone.".into(),
                ]
            } else {
                vec![format!(
                    "Move the group \"{name}\" and everything in it to the recycle bin?"
                )]
            };
            self.dialog = Some(Dialog::yes_no(
                "Delete group",
                body,
                DialogAction::DeleteGroup { id, permanent },
            ));
            self.push_mode(Mode::Dialog);
            return;
        }
        let Some(id) = self.selected_entry_id() else {
            self.set_err("no entry selected");
            return;
        };
        let title = self
            .vault
            .as_ref()
            .and_then(|v| v.entry(id).ok())
            .map(|e| e.title)
            .unwrap_or_default();
        let body = if permanent {
            vec![
                format!("Permanently delete \"{title}\"?"),
                "This cannot be undone.".into(),
            ]
        } else {
            vec![format!("Move \"{title}\" to the recycle bin?")]
        };
        self.dialog = Some(Dialog::yes_no(
            "Delete entry",
            body,
            DialogAction::DeleteEntry { id, permanent },
        ));
        self.push_mode(Mode::Dialog);
    }

    fn open_move_picker(&mut self) {
        if !self.writable() {
            return;
        }
        let label_root = self.db_label.clone();
        let (what, label) = if self.focus == Focus::Tree {
            let Some(id) = self.tree.selected_id() else {
                return;
            };
            if Some(id) == self.vault.as_ref().map(|v| v.root()) {
                self.set_err("cannot move the root group");
                return;
            }
            (
                NodeId::Group(id),
                self.tree
                    .selected_row()
                    .map(|r| r.name.clone())
                    .unwrap_or_default(),
            )
        } else {
            let Some(id) = self.selected_entry_id() else {
                self.set_err("no entry selected");
                return;
            };
            let title = self
                .vault
                .as_ref()
                .and_then(|v| v.entry(id).ok())
                .map(|e| e.title)
                .unwrap_or_default();
            (NodeId::Entry(id), title)
        };
        let Some(v) = self.vault.as_ref() else { return };
        let groups = Tree::all_groups(v, &label_root);
        self.picker = Some(Picker {
            what,
            label,
            groups,
            selected: 0,
        });
        self.push_mode(Mode::Picker);
    }

    fn open_rename_group(&mut self) {
        if !self.writable() {
            return;
        }
        let Some(id) = self.tree.selected_id() else {
            return;
        };
        let name = self
            .tree
            .selected_row()
            .map(|r| r.name.clone())
            .unwrap_or_default();
        self.prompt_box = Some(Prompt {
            title: "Rename group".into(),
            input: TextInput::new(name),
            kind: PromptKind::RenameGroup(id),
        });
        self.push_mode(Mode::Prompt);
    }

    fn open_new_group(&mut self) {
        if !self.writable() {
            return;
        }
        let Some(id) = self.tree.selected_id() else {
            return;
        };
        self.prompt_box = Some(Prompt {
            title: "New group".into(),
            input: TextInput::default(),
            kind: PromptKind::NewGroup(id),
        });
        self.push_mode(Mode::Prompt);
    }
}

/// The zero-based content row `at` falls on inside a bordered pane, or `None`
/// when it landed on the border itself.
fn row_in(area: Rect, at: Position) -> Option<u16> {
    let inner = ui::inner(area);
    inner.contains(at).then(|| at.y - inner.y)
}

impl Unlock {
    fn handle(&mut self, key: KeyEvent) {
        self.error = None;
        self.input.handle_key(key);
    }
}

enum Secret {
    Password,
    Otp,
}

/// Why this vault refuses writes, or `None` when it accepts them.
fn read_only_reason(v: &Vault, forced: bool) -> Option<String> {
    if forced || v.is_read_only() {
        return Some("read-only: this vault was opened read-only".into());
    }
    match v.version() {
        DatabaseVersion::KDB4(_) => None,
        other => Some(format!(
            "read-only: {other} cannot be written; run `chiave upgrade` to convert it"
        )),
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim_end_matches('\n');
    (!s.is_empty()).then(|| s.to_string())
}

fn parse_expiry(text: &str) -> Result<Option<chrono::NaiveDateTime>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(|d| Some(d.and_hms_opt(0, 0, 0).unwrap_or_default()))
        .map_err(|_| format!("expiry \"{text}\" is not a YYYY-MM-DD date"))
}

/// Turn the form into a vault mutation. Returns a status message and the entry id.
fn apply_form(vault: &mut Vault, form: &FormState) -> Result<(String, Option<EntryId>), String> {
    let title = form.value(F_TITLE).trim().to_string();
    if title.is_empty() {
        return Err("title cannot be empty".into());
    }
    let expiry = parse_expiry(form.value(F_EXPIRY))?;
    let custom = form.custom_fields();
    match form.target {
        Some(id) => {
            let mut patch = EntryPatch::default()
                .set_plain("title", title)
                .set_plain("username", form.value(F_USERNAME))
                .set_secret("password", form.value(F_PASSWORD))
                .set_plain("url", form.value(F_URL))
                .set_plain("notes", form.value(F_NOTES));
            for (name, value, protected) in &custom {
                patch = patch.set(name, field_value(value, *protected));
            }
            for old in &form.original_custom {
                if !custom.iter().any(|(n, _, _)| n == old) {
                    patch = patch.remove(old);
                }
            }
            patch.expires = Some(expiry);
            let changed = vault.edit_entry(id, patch).map_err(|e| e.to_string())?;
            Ok((
                if changed {
                    "Entry updated"
                } else {
                    "No changes"
                }
                .to_string(),
                Some(id),
            ))
        }
        None => {
            vault.set_cwd(form.group).map_err(|e| e.to_string())?;
            let new = NewEntry {
                title: title.clone(),
                username: non_empty(form.value(F_USERNAME)),
                password: non_empty(form.value(F_PASSWORD)).map(SecretString::from),
                url: non_empty(form.value(F_URL)),
                notes: non_empty(form.value(F_NOTES)),
                otp: None,
                custom: custom
                    .iter()
                    .map(|(n, v, p)| (n.clone(), field_value(v, *p)))
                    .collect(),
                expires: expiry,
                tags: Vec::new(),
            };
            let id = vault
                .new_entry(&path::escape(&title), new)
                .map_err(|e| e.to_string())?;
            Ok((format!("Created \"{title}\""), Some(id)))
        }
    }
}

fn field_value(text: &str, protected: bool) -> FieldValue {
    if protected {
        FieldValue::Protected(SecretString::from(text.to_string()))
    } else {
        FieldValue::Plain(text.to_string())
    }
}
