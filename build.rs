fn main() {
    linker_be_nice();
    load_dotenv();
    link_quickjs_runtime();
    println!("cargo:rustc-link-arg-tests=-Tembedded-test.x");
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

// The `quickjs` feature compiles the QuickJS C engine, whose objects reference
// libgcc soft-float/long-long helpers (eg `__ashldi3`, `__divdf3`) and libm
// (`pow`, `fmod`, ...). Those live in the espup toolchain's esp32s3 archives,
// which the bare-metal link (`-nodefaultlibs`) otherwise drops; add them on the
// link line. Archive linking pulls only referenced objects, so newlib's startup
// and syscall stubs stay out. `abort` is shimmed in Rust (see `src/lib.rs`) to
// keep newlib's abort (and its syscall tail) from being pulled.
fn link_quickjs_runtime() {
    if std::env::var_os("CARGO_FEATURE_QUICKJS").is_none() {
        return;
    }
    // Compile the newlib/clock shims with the same toolchain (so struct layouts
    // match) and the target CFLAGS (`-mlongcalls` etc). cc picks up CC/CFLAGS
    // from `.cargo/config.toml`'s `[env]`.
    println!("cargo:rerun-if-changed=src/scripting/quickjs_shim.c");
    cc::Build::new().file("src/scripting/quickjs_shim.c").compile("quickjs_shim");

    // ask the esp32s3 gcc driver where its libgcc / newlib archives live.
    let cc = std::env::var("CC_xtensa_esp32s3_none_elf")
        .unwrap_or_else(|_| "xtensa-esp32s3-elf-gcc".into());
    for lib in ["libgcc.a", "libm.a", "libc.a"] {
        let path = std::process::Command::new(&cc)
            .arg(format!("-print-file-name={lib}"))
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|path| path != lib); // gcc echoes the bare name when not found
        let Some(path) = path else {
            continue;
        };
        if let Some(dir) = std::path::Path::new(&path).parent() {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
    }
    // Append as raw link-args (not `rustc-link-lib`) so they land at the very end
    // of the link line, after the rlibs that reference them — the C objects pull
    // from these archives, not the reverse. `--start-group` resolves the
    // circular libc<->libm<->libgcc references.
    // newlib's prebuilt libc.a was built with the stack protector, so it carries
    // its own `__stack_chk_fail`; Rust supplies one too. Let the linker take the
    // first rather than erroring on the duplicate.
    println!("cargo:rustc-link-arg=-Wl,--allow-multiple-definition");

    // `quickjs_shim` joins the group: newlib (`-lc`) calls back into our
    // `__getreent`/syscall stubs, so the references run both ways. cc already
    // emitted its search path.
    for arg in [
        "-Wl,--start-group",
        "-lquickjs_shim",
        "-lc",
        "-lm",
        "-lgcc",
        "-Wl,--end-group",
    ] {
        println!("cargo:rustc-link-arg={arg}");
    }
}

// Expose KEY=VALUE pairs from a local `.env` to the crate via `env!()`.
// Lets examples read secrets (e.g. WIFI_SSID/WIFI_PASSWORD) without committing them.
fn load_dotenv() {
    println!("cargo:rerun-if-changed=.env");
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        println!("cargo:rustc-env={key}={value}");
    }
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
