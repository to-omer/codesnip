use crate::mapping::SnippetMapExt as _;
use codesnip_core::{Filter, FormatOption, SnippetMap, parse_file_recursive};
use git2::build::RepoBuilder;
use serde::{Deserialize, Deserializer};
use serde_with::{DeserializeAs, DisplayFromStr, serde_as};
use std::{
    fmt,
    marker::PhantomData,
    path::{Path, PathBuf},
};
use syn::parse_str;
use tempfile::{TempDir, tempdir};

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Sources {
    pub sources: Vec<Source>,
    #[serde(default)]
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub cfg: Option<Vec<syn::Meta>>,
    #[serde(default)]
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub filter_attr: Option<Vec<syn::Path>>,
    #[serde(default)]
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub filter_item: Option<Vec<syn::Path>>,
    #[serde(default)]
    #[serde_as(as = "DisplayFromStr")]
    pub format: FormatOption,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub prefix: Option<String>,
    pub git: Option<GitHubSource>,
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub cfg: Option<Vec<syn::Meta>>,
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub filter_attr: Option<Vec<syn::Path>>,
    #[serde_as(as = "Option<Vec<SynParse>>")]
    pub filter_item: Option<Vec<syn::Path>>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubSource {
    pub url: String,
    #[serde(flatten)]
    pub dependency: Option<GitDependency>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitDependency {
    Branch(String),
    Tag(String),
    Rev(String),
}

struct SynParse;

impl<'de, T> DeserializeAs<'de, T> for SynParse
where
    T: syn::parse::Parse,
{
    fn deserialize_as<D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Helper<S>(PhantomData<S>);
        impl<S> serde::de::Visitor<'_> for Helper<S>
        where
            S: syn::parse::Parse,
        {
            type Value = S;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                parse_str(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Helper(PhantomData))
    }
}

impl Sources {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        enum SerializeType {
            Json,
            Toml,
        }
        let ty = match path.as_ref().extension() {
            Some(ext) if ext == "json" => SerializeType::Json,
            Some(ext) if ext == "toml" => SerializeType::Toml,
            _ => return Err(anyhow::anyhow!("Invalid file extension")),
        };
        let sources: Self = match ty {
            SerializeType::Json => {
                let file = std::fs::File::open(path)?;
                let reader = std::io::BufReader::new(file);
                serde_json::from_reader(reader)?
            }
            SerializeType::Toml => toml::from_str(&std::fs::read_to_string(path)?)?,
        };
        Ok(sources)
    }
    pub fn snippet_map(&self) -> anyhow::Result<SnippetMap> {
        let mut map = SnippetMap::new();
        for source in &self.sources {
            map.extend(source.snippet_map(self)?);
        }
        map.format_all(&self.format);
        Ok(map)
    }
}

impl Source {
    fn snippet_map(&self, sources: &Sources) -> anyhow::Result<SnippetMap> {
        let (guard, path) = if let Some(git_source) = self.git.as_ref() {
            let dir = git_source.prepare()?;
            let path = dir.path().join(&self.path);
            (Some(dir), path)
        } else {
            (None, self.path.clone())
        };

        let mut map = SnippetMap::new();
        let mut items = Vec::new();
        let cfg = vec![];
        let cfg = self.cfg.as_ref().or(sources.cfg.as_ref()).unwrap_or(&cfg);
        items.append(&mut parse_file_recursive(path, cfg)?.items);
        drop(guard);

        let filter = vec![];
        let filter = Filter::new(
            self.filter_attr
                .as_ref()
                .or(sources.filter_attr.as_ref())
                .unwrap_or(&filter),
            self.filter_item
                .as_ref()
                .or(sources.filter_item.as_ref())
                .unwrap_or(&filter),
        );
        map.collect_entries(&items, filter);

        if let Some(prefix) = &self.prefix {
            map = map
                .into_iter()
                .map(|(k, v)| (format!("{}_{}", prefix, k), v))
                .collect();
        }

        Ok(map)
    }
}

impl GitHubSource {
    fn prepare(&self) -> anyhow::Result<TempDir> {
        let dir = tempdir()?;

        let mut builder = RepoBuilder::new();
        builder.fetch_options({
            let mut fetch_options = git2::FetchOptions::new();
            if matches!(self.dependency, Some(GitDependency::Tag(_))) {
                fetch_options.download_tags(git2::AutotagOption::All);
            }
            if matches!(self.dependency, Some(GitDependency::Branch(_)) | None) {
                fetch_options.depth(1);
            }
            fetch_options
        });
        if let Some(GitDependency::Branch(branch)) = self.dependency.as_ref() {
            builder.branch(branch);
        }
        let repo = builder.clone(&self.url, dir.path())?;

        match self.dependency.as_ref() {
            Some(GitDependency::Tag(tag)) => {
                let object = repo.revparse_single(&format!("refs/tags/{}", tag))?;
                repo.checkout_tree(&object, None)?;
            }
            Some(GitDependency::Rev(rev)) => {
                let object = repo.revparse_single(rev)?;
                repo.checkout_tree(&object, None)?;
            }
            _ => {}
        };
        Ok(dir)
    }
}
