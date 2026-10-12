# atlas

OxcのAstro意味解析を検証する第1段階の調査用crate。参照先の宣言、scriptのスコープ、ASTの属性と位置を、固定した依存とfixtureで検査する。

2026-10-12、macOS / aarch64で実行し、統合テスト12件中11件が成功、1件をignoreとした。ignoreは `define:vars` 付きscriptでフロントマターの変数が未解決になる、固定先Oxcとの差による。

## 環境と依存

- Rust 1.91.1、edition 2024。`rust-toolchain.toml` でtoolchainを固定する。
- 単一crate、`publish = false`。`src/lib.rs` に公開関数・型はない。
- `oxc_allocator`、`oxc_ast`、`oxc_ast_visit`、`oxc_parser`、`oxc_semantic`、`oxc_span` はすべてテスト用依存。
- 依存元は [withastro/oxc のコミット 8bb526fc0c20beb4649b223d3ac39851505caa5a](https://github.com/withastro/oxc/tree/8bb526fc0c20beb4649b223d3ac39851505caa5a)。このコミットのcrateバージョンは0.115.0。
- parserの `regular_expression` とsemanticの `astro` を有効にする。
- `Cargo.lock` をリポジトリに含め、間接依存も固定する。初回実行にはGitHubとcrates.ioへのアクセスが必要。

## 実行方法と結果

rustupが利用できる環境で、リポジトリのルートから各コマンドを実行する。

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

| コマンド | 実測結果 |
| --- | --- |
| `cargo fmt --all -- --check` | 成功、終了コード0 |
| `cargo test --locked --all-targets` | 統合テスト11件成功、0件失敗、1件ignore、終了コード0 |
| `cargo clippy --locked --all-targets -- -D warnings` | 成功、終了コード0 |

第1段階の成果は、以下の全ケースの再現入力、判定、制限の記録である。

個別の再現には、次の `FILTER` を表のフィルターに変更する。各フィルターは1件のテストを選択する。

```sh
cargo test --locked --test astro_semantic FILTER -- --nocapture
```

## ケース別の期待と実測

テスト本体は [tests/astro_semantic.rs](tests/astro_semantic.rs)、入力は [tests/fixtures](tests/fixtures) にある。

| フィルター | fixture | 期待 | 実測・判定 |
| --- | --- | --- | --- |
| `counter_import_parameter` | `counter.astro` | 外のCounterタグはimport、map内のタグは引数を参照。script内のCounterと `Astro` は未解決。importの参照は最初のタグの1件だけ。`client:visible` は最初の要素だけに付く。 | 期待どおり。成功。 |
| `imports_block` | `shadowing.astro` | default import、別名付きnamed import、ブロックの変数、入れ子の引数を区別。内のmapを抜けたタグは外の引数を参照。 | 5宣言のSymbolIdを区別し、6要素とブロック内の参照を確認。成功。 |
| `member_tag` | `member.astro` | `<UI.Counter>` のUIはnamespace importを参照。Counterはプロパティ名。 | 開閉タグのUIが同じ宣言を参照。Counterの位置にIdentifierReferenceはない。成功。 |
| `scripts_resolve` | `scripts.astro` | 同じscript内の宣言を参照。フロントマター、テンプレート引数、別scriptからの参照は未解決。 | 2つのscriptの同名変数を区別。入れ子の関数からの参照、scriptから外への参照、テンプレートからscript内宣言への参照も期待どおり。成功。 |
| `inline_script` | `inline.astro` | `is:inline` もブラウザーで動くJS。AstroScriptとして解析し、script内の宣言を独自スコープで解決する。 | 期待どおり。成功。 |
| `define_vars` | `define-vars.astro` | `define:vars={{ title }}` のscript本文から、フロントマターの `title` を参照できる。 | 属性内の `title` は解決するが、本文の `title` は未解決。ignore。詳細は後述。 |
| `non_javascript_script` | `raw-script.astro` | `type="application/json"` の本文はraw text。本文のJSとして不正な記述やタグ風の文字列を解析しない。 | Textとして原文を復元し、宣言・参照・コンポーネント利用がないことを確認。成功。 |
| `uses_tags` | `usage.astro` | 同じコンポーネントの複数利用、開閉タグ、複数・重複・独自ディレクティブを区別。コメント・文字列・HTMLタグを利用に数えない。 | 3要素、4個のタグ名参照。重複した `client:visible` も別span。`client:idle`、`client:only`、`custom:flag` と値を確認。成功。 |
| `spans_are` | `positions.astro` | 和文・絵文字・CRLFを含む原文から、名前・属性名・属性値・要素全体をUTF-8の半開区間で復元。 | `[start, end)` のバイト位置で一致。import宣言と開閉タグの参照先も一致。成功。 |
| `empty_file` | `empty.astro`、`no-frontmatter.astro` | 空ファイルとフロントマターのないテンプレートも意味解析できる。 | 空の合成フロントマターとスコープを確認。構文診断・未解決参照は0件。成功。 |
| `malformed_input` | `invalid-frontmatter.astro`、`invalid-template.astro`、`invalid-script.astro` | 不完全な入力に構文診断があり、エラーのあるASTに意味解析を実行しない。 | すべて `Unexpected token`。意味解析を省略し、SymbolIdとReferenceIdが未設定のままであることを確認。成功。 |
| `valid_undefined` | `undefined.astro` | 妥当な構文の `<Missing />` は構文エラーではなく未解決参照。 | parser・semanticの構文診断は0件。ReferenceIdが存在し、SymbolIdはNone、未解決参照一覧に含まれる。成功。 |

[counter.astro](tests/fixtures/counter.astro) は設計案の入力例と同じ内容。

不完全な入力の実測診断位置も原文のバイト位置である。

| 入力 | `panicked` | 診断span | 診断 |
| --- | --- | --- | --- |
| フロントマター | false | `[19, 20)` | `Unexpected token` |
| テンプレート | true | `[20, 21)` | `Unexpected token` |
| script | false | `[23, 24)` | `Unexpected token` |

`panicked` だけでは構文エラーを判定できない。テストの共通ヘルパーは `panicked == false` かつ `errors.is_empty()` の場合だけ意味解析を開始する。

## define:varsの差

```sh
cargo test --locked --test astro_semantic define_vars -- --ignored --nocapture
```

```astro
---
const title = 'hello';
---
<script define:vars={{ title }}>
  console.log(title);
</script>
```

Astroは `define:vars` の値をscriptへ注入するため、本文の `title` はフロントマターの宣言を指す。固定先の `build_astro` は、scriptの未解決参照をフロントマターではなくグローバル側へ送る（`resolve_references_for_astro_script`）。そのため本文の参照の `SymbolId` は `None` になる。`define:vars` を考慮しない分離として、upstreamへの報告候補とする。

## 検証方法と制限

`SourceType::astro()` と `Parser::parse_astro()` を使用する。scriptの解析は [parse_astroの実装](https://github.com/withastro/oxc/blob/8bb526fc0c20beb4649b223d3ac39851505caa5a/crates/oxc_parser/src/astro/parse.rs) に含まれているため、追加で呼び出さない。parserの診断が空なら `SemanticBuilder::new().with_check_syntax_error(true).build_astro(...)` を実行し、semanticの構文診断も空であることを確認する。

参照の `ReferenceId` から `SymbolId` を取得し、ASTの `BindingIdentifier.symbol_id` と比較する。宣言span、宣言ノード、宣言から参照への逆引きも確認する。fixtureの文字列検索はASTノードの位置指定に限定し、同名の宣言を文字列で解決しない。IDの数値を固定したassertはない。

ソース、allocator、AST、Semanticは各テスト内で寿命を管理する。共通ヘルパーとAST visitorは統合テスト専用である。`positions.astro` はCRLFで保存し、`.gitattributes` でGitの改行変換を無効にしている。

この調査で確認した範囲には次の制限がある。

- コンポーネント利用はfixture内のJSXElement単位で数える。開閉タグは別々の参照でも1要素である。
- `<UI.Counter>` はUIの宣言までを判定する。Counterのプロパティ値、実行時のコンポーネント、import先ファイルの存在やexportは評価しない。
- ディレクティブはAST上の名前・順序・値・spanを確認する。重複や独自ディレクティブがAstroコンパイラーで有効かどうかは判定しない。
- scriptの結果はこのOxcコミットの参照解決についてのもの。Astroのブラウザー実行やバンドルの挙動は検証していない。
- コメントと文字列の除外はASTで検査する。`build_astro` にはフロントマターのコメントだけを渡しており、全コメントの集約、JSDoc、lint無効化指定は検証対象外。
- 公開API、独自Facts、CLI、Salsa、ファイル横断解析は追加していない。依存先のソースも変更していない。
