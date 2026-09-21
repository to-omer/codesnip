use crate::{
    AttributeExt as _, Error, ItemExt as _, PathExt as _,
    entry::{EntryArgs, anonymous_name},
    format::FormatOption,
};
use quote::{ToTokens as _, quote};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    pub when: BTreeSet<String>,
    pub hidden: bool,
}

#[derive(Debug, Copy, Clone)]
pub struct Filter<'a, 'i> {
    filter_attr: &'a [Path],
    filter_item: &'i [Path],
}

struct CollectEntries<'m, 'i, 'a> {
    map: &'m mut SnippetMap,
    filter: Filter<'i, 'a>,
    in_entry: bool,
    error: Option<Error>,
}

impl SnippetMap {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn extend_with_filter(&mut self, item: &Item, filter: Filter) -> Result<(), Error> {
        let mut collector = CollectEntries {
            map: self,
            filter,
            in_entry: false,
            error: None,
        };
        collector.visit_item(item);
        match collector.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    pub fn extend(
        &mut self,
        entries: impl IntoIterator<Item = (String, LinkedSnippet)>,
    ) -> Result<(), Error> {
        for (name, mut other) in entries {
            let link = self.map.entry(name.clone()).or_default();
            if !link.when.is_empty() && !other.when.is_empty() && link.when != other.when {
                return Err(Error::ConflictingWhen(name));
            }
            link.contents.push_str(&other.contents);
            link.includes.append(&mut other.includes);
            link.when.append(&mut other.when);
            link.hidden |= other.hidden;
        }
        Ok(())
    }
    fn resolve_includes<'s>(
        &'s self,
        used: &BTreeSet<&'s str>,
        names: &[&'s str],
    ) -> Result<BTreeSet<&'s str>, Error> {
        for &name in names {
            if !self.map.contains_key(name) {
                return Err(Error::SnippetNotFound(name.to_owned()));
            }
        }
        let mut visited = used.clone();
        let mut stack = names.to_vec();
        loop {
            while let Some(include) = stack.pop() {
                if visited.insert(include)
                    && let Some(link) = self.map.get(include)
                {
                    for condition in &link.when {
                        if !self.map.contains_key(condition) {
                            return Err(Error::SnippetNotFound(condition.clone()));
                        }
                    }
                    stack.extend(link.includes.iter().chain(&link.when).map(String::as_str));
                }
            }
            for (name, link) in &self.map {
                if !visited.contains(name.as_str())
                    && !link.when.is_empty()
                    && link
                        .when
                        .iter()
                        .all(|condition| visited.contains(condition.as_str()))
                {
                    stack.push(name.as_str());
                }
            }
            if stack.is_empty() {
                break;
            }
        }
        Ok(visited)
    }
    pub fn bundle(
        &self,
        names: &[&str],
        excludes: BTreeSet<&str>,
        guard: bool,
    ) -> Result<String, Error> {
        fn push_guard(contents: &mut String, name: &str) {
            if contents.chars().next_back().is_some_and(|ch| ch != '\n') {
                contents.push('\n');
            }
            contents.push_str("// codesnip-guard: ");
            contents.push_str(name);
            contents.push('\n');
        }

        let visited = self.resolve_includes(&excludes, names)?;
        let roots: BTreeSet<_> = names.iter().copied().collect();
        let mut contents = String::new();
        for include in roots
            .difference(&excludes)
            .chain(
                visited
                    .difference(&excludes)
                    .filter(|name| !roots.contains(**name)),
            )
            .copied()
        {
            if guard {
                push_guard(&mut contents, include);
            }
            if let Some(nlink) = self.map.get(include) {
                contents.push_str(nlink.contents.as_str());
            }
        }
        Ok(contents)
    }
    pub fn keys(&self, hide: bool) -> Vec<&str> {
        if hide {
            self.map
                .iter()
                .filter(|(name, link)| !name.starts_with('_') && !link.hidden)
                .map(|(name, _)| name.as_ref())
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
    pub fn format(&mut self, option: &FormatOption, edition: &str) -> bool {
        if let Some(formatted) = option.format(&self.contents, edition) {
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

impl CollectEntries<'_, '_, '_> {
    fn check_nested_entry(&mut self, attrs: &[Attribute]) {
        if self.in_entry
            && let Some(attr) = attrs.iter().find(|attr| attr.path().is_codesnip_entry())
        {
            self.error.get_or_insert_with(|| {
                syn::Error::new_spanned(attr, "nested snippet entries are not supported").into()
            });
        }
    }
}

impl Visit<'_> for CollectEntries<'_, '_, '_> {
    fn visit_item(&mut self, item: &Item) {
        if self.error.is_some() {
            return;
        }
        let mut has_entry = false;
        if let Some(attrs) = item.get_attributes() {
            self.check_nested_entry(attrs);
            if self.error.is_some() {
                return;
            }
            for attr in attrs.iter().filter(|attr| attr.path().is_codesnip_entry()) {
                let entry = match attr
                    .parse_args_empty_with(EntryArgs::parse)
                    .and_then(|args| args.try_to_entry(item))
                {
                    Ok(entry) => entry,
                    Err(error) => {
                        self.error = Some(error.into());
                        return;
                    }
                };
                has_entry = true;
                let mut link = LinkedSnippet {
                    hidden: entry.name.is_none(),
                    when: entry.when,
                    ..Default::default()
                };
                let name = entry.name.unwrap_or_else(|| anonymous_name(&link.when));
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
                if let Err(error) = self.map.extend([(name, link)]) {
                    self.error = Some(error);
                    return;
                }
            }
        }
        let in_entry = self.in_entry;
        self.in_entry |= has_entry;
        visit::visit_item(self, item);
        self.in_entry = in_entry;
    }

    fn visit_impl_item(&mut self, item: &ImplItem) {
        self.check_nested_entry(match item {
            ImplItem::Const(item) => &item.attrs,
            ImplItem::Fn(item) => &item.attrs,
            ImplItem::Type(item) => &item.attrs,
            ImplItem::Macro(item) => &item.attrs,
            _ => &[],
        });
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &TraitItem) {
        self.check_nested_entry(match item {
            TraitItem::Const(item) => &item.attrs,
            TraitItem::Fn(item) => &item.attrs,
            TraitItem::Type(item) => &item.attrs,
            TraitItem::Macro(item) => &item.attrs,
            _ => &[],
        });
        visit::visit_trait_item(self, item);
    }

    fn visit_foreign_item(&mut self, item: &ForeignItem) {
        self.check_nested_entry(match item {
            ForeignItem::Fn(item) => &item.attrs,
            ForeignItem::Static(item) => &item.attrs,
            ForeignItem::Type(item) => &item.attrs,
            ForeignItem::Macro(item) => &item.attrs,
            _ => &[],
        });
        visit::visit_foreign_item(self, item);
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

#[test]
fn test_collect_entries() {
    let mut map = SnippetMap::new();
    let item = syn::parse_quote! {
        mod sample {
            #[codesnip::entry("C", include("A"))] struct First;
            #[codesnip::entry("C", when("A", "B"), include("B"))] struct Second;
            #[codesnip::entry("C")] struct Third;
            #[codesnip::entry(when("A", "B"))] struct Fourth;
            #[codesnip::entry(when("B", "A", "A"))] struct Fifth;
        }
    };
    map.extend_with_filter(&item, Filter::new(&[], &[]))
        .unwrap();
    assert_eq!(map.keys(true), ["C"]);
    assert_eq!(map.keys(false).len(), 2);
    assert_eq!(map.map["C"].when, ["A".into(), "B".into()].into());
    assert_eq!(map.map["C"].includes, ["A".into(), "B".into()].into());
    assert_eq!(
        map.map["C"].contents,
        "struct First ;struct Second ;struct Third ;"
    );
    let name = anonymous_name(&["A".into(), "B".into()].into());
    assert_eq!(map.map[&name].contents, "struct Fourth ;struct Fifth ;");
    let item = syn::parse_quote!(
        #[codesnip::entry("C", when("A"))]
        struct C;
    );
    assert!(matches!(
        map.extend_with_filter(&item, Filter::new(&[], &[])),
        Err(Error::ConflictingWhen(_))
    ));
    for item in [
        syn::parse_quote!(
            #[codesnip::entry(when())]
            struct A;
        ),
        syn::parse_quote!(
            #[codesnip::entry]
            mod a {
                #[codesnip::entry]
                struct B;
            }
        ),
        syn::parse_quote!(
            #[codesnip::entry(inline)]
            mod a {
                #[codesnip::entry(when("B"))]
                struct C;
            }
        ),
        syn::parse_quote!(
            #[codesnip::entry("A")]
            impl A {
                #[codesnip::entry(when("A", "B"))]
                pub fn bridge(_: B) {}
            }
        ),
        syn::parse_quote!(
            #[codesnip::entry]
            trait A {
                #[codesnip::entry(when("A", "B"))]
                fn bridge(_: B) {}
            }
        ),
        syn::parse_quote!(
            #[codesnip::entry("A")]
            unsafe extern "C" {
                #[codesnip::entry(when("A", "B"))]
                fn bridge(_: B);
            }
        ),
    ] {
        assert!(matches!(
            SnippetMap::new().extend_with_filter(&item, Filter::new(&[], &[])),
            Err(Error::InvalidEntry(_))
        ));
    }
}

#[test]
fn test_bundle() {
    let mut map = SnippetMap::new();
    for (name, includes, when) in [
        ("A", vec![], vec![]),
        ("B", vec![], vec![]),
        ("C", vec!["D"], vec!["A", "B"]),
        ("D", vec![], vec![]),
        ("E", vec![], vec!["C", "D"]),
        ("I", vec!["C"], vec![]),
        ("X", vec![], vec!["Y"]),
        ("Y", vec![], vec!["X"]),
    ] {
        map.map.insert(
            name.into(),
            LinkedSnippet {
                contents: name.into(),
                includes: includes.into_iter().map(String::from).collect(),
                when: when.into_iter().map(String::from).collect(),
                ..Default::default()
            },
        );
    }
    for (names, excludes, expected) in [
        (vec!["A"], vec![], "A"),
        (vec!["B"], vec![], "B"),
        (vec!["A", "B"], vec![], "ABCDE"),
        (vec!["C"], vec![], "CABDE"),
        (vec!["I"], vec![], "IABCDE"),
        (vec!["X"], vec![], "XY"),
        (vec!["B"], vec!["A"], "BCDE"),
        (vec!["A"], vec!["A", "B"], "CDE"),
        (vec!["C"], vec!["A", "B", "C", "D", "E"], ""),
    ] {
        assert_eq!(
            map.bundle(&names, excludes.into_iter().collect(), false)
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        map.bundle(&["A"], Default::default(), true).unwrap(),
        "// codesnip-guard: A\nA"
    );
    assert!(matches!(
        map.bundle(&["missing"], Default::default(), false),
        Err(Error::SnippetNotFound(_))
    ));
}
