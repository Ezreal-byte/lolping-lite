fn main() {
    println!("cargo:rerun-if-changed=../assets/app.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../assets/app.ico")
            .set("ProductName", "LOLPing Lite")
            .set(
                "FileDescription",
                "LOLPing Lite — native desktop ping wheel",
            )
            .set("InternalName", "lolping-lite")
            .set("OriginalFilename", "lolping-lite.exe")
            .set("CompanyName", "Ezreal-byte")
            .set("LegalCopyright", "MIT; game artwork is separately licensed")
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource
            .compile()
            .expect("compile Windows icon and version resources");
    }
}
