#[path = "src/app_icon.rs"]
mod app_icon;
#[path = "src/build_commit.rs"]
mod build_commit;

use std::{path::Path, process::Command};

/// The trimmed standard output of a successful `git` call in the crate folder.
fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The commit label of the checkout, or `unknown` without Git or a checkout.
fn commit_from_git() -> String {
    // Rebuild after a commit, a branch switch or a staging change. Edits that are
    // not staged do not rerun this script; build-product.ps1 passes the label
    // explicitly so that its builds always see them.
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            rerun_if_changed(&path);
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            rerun_if_changed(&path);
        }
    }
    let Some(hash) = git(&["rev-parse", "HEAD"]) else {
        println!("cargo:warning=Git is not available; the build commit is `{}`.", build_commit::UNKNOWN);
        return build_commit::UNKNOWN.into();
    };
    let Some(status) = git(&["status", "--porcelain"]) else {
        println!("cargo:warning=Could not read the Git status; the build commit is `{}`.", build_commit::UNKNOWN);
        return build_commit::UNKNOWN.into();
    };
    build_commit::label(&hash, !status.is_empty()).unwrap_or_else(|| build_commit::UNKNOWN.into())
}

fn rerun_if_changed(path: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed={}", build_commit::VARIABLE);
    let commit = match std::env::var(build_commit::VARIABLE) {
        Ok(value) if value.is_empty() => commit_from_git(),
        Ok(value) if build_commit::is_label(&value) => value,
        Ok(value) => panic!(
            "{} must be seven lowercase hex digits, optionally followed by -dirty, or `{}`; got `{value}`.",
            build_commit::VARIABLE,
            build_commit::UNKNOWN
        ),
        Err(_) => commit_from_git(),
    };
    println!("cargo:rustc-env={}={commit}", build_commit::VARIABLE);
    let development = std::env::var_os("CARGO_FEATURE_DEVELOPMENT_CHANNEL").is_some();
    // Blue for the product, red for the development channel; see app_icon.rs.
    let icon = app_icon::for_channel(development);
    println!("cargo:rerun-if-changed={icon}");
    println!("cargo:rustc-env={}={icon}", app_icon::VARIABLE);
    // The executable embeds Vite assets, including files whose hashed names change.
    if development {
        const DEV_CONFIG: &str = r#"{
            "productName":"InstanceHarbor Dev",
            "identifier":"local.instanceharbor.dev",
            "build":{"frontendDist":"../dist-dev"}
        }"#;
        println!("cargo:rerun-if-changed=../dist-dev");
        // The resource compiler and generate_context! must resolve the same assets.
        std::env::set_var("TAURI_CONFIG", DEV_CONFIG);
        println!("cargo:rustc-env=TAURI_CONFIG={}", DEV_CONFIG.replace('\n', ""));
    } else {
        println!("cargo:rerun-if-changed=../dist");
    }
    let windows = tauri_build::WindowsAttributes::new().window_icon_path(icon);
    if let Err(error) = tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)) {
        panic!("error while running the Tauri build helpers: {error:#}");
    }
}
