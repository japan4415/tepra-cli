# リポジトリの保護設定

`main` は PR 経由で更新する。全ファイルの CODEOWNER は `japan4415`。
GitHub の Ruleset を次の2つに分け、承認だけを管理者がバイパスできるようにする。
JSON は設定内容の保存用であり、コミットしただけでは GitHub に反映されない。

| Ruleset | 内容 | バイパス |
| --- | --- | --- |
| `main-baseline` | PR必須、未解決レビュースレッドの解消、Rust CI成功、最新mainへの追従、force push・削除禁止 | 誰にも許可しない |
| `main-owner-review` | Approve 1件、CODEOWNERの承認、差分更新時の承認失効 | Repository adminのみ、PR経由に限定 |

CI の必須チェックは `Rust (ubuntu-latest)`、発行元は GitHub Actions（integration ID `15368`）。
管理者ロールの ID は `5`。導入時の管理者は `japan4415` のみ。
今後ほかの管理者を追加すると、その人にも承認のバイパス権限が付く。

## 日常の操作

- 自分の PR: CI の成功と最新 main への追従を確認し、GitHub のマージ欄で承認ルールのバイパスを選択してマージする。専用 App やワークフローは不要。
- 他の人や Bot の PR: `japan4415` がレビューして Approve を残してからマージする。
- レビュー後に差分が更新されたら再レビューする。
- `Require approval of the most recent reviewable push` は無効。所有者が他人の PR を修正した後でも自身で承認できるよう、承認の失効で追加差分を扱う。

管理者は他の人の PR でも承認をバイパスできる。そのため「他の人の PR には必ず自分の Approve を残す」は運用規則であり、GitHub が作成者別に強制する条件ではない。
CI 失敗・追従不足・未解決スレッドは、承認ルールのバイパスでは回避できない。
設定自体を変更できる管理者権限は残る。

## 設定の確認と変更

設定画面: https://github.com/japan4415/tepra-cli/settings/rules

保存用定義:

- `.github/rulesets/main-baseline.json`
- `.github/rulesets/main-owner-review.json`

`gh api repos/japan4415/tepra-cli/rulesets` で既存 ID を調べ、各 ID の詳細を取得して比較する。
既存設定を変更するときは該当 ID に PUT する。重複する Ruleset を新規作成しない。
`gh api repos/japan4415/tepra-cli/rules/branches/main` で有効なルールを確認できる。

CODEOWNERS はマージ先ブランチの定義が使われるため、`main` に存在することを確認する。
CI のジョブ名を変更するときは、必須チェック名も合わせて更新する。

## 検証項目

- 共通ルールのバイパス一覧が空で、承認ルールだけが管理者の PR バイパスを許可している。
- `main` の CODEOWNERS に構文エラーがなく、所有者が `japan4415` になっている。
- 自分の PR を Approve なし・CI成功で管理者バイパスによりマージできる。
- 他の人の PR には CODEOWNER の承認条件が表示される。
- 実際の失敗CI・追加push・未解決スレッドによるブロックは、必要に応じて専用の検証PRで確認する。

参考: [Rulesetのバイパス設定](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository)、[利用可能なルール](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets)、[CODEOWNERS](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners)。
