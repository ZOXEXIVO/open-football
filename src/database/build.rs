fn main() {
    // Rebuild when the embedded compiled database changes. The file is built
    // externally by `open-football-database/compiler` and dropped here.
    println!("cargo:rerun-if-changed=src/data/database.db");
}
