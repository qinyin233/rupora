fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=RUPORA_BUILD_TARGET={target}");
    }
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");

    #[cfg(windows)]
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/icon.ico")
            .set("FileDescription", "RUPORA native Markdown editor")
            .set("ProductName", "RUPORA")
            .set("OriginalFilename", "rupora.exe")
            .compile()
            .expect("failed to embed the Windows icon and version resources");
    }
}
