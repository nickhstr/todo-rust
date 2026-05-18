use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GIT_SHA");
    // Pick up new SHAs in dev when HEAD or any ref moves. Paths are relative
    // to the crate root; the repo root is two levels up. Git worktree setups
    // (where .git is a file pointing elsewhere) won't trigger here — in that
    // case `cargo clean -p todo-app` forces a rebuild.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");

    let sha = std::env::var("GIT_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_SHA={sha}");
}
