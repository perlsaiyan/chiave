//! Write the shared sample vault somewhere so the binary can be tried by hand:
//!
//! ```sh
//! cargo run -p chiave-shell --example write-sample -- /tmp/sample.kdbx
//! CHIAVE_PASSWORD=test chiave --kdb /tmp/sample.kdbx ls
//! ```

use std::path::PathBuf;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: write-sample <path/to/sample.kdbx>");
        std::process::exit(2);
    });
    let path = PathBuf::from(arg);
    let dir = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let (written, _) = chiave_core::testdb::sample_file(&dir);
    if written != path {
        std::fs::rename(&written, &path).expect("rename sample");
    }
    println!(
        "wrote {} (password: {})",
        path.display(),
        chiave_core::testdb::PASSWORD
    );
}
