use proc_macro2::TokenStream;
use quote::quote;
use syn::Attribute;
use syn::Expr;
use syn::Ident;
use syn::ItemFn;
use syn::MetaNameValue;
use syn::Token;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

/// Recognised config knobs. Sibling attributes with these names are parsed and
/// stripped from the entry `fn`; any other attribute is left in place. Extend
/// this list (and [`Config`]/[`Config::set`]) to add knobs.
const KNOBS: &[&str] = &["heap_size"];

/// Entry-point config, gathered from `#[beet_esp::main(name = expr, …)]` args
/// and from sibling `#[name(expr)]` attributes.
struct Config {
    heap_size: Expr,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // 96 KiB: room for a Bevy `World` + schedules on this board.
            heap_size: syn::parse_quote!(96 * 1024),
        }
    }
}

impl Config {
    fn set(&mut self, name: &Ident, value: Expr) -> syn::Result<()> {
        match name.to_string().as_str() {
            "heap_size" => self.heap_size = value,
            other => {
                return Err(syn::Error::new_spanned(
                    name,
                    format!("#[beet_esp::main]: unknown config knob `{other}`"),
                ));
            }
        }
        Ok(())
    }

    /// Apply `name = expr` knobs from the `#[beet_esp::main(...)]` arguments.
    fn apply_args(&mut self, args: Punctuated<MetaNameValue, Token![,]>) -> syn::Result<()> {
        for nv in args {
            let name = nv.path.require_ident()?.clone();
            self.set(&name, nv.value)?;
        }
        Ok(())
    }

    /// Consume recognised sibling `#[name(expr)]` attributes from `attrs`,
    /// leaving the rest so they still apply to the generated `fn`.
    fn take_sibling_attrs(&mut self, attrs: &mut Vec<Attribute>) -> syn::Result<()> {
        let mut kept = Vec::with_capacity(attrs.len());
        for attr in core::mem::take(attrs) {
            let is_knob = attr
                .path()
                .get_ident()
                .is_some_and(|id| KNOBS.contains(&id.to_string().as_str()));
            if is_knob {
                let name = attr.path().get_ident().unwrap().clone();
                let value: Expr = attr.parse_args()?;
                self.set(&name, value)?;
            } else {
                kept.push(attr);
            }
        }
        *attrs = kept;
        Ok(())
    }
}

pub fn impl_main_attr(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> syn::Result<TokenStream> {
    let mut func = syn::parse::<ItemFn>(input)?;

    if func.sig.ident != "main" {
        return Err(syn::Error::new_spanned(
            &func.sig.ident,
            "#[beet_esp::main] can only be applied to `fn main`",
        ));
    }
    if func.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &func.sig.fn_token,
            "#[beet_esp::main] requires a synchronous `fn main` — the esp runner owns the executor",
        ));
    }

    let mut config = Config::default();
    if !attr.is_empty() {
        let args = Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse(attr)?;
        config.apply_args(args)?;
    }
    config.take_sibling_attrs(&mut func.attrs)?;

    let heap_size = &config.heap_size;
    let attrs = &func.attrs;
    let block = &func.block;

    Ok(quote! {
        // Link the RTT panic handler (a global symbol) so apps need no panic
        // boilerplate. `as _` forces the crate in without binding a name.
        use ::beet_esp::panic_rtt_target as _;

        // The esp-idf bootloader application descriptor: an item-scope linker
        // static, so it is emitted as a sibling of `main` rather than inside it.
        ::beet_esp::esp_app_desc!();

        #(#attrs)*
        #[::beet_esp::esp_hal::main]
        fn main() -> ! {
            // Binary-local statics (RTT/`defmt` logging + the heap), shared with
            // the manual `init_esp!` path so they can't drift.
            ::beet_esp::init_esp!(heap_size: #heap_size);

            #block

            ::core::unreachable!("the esp runner never returns")
        }
    })
}
