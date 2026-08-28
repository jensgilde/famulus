// Famulus – uniffi-bindgen-Hilfsprogramm v0.10.0.
// Wird per `cargo build --release --bin uniffi-bindgen` gebaut und von
// scripts/build-ffi.sh aufgerufen, um aus src/ffi.udl die Swift-Bindings
// zu erzeugen. Muster: Famulus Games src/bin/uniffi-bindgen.rs.

fn main() {
    uniffi::uniffi_bindgen_main()
}
