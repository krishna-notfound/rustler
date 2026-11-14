fn main() {
    // include_dir! cannot track the embedded folder on stable; do it here.
    println!("cargo:rerun-if-changed=../../knowledge");
}
