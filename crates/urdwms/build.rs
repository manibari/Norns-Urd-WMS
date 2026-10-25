//! Windows: embed the icon and version information in the exe. Explorer
//! shows them, and an exe with a proper VERSIONINFO is less likely to be
//! taken for malware by heuristic scanners than an anonymous one.

fn main() {
    println!("cargo:rerun-if-changed=assets/urdwms.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/urdwms.ico")
            .set("ProductName", "Urd-WMS")
            .set("FileDescription", "Urd-WMS 包材批次管理")
            .set("CompanyName", "Norns")
            .set("LegalCopyright", "Norns")
            .set("OriginalFilename", "Urd-WMS.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no Windows resources embedded: {e}");
        }
    }
}
