//! Shared helpers for the shell tests.

use std::sync::Arc;
use std::time::Duration;

use chiave_clip::MemoryClipboard;
use chiave_core::testdb;
use chiave_core::Vault;
use chiave_shell::{FixedPrompt, ScriptedPrompt, Shell, ShellOptions};
use tempfile::TempDir;

#[allow(dead_code)]
pub struct Harness {
    pub shell: Shell,
    pub clip: Arc<MemoryClipboard>,
    pub prompt: FixedPrompt,
    pub dir: TempDir,
}

/// Hands the shell a `MemoryClipboard` the test keeps a handle on.
struct SharedClip(Arc<MemoryClipboard>);

impl chiave_clip::Clipboard for SharedClip {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn copy_secret(
        &self,
        secret: &chiave_core::SecretString,
        clear_after: Option<Duration>,
    ) -> Result<(), chiave_clip::ClipError> {
        self.0.copy_secret(secret, clear_after)
    }
    fn copy_text(&self, text: &str) -> Result<(), chiave_clip::ClipError> {
        self.0.copy_text(text)
    }
    fn clear(&self) -> Result<(), chiave_clip::ClipError> {
        self.0.clear()
    }
}

#[allow(dead_code)]
pub fn harness() -> Harness {
    harness_with(ShellOptions::default())
}

#[allow(dead_code)]
pub fn harness_with(opts: ShellOptions) -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, creds) = testdb::sample_file(dir.path());
    let read_only = opts.read_only;
    let vault = Vault::open(&path, &creds, read_only).expect("open sample");
    harness_from(dir, vault, opts)
}

/// A harness over a vault the test opened itself; `dir` keeps its files alive.
#[allow(dead_code)]
pub fn harness_from(dir: TempDir, vault: Vault, opts: ShellOptions) -> Harness {
    let clip = Arc::new(MemoryClipboard::new());
    let prompt = FixedPrompt::new(testdb::PASSWORD);
    let mut shell = Shell::with_vault(vault, Box::new(SharedClip(Arc::clone(&clip))), opts);
    shell.set_prompt(Box::new(prompt.clone()));
    Harness {
        shell,
        clip,
        prompt,
        dir,
    }
}

impl Harness {
    /// Run a line and return everything it printed.
    #[allow(dead_code)]
    pub fn run(&mut self, line: &str) -> String {
        let mut out = Vec::new();
        self.shell.run_line(line, &mut out).expect("run_line");
        String::from_utf8(out).expect("utf-8 output")
    }

    /// Run a line and return its flow along with everything it printed.
    #[allow(dead_code)]
    pub fn run_flow(&mut self, line: &str) -> (chiave_shell::Flow, String) {
        let mut out = Vec::new();
        let flow = self.shell.run_line(line, &mut out).expect("run_line");
        (flow, String::from_utf8(out).expect("utf-8 output"))
    }

    /// Answer the next interactive prompts, masked and echoed alike, from a
    /// script. The returned handle shares the queue with the shell.
    #[allow(dead_code)]
    pub fn script<I, S>(&mut self, answers: I) -> ScriptedPrompt
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let p = ScriptedPrompt::new(answers);
        self.shell.set_prompt(Box::new(p.clone()));
        self.shell.set_line_prompt(Box::new(p.clone()));
        p
    }

    /// The vault's view of one entry, by spec.
    #[allow(dead_code)]
    pub fn entry(&mut self, spec: &str) -> chiave_core::EntryView {
        let vault = self.shell.vault().expect("a vault");
        let id = vault.resolve_entry(spec).expect("resolve");
        vault.entry(id).expect("entry")
    }

    /// The entry's password, exposed for assertions.
    #[allow(dead_code)]
    pub fn password(&mut self, spec: &str) -> String {
        use chiave_core::ExposeSecret;
        self.entry(spec)
            .password
            .map(|p| p.expose_secret().to_string())
            .unwrap_or_default()
    }

    /// The database file behind the open vault.
    #[allow(dead_code)]
    pub fn path(&self) -> std::path::PathBuf {
        self.shell.vault().expect("a vault").path().to_path_buf()
    }

    /// Run a line and assert it printed no error.
    #[allow(dead_code)]
    pub fn ok(&mut self, line: &str) -> String {
        let out = self.run(line);
        assert!(!out.contains("error:"), "`{line}` failed:\n{out}");
        out
    }
}
