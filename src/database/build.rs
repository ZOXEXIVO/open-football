fn main() {
    // Ensure the database crate recompiles whenever data files change.
    // Without this, include_dir! won't pick up new/modified JSON files
    // because cargo doesn't track filesystem changes for proc macros.
    println!("cargo:rerun-if-changed=src/data");
}
