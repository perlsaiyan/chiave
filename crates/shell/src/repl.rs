//! The interactive read-eval-print loop.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Context, Editor, Helper};

use crate::complete;
use crate::shell::{default_histfile, Flow, Shell};

/// Tab completion over the live shell state.
struct ChiaveHelper<'a> {
    shell: Rc<RefCell<&'a mut Shell>>,
}

impl Completer for ChiaveHelper<'_> {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let shell = self.shell.borrow();
        Ok(complete::complete(&shell, line, pos))
    }
}

impl Hinter for ChiaveHelper<'_> {
    type Hint = String;
}
impl Highlighter for ChiaveHelper<'_> {}
impl Validator for ChiaveHelper<'_> {}
impl Helper for ChiaveHelper<'_> {}

/// `/dev/null` (or an empty path) turns the history file off.
fn history_path(shell: &Shell) -> Option<PathBuf> {
    let path = shell.options().histfile.clone().or_else(default_histfile)?;
    if path.as_os_str().is_empty() || path == Path::new("/dev/null") {
        return None;
    }
    Some(path)
}

/// Run the interactive shell until `quit`, Ctrl-D or end of input.
pub fn repl(shell: &mut Shell) -> anyhow::Result<()> {
    let histfile = history_path(shell);
    let config = Config::builder()
        .auto_add_history(false)
        .completion_type(CompletionType::List)
        .build();
    let shared = Rc::new(RefCell::new(shell));
    let mut rl: Editor<ChiaveHelper<'_>, DefaultHistory> = Editor::with_config(config)?;
    rl.set_helper(Some(ChiaveHelper {
        shell: Rc::clone(&shared),
    }));
    if let Some(path) = &histfile {
        let _ = rl.load_history(path);
    }

    let mut stdout = std::io::stdout();
    loop {
        let prompt = shared.borrow().prompt_string();
        match rl.readline(&prompt) {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(line.as_str());
                let flow = {
                    let mut shell = shared.borrow_mut();
                    let flow = shell.run_line(&line, &mut stdout)?;
                    // `history -c` forgets the readline history too.
                    if shell.history().is_empty() {
                        let _ = rl.clear_history();
                    }
                    flow
                };
                stdout.flush()?;
                if flow == Flow::Quit {
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => {
                writeln!(stdout)?;
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }

    if let Some(path) = &histfile {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = rl.save_history(path);
    }
    Ok(())
}
