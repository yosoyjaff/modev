// Embeds the icon in the Windows .exe (other platforms take it from the bundle/.desktop).
fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/windows/modev.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/app-icon/windows/modev.ico")
            .compile()
            .expect("could not embed the icon");
    }
}
