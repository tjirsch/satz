use std::process::Command;

fn main() {
    let output = Command::new("date").arg("+%Y-%m-%d %H:%M:%S").output().ok();
    let date = output.and_then(|o| String::from_utf8(o.stdout).ok())
                     .unwrap_or_else(|| "unknown-date".to_string());
    println!("cargo:rustc-env=BUILD_DATE={}", date.trim());
    println!("cargo:rerun-if-changed=build.rs");
}
