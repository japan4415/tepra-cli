# 配布とリリース

`v0.1.0`のようなタグをpushすると、GitHub Actionsが5環境でテスト・ビルドし、配布ファイルを揃えてからGitHub Releaseを公開する。
公開されたバイナリはaquaでインストールできる。バージョンはCargo.tomlとCargo.lockの両方に一致し、タグのコミットはmainへ取り込み済みでなければならない。

## 配布対象

| OS / CPU | Rust target | ビルド環境 | 配布形式 |
| --- | --- | --- | --- |
| macOS Apple Silicon | aarch64-apple-darwin | macos-15 | tar.gz |
| macOS Intel | x86_64-apple-darwin | macos-15-intel | tar.gz |
| Linux x64 | x86_64-unknown-linux-gnu | ubuntu-22.04 | tar.gz |
| Linux ARM64 | aarch64-unknown-linux-gnu | ubuntu-22.04-arm | tar.gz |
| Windows x64 | x86_64-pc-windows-msvc | windows-2022 | zip |

すべてネイティブのランナーでテストする。Linuxはglibc 2.35以降とD-Bus共有ライブラリを使い、musl/Alpine向けではない。macOSのdeployment targetは13.0。WindowsはMicrosoft Cランタイムを静的リンクする。AppleのDeveloper ID署名・公証、Windows Authenticode署名は行わない。GitHub Artifact AttestationsはそれらのOS署名とは異なる、ビルド元を検証する仕組みである。

各環境の起動確認は`--version`と`--help`、機能確認はオフラインテストで行う。プリンターへは接続しない。実機の確認範囲は[USB検証記録](usb-validation.md)を参照する。

ファイル名は`tepra_0.1.0_darwin_arm64.tar.gz`など、`tepra_<version>_<GOOS>_<GOARCH>.<format>`。アーカイブには`tepra`（Windowsでは`tepra.exe`）、READMEとdocsのMarkdownを含む。

## リリース手順

1. Cargo.tomlのversionを更新し、`cargo check --locked`で整合性を確認する。バージョン更新でlock更新が必要なら`cargo check`でCargo.lockも更新する。
2. PRを作成し、Rust CIとRelease CIを通してmainへマージする。
3. 最新mainのコミットにタグを付けてpushする。RCは`v0.2.0-rc.1`のように指定する。

```sh
git fetch origin
git tag -a v0.2.0 origin/main -m 'Release v0.2.0'
git push origin v0.2.0
```

GitHub画面で先に「Publish release」を押す必要はない。タグのpushがリリースの開始操作になる。

`Release`ワークフローは次の順で実行する。

1. タグ形式・Cargoのバージョン・mainへの取り込みを確認。
2. Rust 1.98.1とCargo.lockを使い、5環境でテストとreleaseビルド。
3. 配布アーカイブを作成し、展開したバイナリの起動とバージョンを確認。
4. 5ファイルがすべて存在することを確認して`SHA256SUMS`を生成。
5. アーカイブとチェックサムにGitHub Artifact Attestationsを発行。
6. ドラフトReleaseへアップロードし、全アップロードが成功した後に公開。RCはprereleaseにする。
7. 5環境でaquaから公開済みバイナリをインストールし、起動を確認。

ビルドジョブは読み取り権限だけを持ち、Release書き込み・OIDC・attestation発行は最後の公開ジョブに限定する。外部ActionsはコミットSHA固定。releaseビルドではPR由来のビルドキャッシュを再利用しない。アーカイブのタイムスタンプ等は固定するが、コンパイラ・OSイメージを含む完全な再現可能ビルドを保証するものではない。

公開済みReleaseを上書きする処理は持たない。公開前の失敗は同じActions runの「Re-run failed jobs」で再実行でき、ドラフト内の不完全なアップロードは再作成する。公開後にaquaの確認だけ失敗した場合も失敗ジョブだけを再実行する。公開済みバイナリの修正には新しいバージョンを使う。手動で再起動するときは`gh workflow run release.yml --ref v0.2.0`のように既存タグを指定する（mainでの起動は拒否する）。

## aquaの設定

同梱の[設定例](../examples/aqua/aqua.yaml)と[Gitルートのポリシー](../aqua-policy.yaml)を使う。既存プロジェクトに導入する場合は、それぞれの内容を自分の設定ファイルへ統合する。[コピー用のポリシー例](../examples/aqua/aqua-policy.yaml)も同梱するが、ネストしたディレクトリに置いただけでは自動検出されない。

```yaml
checksum:
  enabled: true
  require_checksum: true
registries:
  - name: tepra
    type: github_content
    repo_owner: japan4415
    repo_name: tepra-cli
    ref: v0.1.0
    path: aqua-registry.yaml
packages:
  - name: japan4415/tepra-cli@v0.1.0
    registry: tepra
```

ポリシーは同梱の例を確認してGitリポジトリのルートへ`aqua-policy.yaml`として配置し、`aqua policy allow`で許可する。aquaはGitルート（またはその直下の`aqua/`・`.aqua/`）だけでポリシーを探す。ファイル名を指定してallowしても、ネストした場所のファイルが自動的に検出対象へ追加されるわけではない。

最初に`aqua -c examples/aqua/aqua.yaml update-checksum`を実行し、その後`aqua -c examples/aqua/aqua.yaml install`する。aqua v2の`require_checksum: true`は、配布元のチェックサムの有無にかかわらず、ローカルの`aqua-checksums.json`に固定値が必要という意味である。`update-checksum`は署名付きのSHA256SUMSを検証して固定値を作る。以後はこのファイルをGit管理し、バージョン更新時に更新する。
レジストリの参照先とパッケージのバージョンはどちらも固定し、更新時は両方を更新する。ポリシーはこのリポジトリの`aqua-registry.yaml`と`japan4415/tepra-cli`だけを許可し、無関係なレジストリを包括的に許可しない。

aqua 2.65以降を使う。インストール時にSHA-256を照合し、配布物とチェックサムの両方についてGitHub Actionsの`release.yml`が署名したビルド証明を検証する。aquaは検証用のGitHub CLIを自動導入する。GitHub APIの未認証アクセスはレート制限に当たりやすいため、GitHub CLIの認証またはaquaがサポートする`AQUA_GITHUB_TOKEN` / `GITHUB_TOKEN`を用意する。トークンは設定ファイルへ書かない。

`aqua-checksums.json`は利用側で生成し、レビューしてGit管理する。このリポジトリにもv0.1.0の5環境分を記録する。チェックサムやattestation検証の無効化を解決策にしない。公開直後にAPIの反映待ちで証明が見つからない場合は少し待って再実行する。

公開済みバージョンの導入だけを再検証するには`gh workflow run verify-install.yml --ref main -f tag=v0.1.0`を使う。リリース時と同じ5環境で検証し、バイナリの再公開は行わない。PRでもv0.1.0を使った導入検証を実行するため、ポリシーやチェックサムの初期設定の不備を公開前に検知できる。

GitHub CLIで証明を手動確認する例:

```sh
gh release download v0.1.0 -R japan4415/tepra-cli -p 'tepra_0.1.0_darwin_arm64.tar.gz'
gh attestation verify tepra_0.1.0_darwin_arm64.tar.gz \
  --repo japan4415/tepra-cli \
  --signer-workflow japan4415/tepra-cli/.github/workflows/release.yml \
  --deny-self-hosted-runners
```

## 調査に基づく選択

- [aquaのレジストリ開発ガイド](https://aquaproj.github.io/docs/develop-registry/)は公開ツールの標準レジストリ登録を推奨する。標準レジストリへの採用は外部レビューに依存するため、初期導入はこのリポジトリのレジストリで完結させる。標準レジストリには未登録。
- [aquaのチェックサム](https://aquaproj.github.io/docs/reference/security/checksum/)と[ビルド証明](https://aquaproj.github.io/docs/reference/registry-config/github-artifact-attestations/)を併用する。配布元のチェックサム、ローカルの固定チェックサム、署名したワークフローを確認する。
- [GitHubのRelease管理](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository)に従い、全資産をドラフトへ揃えてから公開する。Immutable releasesを有効にしても公開後に資産を追加しない構成。
- [GitHubのattestation公式手順](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)に従いOIDCで署名し、長期の署名鍵をSecretsへ置かない。
- [公式ランナー一覧](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)から5種類を選ぶ。既存Rust CLIの小規模な配布なので、OS別ビルド・パッケージングを共有ワークフローにまとめ、追加のリリースフレームワークは導入しない。
