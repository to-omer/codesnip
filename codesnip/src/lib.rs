pub mod mapping;
pub mod source;
pub mod verify;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
pub use codesnip_attr::{entry, skip};
use codesnip_core::{Error::FileNotFound, SnippetMap};
use source::Sources;
use std::{
    fs::File,
    io::{Read as _, Write as _, stdout},
    path::{Path, PathBuf},
};

#[derive(Debug, Parser)]
#[command(
    bin_name = "cargo",
    version,
    propagate_version = true,
    infer_subcommands = true
)]
pub enum Opt {
    /// Extract code snippets.
    Codesnip(Config),
}

#[derive(Debug, Args)]
#[command(rename_all = "kebab-case")]
pub struct Config {
    /// Use cached data.
    #[arg(long, value_name = "FILE", num_args = 1..)]
    pub use_cache: Vec<PathBuf>,

    /// Source config file path. see https://github.com/to-omer/codesnip#source-config
    #[arg(long, value_name = "FILE")]
    pub source_config: Option<PathBuf>,

    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Save analyzed data into file.
    Cache {
        /// Output file.
        #[arg(value_name = "FILE")]
        output: PathBuf,
    },
    /// List names.
    List {
        /// Not hide `entry(name = "_...")`.
        #[arg(long)]
        not_hide: bool,
    },
    /// Bundle
    Bundle {
        /// snippet name.
        #[arg(value_name = "NAME", required = true)]
        names: Vec<String>,
        /// excludes.
        #[arg(short, long, value_name = "NAME", num_args = 1..)]
        excludes: Vec<String>,
    },
    /// Verify
    Verify {
        #[arg(long, value_name = "TOOLCHAIN", default_value = "stable")]
        /// release channel or custom toolchain.
        toolchain: String,
        #[arg(long, value_name = "EDITION", default_value = "2021")]
        /// edition of the compiler.
        edition: String,
        /// compilation target triple.
        #[arg(long, value_name = "TRIPLE")]
        target: Option<String>,
        /// Extra rustc argument (repeat; overrides source config).
        #[arg(long = "rustc-arg", value_name = "ARG")]
        rustc_args: Vec<String>,
        /// Fail if rustc emits any warnings.
        #[arg(long)]
        deny_warnings: bool,
        /// Show more outputs.
        #[arg(long)]
        verbose: bool,
    },
}

impl Opt {
    pub fn from_args() -> Self {
        Self::parse()
    }

    pub fn execute(&self) -> anyhow::Result<()> {
        let Opt::Codesnip(opt) = self;
        opt.execute()
    }
}

const CACHE_HEADER: &[u8] = b"CODESNIP\0\x01";

impl Config {
    pub fn execute(&self) -> anyhow::Result<()> {
        let source_config = self.source_config.as_ref().map(Sources::load).transpose()?;
        let mut map = if let Some(source_config) = &source_config {
            source_config.snippet_map()?
        } else {
            SnippetMap::new()
        };

        let mut buf = Vec::new();
        for cache in self.use_cache.iter() {
            buf.clear();
            let mut file = File::open(cache).map_err(|err| FileNotFound(cache.clone(), err))?;
            file.read_to_end(&mut buf)?;
            let payload = buf
                .strip_prefix(CACHE_HEADER)
                .context("unsupported cache format; regenerate it with `cargo codesnip cache`")?;
            let mapt: SnippetMap = postcard::from_bytes(payload)?;
            map.extend(mapt)?;
        }

        self.cmd.execute(map, source_config.as_ref())
    }
}

impl Command {
    pub fn execute(&self, map: SnippetMap, source_config: Option<&Sources>) -> anyhow::Result<()> {
        match self {
            Self::Cache { output } => {
                let payload = postcard::to_stdvec(&map)?;
                let mut file = create_recursive(output)?;
                file.write_all(CACHE_HEADER)?;
                file.write_all(&payload)?;
            }
            Self::List { not_hide } => {
                let list = map.keys(!not_hide).join(" ");
                stdout().write_all(list.as_bytes())?;
            }
            Self::Bundle { names, excludes } => {
                let names: Vec<_> = names.iter().map(String::as_str).collect();
                let present = excludes.iter().map(String::as_str).collect();
                stdout().write_all(map.bundle(&names, present, true)?.as_bytes())?;
            }
            Self::Verify {
                toolchain,
                verbose,
                edition,
                target,
                rustc_args,
                deny_warnings,
            } => {
                let rustc_args = if rustc_args.is_empty() {
                    source_config.map_or(&[][..], |config| config.rustc_args.as_slice())
                } else {
                    rustc_args.as_slice()
                };
                verify::execute(
                    map,
                    toolchain,
                    edition,
                    target.as_deref(),
                    rustc_args,
                    *deny_warnings || source_config.is_some_and(|config| config.deny_warnings),
                    *verbose,
                )?;
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
