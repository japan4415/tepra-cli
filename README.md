# tepra-cli

エージェントが生成したPNGを印刷用に変換し、TEPRA SR-MK2へ送るRust CLIです。

macOS + USB + 24mmテープで校正ラベルを印刷し、自動カットとCLIのジョブ完了を確認しました。USB印刷は試験的な対応のため、`--experimental-usb`を明示します。Bluetoothは機器・GATTの診断までで、印刷はまだ対応していません。

## インストール

Rust stableを用意して、このリポジトリで実行します。開発時のローカル検証はRust 1.98.1です。

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
```

`render`と`--dry-run`は機器へ接続しません。PNGとジョブは同じディレクトリへ保存し、既存ファイルは上書きしません。印刷直前にPNGのハッシュ・寸法・二値化・360dpi情報を検査します。`preview.png`は実際に送るラスタです。透明部分は白（無印字）へ合成します。

初期USB対応は観測済みの24mmカートリッジ状態、1枚、内容長100mm以下に限定しています。256dotのヘッド幅を用い、50mm指定時は709×256画素になります。内容長と、前後の送り・カットを含む実際のラベル全長は異なります。USBの余白パラメータは公式の画像入力APIの既定値1mmに相当する14dotです。初回校正の物理寸法を定規で測定したわけではありません。

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
| 4/6/9/12/18mmテープ、連続複数枚 | 未対応 |

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

実機でラスタ送信中にCtrl-C相当の中断を行うと、CLI終了後も本体が40秒以上busyのまま残ることを確認しています。CLIの中断は本体の印刷取消ではありません。結果不明のジョブを自動再送せず、本体の状態と出力を確認してから復帰させてください。今回の実機試験では、電源OFF/ON後に待機状態へ戻り、次に明示的に送った10mmジョブが正常に印刷・自動カットされました。

`--timeout-seconds`は既定120秒です。USBは接続前検査を含む送信処理全体、WebAPIは送信開始以後に適用します。WebAPIの事前検査には別途1要求5秒の期限があります。

## 開発と資料

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

自動テストは実機へ送信しません。CIはLinux（Ubuntu）のみでビルド、lint、オフラインテストを実行します。

- [印刷仕様・公式API・既存実装の調査](docs/research.md)
- [CLI設計](docs/design.md)
- [USBプロトコルの解析根拠と実機確認](docs/usb-protocol.md)
