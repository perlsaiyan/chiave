//! The entry edit/create modal and the little line editor it is built from.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use chiave_core::{EntryId, EntryView, ExposeSecret, FieldValue, GroupId};

/// A one-line (or, for notes, many-line) text buffer with a character cursor.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    pub value: String,
    /// Cursor position in characters, `0..=len`.
    pub cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        TextInput { value, cursor }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    pub fn len(&self) -> usize {
        self.value.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let at = self.byte_at(self.cursor - 1);
        self.value.remove(at);
        self.cursor -= 1;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.len() {
            return false;
        }
        let at = self.byte_at(self.cursor);
        self.value.remove(at);
        true
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.len();
    }

    /// Apply an editing key. Returns true when the buffer changed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert(c);
                true
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => {
                self.left();
                false
            }
            KeyCode::Right => {
                self.right();
                false
            }
            KeyCode::Home => {
                self.home();
                false
            }
            KeyCode::End => {
                self.end();
                false
            }
            _ => false,
        }
    }
}

/// What a form row holds, which decides how it is drawn and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    /// Masked unless the form is revealed.
    Secret,
    /// Enter inserts a newline instead of moving on.
    Multiline,
    /// `YYYY-MM-DD` or empty.
    Expiry,
    CustomName,
    CustomValue {
        protected: bool,
    },
}

#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub input: TextInput,
    pub kind: FieldKind,
}

impl FormField {
    fn new(label: &str, value: impl Into<String>, kind: FieldKind) -> Self {
        FormField {
            label: label.to_string(),
            input: TextInput::new(value),
            kind,
        }
    }
}

/// Index of the fixed rows. Custom fields follow in name/value pairs.
pub const F_TITLE: usize = 0;
pub const F_USERNAME: usize = 1;
pub const F_PASSWORD: usize = 2;
pub const F_URL: usize = 3;
pub const F_NOTES: usize = 4;
pub const F_EXPIRY: usize = 5;
pub const FIXED: usize = 6;

/// What the app should do after the form consumed a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    None,
    Save,
    Cancel,
    /// Generate a password with the defaults into the password field.
    Generate,
    /// Open the generator options popup.
    GenerateOptions,
}

/// The new-entry / edit-entry modal.
#[derive(Debug)]
pub struct FormState {
    /// `None` for a new entry.
    pub target: Option<EntryId>,
    pub group: GroupId,
    pub fields: Vec<FormField>,
    pub focus: usize,
    pub reveal: bool,
    pub dirty: bool,
    /// Custom field names the entry had when the form opened, to detect removals.
    pub original_custom: Vec<String>,
}

impl FormState {
    /// An empty form for a new entry in `group`.
    pub fn new(group: GroupId) -> Self {
        FormState {
            target: None,
            group,
            fields: base_fields("", "", "", "", "", ""),
            focus: 0,
            reveal: false,
            dirty: false,
            original_custom: Vec::new(),
        }
    }

    /// A form pre-filled from an existing entry.
    pub fn edit(view: &EntryView, group: GroupId) -> Self {
        let expiry = view
            .expires
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let mut fields = base_fields(
            &view.title,
            view.username.as_deref().unwrap_or(""),
            view.password
                .as_ref()
                .map(|p| p.expose_secret())
                .unwrap_or(""),
            view.url.as_deref().unwrap_or(""),
            view.notes.as_deref().unwrap_or(""),
            &expiry,
        );
        let mut original_custom = Vec::new();
        for (name, value) in &view.custom {
            let (text, protected) = match value {
                FieldValue::Plain(s) => (s.clone(), false),
                FieldValue::Protected(s) => (s.expose_secret().to_string(), true),
            };
            original_custom.push(name.clone());
            fields.push(FormField::new(name, name.clone(), FieldKind::CustomName));
            fields.push(FormField::new(
                name,
                text,
                FieldKind::CustomValue { protected },
            ));
        }
        FormState {
            target: Some(view.id),
            group,
            fields,
            focus: 0,
            reveal: false,
            dirty: false,
            original_custom,
        }
    }

    pub fn value(&self, index: usize) -> &str {
        self.fields
            .get(index)
            .map(|f| f.input.value.as_str())
            .unwrap_or("")
    }

    pub fn set_password(&mut self, value: &str) {
        if let Some(f) = self.fields.get_mut(F_PASSWORD) {
            f.input.set(value);
            self.dirty = true;
        }
    }

    /// Custom fields as (name, value, protected), skipping unnamed rows.
    pub fn custom_fields(&self) -> Vec<(String, String, bool)> {
        let mut out = Vec::new();
        let mut i = FIXED;
        while i + 1 < self.fields.len() {
            let name = self.fields[i].input.value.trim().to_string();
            let protected = matches!(
                self.fields[i + 1].kind,
                FieldKind::CustomValue { protected: true }
            );
            if !name.is_empty() {
                out.push((name, self.fields[i + 1].input.value.clone(), protected));
            }
            i += 2;
        }
        out
    }

    fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.fields.len();
    }

    fn focus_prev(&mut self) {
        self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
    }

    fn add_custom(&mut self) {
        self.fields
            .push(FormField::new("name", "", FieldKind::CustomName));
        self.fields.push(FormField::new(
            "value",
            "",
            FieldKind::CustomValue { protected: false },
        ));
        self.focus = self.fields.len() - 2;
        self.dirty = true;
    }

    fn remove_custom(&mut self) {
        if self.focus < FIXED {
            return;
        }
        let pair = FIXED + ((self.focus - FIXED) / 2) * 2;
        if pair + 1 < self.fields.len() {
            self.fields.drain(pair..pair + 2);
            self.focus = self.focus.min(self.fields.len().saturating_sub(1));
            self.dirty = true;
        }
    }

    /// Toggle memory protection on the custom field under the cursor.
    fn toggle_protected(&mut self) {
        if self.focus < FIXED {
            return;
        }
        let pair = FIXED + ((self.focus - FIXED) / 2) * 2;
        if let Some(f) = self.fields.get_mut(pair + 1) {
            if let FieldKind::CustomValue { protected } = f.kind {
                f.kind = FieldKind::CustomValue {
                    protected: !protected,
                };
                self.dirty = true;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return FormAction::Cancel,
            KeyCode::Char('s') if ctrl => return FormAction::Save,
            KeyCode::Char('g') if ctrl => return FormAction::Generate,
            KeyCode::Char('g') if alt => return FormAction::GenerateOptions,
            KeyCode::Char('r') if ctrl => {
                self.reveal = !self.reveal;
                return FormAction::None;
            }
            KeyCode::Char('n') if ctrl => {
                self.add_custom();
                return FormAction::None;
            }
            KeyCode::Char('d') if ctrl => {
                self.remove_custom();
                return FormAction::None;
            }
            KeyCode::Char('t') if ctrl => {
                self.toggle_protected();
                return FormAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.focus_next();
                return FormAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus_prev();
                return FormAction::None;
            }
            KeyCode::Enter => {
                let kind = self.fields[self.focus].kind;
                if kind == FieldKind::Multiline {
                    self.fields[self.focus].input.insert('\n');
                    self.dirty = true;
                    return FormAction::None;
                }
                if self.focus + 1 == self.fields.len() {
                    return FormAction::Save;
                }
                self.focus_next();
                return FormAction::None;
            }
            _ => {}
        }
        if self.fields[self.focus].input.handle_key(key) {
            self.dirty = true;
        }
        FormAction::None
    }
}

fn base_fields(
    title: &str,
    username: &str,
    password: &str,
    url: &str,
    notes: &str,
    expiry: &str,
) -> Vec<FormField> {
    vec![
        FormField::new("Title", title, FieldKind::Text),
        FormField::new("Username", username, FieldKind::Text),
        FormField::new("Password", password, FieldKind::Secret),
        FormField::new("URL", url, FieldKind::Text),
        FormField::new("Notes", notes, FieldKind::Multiline),
        FormField::new("Expires", expiry, FieldKind::Expiry),
    ]
}
