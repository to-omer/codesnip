use rust_minify::{minify_opt, MinifyOption};
use std::{
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
};

#[derive(Debug, Clone)]
pub enum FormatOption {
    Rustfmt,
    Minify,
}

impl FromStr for FormatOption {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rustfmt" => Ok(Self::Rustfmt),
            "minify" => Ok(Self::Minify),
            _ => Err("expected one of [rustfmt|minify]"),
        }
    }
}

impl FormatOption {
    pub const POSSIBLE_VALUES: [&'static str; 2] = ["rustfmt", "minify"];
    pub fn format(&self, content: &str) -> Option<String> {
        match self {
            Self::Rustfmt => format_with_rustfmt(content),
            Self::Minify => minify_opt(
                content,
                &MinifyOption {
                    remove_skip: true,
                    add_rustfmt_skip: true,
                },
            )
            .ok(),
        }
    }
}

pub fn rustfmt_exits() -> bool {
    let rustfmt = Path::new(env!("CARGO_HOME")).join("bin").join("rustfmt");
    let output = Command::new(rustfmt).arg("--version").output();
    output
        .map(|output| output.status.success())
        .unwrap_or_default()
}

pub fn format_with_rustfmt(s: &str) -> Option<String> {
    let rustfmt = Path::new(env!("CARGO_HOME")).join("bin").join("rustfmt");
    let mut command = Command::new(rustfmt)
        .args(&[
            "--quiet",
            "--config",
            "unstable_features=true,normalize_doc_attributes=true,newline_style=Unix",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    command.stdin.take().unwrap().write_all(s.as_bytes()).ok()?;
    let output = command.wait_with_output().ok()?;
    if output.status.success() {
        Some(unsafe { String::from_utf8_unchecked(output.stdout) })
    } else {
        None
    }
}

#[test]
fn test_format_contents() {
    assert_eq!(
        format_with_rustfmt("fn  main ( ) { }"),
        Some("fn main() {}\n".to_string())
    )
}
