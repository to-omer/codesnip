use anyhow::Context as _;
use codesnip_core::SnippetMap;
use console::{colors_enabled_stderr, strip_ansi_codes, style};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use regex::RegexBuilder;
use std::process::{Command, Stdio};
use std::{fs::File, io::Write as _, sync::atomic::AtomicBool};
use tempfile::tempdir;

pub fn execute(map: SnippetMap, verbose: bool) -> anyhow::Result<()> {
    let ok = AtomicBool::new(true);
    let pb = ProgressBar::new(map.map.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>12.green} [{bar:57}] {pos}/{len}: {msg}")
            .progress_chars("=> "),
    );
    pb.set_prefix("Checking");
    let re = RegexBuilder::new("error\\[.*$")
        .multi_line(true)
        .build()
        .unwrap();

    let is_hidden = pb.is_hidden();
    let colors_enabled = colors_enabled_stderr();

    macro_rules! pb_println {
        ($($t:tt)*) => {
            if is_hidden {
                if colors_enabled {
                    eprintln!($($t)*);
                } else {
                    eprintln!("{}", strip_ansi_codes(&format!($($t)*)));
                }
            } else {
                pb.println(format!($($t)*));
            }
        }
    }

    map.map.par_iter().for_each(|(name, link)| {
        pb.set_message(name);
        for include in link.includes.iter() {
            if !map.map.contains_key(include) {
                ok.store(false, std::sync::atomic::Ordering::Relaxed);
                pb_println!(
                    "{}: Invalid include `{}` in {}.",
                    style("warning").yellow().bright(),
                    include,
                    name
                );
            }
        }
        let contents = map.bundle(name, link, Default::default(), false);
        match check(&contents) {
            Ok(Some(err)) => {
                ok.store(false, std::sync::atomic::Ordering::Relaxed);
                let err = String::from_utf8_lossy(&err);
                pb_println!("{}: Compile failed in {}", style("error").red(), name);
                for msg in re.find_iter(&err) {
                    pb_println!("    {}", msg.as_str());
                }
            }
            Ok(None) => {}
            Err(err) => {
                ok.store(false, std::sync::atomic::Ordering::Relaxed);
                pb_println!("{}: {}", style("error").red(), err);
            }
        }
        pb.inc(1);
        if verbose {
            pb_println!(
                "{:>12} {:.<45}.{:.>8} Byte",
                style("Verified").green().bright(),
                name,
                contents.bytes().len()
            );
        }
    });
    pb.finish_and_clear();
    pb_println!(
        "{:>12} {} Snippets",
        style("Finished").green().bright(),
        map.map.len()
    );
    if ok.load(std::sync::atomic::Ordering::Relaxed) {
        Ok(())
    } else {
        None.with_context(|| "verify failed")
    }
}

pub fn check(contents: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let dir = tempdir()?;
    let lib = dir.path().join("lib.rs");
    {
        let mut file = File::create(&lib)?;
        file.write_all(contents.as_bytes())?;
    }
    let mut out_dir: std::ffi::OsString = "--out-dir=".to_owned().into();
    out_dir.push(dir.path().as_os_str());
    let output = Command::new("rustc")
        .args(&[
            lib.as_os_str(),
            "--edition=2018".as_ref(),
            "--crate-type=lib".as_ref(),
            "--error-format=short".as_ref(),
            out_dir.as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() {
        Ok(Some(output.stderr))
    } else {
        Ok(None)
    }
}
