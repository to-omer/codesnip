# codesnip

## Install
```
$ rustup component add rustfmt
$ cargo install codesnip
```

## Dependencies
```toml
[dependencies]
codesnip = { version = "0.4.0", package = "codesnip_attr" }
```

## Example
Add `#[codesnip::entry]` to snippet item.
```rust
// examples/math.rs
#[codesnip::entry]
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        a %= b;
        std::mem::swap(&mut a, &mut b);
    }
    a
}

#[codesnip::entry(include("gcd"))]
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}
```

This code extracted as below.

```sh
$ cargo codesnip --target=examples/math.rs bundle gcd
// codesnip-guard: gcd
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        a %= b;
        std::mem::swap(&mut a, &mut b);
    }
    a
}

$ cargo codesnip --target=examples/math.rs bundle lcm
// codesnip-guard: lcm
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}
// codesnip-guard: gcd
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        a %= b;
        std::mem::swap(&mut a, &mut b);
    }
    a
}

$ cargo codesnip --target=examples/math.rs bundle lcm --excludes gcd
// codesnip-guard: lcm
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}
```

## Format
```
#[codesnip::entry (AttrList,*)?]       add item for snippet
#[codesnip::skip]                      skip item for snippet

AttrList:
    NAME | INCLUDE | INLINE

NAME:
    Lit
  | name = Lit

INCLUDE:                  specify NAME
    include (Lit,*)

INLINE:
    inline                inline `mod ... { ... }`
  | no_inline             default

Lit:
    "..."
  | "_..."                hidden

...:
    No whitespace string
```

## Usage
```
USAGE:
    cargo codesnip [OPTIONS] <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --use-cache <FILE>...     Use cached data
        --source-config <FILE>    Source config file path

SUBCOMMANDS:
    cache      Save analyzed data into file
    list       List names
    snippet    Output snippet for VSCode
    bundle     Bundle
    verify     Verify
    help       Prints this message or the help of the given subcommand(s)
```

## Source Config
JSON schema for snippet source config.
```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Sources",
  "type": "object",
  "properties": {
    "sources": {
      "type": "array",
      "items": {
        "description": "Source config",
        "type": "object",
        "properties": {
          "path": { "description": "Source path", "type": "string", "examples": ["src/lib.rs"] },
          "prefix": { "description": "Prefix for snippet name", "type": "string" },
          "git": {
            "description": "Specify git repository",
            "type": "object",
            "properties": {
              "url": { "description": "Git repository URL", "type": "string", "examples": ["https://github.com/owner/repo.git"] },
              "branch": { "description": "Git repository branch", "type": "string", "examples": ["main"] },
              "tag": { "description": "Git repository tag", "type": "string", "examples": ["v0.1.0"] },
              "rev": { "description": "Git repository revision", "type": "string", "examples": ["abcdef"] }
            },
            "required": ["url"]
          },
          "cfg": {
            "description": "Configure the environment",
            "type": "array",
            "items": { "type": "string" }
          },
          "filter_attr": {
            "description": "Filter attributes by attributes path",
            "type": "array",
            "items": { "type": "string" }
          },
          "filter_item": {
            "description": "Filter items by attributes path",
            "type": "array",
            "items": { "type": "string" }
          }
        },
        "required": ["path"]
      }
    },
    "cfg": {
      "description": "Configure the environment (global)",
      "type": "array",
      "items": { "type": "string" }
    },
    "filter_attr": {
      "description": "Filter attributes by attributes path (global)",
      "type": "array",
      "items": { "type": "string" }
    },
    "filter_item": {
      "description": "Filter items by attributes path (global)",
      "type": "array",
      "items": { "type": "string" }
    },
    "format": {
      "description": "Format option",
      "type": "string",
      "enum": ["rustfmt", "minify"],
      "default": "rustfmt"
    }
  },
  "required": ["sources"]
}
```

## VSCode Extension
[codesnip-vscode](https://github.com/to-omer/codesnip-vscode.git)
