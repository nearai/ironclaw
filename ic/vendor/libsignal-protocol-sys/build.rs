fn main() {
    let dst = cmake::build("libsignal-protocol-c");

    println!("cargo:rustc-link-search={}", dst.display());
    println!("cargo:rustc-link-search={}/lib", dst.display());
    println!("cargo:rustc-link-lib=signal-protocol-c");
}
