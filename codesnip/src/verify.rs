use anyhow::Context as _;
use cargo_metadata::diagnostic::{Diagnostic, DiagnosticLevel};
use codesnip_core::SnippetMap;
use console::{Color, colors_enabled_stderr, strip_ansi_codes, style};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::{fs::File, io::Write as _, sync::atomic::AtomicBool};
use std::{
    iter::repeat_n,
    process::{Command, Stdio},
};
use tempfile::tempdir;

pub fn execute(
    map: SnippetMap,
    toolchain: &str,
    edition: &str,
    verbose: bool,
) -> anyhow::Result<()> {
    let ok = AtomicBool::new(true);
    let pb = ProgressBar::new(map.map.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>12.green} [{bar:57}] {pos}/{len}: {msg}")
            .unwrap()
            .progress_chars("=> "),
    );
    pb.set_prefix("Checking");

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
        pb.set_message(name.to_owned());
        for include in link.includes.iter() {
            if !map.map.contains_key(include) {
                ok.store(false, std::sync::atomic::Ordering::Relaxed);
                pb_println!(
                    "{}: Invalid include `{}` in {}.",
                    style("warning").yellow(),
                    include,
                    name
                );
            }
        }
        let contents = map.bundle(name, link, Default::default(), false);
        match check(name, &contents, toolchain, edition) {
            Ok((success, messages)) => {
                if !success {
                    ok.store(false, std::sync::atomic::Ordering::Relaxed);
                    pb_println!("{:>12} {}", style("Failed").red(), name);
                } else if verbose {
                    pb_println!(
                        "{:>12} {:.<45}.{:.>8} Byte",
                        style("Verified").green().bright(),
                        name,
                        contents.len()
                    );
                }
                if verbose {
                    for message in messages {
                        if let Some(message) = format_error_message(name, message) {
                            pb_println!("{}", message);
                        }
                    }
                }
            }
            Err(err) => {
                ok.store(false, std::sync::atomic::Ordering::Relaxed);
                pb_println!("{}: {}", style("error").red(), err);
            }
        }
        pb.inc(1);
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

fn check(
    name: &str,
    contents: &str,
    toolchain: &str,
    edition: &str,
) -> anyhow::Result<(bool, Vec<Diagnostic>)> {
    let dir = tempdir()?;
    let lib = dir.path().join(name);
    {
        let mut file = File::create(&lib)?;
        file.write_all(contents.as_bytes())?;
    }
    let mut out_dir: std::ffi::OsString = "--out-dir=".to_owned().into();
    out_dir.push(dir.path().as_os_str());
    let output = Command::new("rustc")
        .args([
            format!("+{}", toolchain).as_ref(),
            lib.as_os_str(),
            format!("--edition={}", edition).as_ref(),
            "--crate-type=lib".as_ref(),
            "--error-format=json".as_ref(),
            out_dir.as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let messages: Vec<Diagnostic> = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Ok((output.status.success(), messages))
}

fn format_error_message(name: &str, message: Diagnostic) -> Option<String> {
    let mut s = String::new();
    let code = message
        .code
        .as_ref()
        .map(|code| format!("[{}]", code.code))
        .unwrap_or_default();
    let (color, status) = match message.level {
        DiagnosticLevel::Error => (Color::Red, "error"),
        DiagnosticLevel::Warning => (Color::Yellow, "warning"),
        _ => {
            return None;
        }
    };
    s.push_str(&format!(
        "{}: {}\n",
        style(format!("{}{}", status, code)).fg(color),
        &message.message
    ));
    for span in message.spans.iter() {
        let k = format!("{}", span.line_end).len();
        s.push_str(&format!(
            "{:>k$} {}:{}:{}\n{:>k$}",
            style("-->").cyan().bright(),
            name,
            span.line_start,
            span.column_start,
            style(" | ").cyan().bright(),
            k = k + 3,
        ));
        for (line, text) in (span.line_start..=span.line_end).zip(span.text.iter()) {
            s.push_str(&format!(
                "\n{}{}\n{:>k$}",
                style(format!("{:>k$} | ", line, k = k)).cyan().bright(),
                &text.text,
                style(" | ").cyan().bright(),
                k = k + 3,
            ));
            s.extend(repeat_n(' ', text.highlight_start - 1));
            s.push_str(
                &style("^".repeat(text.highlight_end - text.highlight_start))
                    .fg(color)
                    .to_string(),
            );
        }
        s.push_str(&format!(
            " {}\n",
            style(span.label.as_ref().cloned().unwrap_or_default()).fg(color)
        ));
    }
    Some(s)
}
