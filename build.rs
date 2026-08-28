// Famulus – Build-Skript v0.10.0.
// Erzeugt aus src/ffi.udl das Rust-Scaffolding für UniFFI.
// Läuft bei jedem cargo build automatisch.
// (Muster: Famulus Games build.rs, dort bewährt seit v0.2.0.)

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let udl = format!("{manifest}/src/ffi.udl");
    println!("cargo:rerun-if-changed={udl}");
    // generate_scaffolding erwartet einen Pfad, von dem aus es die
    // Cargo.toml der Crate finden kann – daher absolut.
    uniffi::generate_scaffolding(udl.as_str()).unwrap();
}
