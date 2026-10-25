//! The browser gives WebAssembly a 1 MiB stack unless the module asks for
//! more; the compiler recurses as deep as the code it checks, so it asks
//! for what a computer's thread has (`wowlua_driver::parallel::STACK`).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size=16777216");
    }
}
