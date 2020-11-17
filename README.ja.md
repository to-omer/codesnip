# codesnip

Rustのライブラリをスニペットとして管理するためのツールです。
[codesnip-vscode](https://github.com/to-omer/codesnip-vscode.git)と連携し、重複を排除したスニペットの挿入ができます。

## インストール
```
$ rustup component add rustfmt
$ cargo install --git https://github.com/to-omer/codesnip.git
```

## 使い方
`Cargo.toml`に依存関係を追加します。
```toml
[dependencies]
codesnip = { git = "https://github.com/to-omer/codesnip.git", package = "codesnip_attr" }
```

次のように`#[codesnip::entry]`をスニペットにしたいitemに付けます。
```rust
// src/lib.rs
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

コマンドを実行すると、標準出力に指定したスニペットが出力されます。
```
$ cargo codesnip --target=src/lib.rs bundle gcd
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        a %= b;
        std::mem::swap(&mut a, &mut b);
    }
    a
}

$ cargo codesnip --target=src/lib.rs bundle lcm
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        a %= b;
        std::mem::swap(&mut a, &mut b);
    }
    a
}
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}

$ cargo codesnip --target=src/lib.rs bundle lcm --excludes gcd
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}
```

## 詳細
### Attribute
Rustでは、クレート内のモジュール構造は木構造になります。この部分木に`#[codesnip::entry]`で名前を付け、名前をキーとしてスニペットを構築します。

`#[codesnip::entry]`には次のようなものが指定できます。
```
#[codesnip::entry (AttrList,*)?]

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
    [a-zA-Z_][a-zA-Z0-9_]*
```

`NAME`ではスニペットのキーを指定します。Rustの変数と同じものが指定できます。`NAME`が指定されていない場合、そのitemの名前っぽいものが（可能なら）使用されます。

`INCLUDE`では依存関係を追加することができます。

`mod`itemに`INLINE`を付けるとモジュール内のitemすべてにattributeを付けたのと同じ効果が得られます。

非インラインモジュールにもattributeを付与することができ、別ファイルにある内容を展開します。ただし、stableではコンパイルできないので`cfg_attr`を使います。
```rust
// src/a.rs
#[cfg_attr(nightly, codesnip::entry(inline))]
mod a;
// $ cargo codesnip --target=src/a.rs --cfg=nightly bundle a
```

itemに`#[codesnip::skip]`を付与して結果に含めないようにできます。


### Command line arguments
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

#### 入力、パースオプション
コマンドライン引数の指定方法は少し特殊です。サブコマンドより前に入力やパースオプションを指定する必要があります。
```
$ cargo codesnip [入力、パースオプション] サブコマンド [サブコマンドオプション]
```

##### target
ライブラリクレートのルートファイル（src/lib.rsなど）を指定します。

##### use-cache
cacheサブコマンドで生成されるファイルを指定します。

##### cfg
パース途中に`cfg`や`cfg_attr`はこのオプションの内容に従って展開されます。

##### filter-item
このオプションにマッチするattributeのあるitemはパース結果から除外されます。

##### filter-attr
このオプションにマッチするattributeはパース結果から除外されます。

#### サブコマンド
##### cache
解析結果をキャッシュし、次回の入力として使えます。
```
USAGE:
    cargo codesnip cache <FILE>

ARGS:
    <FILE>    Output file
```

##### list
スニペットに含まれるキーをスペース区切りで列挙します。

##### snippet
VSCode向けの静的スニペットを生成します。
```
USAGE:
    cargo codesnip snippet [FLAGS] [FILE]

FLAGS:
        --ignore-include    ignore includes

ARGS:
    <FILE>    Output file, default stdout
```

##### bundle
スニペットをキーで取得します。このとき、スニペットの一部を結果に含めないように指定できます。
```
USAGE:
    cargo codesnip bundle [OPTIONS] <NAME>

OPTIONS:
    -e, --excludes <NAME>...    excludes

ARGS:
    <NAME>    snippet name
```

##### verify
すべてのスニペットが単独でコンパイルできるかをチェックします。

## codesnip-vscode
上記のコマンドを使用し、動的スニペットを実現するVSCode拡張機能です。
入力、パースオプションをsetting.jsonで設定すると、それに従って`Codesnip: Update Cache`コマンドでキャッシュファイルを生成します。
以降は生成されたキャッシュファイルからコマンドを実行します。

`Codesnip: Bundle`コマンド、もしくは`Ctrl+Alt+B`でスニペットピッカーが現れ、そこから目的のスニペットを選択することでスニペットを挿入することができます。
挿入時に`// codesnip-guard: NAME`の形式でコメントが挿入されます。このマーカのあるスニペットは以降挿入されないようになります。

## Tips
最終的にスニペットはバイナリクレートのルートに挿入されることを想定しています。
そのため、いくつか制約をかけて使用するとよいです。

- 各スニペットの部分木は重ならない（異なるスニペットに同じ部分が入っていても検知できないので）
- スニペットのルート間で名前衝突が発生しない
- 依存解決したスニペットはそれ単独でコンパイル可能（verifyサブコマンドでチェックできます）
- 各スニペットのルートには`use`itemを極力置かない
- exportしたマクロやその内部で使われるものはネストしたモジュール内に置かない

## Example
[my library](https://github.com/to-omer/competitive-library/tree/master/crates/competitive)

## License
Dual-licensed under [MIT](https://opensource.org/licenses/MIT) or [Apache-2.0](http://www.apache.org/licenses/LICENSE-2.0).
