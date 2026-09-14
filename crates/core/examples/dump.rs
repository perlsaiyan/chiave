use chiave_core::{open, walk, Credentials};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let db = open(
        std::path::Path::new(&a[1]),
        &Credentials::password(a.get(2).cloned().unwrap_or_default()),
    )
    .unwrap();
    println!("version {}", db.config.version);
    for i in walk(&db) {
        println!("{}{}", i.path, if i.is_group { "/" } else { "" });
    }
}
