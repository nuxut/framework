//! Yalıtılmış görsel işçisi (ADR-0007). Ayrı paket: tüketiciler kendi workspace'lerinde
//! `[[bin]]` tanımlamaz, işçiyi framework'ten ayrı bir cargo çağrısıyla derler
//! (`cargo build --release --manifest-path <framework>/Cargo.toml -p nuxut-image-worker`):
//! işçinin özellikleri (JPEG XL) tüketicinin ana ikilisine birleşmez.
fn main() {
    nuxut_image_sandbox::worker_main()
}
