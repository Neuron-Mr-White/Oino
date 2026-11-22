//! Embeds the oino icon into `oino.exe` when building for Windows on a Windows host.

fn main() {
    println!("cargo:rerun-if-changed=assets/oino.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/oino.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=oino: could not embed the Windows icon: {e}");
        }
    }
}
