use proc_macro::TokenStream;

#[cfg(feature = "check")]
mod entry_checked {
    use super::*;
    use codesnip_core::entry::EntryArgs;
    use quote::ToTokens;
    use syn::{Error, Item, parse, parse_macro_input};

    pub(crate) fn entry(attr: TokenStream, item: TokenStream) -> TokenStream {
        let attr = parse_macro_input!(attr as EntryArgs);
        match parse::<Item>(item) {
            Err(_) => Error::new_spanned(attr, "expected to apply to `Item`")
                .to_compile_error()
                .into(),
            Ok(item) => {
                if let Err(err) = attr.try_to_entry(&item) {
                    return err.to_compile_error().into();
                }
                item.into_token_stream().into()
            }
        }
    }
}

#[cfg(feature = "check")]
#[proc_macro_attribute]
pub fn entry(attr: TokenStream, item: TokenStream) -> TokenStream {
    entry_checked::entry(attr, item)
}
#[cfg(not(feature = "check"))]
#[proc_macro_attribute]
pub fn entry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn skip(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
