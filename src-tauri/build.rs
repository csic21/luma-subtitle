fn main() {
    println!("cargo:rerun-if-env-changed=LUMA_NATIVE_REGRESSION_BUILD");
    if std::env::var_os("CARGO_FEATURE_NATIVE_INSTALL_REGRESSION").is_some() {
        // Feature builds must never share the distributable application's target
        // directory. The fixture runner uses a separate non-bundling context.
        assert_eq!(std::env::var("PROFILE").as_deref(), Ok("release"));
        assert_eq!(std::env::var("LUMA_NATIVE_REGRESSION_BUILD").as_deref(), Ok("1"));
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .expect("Native regression requires an explicit isolated CARGO_TARGET_DIR");
        let target = std::path::Path::new(&target);
        assert_eq!(target.file_name().and_then(|v| v.to_str()), Some("native-install-regression"),
            "Refusing to build fixture code into a distributable target directory");
    }
    tauri_build::build();
}
