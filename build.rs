use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let source = "tools/steamwebhelper-wrapper/steamwebhelper-wrapper.c";
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-env-changed=MINGW64_CC");
    let compiler = env::var("MINGW64_CC").unwrap_or_else(|_| "x86_64-w64-mingw32-gcc".into());
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .join("steamwebhelper-wrapper.exe");
    let status = Command::new(&compiler)
        .args(["-O2", "-Wall", "-Wextra", "-municode", "-mwindows", "-static"])
        .arg("-o")
        .arg(&output)
        .arg(source)
        .arg("-lshell32")
        .status()
        .unwrap_or_else(|error| panic!("cannot run {compiler}: {error}; install mingw-w64 or set MINGW64_CC"));
    assert!(status.success(), "{compiler} failed to build the Macroton Steam CEF wrapper");
}
