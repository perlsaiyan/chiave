//! Try the TUI against a throwaway sample vault:
//!
//! ```shell
//! cargo run -p chiave-tui --example demo
//! ```
//!
//! The vault lives in a temp directory and is deleted on exit, so editing,
//! deleting and saving are all safe to play with. The master password is
//! `test` (needed only after the idle lock, which is set to 60 seconds here).

use std::time::Duration;

use chiave_clip::{detect, DetectOptions};
use chiave_core::{testdb, Vault};
use chiave_tui::{run, NoPrompt, TuiOptions};

fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (path, creds) = testdb::sample_file(dir.path());
    let vault = Vault::open(&path, &creds, false)?;

    println!(
        "sample vault: {} (password: {})",
        path.display(),
        testdb::PASSWORD
    );

    let opts = TuiOptions {
        clip_timeout: Some(Duration::from_secs(10)),
        idle_lock: Some(Duration::from_secs(60)),
        read_only: false,
    };
    run(
        vault,
        detect(DetectOptions::default()),
        opts,
        Box::new(NoPrompt),
    )?;
    println!("the sample vault was removed");
    Ok(())
}
