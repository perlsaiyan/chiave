fn main() -> anyhow::Result<()> {
    println!("chiave {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
