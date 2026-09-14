//! Modal overlays that are not the edit form: confirmations, text prompts,
//! the move-target group picker and the password generator options.

use chiave_core::{generate::CharOptions, EntryId, GroupId, NodeId};

use crate::form::TextInput;

/// What a confirmation dialog is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    DeleteEntry {
        id: EntryId,
        permanent: bool,
    },
    DeleteGroup {
        id: GroupId,
        permanent: bool,
    },
    /// `q` with unsaved changes.
    QuitUnsaved,
    /// `Ctrl-c`: quit without saving.
    QuitForce,
    /// `L` with unsaved changes.
    LockUnsaved,
    /// Esc on a dirty edit form.
    DiscardForm,
    /// The file changed on disk since it was opened.
    SaveConflict,
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub key: char,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub title: String,
    pub body: Vec<String>,
    pub choices: Vec<Choice>,
    pub action: DialogAction,
}

impl Dialog {
    pub fn new(
        title: &str,
        body: Vec<String>,
        choices: &[(char, &str)],
        action: DialogAction,
    ) -> Self {
        Dialog {
            title: title.to_string(),
            body,
            choices: choices
                .iter()
                .map(|(key, label)| Choice {
                    key: *key,
                    label: label.to_string(),
                })
                .collect(),
            action,
        }
    }

    pub fn yes_no(title: &str, body: Vec<String>, action: DialogAction) -> Self {
        Dialog::new(title, body, &[('y', "Yes"), ('n', "No")], action)
    }

    pub fn accepts(&self, key: char) -> bool {
        self.choices
            .iter()
            .any(|c| c.key.eq_ignore_ascii_case(&key))
    }
}

/// A single-line text prompt (rename a group, name a new group).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    RenameGroup(GroupId),
    NewGroup(GroupId),
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub title: String,
    pub input: TextInput,
    pub kind: PromptKind,
}

/// The move-target picker: a flat, fully expanded group tree.
#[derive(Debug)]
pub struct Picker {
    pub what: NodeId,
    pub label: String,
    pub groups: Vec<(GroupId, usize, String)>,
    pub selected: usize,
}

impl Picker {
    pub fn selected_group(&self) -> Option<GroupId> {
        self.groups.get(self.selected).map(|g| g.0)
    }

    pub fn next(&mut self) {
        if self.selected + 1 < self.groups.len() {
            self.selected += 1;
        }
    }

    pub fn prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

/// The `Alt-g` generator options popup.
#[derive(Debug, Clone)]
pub struct GenOptions {
    pub length: usize,
    pub special: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            length: CharOptions::default().length,
            special: true,
            exclude_ambiguous: false,
        }
    }
}

impl GenOptions {
    pub fn to_char_options(&self) -> CharOptions {
        CharOptions {
            length: self.length,
            special: if self.special {
                chiave_core::generate::SPECIAL.to_string()
            } else {
                String::new()
            },
            min_special: if self.special { 1 } else { 0 },
            exclude_ambiguous: self.exclude_ambiguous,
            ..CharOptions::default()
        }
    }
}
