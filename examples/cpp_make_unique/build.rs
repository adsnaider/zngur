use zngur::Zngur;

fn main() {
    build::rerun_if_changed("main.zng");
    build::rerun_if_changed("shapes.h");
    build::rerun_if_changed("src/");

    let crate_dir = build::cargo_manifest_dir();

    Zngur::from_zng_file(crate_dir.join("main.zng"))
        .with_cpp_file(crate_dir.join("generated.cpp"))
        .with_h_file(crate_dir.join("generated.h"))
        .with_rs_file(crate_dir.join("./src/generated.rs"))
        .with_crate_name("crate")
        .with_zng_header("zngur.h")
        .generate();

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(".")
        .file("generated.cpp")
        .compile("zngur_generated");
}
