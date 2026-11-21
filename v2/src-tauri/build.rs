fn main() {
    // The executable embeds Vite assets, including files whose hashed names change.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    if std::env::var_os("CARGO_FEATURE_DEVELOPMENT_CHANNEL").is_some() {
        const DEV_CONFIG: &str = r#"{
            "productName":"InstanceHarbor Dev",
            "identifier":"local.instanceharbor.v2.dev",
            "build":{"frontendDist":"../dist-dev"}
        }"#;
        println!("cargo:rerun-if-changed=../dist-dev");
        // The resource compiler and generate_context! must resolve the same assets.
        std::env::set_var("TAURI_CONFIG", DEV_CONFIG);
        println!("cargo:rustc-env=TAURI_CONFIG={}", DEV_CONFIG.replace('\n', ""));
    } else {
        println!("cargo:rerun-if-changed=../dist");
    }
    tauri_build::build();
}
