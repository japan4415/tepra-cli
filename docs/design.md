# SR-MK2向けRust CLI設計案

作成日: 2026-11-10。調査・設計段階であり、以下のコマンドは**未実装**。根拠と未確定事項は[調査資料](research.md)を参照。

## 1. 到達点と初期方針

エージェントが生成した画像をファイルとして受け取り、SR-MK2のテープ幅に合わせて変換し、USBまたはBluetoothで印刷する。RustのCLIとライブラリとして実装する。

初期環境は現在の作業環境に合わせてmacOSを暫定第一候補とする。OSの最終選択は未確定。Linux・Windowsへ移植可能な構造にするが、ライブラリが対応するだけで本機での動作を保証しない。

Windowsを使える場合は公式WebAPIを第一候補にする。通信モジュールと公式ドライバを使い、独自のUSB/BLEコマンド実装を省ける見込みがある。macOSにWindowsのlocalhost WebAPIが存在するとは仮定しない。Windows補助ホストを使う場合はそのホスト上でCLIを実行する構成とし、HTTPポートをそのままLANへ公開しない。

最初の技術検証ではUSB descriptorとBLE GATTを両方調べる。USBで公式ソフトのジョブを観測・再現でき、macOSからinterfaceを利用できる場合はUSBを最初の印刷経路にする。USBがOSドライバーに占有される等で難しい場合はBLEを先行する。通信の選定を実機調査前に固定しない。

画像生成は既存エージェントの画像生成機能を使い、CLIはローカルPNGの受付から担当する。これにより生成サービス、認証、課金、モデルの選択をCLIに固定せずに済む。将来、必要になれば独立した生成サブコマンドを追加できる。

## 2. 最小機能

- SR-MK2の列挙・識別と接続診断。
- PNGの入力、指定サイズへの配置、単色化、印刷前プレビュー。
- 検証済みテープ幅・接続方法で1枚印刷し、実際の完了状態を報告。
- エージェント向けJSON出力、一定の終了コード、同時印刷の排他。
- 通信ログと画像・設定の保存による再現可能な不具合調査。

初期版の対象外は、GUI、常駐サーバー、クラウド画像生成APIの内蔵、複数機種への自動対応、未検証のカット動作。SVGとJPEGはPNGの経路を確認した後に追加する。MCPサーバーはCLIが安定してから薄いラッパーとして追加可能。

## 3. コマンド案

すべて設計上の使用例。`<device-id>`は`devices`が返す識別子に置換する。

```sh
# 標準のデバイス記述子・広告・GATT構造を調査。印刷命令は送らない。
tepra devices --transport usb --json
tepra devices --transport ble --json
tepra doctor --device <device-id> --json

# 実機なしでプレビューを作る。24mmの最大ヘッド幅だけが公式確認済み。
tepra render label.png --model sr-mk2 --tape-width 24 \
  --content-length-mm 50 --fit contain --dither threshold \
  --output preview.png --job job.json

# 検証済みのコマンドだけで状態を取得する。
tepra status --device <device-id> --json

# 保存したラスタと条件を検証し、接続して印刷する。
tepra print --job job.json --device <device-id> --copies 1 --json

# 接続も送信もせず、保存済みジョブの整合性を確認する。
tepra print --job job.json --device <device-id> --dry-run --json
```

`--content-length-mm`は描画領域の送り方向の長さであり、カット後の総ラベル長ではない。機械的な前後余白・送り量は別項目で表示する。把握できない段階では総長を`null`とし、推定で確定値を出さない。

`render`はプリンターに接続しない。`print`は明示的な印刷操作なので追加の対話確認を要求しない。複数候補から先頭を勝手に選ばず、対象指定または保存済みの明確な対応付けを必須とする。`doctor`は接続・列挙を行い得るが、未知のvendor characteristicへの書込みや印字・送り・カットは行わない。

## 4. 内部構成

最初は単一Cargo packageの`lib.rs`と`main.rs`にする。分割の必要が生じてからworkspace化する。

| モジュール | 責務 |
| --- | --- |
| `cli` | 引数、JSON/人間向け出力、終了コード |
| `render` | 入力の検査、配置、単色化、プレビュー |
| `job` | ジョブ形式、画像ハッシュ、変換設定、互換性検証 |
| `model` | SR-MK2の解像度、テープ別描画領域、検証済み機能 |
| `protocol` | コマンド符号化、受信解析、応答照合、機種固有状態遷移 |
| `transport/usb` | descriptor、interface、転送、切断 |
| `transport/ble` | scan、GATT、subscribe、write、通知 |
| `session` | 排他、事前検査、転送、完了待ち、再試行方針 |
| `backend/windows_webapi` | 公式モジュールへの画像送信、状態とジョブ終了の取得 |
| `diagnostics` | 通信記録、環境情報、診断用出力 |

共通の流れは「入力画像 → 正規化ラスタ → 機種プロトコル → transport」。USBとBLEで同じプロトコルを使うかは未確定なので、transportを交換するだけで必ず動く設計にはしない。

上記は直接制御backendの流れ。Windows WebAPI backendは正規化ラスタをPNGにし公式HTTPへ渡すため、独自protocol/transport層を通らない。両backendを`capabilities / status / submit / wait_completion`相当のセッション契約で統一する。CLIには`--backend direct|windows-webapi`を設け、WebAPIでは公式プリンター名を対象にする。USB/BLEの選択はインストール済みドライバ/プリンター設定に従い、CLIが直接GATT接続するわけではない。

内部ラスタは送り方向を`x`、ヘッド方向を`y`とする1画素1バイトの二値配列を基準とする。ワイヤー形式のビット詰め・転置・左右反転・圧縮はprotocol層が行う。256 dotを全部送る形式なら1列は32 byteになるが、これは計算上の値であり、本機の送信形式が32 byte固定と確認できたわけではない。

USBは要求/応答と転送endpointを、BLEはcharacteristicごとのwrite/read/notifyを扱う。両者を無理に`Read + Write`だけへ落とさず、プロトコルが必要とする送受信イベント、サイズ制限、エラーを保持する。

## 5. 画像と物理寸法

1. 入力サイズ・画素数・デコード資源の上限を検査する。初期候補はファイル20 MiB、40メガピクセル、変換作業領域256 MiB。これはCLIの保護設定であって本体上限ではない。
2. 透明部分を白へ合成する。白は無印字であり、実際の見た目はテープの地色に依存する。
3. 指定した物理寸法に合わせて配置する。縦横比を保つ`contain`を標準にし、切り抜く`cover`は明示指定とする。
4. 最終解像度へ縮小してから単色化する。文字・線画はthreshold、写真・階調はFloyd–Steinberg等を選べるようにする。
5. 正規化ラスタからプレビューと印刷データを作る。同じラスタを用い、プレビューと送信結果の食い違いを避ける。

公式値は360 dpi、256 dot、24mmテープで最大印刷幅18.1mm。[公式製品仕様](https://www.kingjim.co.jp/products/tepra/sr-mk2.html)

設計上の換算は`round(mm × 360 / 25.4)`。50mmの内容は709 dot、ヘッド方向は最大256 dotとなる。画像全体を24mm=約340 dotにして送らない。4/6/9/12/18mmの印字可能dot数とオフセットは検証前に設定しない。未対応テープ幅への`render`/`print`は理由を添えて拒否する。

細い日本語文字やQRコードは生成画像に描かせるより、正しい文字列からフォント/QRライブラリで合成する拡張が適する。QRのquiet zoneと整数dotのモジュール幅を保持し、印刷物で読取試験を行う。MVPでは一般PNGとして扱い、QR品質保証はしない。

SVG追加時は`resvg`/`usvg`を候補とし、外部URL参照・ファイル参照を許可せず、フォントを指定可能にする。フォントの内容ハッシュも記録する。

## 6. プロトコルと機器情報

モデル情報は`official`（公式資料）、`observed`（実機観測）、`hypothesis`（未確認）を区別する。仮説だけの値は通常の印刷経路で有効化しない。

USB VID/PID、interface、endpoint、BLE service/characteristic UUID、write方式、フレーム形式、状態コマンド、応答対応、テープコード、カット方法は実機から確定する。同じキングジム製品だからという理由で既存実装の定数を転記しない。標準ESC/POS対応とも仮定しない。

BLEはATT MTU、OS/libraryのwrite上限、プリンター固有上限のうち許される範囲へ分割する。応答なしwriteでも受信側の処理速度に追従させる。20/182/512 byte等を根拠なく固定しない。GATT write成功は物理印刷完了を意味しない。

macOSのBLE識別子はMACアドレス前提にしない。OS由来のID、観測した製品情報、ユーザーのaliasで管理する。USB/BLEが同一物理機器と確認できたら同一ロックへ結び付ける。確認できるまではMVPの送信全体を単一ロックで直列化し、別transport経由の重複送信も抑える。公式アプリとの同時利用は機器エラーとして案内する。

## 7. ジョブと結果の契約

`job.json`には`schema_version`、`model`、`profile_version`、テープ幅、解像度、内容長、正規化ラスタの寸法・ファイル名・SHA-256、元画像のSHA-256、変換オプションを保存する。ラスタはジョブに隣接するPNGへ保存し、印刷直前にハッシュを照合する。外部の任意コマンド列をジョブに格納して実行する形式にはしない。

初期版の自動カット設定は検証済みの標準動作だけとする。後で`--cut auto|none`等を追加する際は、テープ種類による禁止条件と連続印刷時の動作をprofileに含める。

`--json`のstdoutは単一JSONオブジェクト、ログと進捗はstderr。例外メッセージをstdoutへ混ぜない。機器の能力を取得できない項目は`null`または`unknown`とし、falseや0で埋めない。

| 終了コード案 | 意味 |
| --- | --- |
| 0 | そのコマンドが成功。`print`ではbackendが定義する正常なジョブ終了を確認できた場合のみ |
| 2 | 引数・画像・ジョブ形式不正 |
| 3 | 機器未発見・対象曖昧・識別不一致 |
| 4 | 権限不足・接続不可・他プロセス使用中 |
| 5 | テープ不一致・カバー開・その他確認できた機器エラー |
| 6 | 未対応profile・応答不正・送信開始前の通信失敗 |
| 7 | 送信開始後に切断等が起き、印刷結果を確定できない |

結果には`job_id`、`device_id`、`state`、`completion_evidence`、`error.code`、`retryable`を含める。`completed`は確認済みの終了応答でのみ使用する。単に全byteを書いた場合は`submitted`、完了を判定できなければ`unknown`とする。WebAPIの場合は`completion_evidence: "vendor_job_end"`とし、公式モジュールによるジョブ終了を示す。排紙や印字品質を独立したセンサーで確認したという意味ではない。direct backendは実機で意味を確認した終了応答を根拠にする。完了通知が存在しないことが判明した場合、契約を再検討し、送信完了を印刷完了として報告しない。

送信前は限定的な接続再試行を可能にする。送信後のエラーは自動でジョブ全体を再送しない。ローカルのjob IDは本体にexactly-onceを保証させるものではない。Ctrl-C時も既に動いた機器を止められるとは限らず、結果不明を記録する。

待機には必ず期限を設ける。暫定値は接続10秒、HTTP要求5秒、送信前の接続試行は最大2回かつ全体30秒以内、ジョブ開始から終了まで120秒。実機の所要時間に合わせて設定変更可能にするが、無期限待機を既定にしない。busyは明示した待機オプションがなければ送信前に終了する。送信後の期限超過は`unknown`・終了コード7とし、バックグラウンドの印刷は継続している可能性を返す。送信進捗が更新されてもジョブ全体の期限を無制限に延長しない。

## 8. Rust依存候補

| 用途 | 候補 | 選定条件 |
| --- | --- | --- |
| BLE | `btleplug` | GATTの列挙/write/notifyに対応。macOSのBluetooth権限を診断する |
| USB | `nusb` | descriptorとdriverの状態を確認して採用。WindowsはWinUSB前提を要検討 |
| ラスタ | `image` | PNGから開始し、不要なcodec featureを無効化する |
| SVG拡張 | `resvg` / `usvg` | 入力参照とフォントを制御する |
| Windows公式API | `reqwest`, `base64` | 公式SDKと同じJSONを構成。通信モジュールとの互換性を検証する |
| CLI/JSON/async | `clap`, `serde`, `serde_json`, `tokio` | 実装着手時に互換性・MSRVを確認して固定 |
| エラー/ログ | `thiserror`, `tracing` | 機械判定用コードと表示文を分ける |

依存の最新版番号を設計要件にせず、実装時に採用版と`Cargo.lock`を固定する。根拠: [btleplug](https://github.com/deviceplug/btleplug)、[Peripheral API](https://docs.rs/btleplug/latest/btleplug/api/trait.Peripheral.html)、[nusb](https://docs.rs/nusb/latest/nusb/)、[image](https://docs.rs/image/latest/image/)、[resvg](https://docs.rs/resvg/latest/resvg/)。

WindowsのUSBを対応させるために標準でドライバーを置換する設計にはしない。公式API経由の別backendを優先比較する。macOSの公式アプリ対応はCUPS/`lp`から印刷できる根拠にはならない。

## 9. 実装順序と判定基準

| 段階 | 作業 | 完了条件 |
| --- | --- | --- |
| P0: 接続調査 | 実機情報、USB descriptor、BLE GATT。Windows採用時は公式モジュール疎通を優先 | 採用経路、機器識別、未確定の通信項目を記録 |
| P1: オフライン | PNG変換、プレビュー、ジョブ、JSON契約 | 物理寸法・透明色・向き・入力制限のテストが通る |
| P2: 1枚印刷 | 公式出力との差分調査、1経路・1テープ幅を実装 | 非対称パターンが正しい向き/寸法で印字され、完了を判定 |
| P3: 異常系 | 排他、テープ/電源/カバー/切断、再接続 | エラーと結果不明を区別し、重複印刷しない |
| P4: 対応拡大 | 2つ目のtransport、各テープ幅、SVG等 | OS×接続×テープ幅ごとに実機結果を記録 |

P0/P2でプロトコルが確定できなければ、一般公開資料だけでRust直結を完成できるとは約束しない。公式SDKの利用やWindows補助ホストを代替案として評価する。USB・BLEいずれかで使える最小版を優先し、両方を初版の必須条件にはしない。

## 10. 検証計画

オフラインではラスタの寸法換算、余白、アルファ合成、二値化、protocol符号化/解析をgolden fixtureで確認する。機器から得た正常・異常・分割受信のトレースを保存し、切断時に再送しない状態遷移をmock transportで試す。未確認フレームをfixtureの正解にしない。

実機ではファームウェア、OS、接続、テープ品番・幅、電源、CLI/profile版を記録する。最初は小さな非対称パターンに左右・上下マークを入れ、公式アプリでの印字を基準とする。長さ、上下の欠け、鏡像、送り余白、カットを実測する。ドライバー・BLE接続・プリンター状態を伴う試験はオフラインCIとは分ける。

通信観測の準備ができてから、白/単点/線/文字の差分を一条件ずつ比較する。印刷途中の切断試験は成功する小さなジョブと状態判定ができてから実施する。通信ログにはラベル内容を復元可能なデータが含まれるため、通常ログは長さ・ハッシュ中心にし、生トレースは診断時に明示的に保存する。

## 11. Windows公式WebAPI backendの具体化

公式通信モジュールは別途インストールする。CLIは起動時に`GET http://localhost:29108/api/printer`を呼び、接続失敗なら導入/起動状態を案内する。モジュールの自動起動方式は未確認なので、CLIが未確認のEXEやサービスを起動する設計にはしない。

公式SDKのHTTP契約を調査した結果に基づく手順案は以下。Rustからの接続・印刷は未検証。[公式SDK](https://www.kingjim.co.jp/resource-files/download/tepra/tc_api/tepra_creator_webapi_sdk.zip)

| 順序 | 基点`/api/printer`からの相対パス | 判断 |
| --- | --- | --- |
| 1 | `GET` 基点そのもの | インストール済みプリンターを列挙 |
| 2 | `GET /info/{name}` | driver名、dpi、対応テープを取得 |
| 3 | `GET /onlinestatus/{name}` と `/lwstatus/{name}` | online、装着テープ、error、使用状態を検査 |
| 4 | `POST /print/{name}` | `result:1`を確認し`jobid`を保存 |
| 5 | `GET /job/progress/{name}?jobid={id}` | 終了までpollし、正常/中止/エラーを記録 |

`name`はURLのpath segmentとしてencodeする。画像はJSONの`printFile.imageFile`内の`fileName`と`base64Str`に入れ、Base64はPNG全体を符号化したものとする。data URLのprefixやローカルパスを送らない。`printParameter`はSDKの変換後の値で構成し、JS公開定数をそのまま送信しない。初期版は通常テープ1枚を対象にし、装着テープIDと事前照合する。

`stretchImage:0`で自動伸縮を避ける候補とするが、PNG解像度情報とドライバによるmm換算は印字実測で確認する。`marginLeftRight:0`は未指定であり、物理的ゼロ余白とは解釈しない。プレビューの描画とドライバの配置が一致することをP2の受入条件とする。

`dataProgress:100`だけでは終了しない。`jobEnd:true`かつ`canceled:false`、`statusError:0`を正常なジョブ終了とする。公式資料では終了情報の参照期間が約30秒なので、初期値500ms程度のpollを候補にして直ちにローカルへ保存する。POST後の応答消失や照会情報消失は結果不明とし、POSTを自動再試行しない。[WebAPIリファレンス pp.43–44](https://www.kingjim.co.jp/resource-files/download/tepra/tc_api/tepra_creator_webapi_reference.pdf)
