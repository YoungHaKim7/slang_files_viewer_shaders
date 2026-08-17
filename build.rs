use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/triangle.slang");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let shader = "assets/triangle.slang";

    compile_shader(
        shader,
        "vertMain",
        "vertex",
        &out_dir.join("triangle.vert.spv"),
    );

    compile_shader(
        shader,
        "fragMain",
        "fragment",
        &out_dir.join("triangle.frag.spv"),
    );
}

fn compile_shader(source: &str, entry: &str, stage: &str, output: &PathBuf) {
    let status = Command::new("slangc")
        .arg(source)
        .arg("-target")
        .arg("spirv")
        .arg("-entry")
        .arg(entry)
        .arg("-stage")
        .arg(stage)
        // Without this, slangc renames the SPIR-V entry point to "main",
        // which no longer matches the name requested at pipeline creation.
        .arg("-fvk-use-entrypoint-name")
        .arg("-o")
        .arg(output)
        .status()
        .expect("failed to execute slangc");

    if !status.success() {
        panic!("slangc failed for {entry}");
    }

    println!("cargo:rerun-if-changed={source}");

    assert!(
        fs::metadata(output).is_ok(),
        "Slang compiler did not produce {:?}",
        output
    );
}
