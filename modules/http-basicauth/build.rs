use std::path::PathBuf;

fn main() {
    let fips = std::env::var_os("CARGO_FEATURE_FIPS").is_some();
    // Don't build Argon2 C library, as Argon2 isn't FIPS-approved
    if !fips {
        build_argon2();
    }
}

fn build_argon2() {
    let crate_root_dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let argon2_dir = crate_root_dir.join("phc-winner-argon2");

    if !argon2_dir.join("src").exists() {
        if let Some(git_path) = find_git::git_path() {
            std::process::Command::new(git_path)
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive")
                .arg("--")
                .arg("phc-winner-argon2")
                .current_dir(&crate_root_dir)
                .status()
                .expect("failed to update the phc-winner-argon2 submodule");
        }
    }

    if !argon2_dir.join("src").exists() {
        panic!(
            "failed to obtain the phc-winner-argon2 Git repository, perhaps you need \
            to run `git submodule update --init --recursive`?"
        );
    }

    let target = std::env::var("TARGET").unwrap();
    let target_features = std::env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();

    // SIMD-optimized version of Argon2 C reference impl is tied to x86.
    // While non-SIMD-optimized one is true cross-platform.
    //
    // Also, compilation fails if SSE2 support isn't declared.
    let enable_simd = (target.starts_with("x86_64-")
        || target.starts_with("i686-")
        || target.starts_with("i586-"))
        && target_features.split(",").any(|f| f == "sse2");

    // Sourced from phc-winner-argon2 Makefile
    // Don't include -march=native to ensure portability
    let mut builder = cc::Build::new();
    builder
        .files([
            "phc-winner-argon2/src/argon2.c",
            "phc-winner-argon2/src/core.c",
            "phc-winner-argon2/src/blake2/blake2b.c",
            "phc-winner-argon2/src/thread.c",
            "phc-winner-argon2/src/encoding.c",
            if enable_simd {
                "phc-winner-argon2/src/opt.c"
            } else {
                "phc-winner-argon2/src/ref.c"
            },
        ])
        .include("phc-winner-argon2/include")
        .flag_if_supported("-pthread")
        .flag_if_supported("-std=c89")
        .warnings(false)
        .extra_warnings(false);

    if enable_simd {
        // Different arguments for GCC/Clang and MSVC/clang-cl...
        if builder.try_get_compiler().is_ok_and(|c| c.is_like_msvc()) {
            builder.flag_if_supported("/arch:SSE2");
        } else {
            builder.flag_if_supported("-msse2");
        }
    }

    builder.compile("argon2");

    let bindings = bindgen::Builder::default()
        .header("phc-winner-argon2/include/argon2.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .opaque_type("max_align_t")
        .generate()
        .expect("unable to generate bindings to argon2");

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("argon2_bindings.rs"))
        .expect("couldn't write bindings to argon2");
}
