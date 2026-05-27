//! Proc macros for `beet_esp`. Re-exported by the `beet_esp` crate, so use them
//! as `#[beet_esp::main]` rather than depending on this crate directly.

mod main_attr;

/// Entry point for a `beet_esp` Bevy app: wraps `fn main` with the ESP32
/// boilerplate so examples read as pure Bevy.
///
/// Emits the esp-idf app descriptor, an `#[esp_hal::main] fn main() -> !`, the
/// RTT/`defmt` and heap setup, the user's body, and the trailing divergence the
/// esp runner needs.
///
/// # Config
///
/// Heap size (and future knobs) are declared as sibling attributes or as
/// arguments; both forms are equivalent:
///
/// ```ignore
/// #[beet_esp::main]
/// #[heap_size(96 * 1024)]   // sibling form (default 96 KiB when absent)
/// fn main() { App::new().run(); }
///
/// #[beet_esp::main(heap_size = 96 * 1024)]   // argument form
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
