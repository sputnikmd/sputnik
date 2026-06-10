use std::io;

fn main() -> io::Result<()> {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            .set_icon("../assets/icon.ico")
            .set("FileDescription", "Note taking app")
            .set("ProductName", "Sputnik")
            .compile()?;
    }

    Ok(())
}
