fn main() {
    println!("cargo:rerun-if-changed=client/dist");
    assert!(
        std::path::Path::new("client/dist/index.html").is_file(),
        "Frontend is missing: run `npm --prefix client ci` and `npm --prefix client run build` before compiling Rust"
    );
}
