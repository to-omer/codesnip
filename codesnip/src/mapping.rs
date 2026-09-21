use codesnip_core::{Error, Filter, FormatOption, SnippetMap, rustfmt_exits};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use syn::Item;

pub trait SnippetMapExt {
    fn collect_entries(&mut self, items: &[Item], filter: Filter) -> Result<(), Error>;
    fn format_all(&mut self, option: &FormatOption);
}

impl SnippetMapExt for SnippetMap {
    fn collect_entries(&mut self, items: &[Item], filter: Filter) -> Result<(), Error> {
        let pb = ProgressBar::new(items.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{prefix:>12.green} [{bar:57}] {pos}/{len}")
                .unwrap()
                .progress_chars("=> "),
        );
        pb.set_prefix("Collecting");
        for item in items {
            self.extend_with_filter(item, filter)?;
            pb.inc(1);
        }
        pb.finish_and_clear();
        Ok(())
    }
    fn format_all(&mut self, option: &FormatOption) {
        if matches!(option, FormatOption::Rustfmt) && !rustfmt_exits() {
            eprintln!("warning: rustfmt not found.");
            return;
        }
        let pb = ProgressBar::new(self.map.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{prefix:>12.green} [{bar:57}] {pos}/{len}: {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        pb.set_prefix("Formatting");
        self.map.par_iter_mut().for_each(|(name, link)| {
            pb.set_message(name.to_owned());
            if !link.format(option) {
                pb.println(format!("warning: Failed to format `{}`.", name));
            }
            pb.inc(1);
        });
        pb.finish_and_clear();
    }
}
