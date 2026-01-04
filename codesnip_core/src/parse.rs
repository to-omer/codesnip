use crate::ItemExt as _;
use Error::{FileNotFound, ModuleNotFound, ParseFile};
use proc_macro2::TokenStream;
use quote::{ToTokens as _, quote};
use std::path::{Path, PathBuf};
use syn::{
    AttrStyle, Attribute, Expr, ExprLit, File, Item, ItemMod, Lit, Meta, MetaNameValue, Token,
    parse_file, parse2,
    punctuated::Punctuated,
    visit_mut::{self, VisitMut},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
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
}

impl VisitMut for ExtractAst<'_> {
    fn visit_item_mod_mut(&mut self, node: &mut ItemMod) {
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
        visit_mut::visit_item_mod_mut(self, node);
        self.mod_dir = prev.0;
        self.cwd = prev.1;
    }
    fn visit_item_mut(&mut self, node: &mut Item) {
        let mut is_skip = false;
        if let Some(attrs) = node.get_attributes_mut() {
            if !check_cfg(attrs, self.cfg_enable, self.cfg_disable) {
                is_skip = true;
            } else {
                flatten_cfg_attr(attrs, self.cfg_enable, self.cfg_disable);
            }
        }
        if is_skip {
            *node = Item::Verbatim(TokenStream::new());
        } else {
            visit_mut::visit_item_mut(self, node);
        }
    }
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
        .filter(|attr| attr.path().is_ident("path"))
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

fn check_cfg(attrs: &mut Vec<Attribute>, cfg_enable: &[Meta], cfg_disable: &[Meta]) -> bool {
    let mut next = Vec::new();
    let mut cond = true;
    for attr in attrs.drain(..) {
        if attr.path().is_ident("cfg")
            && let Meta::List(list) = &attr.meta
            && let Ok(pred) = list.parse_args()
        {
            match cfg_condition(&pred, cfg_enable, cfg_disable) {
                Some(true) => {}
                Some(false) => cond = false,
                None => next.push(attr),
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
        if attr.path().is_ident("cfg_attr")
            && let Meta::List(list) = &attr.meta
            && let Ok(preds) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        {
            let mut it = preds.iter();
            if let Some(pred) = it.next() {
                match cfg_condition(pred, cfg_enable, cfg_disable) {
                    Some(true) => {
                        next.extend(it.map(to_attribute));
                        continue;
                    }
                    Some(false) => continue,
                    None => {}
                }
            }
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
fn test_cfg_condition_enable_disable() {
    let enable = vec![syn::parse_str::<Meta>("feature = \"foo\"").unwrap()];
    let disable = vec![syn::parse_str::<Meta>("feature = \"bar\"").unwrap()];

    let pred_enable = syn::parse_str::<Meta>("feature = \"foo\"").unwrap();
    let pred_disable = syn::parse_str::<Meta>("feature = \"bar\"").unwrap();
    let pred_unknown = syn::parse_str::<Meta>("target_arch = \"x86_64\"").unwrap();
    let pred_any =
        syn::parse_str::<Meta>("any(feature = \"foo\", target_arch = \"x86_64\")").unwrap();

    assert_eq!(cfg_condition(&pred_enable, &enable, &disable), Some(true));
    assert_eq!(cfg_condition(&pred_disable, &enable, &disable), Some(false));
    assert_eq!(cfg_condition(&pred_unknown, &enable, &disable), None);
    assert_eq!(cfg_condition(&pred_any, &enable, &disable), None);
}

fn cfg_condition(pred: &Meta, cfg_enable: &[Meta], cfg_disable: &[Meta]) -> Option<bool> {
    if let Some(id) = pred.path().get_ident() {
        match id.to_string().as_str() {
            "all" => {
                if let Meta::List(list) = pred {
                    let preds = list
                        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                        .ok()?;
                    let mut result = true;
                    for pred in preds.iter() {
                        match cfg_condition(pred, cfg_enable, cfg_disable) {
                            Some(true) => {}
                            Some(false) => result = false,
                            None => return None,
                        }
                    }
                    return Some(result);
                }
            }
            "any" => {
                if let Meta::List(list) = pred {
                    let preds = list
                        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                        .ok()?;
                    let mut result = false;
                    for pred in preds.iter() {
                        match cfg_condition(pred, cfg_enable, cfg_disable) {
                            Some(true) => result = true,
                            Some(false) => {}
                            None => return None,
                        }
                    }
                    return Some(result);
                }
            }
            "not" => {
                if let Meta::List(list) = pred {
                    let pred = list.parse_args().ok()?;
                    return cfg_condition(&pred, cfg_enable, cfg_disable).map(|pred| !pred);
                }
            }
            _ => {}
        }
    }
    if cfg_disable.iter().any(|spec| spec == pred) {
        Some(false)
    } else if cfg_enable.iter().any(|spec| spec == pred) {
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
