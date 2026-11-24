fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("failed to embed Windows icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
