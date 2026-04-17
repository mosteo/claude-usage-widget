use std::error::Error;

mod build_vendored;

fn main() -> Result<(), Box<dyn Error>> {
    // Invalidate the built crate whenever these files change
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_vendored.rs");

    // See https://github.com/joshtriplett/metadeps/issues/9 for why we don't use
    // metadeps here, but instead keep this manually in sync with Cargo.toml.
    build_vendored::build_libdbus()?;
    Ok(())
}
