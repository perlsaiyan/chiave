//! The commands that change a vault: groups, entries, fields, attachments and
//! everything around saving. They live here to keep [`crate::shell`] readable;
//! the dispatch table is still the one in [`crate::command`].
//!
//! Every prompt goes through [`crate::PasswordPrompt`] (masked) or
//! [`crate::LinePrompt`] (echoed), so tests can script a whole `new` or `edit`
//! session. Generated passwords are never printed unless the user asks for them
//! by name at the confirmation prompt.

use std::io::{BufRead, Write};
use std::path::Path;

use chiave_core::generate::{self, CharOptions, WordOptions};
use chiave_core::path::{self, Segment};
use chiave_core::{
    fields, Credentials, EntryId, EntryPatch, ExposeSecret, FieldValue, GroupId, NewEntry, NodeId,
    SaveError, SaveOptions, SaveReport, SecretString, Vault,
};
use chrono::{NaiveDate, NaiveDateTime};

use crate::shell::{Shell, ShellError};

/// The flags of `new`, gathered so the dispatch table stays readable.
#[derive(Debug, Default, Clone)]
pub struct NewArgs {
    pub title: Option<String>,
    pub user: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub password_from_stdin: bool,
    pub generate: bool,
    pub length: Option<usize>,
    pub no_special: bool,
    pub path: Option<String>,
}

impl NewArgs {
    /// Any flag at all means "do not ask questions".
    fn non_interactive(&self) -> bool {
        self.title.is_some()
            || self.user.is_some()
            || self.url.is_some()
            || self.notes.is_some()
            || self.password_from_stdin
            || self.generate
    }
}

/// `\n` in a one-line answer becomes a real line break.
fn unescape_notes(s: &str) -> String {
    s.replace("\\n", "\n")
}

/// The inverse, for showing a multi-line value as a one-line default.
fn escape_notes(s: &str) -> String {
    s.replace('\n', "\\n")
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The unescaped last component of a path spec, e.g. `/a/b\/c` -> `b/c`.
fn last_component(spec: &str) -> String {
    let (_, last) = path::split_for_completion(spec.trim_end_matches('/'));
    match path::parse(last).as_slice() {
        [Segment::Name(n)] => n.clone(),
        _ => String::new(),
    }
}

/// Everything before the last component, kept in spec (escaped) form.
fn parent_spec(spec: &str) -> String {
    path::split_for_completion(spec.trim_end_matches('/'))
        .0
        .to_string()
}

fn parse_date(text: &str) -> anyhow::Result<NaiveDateTime> {
    let d = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("{text}: not a date (use YYYY-MM-DD or `never`)"))?;
    Ok(d.and_hms_opt(23, 59, 59).expect("valid time"))
}

impl Shell {
    // ----- prompting --------------------------------------------------------

    fn ask_line(&self, msg: &str) -> anyhow::Result<String> {
        self.line
            .line(msg)
            .map_err(|e| anyhow::anyhow!("cannot read the answer: {e}"))
    }

    fn ask_secret(&self, msg: &str) -> anyhow::Result<SecretString> {
        self.prompt
            .prompt(msg)
            .map_err(|e| anyhow::anyhow!("cannot read the answer: {e}"))
    }

    fn char_options(&self, length: Option<usize>, no_special: bool) -> CharOptions {
        let mut o = CharOptions::default();
        if let Some(n) = length {
            o.length = n;
        }
        if no_special {
            o.special = String::new();
            o.min_special = 0;
        }
        o
    }

    /// The configured word list, one word per line (a leading diceware number is
    /// ignored).
    fn wordlist(&self) -> anyhow::Result<Vec<String>> {
        let path = self.opts.pwwords.as_ref().ok_or_else(|| {
            anyhow::anyhow!("no word list configured (set `pwwords` in the configuration file)")
        })?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
        let words: Vec<String> = text
            .lines()
            .filter_map(|l| l.split_whitespace().next_back().map(str::to_string))
            .collect();
        if words.is_empty() {
            anyhow::bail!("{} holds no words", path.display());
        }
        Ok(words)
    }

    fn gen_password(
        &self,
        length: Option<usize>,
        no_special: bool,
    ) -> anyhow::Result<(SecretString, f64)> {
        let o = self.char_options(length, no_special);
        let pw = generate::password(&o)?;
        Ok((pw, generate::entropy_bits(&o)))
    }

    fn gen_passphrase(&self, words: Option<usize>) -> anyhow::Result<(SecretString, f64)> {
        let list = self.wordlist()?;
        let mut o = WordOptions::default();
        if let Some(n) = words {
            o.words = n;
        }
        let pw = generate::passphrase(&list, &o)?;
        let bits = o.words as f64 * (list.len() as f64).log2();
        Ok((pw, bits))
    }

    /// Report a generated secret by shape only, and let the user look at it,
    /// accept it or ask for another one.
    fn confirm_generated(
        &self,
        pw: &SecretString,
        bits: f64,
        out: &mut dyn Write,
    ) -> anyhow::Result<bool> {
        writeln!(
            out,
            "Generated {} characters, about {bits:.0} bits of entropy.",
            pw.expose_secret().chars().count()
        )?;
        loop {
            match self
                .ask_line("Accept? [Y/n/show]: ")?
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "" | "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                "s" | "show" => writeln!(out, "{}", pw.expose_secret())?,
                other => writeln!(out, "Please answer y, n or show (not {other:?}).")?,
            }
        }
    }

    /// The `new` password question: empty offers generation, anything else is
    /// taken literally.
    fn ask_new_password(&self, out: &mut dyn Write) -> anyhow::Result<Option<SecretString>> {
        let first = self.ask_secret("Password (empty to generate): ")?;
        if !first.expose_secret().is_empty() {
            return Ok(Some(first));
        }
        loop {
            let answer =
                self.ask_secret("Generate: `g` random, `w` passphrase, or type a password: ")?;
            let (pw, bits) = match answer.expose_secret() {
                "" => return Ok(None),
                "g" => self.gen_password(None, false)?,
                "w" => self.gen_passphrase(None)?,
                _ => return Ok(Some(answer)),
            };
            if self.confirm_generated(&pw, bits, out)? {
                return Ok(Some(pw));
            }
        }
    }

    /// The `edit` password question: empty keeps the current one.
    fn ask_edit_password(&self, out: &mut dyn Write) -> anyhow::Result<Option<SecretString>> {
        let answer = self
            .ask_secret("Password [********] (empty keeps it, `g` generates, `w` passphrase): ")?;
        let want_words = match answer.expose_secret() {
            "" => return Ok(None),
            "g" => false,
            "w" => true,
            _ => return Ok(Some(answer)),
        };
        loop {
            let (pw, bits) = if want_words {
                self.gen_passphrase(None)?
            } else {
                self.gen_password(None, false)?
            };
            if self.confirm_generated(&pw, bits, out)? {
                return Ok(Some(pw));
            }
        }
    }

    // ----- groups -----------------------------------------------------------

    pub(crate) fn cmd_mkdir(&mut self, spec: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.mkdir(spec)?;
        writeln!(out, "Created {}", vault.group_path(id))?;
        Ok(())
    }

    pub(crate) fn cmd_rmdir(
        &mut self,
        spec: &str,
        recursive: bool,
        permanent: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.resolve_group(spec)?;
        let was = vault.group_path(id);
        vault.rmdir(id, recursive, permanent)?;
        if vault.db().group(id).is_some() {
            writeln!(out, "Moved {was} to {}", vault.group_path(id))?;
        } else {
            writeln!(out, "Deleted {was}")?;
        }
        Ok(())
    }

    pub(crate) fn cmd_rename(
        &mut self,
        spec: &str,
        new_name: &str,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let id = vault.resolve_group(spec)?;
        let was = vault.group_path(id);
        vault.rename_group(id, new_name)?;
        writeln!(out, "Renamed {was} to {}", vault.group_path(id))?;
        Ok(())
    }

    // ----- new --------------------------------------------------------------

    pub(crate) fn cmd_new(&mut self, args: NewArgs, out: &mut dyn Write) -> anyhow::Result<()> {
        // Fail before asking anything when the vault cannot be written at all.
        self.check_writable()?;
        let spec = args.path.clone().unwrap_or_default();
        let parent = parent_spec(&spec);
        let default_title = args.title.clone().unwrap_or_else(|| last_component(&spec));

        let mut new = NewEntry::default();
        if args.non_interactive() {
            new.title = default_title;
            new.username = args.user.clone().filter(|s| !s.is_empty());
            new.url = args.url.clone().filter(|s| !s.is_empty());
            new.notes = args.notes.as_deref().map(unescape_notes);
            if args.password_from_stdin {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                new.password = Some(SecretString::from(
                    line.trim_end_matches(['\r', '\n']).to_string(),
                ));
            } else if args.generate {
                let (pw, bits) = self.gen_password(args.length, args.no_special)?;
                writeln!(
                    out,
                    "Generated {} characters, about {bits:.0} bits of entropy.",
                    pw.expose_secret().chars().count()
                )?;
                new.password = Some(pw);
            }
        } else {
            let title = {
                let msg = if default_title.is_empty() {
                    "Title: ".to_string()
                } else {
                    format!("Title [{default_title}]: ")
                };
                let typed = self.ask_line(&msg)?;
                if typed.trim().is_empty() {
                    default_title
                } else {
                    typed
                }
            };
            new.title = title;
            new.username = Some(self.ask_line("Username: ")?).filter(|s| !s.is_empty());
            new.password = self.ask_new_password(out)?;
            new.url = Some(self.ask_line("URL: ")?).filter(|s| !s.is_empty());
            new.notes = Some(unescape_notes(
                &self.ask_line("Notes (`\\n` for a line break): ")?,
            ))
            .filter(|s| !s.is_empty());
            loop {
                let name = self.ask_line("Add another field? (name or empty): ")?;
                let name = name.trim().to_string();
                if name.is_empty() {
                    break;
                }
                let protected = is_yes(&self.ask_line(&format!("{name}: protected? [y/N]: "))?);
                let value = if protected {
                    FieldValue::Protected(self.ask_secret(&format!("{name}: "))?)
                } else {
                    FieldValue::Plain(self.ask_line(&format!("{name}: "))?)
                };
                new.custom.push((name, value));
            }
        }

        if new.title.is_empty() {
            anyhow::bail!(ShellError::Refused(
                "an entry needs a title (pass a path or --title)".into()
            ));
        }
        let spec = format!("{parent}{}", path::escape(&new.title));
        let vault = self.need_vault()?;
        let id = vault.new_entry(&spec, new)?;
        writeln!(out, "Created {}", vault.entry_path(id))?;
        Ok(())
    }

    // ----- edit -------------------------------------------------------------

    pub(crate) fn cmd_edit(&mut self, spec: &str, out: &mut dyn Write) -> anyhow::Result<()> {
        self.check_writable()?;
        let id = self.need_vault()?.resolve_entry(spec)?;
        self.edit_entry_interactive(id, out)
    }

    fn edit_entry_interactive(&mut self, id: EntryId, out: &mut dyn Write) -> anyhow::Result<()> {
        let view = self.need_vault()?.entry(id)?;
        let mut patch = EntryPatch::default();

        let title = self.ask_line(&format!("Title [{}]: ", view.title))?;
        if !title.is_empty() {
            patch = patch.set_plain(fields::TITLE, title);
        }
        let user = self.ask_line(&format!(
            "Username [{}]: ",
            view.username.as_deref().unwrap_or("")
        ))?;
        if !user.is_empty() {
            patch = patch.set_plain(fields::USERNAME, user);
        }
        if let Some(pw) = self.ask_edit_password(out)? {
            patch = patch.set(fields::PASSWORD, FieldValue::Protected(pw));
        }
        let url = self.ask_line(&format!("URL [{}]: ", view.url.as_deref().unwrap_or("")))?;
        if !url.is_empty() {
            patch = patch.set_plain(fields::URL, url);
        }
        let notes = self.ask_line(&format!(
            "Notes [{}]: ",
            escape_notes(view.notes.as_deref().unwrap_or(""))
        ))?;
        if !notes.is_empty() {
            patch = patch.set_plain(fields::NOTES, unescape_notes(&notes));
        }

        for (name, value) in &view.custom {
            let protected = matches!(value, FieldValue::Protected(_));
            let answer = if protected {
                self.ask_secret(&format!(
                    "{name} [********] (empty keeps it, `d` deletes it): "
                ))?
                .expose_secret()
                .to_string()
            } else {
                let shown = match value {
                    FieldValue::Plain(v) => escape_notes(v),
                    FieldValue::Protected(_) => unreachable!("handled above"),
                };
                self.ask_line(&format!(
                    "{name} [{shown}] (empty keeps it, `d` deletes it): "
                ))?
            };
            patch = match answer.as_str() {
                "" => patch,
                "d" => patch.remove(name),
                v if protected => patch.set(name, FieldValue::Protected(SecretString::from(v))),
                v => patch.set(name, FieldValue::Plain(unescape_notes(v))),
            };
        }

        if patch.is_empty() {
            writeln!(out, "No changes.")?;
            return Ok(());
        }
        let vault = self.need_vault()?;
        if vault.edit_entry(id, patch)? {
            writeln!(out, "Updated {}", vault.entry_path(id))?;
        } else {
            writeln!(out, "No changes.")?;
        }
        Ok(())
    }

    // ----- set --------------------------------------------------------------

    pub(crate) fn cmd_set(
        &mut self,
        spec: &str,
        field: &str,
        value: Option<&str>,
        delete: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        self.check_writable()?;
        let id = self.need_vault()?.resolve_entry(spec)?;

        if matches!(field.to_ascii_lowercase().as_str(), "expires" | "expiry") {
            let text = match value {
                Some(v) => v.to_string(),
                None => self.ask_line("Expires (YYYY-MM-DD or `never`): ")?,
            };
            let expires = match text.trim() {
                "" | "never" | "Never" => None,
                t => Some(parse_date(t)?),
            };
            let patch = EntryPatch {
                expires: Some(expires),
                ..EntryPatch::default()
            };
            return self.apply(id, patch, "expires", out);
        }

        let key = fields::canonical(field);
        let view = self.need_vault()?.entry(id)?;
        let current = view.custom.iter().find(|(k, _)| *k == key);
        let protected =
            fields::is_secret(&key) || matches!(current, Some((_, FieldValue::Protected(_))));

        if delete {
            if fields::is_standard(&key) {
                anyhow::bail!(ShellError::Refused(format!(
                    "{key} is a standard field; set it to an empty value instead of deleting it"
                )));
            }
            if current.is_none() {
                anyhow::bail!(ShellError::NoSuchField(field.to_string()));
            }
            return self.apply(id, EntryPatch::default().remove(&key), &key, out);
        }

        let text = match value {
            Some(v) => v.to_string(),
            None if protected => self
                .ask_secret(&format!("{key}: "))?
                .expose_secret()
                .to_string(),
            None => self.ask_line(&format!("{key}: "))?,
        };
        let text = if key == fields::NOTES {
            unescape_notes(&text)
        } else {
            text
        };
        let value = if protected {
            FieldValue::Protected(SecretString::from(text))
        } else {
            FieldValue::Plain(text)
        };
        self.apply(id, EntryPatch::default().set(&key, value), &key, out)
    }

    fn apply(
        &mut self,
        id: EntryId,
        patch: EntryPatch,
        what: &str,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        if vault.edit_entry(id, patch)? {
            writeln!(out, "Updated {} ({what})", vault.entry_path(id))?;
        } else {
            writeln!(out, "No changes.")?;
        }
        Ok(())
    }

    // ----- rm / mv / cp -----------------------------------------------------

    pub(crate) fn cmd_rm(
        &mut self,
        spec: &str,
        permanent: bool,
        force: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        self.check_writable()?;
        let id = self.need_vault()?.resolve_entry(spec)?;
        let was = self.need_vault()?.entry_path(id);
        if !force {
            let answer = self.ask_line(&format!("Delete {was}? [y/N] "))?;
            if !is_yes(&answer) {
                writeln!(out, "Cancelled.")?;
                return Ok(());
            }
        }
        let vault = self.need_vault()?;
        vault.rm_entry(id, permanent)?;
        if vault.db().entry(id).is_some() {
            writeln!(out, "Moved {was} to {}", vault.entry_path(id))?;
        } else {
            writeln!(out, "Deleted {was}")?;
        }
        Ok(())
    }

    pub(crate) fn cmd_mv(
        &mut self,
        spec: &str,
        dest: &str,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        let node: NodeId = vault.resolve_one(spec)?;
        let group = vault.resolve_group(dest)?;
        let was = vault.node_path(node);
        vault.mv(node, group)?;
        writeln!(out, "Moved {was} to {}", vault.node_path(node))?;
        Ok(())
    }

    pub(crate) fn cmd_cp(
        &mut self,
        spec: &str,
        dest: &str,
        then_edit: bool,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        self.check_writable()?;
        let new_id = {
            let vault = self.need_vault()?;
            let id = vault.resolve_entry(spec)?;
            let (group, title) = destination(vault, dest)?;
            let was = vault.entry_path(id);
            let new_id = vault.copy_entry(id, group, title.as_deref())?;
            writeln!(out, "Copied {was} to {}", vault.entry_path(new_id))?;
            new_id
        };
        if then_edit {
            self.edit_entry_interactive(new_id, out)?;
        }
        Ok(())
    }

    // ----- attachments ------------------------------------------------------

    pub(crate) fn cmd_attach(
        &mut self,
        spec: &str,
        add: Option<&Path>,
        name: Option<&str>,
        export: Option<Vec<String>>,
        rm: Option<&str>,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        let id = self.need_vault()?.resolve_entry(spec)?;
        let acted = add.is_some() || export.is_some() || rm.is_some();

        if let Some(file) = add {
            let data = std::fs::read(file)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
            let name = name
                .map(str::to_string)
                .or_else(|| file.file_name().map(|n| n.to_string_lossy().into_owned()))
                .ok_or_else(|| {
                    ShellError::Refused(format!("{}: no file name; pass --name", file.display()))
                })?;
            let size = data.len();
            self.need_vault()?.add_attachment(id, &name, data)?;
            writeln!(out, "Attached {name} ({size} bytes).")?;
        }
        if let Some(args) = export {
            let [from, to] = <[String; 2]>::try_from(args)
                .map_err(|_| ShellError::Refused("--export takes a name and a file".into()))?;
            let data = self.need_vault()?.attachment_data(id, &from)?;
            std::fs::write(&to, &*data).map_err(|e| anyhow::anyhow!("cannot write {to}: {e}"))?;
            writeln!(out, "Wrote {to} ({} bytes).", data.len())?;
        }
        if let Some(name) = rm {
            self.need_vault()?.remove_attachment(id, name)?;
            writeln!(out, "Removed attachment {name}.")?;
        }
        if acted {
            return Ok(());
        }

        let vault = self.need_vault()?;
        let list = vault.attachments(id)?;
        if list.is_empty() {
            writeln!(out, "No attachments.")?;
            return Ok(());
        }
        let width = list
            .iter()
            .map(|(n, _)| n.chars().count())
            .max()
            .unwrap_or(0);
        for (name, size) in list {
            writeln!(out, "{name:width$}  {size} bytes")?;
        }
        Ok(())
    }

    // ----- the database itself ----------------------------------------------

    /// Refuse early, with the core's own wording, when nothing can be written.
    fn check_writable(&mut self) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        if vault.is_read_only() {
            anyhow::bail!(chiave_core::WriteError::ReadOnly);
        }
        if !vault.can_save() {
            anyhow::bail!(chiave_core::WriteError::UnsupportedVersion(
                vault.version().to_string()
            ));
        }
        Ok(())
    }

    fn report_save(&self, report: &SaveReport, out: &mut dyn Write) -> anyhow::Result<()> {
        let verified = if report.verified { ", verified" } else { "" };
        writeln!(
            out,
            "Saved {} ({} bytes{verified}).",
            report.path.display(),
            report.bytes
        )?;
        if let Some(bak) = &report.backup {
            writeln!(out, "Backup: {}", bak.display())?;
        }
        Ok(())
    }

    pub(crate) fn cmd_save(&mut self, force: bool, out: &mut dyn Write) -> anyhow::Result<()> {
        let opts = SaveOptions {
            force,
            ..SaveOptions::default()
        };
        let report = match self.need_vault()?.save(opts) {
            Ok(r) => r,
            Err(e @ SaveError::ChangedOnDisk(_)) => anyhow::bail!(
                "{e}\nAnother program wrote the file after chiave opened it. \
                 Re-open it to keep both sets of changes, or run `save --force` to overwrite it."
            ),
            Err(e) => return Err(e.into()),
        };
        self.report_save(&report, out)
    }

    pub(crate) fn cmd_saveas(&mut self, file: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
        let report = self.need_vault()?.save_to(file, SaveOptions::default())?;
        self.report_save(&report, out)
    }

    pub(crate) fn cmd_passwd(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        self.check_writable()?;
        let keyfile = self.keyfile.clone();
        let password = self.ask_new_master("New master password: ", keyfile.is_some())?;
        let creds = Credentials { password, keyfile };
        self.need_vault()?.change_credentials(&creds)?;
        writeln!(
            out,
            "Master password changed. Run `save` to write it to disk."
        )?;
        Ok(())
    }

    /// Ask for a new master password twice and check the two answers match.
    fn ask_new_master(
        &self,
        msg: &str,
        have_keyfile: bool,
    ) -> anyhow::Result<Option<SecretString>> {
        let first = self.ask_secret(msg)?;
        let again = self.ask_secret("Repeat the password: ")?;
        if first.expose_secret() != again.expose_secret() {
            anyhow::bail!(ShellError::Refused("the passwords do not match".into()));
        }
        if first.expose_secret().is_empty() {
            if !have_keyfile {
                anyhow::bail!(ShellError::Refused(
                    "an empty password needs a key file".into()
                ));
            }
            return Ok(None);
        }
        Ok(Some(first))
    }

    pub(crate) fn cmd_newdb(&mut self, file: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
        // Switching databases drops the open one; never do that silently.
        if self.is_dirty() {
            anyhow::bail!(ShellError::UnsavedChanges);
        }
        let msg = format!("New master password for {}: ", file.display());
        let password = self.ask_new_master(&msg, false)?;
        let creds = Credentials {
            password,
            keyfile: None,
        };
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "chiave".to_string());
        let vault = Vault::create(file, &creds, &name)?;
        self.vault = Some(vault);
        self.locked = None;
        self.keyfile = None;
        writeln!(out, "Created {}.", file.display())?;
        Ok(())
    }

    pub(crate) fn cmd_upgrade(&mut self, out: &mut dyn Write) -> anyhow::Result<()> {
        let vault = self.need_vault()?;
        if vault.upgrade_to_kdbx4()? {
            writeln!(
                out,
                "Converted to KDBX4 with Argon2id in memory. Run `save` to write it; \
                 the KDBX3 file is kept as a .bak next to it."
            )?;
        } else {
            writeln!(out, "Already KDBX4; nothing to do.")?;
        }
        Ok(())
    }

    // ----- pwgen ------------------------------------------------------------

    pub(crate) fn cmd_pwgen(
        &mut self,
        length: Option<usize>,
        words: Option<usize>,
        no_special: bool,
        count: usize,
        out: &mut dyn Write,
    ) -> anyhow::Result<()> {
        for _ in 0..count.max(1) {
            let (pw, _) = if words.is_some() {
                self.gen_passphrase(words)?
            } else {
                self.gen_password(length, no_special)?
            };
            writeln!(out, "{}", pw.expose_secret())?;
        }
        Ok(())
    }
}

/// Resolve a `cp`/`clone` destination: an existing group, or `group/NewTitle`.
fn destination(vault: &Vault, dest: &str) -> anyhow::Result<(GroupId, Option<String>)> {
    if let Ok(g) = vault.resolve_group(dest) {
        return Ok((g, None));
    }
    let title = last_component(dest);
    if title.is_empty() {
        anyhow::bail!(ShellError::Refused(format!("{dest}: not a group")));
    }
    let parent = parent_spec(dest);
    let group = if parent.is_empty() {
        vault.cwd()
    } else {
        vault.resolve_group(&parent)?
    };
    Ok((group, Some(title)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_round_trip_through_one_line() {
        assert_eq!(unescape_notes(r"a\nb"), "a\nb");
        assert_eq!(escape_notes("a\nb"), r"a\nb");
    }

    #[test]
    fn splits_a_spec_into_parent_and_name() {
        assert_eq!(parent_spec("/a/b"), "/a/");
        assert_eq!(last_component("/a/b"), "b");
        assert_eq!(last_component(r"/a/b\/c"), "b/c");
        assert_eq!(parent_spec("Title"), "");
    }

    #[test]
    fn parses_dates() {
        assert_eq!(
            parse_date("2030-01-02").unwrap().to_string(),
            "2030-01-02 23:59:59"
        );
        assert!(parse_date("tomorrow").is_err());
    }
}
