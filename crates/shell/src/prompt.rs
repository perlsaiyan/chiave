//! Asking the user for passwords and for plain answers to interactive prompts.

use std::collections::VecDeque;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chiave_core::SecretString;

/// How the shell asks for a master password (idle unlock, `open`) and for any
/// value that must not be echoed (entry passwords, protected custom fields).
pub trait PasswordPrompt {
    fn prompt(&self, msg: &str) -> io::Result<SecretString>;
}

/// How the shell asks for a plain, echoed answer (`new`, `edit`, confirmations).
pub trait LinePrompt {
    fn line(&self, msg: &str) -> io::Result<String>;
}

/// Reads from the terminal without echo. With input coming from a pipe or a
/// file there is no terminal to turn the echo off on, so the answer is read from
/// stdin like any other line; that is what lets `chiave < script` drive `new`,
/// `edit` and `passwd`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RpasswordPrompt;

impl PasswordPrompt for RpasswordPrompt {
    fn prompt(&self, msg: &str) -> io::Result<SecretString> {
        if !io::stdin().is_terminal() {
            return StdinPrompt.line(msg).map(SecretString::from);
        }
        let pw = rpassword::prompt_password(msg)?;
        Ok(SecretString::from(pw))
    }
}

/// Reads one echoed line from stdin. End of input answers with an empty line,
/// which every prompt treats as "keep the default".
#[derive(Debug, Default, Clone, Copy)]
pub struct StdinPrompt;

impl LinePrompt for StdinPrompt {
    fn line(&self, msg: &str) -> io::Result<String> {
        let mut out = io::stdout();
        write!(out, "{msg}")?;
        out.flush()?;
        let mut buf = String::new();
        io::stdin().lock().read_line(&mut buf)?;
        while buf.ends_with('\n') || buf.ends_with('\r') {
            buf.pop();
        }
        Ok(buf)
    }
}

/// Always answers with the same password; for tests and `--pwfile`-style flows.
/// Clones share the call counter.
#[derive(Debug, Clone)]
pub struct FixedPrompt {
    password: SecretString,
    calls: Arc<AtomicUsize>,
}

impl FixedPrompt {
    pub fn new(password: impl Into<String>) -> Self {
        FixedPrompt {
            password: SecretString::from(password.into()),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many times the shell has asked for a password.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PasswordPrompt for FixedPrompt {
    fn prompt(&self, _msg: &str) -> io::Result<SecretString> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.password.clone())
    }
}

/// Answers interactive prompts from a prepared script, in order. Masked and
/// echoed prompts draw from the same queue, so a test writes the answers exactly
/// as a user would type them. Clones share the queue and the transcript.
#[derive(Debug, Clone, Default)]
pub struct ScriptedPrompt {
    answers: Arc<Mutex<VecDeque<String>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl ScriptedPrompt {
    pub fn new<I, S>(answers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ScriptedPrompt {
            answers: Arc::new(Mutex::new(
                answers.into_iter().map(Into::into).collect::<VecDeque<_>>(),
            )),
            asked: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Every prompt string shown so far, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("lock").clone()
    }

    /// Answers not consumed yet.
    pub fn remaining(&self) -> usize {
        self.answers.lock().expect("lock").len()
    }

    fn next(&self, msg: &str) -> io::Result<String> {
        self.asked.lock().expect("lock").push(msg.to_string());
        self.answers
            .lock()
            .expect("lock")
            .pop_front()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("the scripted prompt ran out of answers at {msg:?}"),
                )
            })
    }
}

impl LinePrompt for ScriptedPrompt {
    fn line(&self, msg: &str) -> io::Result<String> {
        self.next(msg)
    }
}

impl PasswordPrompt for ScriptedPrompt {
    fn prompt(&self, msg: &str) -> io::Result<SecretString> {
        Ok(SecretString::from(self.next(msg)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chiave_core::ExposeSecret;

    #[test]
    fn scripted_answers_come_back_in_order() {
        let p = ScriptedPrompt::new(["one", "two"]);
        assert_eq!(p.line("a: ").unwrap(), "one");
        assert_eq!(p.prompt("b: ").unwrap().expose_secret(), "two");
        assert!(p.line("c: ").is_err());
        assert_eq!(p.asked(), ["a: ", "b: ", "c: "]);
    }
}
