# codesnip

## Install
```
$ rustup component add rustfmt
$ cargo install codesnip
```

## Dependencies
```toml
[dependencies]
codesnip = { version = "0.2.0", package = "codesnip_attr" }
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
    -t, --target <FILE>...         Target file paths
        --use-cache <FILE>...      Use cached data
        --cfg <SPEC>...            Configure the environment: e.g. --cfg=nightly
        --filter-item <PATH>...    Filter items by attributes path: e.g. --filter-item=test
        --filter-attr <PATH>...    Filter attributes by attributes path: e.g. --filter-attr=path

SUBCOMMANDS:
    cache      Save analyzed data into file
    list       List names
    snippet    Output snippet for VSCode
    bundle     Bundle
    verify     Verify
    help       Prints this message or the help of the given subcommand(s)
```

## VSCode Extension
[codesnip-vscode](https://github.com/to-omer/codesnip-vscode.git)
