use std::path::Path;
use std::process::Command;

pub fn release_version() -> Result<(), String> {
    for name in ["TUMPEL_RELEASE_VERSION", "GITHUB_REF_TYPE", "GITHUB_SHA"] {
        println!("cargo::rerun-if-env-changed={name}");
    }
    let packaged = !Path::new(env!("CARGO_MANIFEST_DIR")).join(".git").exists();
    let version = match (
        std::env::var("TUMPEL_RELEASE_VERSION"),
        std::env::var("GITHUB_REF_TYPE").as_deref(),
    ) {
        (Ok(version), _) => version,
        (Err(_), Ok("tag")) => env!("CARGO_PKG_VERSION").to_string(),
        (Err(_), _) if packaged => env!("CARGO_PKG_VERSION").to_string(),
        (Err(_), _) => {
            let commit = head_commit()?;
            let short = commit
                .get(..12)
                .ok_or_else(|| format!("the commit {commit} is shorter than 12 characters"))?;
            format!("0.0.0-{short}")
        }
    };
    println!("cargo::rustc-env=TUMPEL_RELEASE_VERSION={version}");
    Ok(())
}

fn head_commit() -> Result<String, String> {
    if let Ok(commit) = std::env::var("GITHUB_SHA") {
        return Ok(commit);
    }
    println!(
        "cargo::rerun-if-changed={}",
        git(&["rev-parse", "--git-path", "logs/HEAD"])?
    );
    git(&["rev-parse", "HEAD"])
}

fn git(args: &[&str]) -> Result<String, String> {
    let fix = "build inside the repository or set TUMPEL_RELEASE_VERSION";
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("git {} did not start: {error}; {fix}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}; {fix}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
