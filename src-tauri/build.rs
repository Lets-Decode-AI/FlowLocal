fn main() {
    // The prebuilt static onnxruntime inside sherpa-onnx calls the Windows
    // event tracing API (EventRegister, EventWriteTransfer, ...) from advapi32.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
    tauri_build::build()
}
