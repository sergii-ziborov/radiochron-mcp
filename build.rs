use std::process::Command;

fn main() {
    let sha = std::env::var("RADIOCHRON_GIT_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=RADIOCHRON_GIT_SHA={sha}");
    println!("cargo:rerun-if-env-changed=RADIOCHRON_GIT_SHA");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Some(reference) = git_output(&["symbolic-ref", "HEAD"]) {
        println!("cargo:rerun-if-changed=../../.git/{reference}");
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
