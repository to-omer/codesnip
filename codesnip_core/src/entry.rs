use crate::ext::ItemExt as _;
use quote::ToTokens;
use std::{collections::BTreeSet, fmt::Write as _};
use syn::{
    Error, Ident, Item, LitStr, Token,
    ext::IdentExt,
    parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token::{self, Paren},
};

#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub name: Option<String>,
    pub include: Vec<String>,
    pub when: BTreeSet<String>,
    pub inline: bool,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgs {
    pub args: Punctuated<EntryArg, Token![,]>,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub enum EntryArg {
    Name(EntryArgName),
    Include(EntryArgInclude),
    When(EntryArgWhen),
    Inline(EntryArgInline),
    NoInline(EntryArgNoInline),
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgName {
    pub name_token: Option<(Ident, token::Eq)>,
    pub name: NoWhitespaceLitStr,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgInclude {
    pub include_token: Ident,
    pub paren_token: Paren,
    pub includes: Punctuated<NoWhitespaceLitStr, Token![,]>,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgWhen {
    pub when_token: Ident,
    pub paren_token: Paren,
    pub conditions: Punctuated<NoWhitespaceLitStr, Token![,]>,
}

const ANONYMOUS_PREFIX: &str = "_codesnip_and_";

pub fn anonymous_name(conditions: &BTreeSet<String>) -> String {
    let mut name = ANONYMOUS_PREFIX.to_owned();
    for (index, condition) in conditions.iter().enumerate() {
        if index != 0 {
            name.push('_');
        }
        for byte in condition.bytes() {
            write!(&mut name, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    name
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgInline {
    pub token: Ident,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct EntryArgNoInline {
    pub token: Ident,
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub struct NoWhitespaceLitStr {
    pub litstr: LitStr,
}

impl EntryArgs {
    pub fn try_to_entry(&self, item: &Item) -> syn::Result<Entry> {
        let default_name = item.get_default_name();
        let mut entry = Entry::default();
        let mut name = None;
        let mut inline = None;
        for arg in self.args.iter() {
            match arg {
                EntryArg::Name(arg) => {
                    if name.is_some() {
                        return Err(Error::new_spanned(arg, "duplicate `name` specified"));
                    }
                    name = Some(arg.name.value());
                }
                EntryArg::Include(arg) => {
                    entry
                        .include
                        .extend(arg.includes.iter().map(|lit| lit.value()));
                }
                EntryArg::When(arg) => {
                    let conditions = arg.conditions.iter().map(|lit| lit.value()).collect();
                    if !entry.when.is_empty() && entry.when != conditions {
                        return Err(Error::new_spanned(arg, "conflicting `when` conditions"));
                    }
                    entry.when = conditions;
                }
                EntryArg::Inline(arg) => {
                    if !item.is_mod() {
                        return Err(Error::new_spanned(arg, "expected to apply to `Module`"));
                    }
                    if let Some(inline) = inline {
                        return Err(Error::new_spanned(
                            arg,
                            if inline {
                                "duplicate `inline` specified"
                            } else {
                                "already `no_inline` specified"
                            },
                        ));
                    }
                    inline = Some(true);
                }
                EntryArg::NoInline(arg) => {
                    if !item.is_mod() {
                        return Err(Error::new_spanned(arg, "expected to apply to `Module`"));
                    }
                    if let Some(inline) = inline {
                        return Err(Error::new_spanned(
                            arg,
                            if inline {
                                "already `inline` specified"
                            } else {
                                "duplicate `no_inline` specified"
                            },
                        ));
                    }
                    inline = Some(false);
                }
            }
        }
        if let Some(inline) = inline {
            entry.inline = inline;
        }
        if name.is_none() && entry.when.is_empty() {
            name = default_name;
        }
        if name.is_none() && entry.when.is_empty() {
            return Err(Error::new_spanned(self, "`name` unspecified"));
        }
        if name
            .as_ref()
            .is_some_and(|name| name.starts_with(ANONYMOUS_PREFIX))
        {
            return Err(Error::new_spanned(
                self,
                "`_codesnip_and_` is reserved for anonymous snippets",
            ));
        }
        entry.name = name;
        Ok(entry)
    }
}

impl Parse for EntryArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            args: input.parse_terminated(EntryArg::parse, Token![,])?,
        })
    }
}

impl Parse for EntryArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(LitStr) {
            input.parse().map(Self::Name)
        } else if lookahead.peek(Ident::peek_any) {
            let token: Ident = input.parse()?;
            match token.to_string().as_str() {
                "name" => EntryArgName::parse_after_token(token, input).map(Self::Name),
                "include" => EntryArgInclude::parse_after_token(token, input).map(Self::Include),
                "when" => EntryArgWhen::parse_after_token(token, input).map(Self::When),
                "inline" => EntryArgInline::parse_after_token(token, input).map(Self::Inline),
                "no_inline" => {
                    EntryArgNoInline::parse_after_token(token, input).map(Self::NoInline)
                }
                _ => {
                    Err(input
                        .error("expected `name` | `include` | `when` | `inline` | `no_inline`"))
                }
            }
        } else {
            Err(input.error("expected `name` | `include` | `when` | `inline` | `no_inline`"))
        }
    }
}

impl EntryArgName {
    fn parse_after_token(token: Ident, input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            name_token: Some((token, input.parse()?)),
            name: input.parse()?,
        })
    }
}

impl Parse for EntryArgName {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            name_token: None,
            name: input.parse()?,
        })
    }
}

impl Parse for NoWhitespaceLitStr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let litstr: LitStr = input.parse()?;
        if litstr.value().contains(char::is_whitespace) {
            return Err(Error::new_spanned(
                litstr,
                "string literal should not contain whitespace",
            ));
        }
        if litstr.value().starts_with(ANONYMOUS_PREFIX) {
            return Err(Error::new_spanned(
                litstr,
                "`_codesnip_and_` is reserved; give the snippet an explicit name to reference it",
            ));
        }
        Ok(Self { litstr })
    }
}

#[allow(clippy::mixed_read_write_in_expression)]
impl EntryArgInclude {
    fn parse_after_token(include_token: Ident, input: ParseStream) -> syn::Result<Self> {
        let content;
        Ok(Self {
            include_token,
            paren_token: parenthesized!(content in input),
            includes: content.call(Punctuated::parse_separated_nonempty)?,
        })
    }
}

#[allow(clippy::mixed_read_write_in_expression)]
impl EntryArgWhen {
    fn parse_after_token(when_token: Ident, input: ParseStream) -> syn::Result<Self> {
        let content;
        let paren_token = parenthesized!(content in input);
        let conditions = content.parse_terminated(NoWhitespaceLitStr::parse, Token![,])?;
        if conditions.is_empty() || conditions.iter().any(|name| name.value().is_empty()) {
            return Err(content.error("`when` requires nonempty snippet names"));
        }
        Ok(Self {
            when_token,
            paren_token,
            conditions,
        })
    }
}

#[allow(clippy::unnecessary_wraps)]
impl EntryArgInline {
    fn parse_after_token(token: Ident, _input: ParseStream) -> syn::Result<Self> {
        Ok(Self { token })
    }
}

#[allow(clippy::unnecessary_wraps)]
impl EntryArgNoInline {
    fn parse_after_token(token: Ident, _input: ParseStream) -> syn::Result<Self> {
        Ok(Self { token })
    }
}

impl NoWhitespaceLitStr {
    fn value(&self) -> String {
        self.litstr.value()
    }
}

impl ToTokens for EntryArgs {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.args.to_tokens(tokens);
    }
}

impl ToTokens for EntryArg {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            EntryArg::Name(arg) => arg.to_tokens(tokens),
            EntryArg::Include(arg) => arg.to_tokens(tokens),
            EntryArg::When(arg) => arg.to_tokens(tokens),
            EntryArg::Inline(arg) => arg.to_tokens(tokens),
            EntryArg::NoInline(arg) => arg.to_tokens(tokens),
        }
    }
}

impl ToTokens for EntryArgName {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        if let Some((name_token, eq)) = &self.name_token {
            name_token.to_tokens(tokens);
            eq.to_tokens(tokens);
        }
        self.name.to_tokens(tokens);
    }
}

impl ToTokens for EntryArgInclude {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.include_token.to_tokens(tokens);
        self.paren_token
            .surround(tokens, |tokens| self.includes.to_tokens(tokens));
    }
}

impl ToTokens for EntryArgWhen {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.when_token.to_tokens(tokens);
        self.paren_token
            .surround(tokens, |tokens| self.conditions.to_tokens(tokens));
    }
}

impl ToTokens for EntryArgInline {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.token.to_tokens(tokens)
    }
}

impl ToTokens for EntryArgNoInline {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.token.to_tokens(tokens)
    }
}

impl ToTokens for NoWhitespaceLitStr {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.litstr.to_tokens(tokens)
    }
}

#[test]
fn test_entry_args() {
    let item: Item = syn::parse_quote!(
        struct A;
    );
    for (args, name, when) in [
        ("", Some("A"), vec![]),
        (r#"when("A", "B")"#, None, vec!["A", "B"]),
        (
            r#""C", when("B", "A", "A"), when("A", "B")"#,
            Some("C"),
            vec!["A", "B"],
        ),
    ] {
        let entry = syn::parse_str::<EntryArgs>(args)
            .unwrap()
            .try_to_entry(&item)
            .unwrap();
        assert_eq!(entry.name.as_deref(), name);
        assert_eq!(entry.when, when.into_iter().map(String::from).collect());
    }
    for args in [
        "when()",
        r#"when("")"#,
        r#"when("A B")"#,
        r#"when("A"), when("B")"#,
        r#""A", "B""#,
        r#""_codesnip_and_41""#,
        r#"include("_codesnip_and_41")"#,
        "unknown",
        "inline",
    ] {
        assert!(
            syn::parse_str::<EntryArgs>(args)
                .and_then(|args| args.try_to_entry(&item))
                .is_err(),
            "{}",
            args
        );
    }
}
