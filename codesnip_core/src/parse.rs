use crate::ItemExt as _;
use Error::{FileNotFound, ModuleNotFound, ParseFile};
use proc_macro2::TokenStream;
use quote::{ToTokens as _, quote};
use std::path::{Path, PathBuf};
use syn::{
    AttrStyle, Attribute, Expr, ExprLit, File, ForeignItem, ImplItem, Item, ItemMod, Lit, Meta,
    MetaNameValue, Token, TraitItem,
    ext::IdentExt as _,
    parse::ParseStream,
    parse_file, parse2,
    punctuated::Punctuated,
    visit_mut::{self, VisitMut},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid snippet entry: {0}")]
    InvalidEntry(#[from] syn::Error),
    #[error("conflicting `when` conditions for snippet `{0}`")]
    ConflictingWhen(String),
    #[error("snippet `{0}` not found")]
    SnippetNotFound(String),
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to parse ")]
    ParseFile(PathBuf, #[source] syn::Error),
    #[error("Module `{name}` not found where `{path}`.", name = .0, path = .1.display())]
    ModuleNotFound(String, PathBuf),
    #[error("File `{}` not found.", .0.display())]
    FileNotFound(PathBuf, #[source] std::io::Error),
}

pub fn parse_file_recursive(
    path: PathBuf,
    cfg_enable: &[Meta],
    cfg_disable: &[Meta],
) -> Result<File, Error> {
    let mut mod_dir = path.clone();
    mod_dir.pop();
    let cwd = mod_dir.clone();
    let mut ext = ExtractAst {
        mod_dir,
        cwd,
        error: None,
        cfg_enable,
        cfg_disable,
    };
    let mut ast = parse_file_from_path(&path)?;
    ext.visit_file_mut(&mut ast);
    match ext.error {
        Some(err) => Err(err),
        _ => Ok(ast),
    }
}

#[derive(Debug)]
struct ExtractAst<'c> {
    mod_dir: PathBuf,
    cwd: PathBuf,
    error: Option<Error>,
    cfg_enable: &'c [Meta],
    cfg_disable: &'c [Meta],
}

impl ExtractAst<'_> {
    fn find_mod_file(&mut self, node: &ItemMod) -> Result<PathBuf, Error> {
        let mod_name = node.ident.to_string();
        if let Some(pathstr) = find_pathstr_from_attrs(&node.attrs) {
            let path = self.cwd.join(pathstr);
            if path.exists() {
                Ok(path)
            } else {
                Err(ModuleNotFound(mod_name, path))
            }
        } else {
            self.mod_dir.push(&mod_name);
            let path1 = self.mod_dir.with_extension("rs");
            let path2 = self.mod_dir.join("mod.rs");
            if path1.exists() {
                Ok(path1)
            } else if path2.exists() {
                self.cwd.push(mod_name);
                Ok(path2)
            } else {
                Err(ModuleNotFound(mod_name, path1))
            }
        }
    }

    fn expand_file(&mut self, node: &mut ItemMod) -> Result<(), Error> {
        let path = self.find_mod_file(node)?;
        let ast = parse_file_from_path(&path)?;

        node.attrs.extend(ast.attrs);
        let mut tokens = TokenStream::new();
        for attr in node.attrs.iter() {
            if attr.style == AttrStyle::Outer {
                attr.to_tokens(&mut tokens);
            }
        }
        node.vis.to_tokens(&mut tokens);
        node.mod_token.to_tokens(&mut tokens);
        node.ident.to_tokens(&mut tokens);

        let mut file_items = TokenStream::new();
        for attr in node.attrs.iter() {
            if attr.style != AttrStyle::Outer {
                attr.to_tokens(&mut file_items);
            }
        }
        for item in ast.items.iter() {
            item.to_tokens(&mut file_items);
        }
        let braced = quote! { { #file_items } };
        braced.to_tokens(&mut tokens);

        let item_mod = parse2::<ItemMod>(tokens).expect("failed to parse no-inline `mod`");
        *node = item_mod;
        Ok(())
    }
    fn visit_module(&mut self, node: &mut ItemMod) -> bool {
        let prev = (self.mod_dir.clone(), self.cwd.clone());
        if node.content.is_none() {
            if let Err(err) = self.expand_file(node) {
                self.error.get_or_insert(err);
            }
        } else if let Some(pathstr) = find_pathstr_from_attrs(&node.attrs) {
            self.cwd.push(pathstr);
            self.mod_dir = self.cwd.clone();
        } else {
            self.mod_dir.push(node.ident.to_string());
            self.cwd = self.mod_dir.clone();
        }
        let enabled = check_cfg(&mut node.attrs, self.cfg_enable, self.cfg_disable);
        if enabled {
            if let Some((_, items)) = &mut node.content {
                for item in items {
                    inherit_cfg(&node.attrs, item);
                }
            }
            visit_mut::visit_item_mod_mut(self, node);
        }
        self.mod_dir = prev.0;
        self.cwd = prev.1;
        enabled
    }

    fn retain_generic_params(&self, params: &mut Punctuated<syn::GenericParam, Token![,]>) {
        retain_punctuated(params, |param| {
            let attrs = match param {
                syn::GenericParam::Lifetime(param) => &mut param.attrs,
                syn::GenericParam::Type(param) => &mut param.attrs,
                syn::GenericParam::Const(param) => &mut param.attrs,
            };
            check_cfg(attrs, self.cfg_enable, self.cfg_disable)
        });
    }
}

impl VisitMut for ExtractAst<'_> {
    fn visit_attributes_mut(&mut self, attrs: &mut Vec<Attribute>) {
        check_cfg(attrs, self.cfg_enable, self.cfg_disable);
    }

    fn visit_file_mut(&mut self, node: &mut File) {
        if check_cfg(&mut node.attrs, self.cfg_enable, self.cfg_disable) {
            for item in &mut node.items {
                inherit_cfg(&node.attrs, item);
            }
            visit_mut::visit_file_mut(self, node);
        } else {
            node.items.clear();
        }
    }

    fn visit_item_mut(&mut self, node: &mut Item) {
        if let Some(attrs) = node.get_attributes_mut()
            && !check_cfg(attrs, self.cfg_enable, self.cfg_disable)
        {
            *node = Item::Verbatim(TokenStream::new());
        } else if let Item::Mod(module) = node {
            if !self.visit_module(module) {
                *node = Item::Verbatim(TokenStream::new());
            }
        } else {
            visit_mut::visit_item_mut(self, node);
        }
    }

    fn visit_impl_item_mut(&mut self, node: &mut ImplItem) {
        let attrs = match node {
            ImplItem::Const(item) => &mut item.attrs,
            ImplItem::Fn(item) => &mut item.attrs,
            ImplItem::Type(item) => &mut item.attrs,
            ImplItem::Macro(item) => &mut item.attrs,
            _ => return,
        };
        if check_cfg(attrs, self.cfg_enable, self.cfg_disable) {
            visit_mut::visit_impl_item_mut(self, node);
        } else {
            *node = ImplItem::Verbatim(TokenStream::new());
        }
    }

    fn visit_trait_item_mut(&mut self, node: &mut TraitItem) {
        let attrs = match node {
            TraitItem::Const(item) => &mut item.attrs,
            TraitItem::Fn(item) => &mut item.attrs,
            TraitItem::Type(item) => &mut item.attrs,
            TraitItem::Macro(item) => &mut item.attrs,
            _ => return,
        };
        if check_cfg(attrs, self.cfg_enable, self.cfg_disable) {
            visit_mut::visit_trait_item_mut(self, node);
        } else {
            *node = TraitItem::Verbatim(TokenStream::new());
        }
    }

    fn visit_foreign_item_mut(&mut self, node: &mut ForeignItem) {
        let attrs = match node {
            ForeignItem::Fn(item) => &mut item.attrs,
            ForeignItem::Static(item) => &mut item.attrs,
            ForeignItem::Type(item) => &mut item.attrs,
            ForeignItem::Macro(item) => &mut item.attrs,
            _ => return,
        };
        if check_cfg(attrs, self.cfg_enable, self.cfg_disable) {
            visit_mut::visit_foreign_item_mut(self, node);
        } else {
            *node = ForeignItem::Verbatim(TokenStream::new());
        }
    }

    fn visit_item_enum_mut(&mut self, node: &mut syn::ItemEnum) {
        retain_punctuated(&mut node.variants, |variant| {
            check_cfg(&mut variant.attrs, self.cfg_enable, self.cfg_disable)
        });
        visit_mut::visit_item_enum_mut(self, node);
    }

    fn visit_fields_named_mut(&mut self, node: &mut syn::FieldsNamed) {
        retain_punctuated(&mut node.named, |field| {
            check_cfg(&mut field.attrs, self.cfg_enable, self.cfg_disable)
        });
        visit_mut::visit_fields_named_mut(self, node);
    }

    fn visit_fields_unnamed_mut(&mut self, node: &mut syn::FieldsUnnamed) {
        retain_punctuated(&mut node.unnamed, |field| {
            check_cfg(&mut field.attrs, self.cfg_enable, self.cfg_disable)
        });
        visit_mut::visit_fields_unnamed_mut(self, node);
    }

    fn visit_generics_mut(&mut self, node: &mut syn::Generics) {
        self.retain_generic_params(&mut node.params);
        visit_mut::visit_generics_mut(self, node);
    }

    fn visit_bound_lifetimes_mut(&mut self, node: &mut syn::BoundLifetimes) {
        self.retain_generic_params(&mut node.lifetimes);
        visit_mut::visit_bound_lifetimes_mut(self, node);
    }

    fn visit_signature_mut(&mut self, node: &mut syn::Signature) {
        retain_punctuated(&mut node.inputs, |arg| {
            let attrs = match arg {
                syn::FnArg::Receiver(arg) => &mut arg.attrs,
                syn::FnArg::Typed(arg) => &mut arg.attrs,
            };
            check_cfg(attrs, self.cfg_enable, self.cfg_disable)
        });
        if let Some(variadic) = &mut node.variadic
            && !check_cfg(&mut variadic.attrs, self.cfg_enable, self.cfg_disable)
        {
            node.variadic = None;
        }
        visit_mut::visit_signature_mut(self, node);
    }

    fn visit_type_fn_ptr_mut(&mut self, node: &mut syn::TypeFnPtr) {
        retain_punctuated(&mut node.inputs, |arg| {
            check_cfg(&mut arg.attrs, self.cfg_enable, self.cfg_disable)
        });
        if let Some(variadic) = &mut node.variadic
            && !check_cfg(&mut variadic.attrs, self.cfg_enable, self.cfg_disable)
        {
            node.variadic = None;
        }
        visit_mut::visit_type_fn_ptr_mut(self, node);
    }

    fn visit_block_mut(&mut self, node: &mut syn::Block) {
        node.stmts.retain_mut(|stmt| {
            let attrs = match stmt {
                syn::Stmt::Local(local) => Some(&mut local.attrs),
                syn::Stmt::Item(item) => item.get_attributes_mut(),
                syn::Stmt::Expr(expr, _) => expr_attributes_mut(expr),
                syn::Stmt::Macro(mac) => Some(&mut mac.attrs),
            };
            if let Some(attrs) = attrs
                && !check_cfg(attrs, self.cfg_enable, self.cfg_disable)
            {
                return false;
            }
            self.visit_stmt_mut(stmt);
            true
        });
    }

    fn visit_expr_mut(&mut self, node: &mut Expr) {
        match node {
            Expr::Array(syn::ExprArray { elems, .. })
            | Expr::Tuple(syn::ExprTuple { elems, .. })
            | Expr::Call(syn::ExprCall { args: elems, .. })
            | Expr::MethodCall(syn::ExprMethodCall { args: elems, .. }) => {
                retain_punctuated(elems, |expr| {
                    expr_attributes_mut(expr)
                        .is_none_or(|attrs| check_cfg(attrs, self.cfg_enable, self.cfg_disable))
                });
            }
            Expr::Match(expr) => expr
                .arms
                .retain_mut(|arm| check_cfg(&mut arm.attrs, self.cfg_enable, self.cfg_disable)),
            Expr::Struct(expr) => retain_punctuated(&mut expr.fields, |field| {
                check_cfg(&mut field.attrs, self.cfg_enable, self.cfg_disable)
            }),
            Expr::Closure(expr) => retain_punctuated(&mut expr.inputs, |pat| {
                pat_attributes_mut(pat)
                    .is_none_or(|attrs| check_cfg(attrs, self.cfg_enable, self.cfg_disable))
            }),
            _ => {}
        }
        visit_mut::visit_expr_mut(self, node);
    }

    fn visit_pat_struct_mut(&mut self, node: &mut syn::PatStruct) {
        retain_punctuated(&mut node.fields, |field| {
            check_cfg(&mut field.attrs, self.cfg_enable, self.cfg_disable)
        });
        visit_mut::visit_pat_struct_mut(self, node);
    }
}

fn retain_punctuated<T>(
    nodes: &mut Punctuated<T, Token![,]>,
    mut keep: impl FnMut(&mut T) -> bool,
) {
    *nodes = std::mem::take(nodes)
        .into_pairs()
        .filter_map(|mut pair| keep(pair.value_mut()).then_some(pair))
        .collect();
}

fn pat_attributes_mut(pat: &mut syn::Pat) -> Option<&mut Vec<Attribute>> {
    Some(match pat {
        syn::Pat::Const(pat) => &mut pat.attrs,
        syn::Pat::Guard(pat) => &mut pat.attrs,
        syn::Pat::Ident(pat) => &mut pat.attrs,
        syn::Pat::Lit(pat) => &mut pat.attrs,
        syn::Pat::Macro(pat) => &mut pat.attrs,
        syn::Pat::Or(pat) => &mut pat.attrs,
        syn::Pat::Paren(pat) => &mut pat.attrs,
        syn::Pat::Path(pat) => &mut pat.attrs,
        syn::Pat::Range(pat) => &mut pat.attrs,
        syn::Pat::Reference(pat) => &mut pat.attrs,
        syn::Pat::Rest(pat) => &mut pat.attrs,
        syn::Pat::Slice(pat) => &mut pat.attrs,
        syn::Pat::Struct(pat) => &mut pat.attrs,
        syn::Pat::Tuple(pat) => &mut pat.attrs,
        syn::Pat::TupleStruct(pat) => &mut pat.attrs,
        syn::Pat::Type(pat) => &mut pat.attrs,
        syn::Pat::Wild(pat) => &mut pat.attrs,
        _ => return None,
    })
}

fn expr_attributes_mut(expr: &mut Expr) -> Option<&mut Vec<Attribute>> {
    Some(match expr {
        Expr::Array(expr) => &mut expr.attrs,
        Expr::Assign(expr) => &mut expr.attrs,
        Expr::Async(expr) => &mut expr.attrs,
        Expr::Await(expr) => &mut expr.attrs,
        Expr::Binary(expr) => &mut expr.attrs,
        Expr::Block(expr) => &mut expr.attrs,
        Expr::Break(expr) => &mut expr.attrs,
        Expr::Call(expr) => &mut expr.attrs,
        Expr::Cast(expr) => &mut expr.attrs,
        Expr::Closure(expr) => &mut expr.attrs,
        Expr::Const(expr) => &mut expr.attrs,
        Expr::Continue(expr) => &mut expr.attrs,
        Expr::Field(expr) => &mut expr.attrs,
        Expr::ForLoop(expr) => &mut expr.attrs,
        Expr::Group(expr) => &mut expr.attrs,
        Expr::If(expr) => &mut expr.attrs,
        Expr::Index(expr) => &mut expr.attrs,
        Expr::Infer(expr) => &mut expr.attrs,
        Expr::Let(expr) => &mut expr.attrs,
        Expr::Lit(expr) => &mut expr.attrs,
        Expr::Loop(expr) => &mut expr.attrs,
        Expr::Macro(expr) => &mut expr.attrs,
        Expr::Match(expr) => &mut expr.attrs,
        Expr::MethodCall(expr) => &mut expr.attrs,
        Expr::Paren(expr) => &mut expr.attrs,
        Expr::Path(expr) => &mut expr.attrs,
        Expr::Range(expr) => &mut expr.attrs,
        Expr::RawAddr(expr) => &mut expr.attrs,
        Expr::Reference(expr) => &mut expr.attrs,
        Expr::Repeat(expr) => &mut expr.attrs,
        Expr::Return(expr) => &mut expr.attrs,
        Expr::Struct(expr) => &mut expr.attrs,
        Expr::Try(expr) => &mut expr.attrs,
        Expr::TryBlock(expr) => &mut expr.attrs,
        Expr::Tuple(expr) => &mut expr.attrs,
        Expr::Unary(expr) => &mut expr.attrs,
        Expr::Unsafe(expr) => &mut expr.attrs,
        Expr::While(expr) => &mut expr.attrs,
        Expr::Yield(expr) => &mut expr.attrs,
        _ => return None,
    })
}

fn parse_file_from_path<P: AsRef<Path>>(path: P) -> Result<File, Error> {
    use std::io::Read as _;
    let mut content = String::new();
    let mut file =
        std::fs::File::open(&path).map_err(|err| FileNotFound(path.as_ref().to_path_buf(), err))?;
    file.read_to_string(&mut content)?;
    parse_file(&content).map_err(|err| ParseFile(path.as_ref().to_path_buf(), err))
}

fn find_pathstr_from_attrs(attrs: &[Attribute]) -> Option<String> {
    attrs
        .iter()
        .filter(|attr| is_ident(attr.path(), "path"))
        .find_map(|attr| match &attr.meta {
            Meta::NameValue(
                MetaNameValue {
                    value:
                        Expr::Lit(ExprLit {
                            lit: Lit::Str(litstr),
                            ..
                        }),
                    ..
                },
                ..,
            ) => Some(litstr.value()),
            _ => None,
        })
}

fn is_ident(path: &syn::Path, name: &str) -> bool {
    path.get_ident().is_some_and(|ident| ident.unraw() == name)
}

pub(crate) fn inherit_cfg(attrs: &[Attribute], item: &mut Item) {
    if let Some(child_attrs) = item.get_attributes_mut() {
        for mut attr in attrs
            .iter()
            .filter(|attr| is_ident(attr.path(), "cfg"))
            .cloned()
        {
            attr.style = AttrStyle::Outer;
            if !child_attrs.contains(&attr) {
                child_attrs.push(attr);
            }
        }
    }
}

fn check_cfg(attrs: &mut Vec<Attribute>, cfg_enable: &[Meta], cfg_disable: &[Meta]) -> bool {
    flatten_cfg_attr(attrs, cfg_enable, cfg_disable);
    let mut next = Vec::new();
    let mut cond = true;
    for mut attr in attrs.drain(..) {
        if is_ident(attr.path(), "cfg")
            && let Meta::List(list) = &mut attr.meta
            && let Ok(mut pred) = list.parse_args_with(parse_cfg_argument)
        {
            match resolve_cfg(&mut pred, cfg_enable, cfg_disable) {
                Some(true) => {}
                Some(false) => {
                    cond = false;
                    // Keep the condition for nodes whose removal is left to rustc.
                    attr.meta = syn::parse_quote!(cfg(any()));
                    next.push(attr);
                }
                None => {
                    attr.meta = syn::parse_quote!(cfg(#pred));
                    next.push(attr);
                }
            }
            continue;
        }
        next.push(attr);
    }
    *attrs = next;
    cond
}

fn flatten_cfg_attr(attrs: &mut Vec<Attribute>, cfg_enable: &[Meta], cfg_disable: &[Meta]) {
    let mut next = Vec::new();
    for attr in attrs.drain(..) {
        if is_ident(attr.path(), "cfg_attr")
            && let Meta::List(list) = &attr.meta
            && let Ok((mut pred, attrs)) = list.parse_args_with(|input: ParseStream<'_>| {
                let pred = parse_cfg_predicate(input)?;
                input.parse::<Token![,]>()?;
                let attrs = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
                Ok((pred, attrs))
            })
        {
            let condition = resolve_cfg(&mut pred, cfg_enable, cfg_disable);
            if condition == Some(false) {
                continue;
            }
            let mut expanded: Vec<_> = attrs.iter().map(to_attribute).collect();
            flatten_cfg_attr(&mut expanded, cfg_enable, cfg_disable);
            for mut expanded in expanded {
                expanded.style = attr.style;
                if condition.is_none() {
                    let meta = &expanded.meta;
                    expanded.meta = if is_ident(meta.path(), "cfg")
                        && let Meta::List(list) = meta
                        && let Ok(inner) = list.parse_args_with(parse_cfg_argument)
                    {
                        syn::parse_quote!(cfg(any(not(#pred), #inner)))
                    } else {
                        syn::parse_quote!(cfg_attr(#pred, #meta))
                    };
                }
                next.push(expanded);
            }
            continue;
        }
        next.push(attr);
    }
    *attrs = next;
}

fn to_attribute(meta: &Meta) -> Attribute {
    let meta = meta.to_token_stream();
    let attr: Attribute = syn::parse_quote!(#[ #meta ]);
    attr
}

#[test]
fn test_to_attribute() {
    let attr: Attribute = syn::parse_quote!(#[codesnip::entry("name", inline)]);
    let attr = to_attribute(&attr.meta).to_token_stream().to_string();
    assert_eq!(
        attr.as_str(),
        r##"# [codesnip :: entry ("name" , inline)]"##
    );
}

#[test]
fn test_resolve_cfg_enable_disable() {
    let enable = vec![syn::parse_str::<Meta>("feature = \"foo\"").unwrap()];
    let disable = vec![syn::parse_str::<Meta>("feature = \"bar\"").unwrap()];

    let mut pred_enable = syn::parse_str::<Meta>("feature = \"foo\"").unwrap();
    let mut pred_disable = syn::parse_str::<Meta>("feature = \"bar\"").unwrap();
    let mut pred_unknown = syn::parse_str::<Meta>("target_arch = \"x86_64\"").unwrap();
    let mut pred_any =
        syn::parse_str::<Meta>("any(feature = \"foo\", target_arch = \"x86_64\")").unwrap();

    assert_eq!(resolve_cfg(&mut pred_enable, &enable, &disable), Some(true));
    assert_eq!(
        resolve_cfg(&mut pred_disable, &enable, &disable),
        Some(false)
    );
    assert_eq!(resolve_cfg(&mut pred_unknown, &enable, &disable), None);
    assert_eq!(resolve_cfg(&mut pred_any, &enable, &disable), Some(true));

    for (left, left_value) in [("yes", Some(true)), ("no", Some(false)), ("unknown", None)] {
        for (right, right_value) in [("yes", Some(true)), ("no", Some(false)), ("unknown", None)] {
            let values = [left_value, right_value];
            for (op, decisive) in [("all", false), ("any", true)] {
                let expected = if values.contains(&Some(decisive)) {
                    Some(decisive)
                } else if values.contains(&None) {
                    None
                } else {
                    Some(!decisive)
                };
                let mut pred = syn::parse_str(&format!("{op}({left}, {right})")).unwrap();
                assert_eq!(
                    resolve_cfg(
                        &mut pred,
                        &[syn::parse_quote!(yes)],
                        &[syn::parse_quote!(no)]
                    ),
                    expected,
                    "{op}({left}, {right})"
                );
            }
        }
    }
}

fn parse_cfg_predicate(input: ParseStream<'_>) -> syn::Result<Meta> {
    if input.peek(syn::LitBool) {
        let literal: syn::LitBool = input.parse()?;
        Ok(if literal.value {
            syn::parse_quote!(all())
        } else {
            syn::parse_quote!(any())
        })
    } else {
        input.parse()
    }
}

fn parse_cfg_argument(input: ParseStream<'_>) -> syn::Result<Meta> {
    let pred = parse_cfg_predicate(input)?;
    input.parse::<Option<Token![,]>>()?;
    Ok(pred)
}

// Returns a known truth value, or leaves only unresolved conditions in `pred`.
fn resolve_cfg(pred: &mut Meta, cfg_enable: &[Meta], cfg_disable: &[Meta]) -> Option<bool> {
    if let Some(id) = pred.path().get_ident() {
        match id.unraw().to_string().as_str() {
            op @ ("all" | "any") => {
                if let Meta::List(list) = pred {
                    let preds = list
                        .parse_args_with(|input: ParseStream<'_>| {
                            Punctuated::<Meta, Token![,]>::parse_terminated_with(
                                input,
                                parse_cfg_predicate,
                            )
                        })
                        .ok()?;
                    let decisive = op == "any";
                    let mut remaining = Punctuated::<Meta, Token![,]>::new();
                    for mut pred in preds {
                        match resolve_cfg(&mut pred, cfg_enable, cfg_disable) {
                            Some(value) if value == decisive => return Some(decisive),
                            Some(_) => {}
                            None => remaining.push(pred),
                        }
                    }
                    if remaining.is_empty() {
                        return Some(!decisive);
                    }
                    list.tokens = remaining.to_token_stream();
                    return None;
                }
            }
            "not" => {
                if let Meta::List(list) = pred {
                    let mut inner = list.parse_args_with(parse_cfg_argument).ok()?;
                    let result =
                        resolve_cfg(&mut inner, cfg_enable, cfg_disable).map(|value| !value);
                    list.tokens = inner.to_token_stream();
                    return result;
                }
            }
            _ => {}
        }
    }
    let matches = |spec: &Meta| {
        if spec == pred {
            return true;
        }
        let (Some(left), Some(right)) = (spec.path().get_ident(), pred.path().get_ident()) else {
            return false;
        };
        if left.unraw() != right.unraw() {
            return false;
        }
        match (spec, &*pred) {
            (Meta::Path(_), Meta::Path(_)) => true,
            (Meta::NameValue(left), Meta::NameValue(right)) => match (&left.value, &right.value) {
                (
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(left),
                        ..
                    }),
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(right),
                        ..
                    }),
                ) => left.value() == right.value(),
                (left, right) => left == right,
            },
            _ => false,
        }
    };
    if cfg_disable.iter().any(&matches) {
        Some(false)
    } else if cfg_enable.iter().any(matches) {
        Some(true)
    } else {
        None
    }
}

#[test]
fn test_parse() {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "examples", "mod_path", "lib.rs"]
        .iter()
        .collect();
    if let Err(err) = parse_file_recursive(path, &[], &[]) {
        panic!("{}", err);
    }
}
