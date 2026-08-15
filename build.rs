use std::{env, path::PathBuf, process::Command};

fn main() {
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let blueprint_output = output_directory.join("main-window.ui");
    let status = Command::new("blueprint-compiler")
        .args(["compile", "data/ui/main-window.blp", "--output"])
        .arg(&blueprint_output)
        .status()
        .expect("blueprint-compiler must be installed");
    assert!(
        status.success(),
        "failed to compile the main window blueprint"
    );

    glib_build_tools::compile_resources(
        &[output_directory.as_path(), PathBuf::from("data").as_path()],
        "data/io.github.wuzi.RemindMe.gresource.xml",
        "remind-me.gresource",
    );

    println!("cargo:rerun-if-changed=data/ui/main-window.blp");
    println!("cargo:rerun-if-changed=data/style.css");
}
