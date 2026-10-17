fn main() {
    // The interface in ../ui/dist is embedded into the program while the crate compiles, and cargo cannot see that on
    // its own: without this line a rebuilt interface is ignored and the program keeps the one it was last built with.
    println!("cargo:rerun-if-changed=../ui/dist");
    tauri_build::build();
}
