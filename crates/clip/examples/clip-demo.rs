//! Manual verification tool for the clipboard crate.
//!
//! It plays the role the `chiave` binary will play: dispatch `__clip-serve` to
//! `helper_main`, and otherwise copy something and exit immediately, so that the
//! "does the clipboard survive the caller exiting?" question can actually be
//! answered.
//!
//! ```sh
//! cargo build -p chiave-clip --example clip-demo
//! # copy a secret, auto-clear after 10s, then exit straight away
//! ./target/debug/examples/clip-demo secret hunter2 10
//! wl-paste --list-types      # expect x-kde-passwordManagerHint
//! cliphist list              # expect no hunter2
//! ./target/debug/examples/clip-demo text tom
//! ./target/debug/examples/clip-demo read
//! ./target/debug/examples/clip-demo clear
//! ```

use std::process::ExitCode;
use std::time::Duration;

use chiave_clip::{detect, DetectOptions, HELPER_SUBCOMMAND};
use secrecy::SecretString;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("usage: clip-demo <secret VALUE [SECS] | text VALUE | read | clear | name>");
        return ExitCode::FAILURE;
    };

    if command == HELPER_SUBCOMMAND {
        return chiave_clip::helper_main(args.collect());
    }

    let clip = detect(DetectOptions {
        helper_exe: std::env::current_exe().ok(),
        paste_once: std::env::var("CHIAVE_PASTE_ONCE").is_ok(),
    });
    eprintln!(
        "backend: {} (sensitive hint: {})",
        clip.name(),
        clip.supports_sensitive_hint()
    );

    let result = match command.as_str() {
        "secret" => {
            let value = args.next().unwrap_or_default();
            let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
            let clear_after = (secs > 0).then(|| Duration::from_secs(secs));
            clip.copy_secret(&SecretString::from(value), clear_after)
        }
        "text" => clip.copy_text(&args.next().unwrap_or_default()),
        "clear" => clip.clear(),
        "name" => Ok(()),
        "read" => match clip.read_back() {
            Ok(Some(value)) => {
                println!("{}", String::from_utf8_lossy(&value));
                Ok(())
            }
            Ok(None) => {
                println!("(empty)");
                Ok(())
            }
            Err(err) => Err(err),
        },
        other => {
            eprintln!("unknown command {other:?}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
