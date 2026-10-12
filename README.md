# atlas

Astroの単一ファイルを解析し、import、コンポーネント利用、script内の参照を返すRustライブラリ。依存元は [withastro/oxc@8bb526fc](https://github.com/withastro/oxc/tree/8bb526fc0c20beb4649b223d3ac39851505caa5a)。Rust 1.91.1を使用する。

```sh
cargo test --locked
```

テストは [tests/astro_semantic.rs](tests/astro_semantic.rs)、入力は [tests/fixtures](tests/fixtures)。

## 確認できたこと

- 同じ名前の `Counter` でも、import、map の引数、ブロック内の変数をそれぞれ別の宣言に解決する。
- `<script>`（`is:inline` を含む）は独立したスコープを持ち、フロントマターやテンプレートの変数を参照しない。
- `<UI.Counter>` は、`UI` の宣言にだけ解決する。
- ディレクティブの名前・値・span を利用箇所ごとに取得できる（重複指定も含む）。
- span は元ソースの UTF-8 バイト `[start, end)`。和文・絵文字・CRLF でも正しい。
- 構文エラーは parser の診断になる。宣言のない `<Missing />` は未解決参照として扱われる。

## 未解決の差（ignore）

`<script define:vars={{ title }}>` では、script 本文の `title` がフロントマターの宣言に解決されない。script の未解決参照はグローバルの未解決参照として登録される（`resolve_references_for_astro_script`）。upstream への報告候補。

```sh
cargo test --locked define_vars -- --ignored
```

## 第2段階の解析API

```rust
use atlas::{Resolution, analyze};

let facts = analyze("<Missing client:visible />").unwrap();
let component = &facts.components[0];
assert_eq!(component.name, "Missing");
assert_eq!(component.reference.resolution, Resolution::Unresolved);
assert_eq!(component.directives[0].name, "client:visible");
```

`analyze(&str) -> Result<FileFacts, AnalysisError>` は、OxcのASTとallocatorを関数内で解放し、文字列とバイト位置を所有する結果を返す。入力の文字列を解放した後もFactsを利用できる。

| 返す情報 | 内容 |
| --- | --- |
| `FileFacts.imports` | static importのモジュール指定、default・named・namespaceの区別、別名、型専用指定、宣言span。副作用importも含む。 |
| `FileFacts.components` | テンプレート内の要素、タグ名、開閉タグの名前span、識別子の参照先、ディレクティブ。開閉タグを1利用として数える。 |
| `ComponentUse.reference` | 単純なタグの識別子、またはメンバー形式の先頭の識別子。`Resolved(Binding)` または `Unresolved`。 |
| `Binding` | 宣言名と宣言span。importへの参照にはモジュール指定とimport形式も付く。 |
| `Directive` | コロンを含む属性の名前、span、値の原文と種別。順序と重複を保持する。 |
| `FileFacts.scripts` | 解析されたscript要素のspanと、本文内の識別子参照。入れ子の関数内も含む。 |

すべてのspanは元ソースのUTF-8バイト `[start, end)` であり、配列は原文の順序で返す。参照先は解析中のSymbolIdで決定する。返す結果にOxcの型やIDは含めず、同じ解析結果内では宣言spanで宣言を区別する。

構文診断がある場合は `Err(AnalysisError)` を返す。parserの診断があれば意味解析を開始せず、semanticの診断があればFactsを作成しない。診断は段階、メッセージ、ラベルのspanを所有する。構文が妥当な未解決参照は `Ok(FileFacts)` の中で表現する。

第2段階で検証する範囲は次のとおり。

- コンポーネント利用は識別子を先頭に持つテンプレートの要素。HTMLタグ、コメント、文字列、フロントマターとscript本文のJSXは含めない。`this` を先頭に持つタグは対象外。
- `<UI.Forms.Counter>` はUIの宣言までを返す。プロパティの値、import先ファイル、実行時のコンポーネントは評価しない。
- ディレクティブは構文上のコロン属性として返す。独自ディレクティブの有効性は判定せず、spread属性の実行時の値も展開しない。
- `is:inline` はJSとして解析し、JS以外の `type` を持つscriptの本文はraw textとして扱う。`define:vars` は前述した固定先Oxcの制限を引き継ぐ。
- 入力の原文全体、全スコープ、全宣言、テンプレートのタグ以外の参照はFactsに含めない。CLI、Salsa、ファイル横断解析は今回の対象外。

APIのテストは [tests/file_facts.rs](tests/file_facts.rs)。元ソースの解放、同名宣言の区別、型専用import、scriptの分離、重複ディレクティブ、和文・絵文字・CRLF、parserとsemanticの診断を検査する。

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo test --locked --doc
cargo clippy --locked --all-targets -- -D warnings
```

実測はAPIテスト13件成功、第1段階のテスト11件成功・1件ignore、ドキュメントテスト1件成功。fmtとClippyも成功した。
