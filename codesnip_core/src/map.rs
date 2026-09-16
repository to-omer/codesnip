use crate::{
    AttributeExt as _, ItemExt as _, PathExt as _, entry::EntryArgs, format::FormatOption,
};
use quote::{ToTokens as _, quote};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    iter::FromIterator,
};
use syn::{
    Attribute, ForeignItem, ImplItem, Item, ItemMod, Meta, Path, Token, TraitItem,
    parse::Parse as _,
    punctuated::Punctuated,
    visit::{self, Visit},
    visit_mut::{self, VisitMut},
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SnippetMap {
    pub map: BTreeMap<String, LinkedSnippet>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LinkedSnippet {
    pub contents: String,
    pub includes: BTreeSet<String>,
}

#[derive(Debug, Copy, Clone)]
pub struct Filter<'a, 'i> {
    filter_attr: &'a [Path],
    filter_item: &'i [Path],
}

struct CollectEntries<'m, 'i, 'a> {
    map: &'m mut SnippetMap,
    filter: Filter<'i, 'a>,
}

impl SnippetMap {
    pub fn new() -> Self {
        Default::default()
    }
    fn get_mut(&mut self, name: &str) -> &mut LinkedSnippet {
        if !self.map.contains_key(name) {
            self.map.insert(name.to_string(), Default::default());
        }
        self.map
            .get_mut(name)
            .expect("BTreeMap is not working properly.")
    }
    pub fn extend_with_filter(&mut self, item: &Item, filter: Filter) {
        CollectEntries { map: self, filter }.visit_item(item);
    }
    fn resolve_includes<'s>(
        &'s self,
        used: &BTreeSet<&'s str>,
        includes: impl IntoIterator<Item = &'s str>,
    ) -> BTreeSet<&'s str> {
        let mut visited = used.clone();
        let mut stack: Vec<_> = includes.into_iter().collect();
        visited.extend(&stack);
        while let Some(include) = stack.pop() {
            if let Some(nlink) = self.map.get(include) {
                for ninclude in nlink.includes.iter().map(|s| s.as_str()) {
                    if !visited.contains(ninclude) {
                        visited.insert(ninclude);
                        stack.push(ninclude);
                    }
                }
            }
        }
        visited
    }
    pub fn bundle<'s>(
        &self,
        name: &'s str,
        link: &LinkedSnippet,
        mut excludes: BTreeSet<&'s str>,
        guard: bool,
    ) -> String {
        fn push_guard(contents: &mut String, name: &str) {
            if contents.chars().next_back().is_some_and(|ch| ch != '\n') {
                contents.push('\n');
            }
            contents.push_str("// codesnip-guard: ");
            contents.push_str(name);
            contents.push('\n');
        }

        if excludes.contains(name) {
            return Default::default();
        }
        excludes.insert(name);
        let visited = self.resolve_includes(&excludes, link.includes.iter().map(|s| s.as_str()));
        let mut contents = String::new();
        if guard {
            push_guard(&mut contents, name);
        }
        contents.push_str(link.contents.as_str());
        for include in visited.difference(&excludes).cloned() {
            if guard {
                push_guard(&mut contents, include);
            }
            if let Some(nlink) = self.map.get(include) {
                contents.push_str(nlink.contents.as_str());
            }
        }
        contents
    }
    pub fn keys(&self, hide: bool) -> Vec<&str> {
        if hide {
            self.map
                .keys()
                .filter(|name| !name.starts_with('_'))
                .map(|name| name.as_ref())
                .collect()
        } else {
            self.map.keys().map(|name| name.as_ref()).collect()
        }
    }
}

impl IntoIterator for SnippetMap {
    type Item = (String, LinkedSnippet);
    type IntoIter = <BTreeMap<String, LinkedSnippet> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter()
    }
}

impl Extend<(String, LinkedSnippet)> for SnippetMap {
    fn extend<T: IntoIterator<Item = (String, LinkedSnippet)>>(&mut self, iter: T) {
        for (name, link) in iter {
            self.map.entry(name).or_default().append(link);
        }
    }
}

impl FromIterator<(String, LinkedSnippet)> for SnippetMap {
    fn from_iter<T: IntoIterator<Item = (String, LinkedSnippet)>>(iter: T) -> Self {
        let mut map = Self::new();
        map.extend(iter);
        map
    }
}

impl LinkedSnippet {
    pub fn push_contents(&mut self, contents: &str) {
        self.contents.push_str(contents);
    }
    pub fn push_item_with_filter(&mut self, item: &Item, filter: Filter) {
        if let Some(item) = filter.modify_item(item.clone()) {
            self.contents
                .push_str(&item.into_token_stream().to_string());
        }
    }
    pub fn push_include(&mut self, include: String) {
        self.includes.insert(include);
    }
    pub fn push_includes(&mut self, includes: impl IntoIterator<Item = String>) {
        self.includes.extend(includes);
    }
    pub fn append(&mut self, mut other: Self) {
        self.contents.push_str(&other.contents);
        self.includes.append(&mut other.includes);
    }
    pub fn format(&mut self, option: &FormatOption) -> bool {
        if let Some(formatted) = option.format(&self.contents) {
            self.contents = formatted;
            true
        } else {
            false
        }
    }
}

impl<'a, 'i> Filter<'a, 'i> {
    pub fn new(filter_attr: &'a [Path], filter_item: &'i [Path]) -> Self {
        Self {
            filter_attr,
            filter_item,
        }
    }
}

impl Visit<'_> for CollectEntries<'_, '_, '_> {
    fn visit_item(&mut self, item: &Item) {
        if let Some(attrs) = item.get_attributes() {
            for entry in attrs
                .iter()
                .filter(|attr| attr.path().is_codesnip_entry())
                .filter_map(|attr| attr.parse_args_empty_with(EntryArgs::parse).ok())
                .filter_map(|args| args.try_to_entry(item).ok())
            {
                let link = self.map.get_mut(&entry.name);
                let filter = self.filter;
                match (entry.inline, item) {
                    (true, Item::Mod(ItemMod { attrs, content, .. })) => {
                        if !filter.is_skip_item(attrs)
                            && let Some((_, items)) = content
                        {
                            for item in items {
                                let mut item = item.clone();
                                if let Some(child_attrs) = item.get_attributes_mut() {
                                    child_attrs.extend(
                                        attrs
                                            .iter()
                                            .filter(|attr| attr.path().is_ident("cfg"))
                                            .cloned()
                                            .map(|mut attr| {
                                                attr.style = syn::AttrStyle::Outer;
                                                attr
                                            }),
                                    );
                                }
                                link.push_item_with_filter(&item, filter);
                            }
                        }
                    }
                    _ => link.push_item_with_filter(item, filter),
                }
                link.push_includes(entry.include);
            }
        }
        visit::visit_item(self, item);
    }
}

impl Filter<'_, '_> {
    fn is_skip_item(self, attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_codesnip_skip() || self.filter_item.iter().any(|pat| pat == attr.path())
        })
    }

    fn filter_attributes(self, attrs: &mut Vec<Attribute>) {
        attrs.retain_mut(|attr| self.retain_meta(&mut attr.meta));
    }

    fn retain_meta(self, meta: &mut Meta) -> bool {
        if meta.path().is_codesnip_entry() || self.filter_attr.iter().any(|pat| pat == meta.path())
        {
            return false;
        }
        if meta.path().is_ident("cfg_attr")
            && let Meta::List(list) = meta
            && let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        {
            let mut args = args.into_iter();
            if let Some(pred) = args.next() {
                let attrs: Vec<_> = args
                    .filter_map(|mut attr| self.retain_meta(&mut attr).then_some(attr))
                    .collect();
                if attrs.is_empty() {
                    return false;
                }
                list.tokens = quote!(#pred, #(#attrs),*);
            }
        }
        true
    }

    fn modify_item(mut self, mut item: Item) -> Option<Item> {
        if let Some(attrs) = item.get_attributes()
            && self.is_skip_item(attrs)
        {
            return None;
        }

        visit_mut::visit_item_mut(&mut self, &mut item);
        Some(item)
    }
}

impl VisitMut for Filter<'_, '_> {
    fn visit_attributes_mut(&mut self, attrs: &mut Vec<Attribute>) {
        self.filter_attributes(attrs);
    }

    fn visit_item_mut(&mut self, item: &mut Item) {
        let old = std::mem::replace(item, Item::Verbatim(Default::default()));
        if let Some(filtered) = self.modify_item(old) {
            *item = filtered;
        }
    }

    fn visit_impl_item_mut(&mut self, item: &mut ImplItem) {
        let attrs = match item {
            ImplItem::Const(item) => &item.attrs,
            ImplItem::Fn(item) => &item.attrs,
            ImplItem::Type(item) => &item.attrs,
            ImplItem::Macro(item) => &item.attrs,
            _ => return,
        };
        if self.is_skip_item(attrs) {
            *item = ImplItem::Verbatim(Default::default());
        } else {
            visit_mut::visit_impl_item_mut(self, item);
        }
    }

    fn visit_trait_item_mut(&mut self, item: &mut TraitItem) {
        let attrs = match item {
            TraitItem::Const(item) => &item.attrs,
            TraitItem::Fn(item) => &item.attrs,
            TraitItem::Type(item) => &item.attrs,
            TraitItem::Macro(item) => &item.attrs,
            _ => return,
        };
        if self.is_skip_item(attrs) {
            *item = TraitItem::Verbatim(Default::default());
        } else {
            visit_mut::visit_trait_item_mut(self, item);
        }
    }

    fn visit_foreign_item_mut(&mut self, item: &mut ForeignItem) {
        let attrs = match item {
            ForeignItem::Fn(item) => &item.attrs,
            ForeignItem::Static(item) => &item.attrs,
            ForeignItem::Type(item) => &item.attrs,
            ForeignItem::Macro(item) => &item.attrs,
            _ => return,
        };
        if self.is_skip_item(attrs) {
            *item = ForeignItem::Verbatim(Default::default());
        } else {
            visit_mut::visit_foreign_item_mut(self, item);
        }
    }
}
