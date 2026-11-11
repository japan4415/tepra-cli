# tepra-cli

エージェントが生成したPNGを印刷用に変換し、TEPRA SR-MK2へ送るRust CLIです。

印刷向け画像の生成には、同梱の[画像プロンプト設計スキル](.agents/skills/image-prompt-design/SKILL.md)を使えます。白黒ラベルの描画サイズ、線幅、余白、文字、二値化後の確認に絞ったガイドです。

macOS + USB + 24mmテープで校正ラベルを印刷し、自動カットとCLIのジョブ完了を確認しました。USB印刷は試験的な対応のため、`--experimental-usb`を明示します。Bluetoothは機器・GATTの診断までで、印刷はまだ対応していません。

## インストール

### aquaで導入

[GitHub Releases](https://github.com/japan4415/tepra-cli/releases)に、macOS（Intel/Apple Silicon）、Linux（x64/ARM64）、Windows（x64）のビルド済みバイナリを配布します。Rustのインストールは不要です。aqua 2.65以降を用意し、次のように導入できます。

```sh
git clone --branch v0.1.0 --depth 1 https://github.com/japan4415/tepra-cli.git
cd tepra-cli
# v0.1.0の設定例をaquaが自動検出するGitルートへ配置
cp examples/aqua/aqua-policy.yaml aqua-policy.yaml
aqua policy allow
aqua -c examples/aqua/aqua.yaml update-checksum
aqua -c examples/aqua/aqua.yaml install
aqua -c examples/aqua/aqua.yaml exec -- tepra --version
```

標準レジストリには未登録のため、このリポジトリのレジストリを明示的に許可します。設定は`japan4415/tepra-cli`だけを許可し、バージョン固定・SHA-256・GitHub Artifact Attestationsによる配布物の検証を有効にしています。`require_checksum: true`のため、初回は`update-checksum`で署名付きチェックサムから固定値を作成してからインストールします。生成される`aqua-checksums.json`は利用側のリポジトリへコミットしてください。既存のaqua設定への追加方法とGitHub認証については[配布とリリース](docs/releasing.md)を参照してください。

Linuxの配布バイナリはglibc 2.35以降と`libdbus-1.so.3`が必要です（Ubuntuでは`sudo apt-get install libdbus-1-3`）。macOSは13以降を対象とします。Windows ARM64・Alpine Linux向けバイナリは配布していません。バイナリのビルド・起動確認と、プリンター実機の対応確認は別です。

### ソースから導入

rustupを用意して、このリポジトリで実行します。`rust-toolchain.toml`でRust 1.98.1を固定しています。

```sh
cargo install --path . --locked
tepra --help
```

LinuxのビルドにはD-Bus開発パッケージとpkg-configが必要です。USBの利用権限や他ドライバーとの競合は環境に依存します。macOSのBluetooth診断には実行するターミナルやエージェントホストのBluetooth権限が必要です。

## このMacからUSB印刷する

本体の電源を入れ、USB接続と24mmテープを準備します。

```sh
tepra devices --transport usb --json
tepra doctor --device usb:00:1 --json
tepra status --device usb:00:1 --json
```

`usb:00:1`は今回の実機で返された例です。再接続等で変わるため、通常は`devices`の`result.devices[].preferred_selector`を使ってください。シリアル番号が取得でき、一意なら`usb:serial:<VID>:<PID>:<シリアルのUTF-8を16進符号化した値>`になります。同じVID/PID/シリアルの機器が複数あると選択を拒否します。シリアルがない場合、`preferred_selector`は従来のbus/address形式です。

SR-MK2を1台だけ接続する運用では、`--device usb:sr-mk2`も使えます。対象VID/PIDの機器が0台または複数ならエラーになります。これは接続中の機種を選ぶ指定で、同じ物理個体を保証するものではありません。`doctor`・`status`・USBの`print`で同じ指定を使えます。

`doctor`は記述子を読み、`status`は本機から22バイトの状態応答を取得します。`status.summary`に`activity`（idle/busy/waiting/unknown）、エラーの有無、観測済みカートリッジとの一致を表示します。`experimental_usb_preflight_matches`は試験的印刷の事前条件との一致であり、印刷成功の保証ではありません。未知のエラー原因やテープ品種は推測せず、生値を残します。`ready`と`printing_completed`は引き続き`null`です。

エージェントの画像生成機能で作ったPNGを`label.png`として保存し、印刷用のプレビューとジョブへ変換します。

```sh
tepra render label.png --model sr-mk2 --tape-width 24 \
  --content-length-mm 50 --fit contain --dither threshold \
  --output preview.png --job job.json --json

tepra print --job job.json --device usb:00:1 \
  --experimental-usb --dry-run --json

tepra print --job job.json --device usb:00:1 \
  --experimental-usb --json

# 同じラベルを2枚、1つのジョブとして連続印刷
tepra print --job job.json --device usb:sr-mk2 \
  --experimental-usb --copies 2 --json
```

`render`と`--dry-run`は機器へ接続しません。PNGとジョブは同じディレクトリへ保存し、既存ファイルは上書きしません。印刷直前にPNGのハッシュ・寸法・二値化・360dpi情報を検査します。`preview.png`は実際に送るラスタです。透明部分は白（無印字）へ合成します。

初期USB対応は観測済みの24mmカートリッジ状態、1〜20枚、1枚あたり追加余白を含むラスタ幅1417dot（100mm相当）以下に限定しています。256dotのヘッド幅を用い、余白指定なしの50mm指定時は709×256画素になります。内容長と、前後の送り・カットを含む実際のラベル全長は異なります。USBの余白パラメータは公式の画像入力APIの既定値1mmに相当する14dotです。初回校正の物理寸法を定規で測定したわけではありません。

`--copies N`（既定1）はUSBで同じジョブのラベルを連続送信します。ジョブ開始と終了を1回にまとめ、各ラベルのページ設定・ラスタ・改ページを繰り返します。カット設定は従来と同じ自動カットです。[公式説明](https://api.hello-tepra.jp/articles/145)では連続印刷により前カットを最初の1回に減らせます。短い余白のまま前カットを完全にゼロにする設定ではありません。20枚はCLIの初期上限で、本体の最大枚数を示しません。実機確認の範囲は[USB実機検証](docs/usb-validation.md)を参照してください。

`--timeout-seconds`は全枚数を通した期限です。長いラベルを多数印刷するときは必要に応じて延ばしてください。途中で切断・エラー・中断があっても自動再送しません。結果の`copies_requested`は指定枚数、`copies_submitted`はラスタと改ページの送信を終えた枚数であり、物理的に印字・カットできた枚数の保証ではありません。失敗時は`current_copy`も記録します。排他ロックは一連の印刷中保持し、1つの試行記録へ保存します。ジョブIDは画像と変換設定に基づくため枚数で変わらず、印刷条件は試行記録の`print_options`に残ります。Windows WebAPIでの複数枚指定は引き続き未対応です。

この環境では左側に本体由来の約1mmの隙間が生じるとユーザーが確認しています。左右のバランスを取る場合は、`render`に`--padding-right-mm 1`を加えてください。配置・二値化の後に右端へ14dotの白い領域を追加し、絵柄のサイズや画素を維持します。`--content-length-mm 50 --padding-right-mm 1`なら723×256画素です。余白の既定値は0で、保存済みジョブも従来どおり読めます。指定は有限の非負値とし、内容長と余白の合計は1000mm以下、USB印刷では上記のラスタ幅上限に収めます。内容長と余白はそれぞれドットへ丸めます。プレビューは右余白を含み、本体側の送り余白は含みません。

`--fit cover`は中央を切り抜いて全面へ配置します。写真等の階調には`--dither floyd-steinberg`を指定できます。文字やQRの生成はこのCLIに内蔵していません。画像生成サービスへの認証や課金もエージェント側で扱います。

再現用の校正画像をRustだけで作れます。

```sh
cargo run --example calibration -- calibration.png
# 1024行をまたぐ75mmの校正画像（既定は50mm）
cargo run --example calibration -- calibration-75.png 75
```

校正画像の指定長と`render --content-length-mm`は同じにしてください。横軸の目盛りは5mm間隔です。実寸・余白・異常系の確認方法は[USB実機検証](docs/usb-validation.md)にまとめています。

## 機能と検証範囲

| 機能 | 実装・検証 |
| --- | --- |
| PNG変換、プレビュー、ジョブ検証 | 実装済み。オフラインテストあり |
| USB列挙、記述子、状態取得 | macOS + SR-MK2実機で確認 |
| USB印刷、自動カット、完了検出 | 上記24mm条件で実機確認。`--experimental-usb`が必要 |
| BLE列挙・GATT診断 | 実装済み。Bluetooth実機検証は未実施 |
| BLE印刷 | 未実装 |
| Windows公式WebAPI | 実装済み。HTTPモックテスト済み、Windows実機は未検証 |
| Linux USB印刷 | コード経路あり、Linux実機は未検証 |
| USB連続複数枚 | 1〜20枚を1ジョブで送信。10mm＋右余白1mmの2枚で、小片1回・各ラベルのカットを実機確認 |
| 4/6/9/12/18mmテープ | 未対応 |

## Windows公式WebAPI

Windowsに公式のテプラクリエイターとWebAPI用通信モジュールを導入して使う別経路です。CLIはlocalhost:29108へ接続します。公式モジュールは同梱しません。

```sh
tepra devices --backend windows-webapi --json
tepra status --backend windows-webapi --device "KING JIM SR-MK2" --json
tepra print --backend windows-webapi --device "KING JIM SR-MK2" --job job.json --json
```

`--device`には実際に列挙された`printerName`を指定します。この経路でも現時点では通常24mmテープ・1枚を検査します。`--experimental-usb`とは併用しません。ドライバーの伸縮や余白を含め、Windowsでの物理出力は今後の確認が必要です。

## エージェント向けの契約

`--json`のstdoutは単一JSONです。成功時は`{"schema_version":1,"ok":true,"result":...}`、失敗時は`ok:false`と`error.code`、`error.message`、`error.retryable`を返します。ヘルプとバージョン表示は通常のテキストです。`--json`を付けない場合、成功結果は読みやすいJSON、エラーはstderrに表示します。

| 終了コード | 意味 |
| --- | --- |
| 0 | コマンド成功。印刷ではbackendの正常なジョブ終了を確認 |
| 2 | 入力・画像・ジョブ不正 |
| 3 | 機器なし・識別不一致 |
| 4 | 接続・権限・排他・記録ファイル等の問題 |
| 5 | 確認できた機器エラー・中止・テープ不一致 |
| 6 | 未対応機能/機器状態・不正な応答 |
| 7 | 送信後の結果不明。自動再印刷しない |

USBは送信後の`ST=1/2`から`ST=0, ER=0`への変化、WebAPIは正常な`jobEnd`を完了の根拠にします。全バイト送信や進捗100%だけでは成功にしません。印字品質をセンサーで検査しているわけではありません。

USBの送信前の中断は終了コード4、`state: "not_submitted"`になります。最初のUSB転送を投入した後のタイムアウト・切断・Ctrl-Cは終了コード7、`state: "unknown"`です。機器から明確なエラーを受けた場合は終了コード5ですが、送信後なら一部が印字されている可能性があります。送信後の失敗では自動再送しません。ジョブIDは同じ内容を識別するための値で、再度`print`を明示すればもう1枚印刷します。exactly-onceを保証するものではありません。

失敗JSONと最終試行記録には同じ`job_id`・`attempt_id`・`device_id`を含めます。USBの`error.context.backend_context`には`phase`、`transmission_started`、直近の生ステータス、ST遷移、解決した`resolved_device_id`が残ります。これらは診断用で、シリアル番号自体をハードウェア認証するものではありません。

同じユーザー・状態ディレクトリを使うCLIの印刷をファイルロックで直列化します。公式アプリの利用はこのロックの対象外なので、同時印刷は避けてください。試行記録はmacOS/Linuxで`$XDG_STATE_HOME/tepra-cli`または`~/.local/state/tepra-cli`、Windowsで`%LOCALAPPDATA%/tepra-cli`にJSONLとして残します。生画像は保存しません。異常終了時に最終結果が記録されていなければ、結果不明として扱います。

実機でラスタ送信中にCtrl-C相当の中断を行うと、CLI終了後も本体が40秒以上busyのまま残ることを確認しています。CLIの中断は本体の印刷取消ではありません。結果不明のジョブを自動再送せず、本体の状態と出力を確認してから復帰させてください。今回の実機試験では、電源OFF/ON後に待機状態へ戻り、次に明示的に送った10mmジョブが正常に印刷・自動カットされました。印刷中に電源ボタンでOFFにした試行では、通信が切れる前に本体が`power_off_cancel`（ER=35）を返し、CLIは終了コード5で停止しました。この場合も部分印字があり、自動再送はしません。

`--timeout-seconds`は既定120秒です。USBは接続前検査を含む送信処理全体、WebAPIは送信開始以後に適用します。WebAPIの事前検査には別途1要求5秒の期限があります。

## 開発と資料

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

自動テストは実機へ送信しません。Rust CIはLinuxでformat・lint・オフラインテスト、Release CIは配布対象の5環境でテスト・releaseビルド・アーカイブ展開後の起動を検証します。リリースは`v*`タグのpushから自動実行します。

- [印刷仕様・公式API・既存実装の調査](docs/research.md)
- [CLI設計](docs/design.md)
- [USBプロトコルの解析根拠と実機確認](docs/usb-protocol.md)
- [配布とリリース](docs/releasing.md)

## ライセンス

MIT License。詳細は[LICENSE](LICENSE)を参照してください。
