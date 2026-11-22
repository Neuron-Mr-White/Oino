fn main() {
    // iroh → netwatch → netdev reads iOS network interfaces/DNS through
    // SystemConfiguration, but nothing links that framework on iOS, so the
    // final Xcode link failed with undefined _kSCNetworkInterfaceType*.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");
    }
    tauri_build::build()
}
