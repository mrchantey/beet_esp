//! Proc macros for `beet_esp`. Re-exported by the `beet_esp` crate, so use them
//! as `#[beet_esp::main]` rather than depending on this crate directly.

mod main_attr;

/// Entry point for a `beet_esp` Bevy app: wraps `fn main` with the ESP32
/// boilerplate so examples read as pure Bevy.
///
/// Emits the esp-idf app descriptor, an `#[esp_hal::main] fn main() -> !`, the
/// RTT/`defmt` and memory setup (PSRAM as the default bulk pool, internal SRAM
/// reserved for the radio/hot use — see [`beet_esp::mem`]), the user's body, and
/// the trailing divergence the esp runner needs.
///
/// # Config
///
/// Attributes are declared as sibling attributes or as arguments; both forms are
/// equivalent. `internal_reserve_kb` is how much internal SRAM to reserve for
/// the radio + DMA + hot allocations, in **kilobytes** (bytes = kb * 1024); it
/// defaults to 64 KiB. The big cold structures (World, type registry, request
/// buffers) live in PSRAM regardless of this value.
///
/// ```ignore
/// #[beet_esp::main]
/// #[internal_reserve_kb(64)]   // sibling form (default 64 KiB when absent)
/// fn main() { App::new().run(); }
///
/// #[beet_esp::main(internal_reserve_kb = 64)]   // argument form
/// fn main() { App::new().run(); }
/// ```
#[proc_macro_attribute]
pub fn main(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    main_attr::impl_main_attr(attr, input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
