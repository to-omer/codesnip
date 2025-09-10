pub mod mapping;
pub mod source;
pub mod verify;

use crate::mapping::SnippetMapExt as _;
use anyhow::Context as _;
pub use codesnip_attr::{entry, skip};
use codesnip_core::{Error::FileNotFound, SnippetMap};
use serde_json::to_string;
use source::Sources;
use std::{
    fs::File,
    io::{Read as _, Write as _, stdout},
    path::{Path, PathBuf},
};
use structopt::{
    StructOpt,
    clap::AppSettings::{DeriveDisplayOrder, InferSubcommands},
};

#[derive(Debug, StructOpt)]
#[structopt(
    bin_name = "cargo",
    global_settings = &[DeriveDisplayOrder, InferSubcommands]
)]
pub enum Opt {
    /// Extract code snippets.
    Codesnip(Config),
}

#[derive(Debug, StructOpt)]
#[structopt(rename_all = "kebab-case")]
pub struct Config {
    /// Use cached data.
    #[structopt(long, value_name = "FILE", parse(from_os_str))]
    pub use_cache: Vec<PathBuf>,

    /// Source config file path. see https://github.com/to-omer/codesnip#source-config
    #[structopt(long, value_name = "FILE", parse(from_os_str))]
    pub source_config: Option<PathBuf>,

    #[structopt(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Save analyzed data into file.
    Cache {
        /// Output file.
        #[structopt(value_name = "FILE", parse(from_os_str))]
        output: PathBuf,
    },
    /// List names.
    List {
        /// Not hide `entry(name = "_...")`.
        #[structopt(long)]
        not_hide: bool,
    },
    /// Output snippet for VSCode.
    Snippet {
        /// Output file, default stdout.
        #[structopt(value_name = "FILE", parse(from_os_str))]
        output: Option<PathBuf>,
        /// ignore includes
        #[structopt(long)]
        ignore_include: bool,
    },
    /// Bundle
    Bundle {
        /// snippet name.
        #[structopt(value_name = "NAME")]
        name: String,
        /// excludes.
        #[structopt(short, long, value_name = "NAME")]
        excludes: Vec<String>,
    },
    /// Verify
    Verify {
        #[structopt(long, value_name = "TOOLCHAIN", default_value = "stable")]
        /// release channel or custom toolchain.
        toolchain: String,
        #[structopt(long, value_name = "EDITION", default_value = "2021")]
        /// edition of the compiler.
        edition: String,
        /// Show more outputs.
        #[structopt(long)]
        verbose: bool,
    },
}

impl Opt {
    pub fn from_args() -> Self {
        StructOpt::from_args()
    }

    pub fn execute(&self) -> anyhow::Result<()> {
        let Opt::Codesnip(opt) = self;
        opt.execute()
    }
}

impl Config {
    pub fn execute(&self) -> anyhow::Result<()> {
        let mut map = if let Some(source_config) = &self.source_config {
            let target_config = Sources::load(source_config)?;
            target_config.snippet_map()?
        } else {
            SnippetMap::new()
        };

        let mut buf = Vec::new();
        for cache in self.use_cache.iter() {
            buf.clear();
            let mut file = File::open(cache).map_err(|err| FileNotFound(cache.clone(), err))?;
            file.read_to_end(&mut buf)?;
            let (mapt, _): (SnippetMap, _) =
                bincode::serde::decode_from_slice(&buf, bincode::config::standard())?;
            map.extend(mapt);
        }

        self.cmd.execute(map)
    }
}

impl Command {
    pub fn execute(&self, map: SnippetMap) -> anyhow::Result<()> {
        match self {
            Self::Cache { output } => {
                create_recursive(output)?.write_all(&bincode::serde::encode_to_vec(
                    &map,
                    bincode::config::standard(),
                )?)?;
            }
            Self::List { not_hide } => {
                let list = map.keys(!not_hide).join(" ");
                stdout().write_all(list.as_bytes())?;
            }
            Self::Snippet {
                output,
                ignore_include,
            } => {
                let snippet = to_string(&map.to_vscode(*ignore_include))?;
                match output {
                    Some(file) => create_recursive(file)?.write_all(snippet.as_bytes())?,
                    None => stdout().write_all(snippet.as_bytes())?,
                }
            }
            Self::Bundle { name, excludes } => {
                let link = map
                    .map
                    .get(name)
                    .with_context(|| format!("snippet `{}` not found", name))?;
                let excludes = excludes.iter().map(|s| s.as_str()).collect();
                stdout().write_all(map.bundle(name, link, excludes, true).as_bytes())?;
            }
            Self::Verify {
                toolchain,
                verbose,
                edition,
            } => {
                verify::execute(map, toolchain, edition, *verbose)?;
            }
        }
        Ok(())
    }
}

fn create_recursive<P: AsRef<Path>>(path: P) -> std::io::Result<File> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    File::create(path)
}
