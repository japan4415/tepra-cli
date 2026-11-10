# TEPRA SR-MK2 印刷仕様調査

調査日: 2026-11-10。公開一次資料と既存実装を調査した。SR-MK2実機への接続・送信・印刷はまだ行っていない。

## 結論

SR-MK2はUSBデータ印刷とBluetooth 5.1 GATTに対応する。Windowsには画像を直接受け取る公式WebAPIがあり、Rust CLIからの利用を最初に検証する価値がある。macOSには公式アプリがあるが、公開CLI/APIは今回確認できなかった。macOSで直接制御するにはUSBまたはBLEの通信仕様を追加調査する必要がある。[公式仕様](https://www.kingjim.co.jp/download/manual/tepra/sr-mk2/06-01-01-01.html)、[公式開発者情報](https://www.kingjim.co.jp/sp/tepra_creator/developer/)、[対応アプリ](https://www.kingjim.co.jp/download/tepra/model/07.html)

SR-MK2を実機検証したOSSは今回の検索範囲では発見できなかった。「存在しない」「既存機種と互換性がない」という結論ではない。SR-MK1・SR-R2500P・旧USB機の実装は調査手法の参考になるが、本機の動作保証には使えない。

## 1. 公式に確認できた仕様

| 項目 | 内容 | 根拠 |
| --- | --- | --- |
| USB | USB 1.1 Full-speed、Type-C、印刷データ接続対応 | [本体仕様](https://www.kingjim.co.jp/download/manual/tepra/sr-mk2/06-01-01-01.html) |
| Bluetooth | Ver.5.1、GATT。SPP/RFCOMM前提ではない | [本体仕様](https://www.kingjim.co.jp/download/manual/tepra/sr-mk2/06-01-01-01.html) |
| 印刷 | 熱転写PRO、360dpi、256dot | [製品仕様](https://www.kingjim.co.jp/products/tepra/sr-mk2.html) |
| 最大印刷幅 | 18.1mm（24mmテープ時） | 同上 |
| テープ | PRO 4/6/9/12/18/24mm。転写・カットラベル・点字テープ除外 | 同上 |
| カッター | オートカッター。RH24は別売外付けハーフカッター | 同上 |
| 電源 | AC1116Jまたは単3電池6本。USB接続手順も別途電源を入れる指示 | [接続手順](https://www.kingjim.co.jp/download/manual/tepra/sr-mk2/03-01-02-01.html) |

USB Type-Cの存在をUSB給電の保証にしない。今回確認した公式手順ではACアダプタまたは電池を使用する。「USB給電不可」の直接記載を確認したという意味ではない。

| 環境 | 公式ソフトと本機への接続 | 注意 |
| --- | --- | --- |
| Windows | テプラクリエイター、USB/Bluetooth | 現行ページは日本語Windows 10/11、ARM版非対応 |
| macOS | TEPRA LINK 2、USB/Bluetooth | macOS 13.0以降。CUPS対応を意味しない |
| iOS / Android | Hello / TEPRA LINK 2、Bluetooth | スマートフォン版LINK 2のUSB接続は非対応 |
| Linux | 今回調べた公式アプリ表に掲載なし | Rust直結は独自対応になる |

根拠: [テプラクリエイター](https://www.kingjim.co.jp/sp/tepra_creator/)、[TEPRA LINK 2](https://www.kingjim.co.jp/sp/tepra_link2/)、[LINK 2接続表](https://www.kingjim.co.jp/sp/tepra_link2/manual/02_00.html)。OS要件は一般アプリとSDKで異なるため、採用時に再確認する。

Windowsで市販アプリから印刷する場合の公式ドライバ余白は、上下がテープ幅4/6/9/12/18/24mmに対して0.6/0.7/0.9/1.1/1.7/3.1mm、左右10mm。この表はドライバの設定値であり、独自通信での最小余白や各テープの厳密な有効dot数には転用しない。[公式マニュアル](https://www.kingjim.co.jp/download/manual/tepra/tepra_creator/04-01-16-10.html)

## 2. 公式APIの選択肢

| 経路 | 公開資料で確認した仕組み | Rustからの見通し |
| --- | --- | --- |
| テプラクリエイターAPI | Windowsの`SPC10.exe /pt`・`/p`呼出。LW1レイアウトとCSVが基本 | `std::process::Command`で呼べる候補。画像単独印刷は今回未確認 |
| テプラクリエイターWebAPI | Windows上のHTTP通信モジュール。`imageFile`でPNG/JPG/TIF/GIF/BMPを印刷可能 | ローカルHTTPを呼ぶbackendが有力。Rustでの実動作は未検証 |
| TEPRA-Print SDK | iOS/Android向け。SR-MK2を対象に含む | モバイルSDKを介する別アプリが必要。デスクトップRust CLIの第一候補にはしない |
| macOS公開API | 今回の開発者情報では確認できず | LINK 2アプリが動くことだけで自動化APIがあるとはしない |

APIの`/B`はプレビューBMPの出力指定であり、画像入力オプションではない。[公式APIガイド pp.2–3, 8](https://www.kingjim.co.jp/resource-files/download/tepra/tc_api/tepra_creator_users-guide.pdf)

WebAPIの公式資料は画像ファイルのみの印刷、画像伸縮、余白、機器状態、ジョブ進捗を説明している。日本語版Windowsプリンタドライバ5.53と通信モジュールが必要。これはインターネット上のプリンタークラウドAPIではない。[WebAPIリファレンス pp.2, 11–12, 55](https://www.kingjim.co.jp/resource-files/download/tepra/tc_api/tepra_creator_webapi_reference.pdf)

公式SDK ZIPの`tepraprint.js`では基点が`http://localhost:29108/api/printer`、印刷は`POST /print/{encodedPrinterName}`。JavaScript SDKという名前でも、通信自体はHTTPなのでRustから呼べる可能性が高い。ただし画像のエンコード、リクエスト形式、エラーと完了状態を公式SDKと一致させて検証する必要がある。[公式WebAPI SDK](https://www.kingjim.co.jp/resource-files/download/tepra/tc_api/tepra_creator_webapi_sdk.zip)

開発者ページでは2026-08-03の更新にSR-MK2を含み、WebAPI SDK 1.1.1、iOS SDK 1.7.0、Android SDK 1.4.0を掲載する。モバイルSDKのOS条件と一般アプリのOS条件を混同しない。[開発者情報](https://www.kingjim.co.jp/sp/tepra_creator/developer/)

公式モジュールはRust CLIのリポジトリへ同梱しない方針とする。利用・配布条件はAPI/WebAPIとモバイルSDKで異なるため、実装時に該当する許諾を確認する。[API/WebAPI許諾](https://www.kingjim.co.jp/download/tepra/tc_webapi/agree.html)、[SDK許諾](https://www.kingjim.co.jp/download/tepra/sdk/agree.html)

## 3. 公開プロトコル実装

以下は各作者の一次実装/調査記録に基づく。ライセンスの記載は採用候補の識別用で、コード移植前には採用ファイルのライセンス・通知を再確認する。

| 実装 | 実機検証の範囲 | 参考になる点 / 本機との差 |
| --- | --- | --- |
| [labelbridge](https://github.com/takustaqu/labelbridge/tree/6d6a8b3dcc0235953bbd35fe7679ec4ef19081de) | SR-R2500P Bluetooth、SR3500P USB | Rust、MIT/Apache-2.0。R2500PのSPPと180dpiはMK2のGATT/360dpiと異なる |
| [tepra_web](https://github.com/nikachu2012/tepra_web/tree/74c181c58e38) | SR-R2500Pで印刷検証。MK1は未検証と記載 | MIT。ラスタ/状態解析の参考。MK2対応根拠なし |
| [tpsr3700](https://github.com/ruimo/tpsr3700/tree/48e4f146b8b890a9d4030705051ecb0d3eeddf10) | SR3700P USB | Rust/rusb、Apache-2.0。同梱フォントOFL。公式ドライバの送信差分解析 |
| [tepraprint](https://github.com/nyacom/tepraprint/tree/5fc9df5020b2e0f380ea884ea7a1cc5b8cebc0c9) | SR920 USB | Python/PyUSB/CUPS、READMEにMIT表記。ESC/P2L風の独自制御解析 |

R2500Pの記録には`ESC {`設定フレーム、`ESC .`ラスタ、MSB first・1=黒、状態取得や送信の流量制御がある。SR3500Pでは別のESC/P2系コマンドとUSB Printer Class resetを扱う。SR3700Pには2179 byteのジョブヘッダがある。**本機でこれらのフレームやresetを送信してよいと確認できたわけではない。**

参照: [R2500P解析](https://github.com/takustaqu/labelbridge/blob/6d6a8b3dcc0235953bbd35fe7679ec4ef19081de/docs/protocol.md)、[SR3500P実装](https://github.com/takustaqu/labelbridge/blob/6d6a8b3dcc0235953bbd35fe7679ec4ef19081de/crates/labelbridge/src/sr3500p.rs)、[SR3700P解析](https://github.com/ruimo/tpsr3700/blob/48e4f146b8b890a9d4030705051ecb0d3eeddf10/docs/protocol.md)、[tepra_web解析](https://github.com/nikachu2012/tepra_web/blob/74c181c58e38/PROTOCOL.md)。

結論として、汎用レシートプリンターのESC/POSライブラリを採用する根拠はない。一方、本機のESC/POS非対応そのものを実測したわけでもない。BLEとUSBで共通の印刷payloadを使うかも未確定。

## 4. Rust接続ライブラリの制約

`btleplug`はBLE centralの列挙、GATT read/write/notifyに対応し、macOS/Windows/Linuxを対象にする。Bluetooth Classicは対象外だが、MK2の公式GATT仕様と方向性は合う。macOSでは実行ホストのBluetooth権限が必要で、端末で動いてもエージェントの実行ホストで動くかは別途確認する。[btleplug README](https://github.com/deviceplug/btleplug)、[Peripheral API](https://docs.rs/btleplug/latest/btleplug/api/trait.Peripheral.html)

`nusb`はmacOS/Linux/Windows向けUSBライブラリ。macOSではkernel driverに占有された機器を利用できず、WindowsではWinUSBとの関連付けが必要。公式ドライバを壊さずに利用できるかが採用条件になる。Linuxは対象VID/PIDに限定した権限設定を検討する。[nusb platform support](https://docs.rs/nusb/latest/nusb/)

画像処理は`image`、SVG拡張は`resvg`/`usvg`が候補。入力画素数の制限はCLI側でも設ける。`image`のメモリ上限はbest effortの項目があるため、それだけを厳密な制限として扱わない。[image](https://docs.rs/image/latest/image/)、[Limits](https://docs.rs/image/latest/image/struct.Limits.html)、[resvg](https://docs.rs/resvg/latest/resvg/)

## 5. 残る未確定事項と確認方法

| 未確定事項 | 確認方法 | 設計への影響 |
| --- | --- | --- |
| USB VID/PID、class、interface、endpoint | 接続した実機のdescriptor列挙 | transport選定、ドライバ共存 |
| GATT service/characteristic、属性、認証 | 実機のscanとservice discovery、公式アプリ通信の観測 | read/write/notifyと接続手順 |
| フレーム、checksum、ラスタ方向/bit順、圧縮 | 公式アプリの小さな非対称画像を条件ごとに比較 | encoder/decoderの正解fixture |
| 最大転送サイズ、背圧、応答照合 | 正常通信の時系列記録、少量からの送信試験 | 分割・timeout・再試行 |
| テープ別有効dotと原点、前後余白 | 公式出力と独自出力の印字実測 | レイアウトと実消費長 |
| 最小/最大ラベル長、連続枚数、cut制約 | 公式APIの仕様確認と段階的実測 | CLIの入力制限 |
| テープ種/幅、電池、カバー、busy、完了 | 状態応答を条件ごとに記録 | 事前検査と結果の信頼性 |
| USB/BLEのpayload共通性 | 同じ画像・設定の両経路を比較 | protocol共有可能範囲 |
| WebAPIのRust直接呼出 | Windowsで公式モジュールを用意し画像1枚・状態取得 | 最短の公式経路が成立するか |

通信観測は環境に応じてWindowsのUSBPcap/WiresharkやAndroidのBluetooth HCIログ等を候補にするが、本機と利用環境で取得できることは未検証。まず利用可能な機器・OSとキャプチャ手段を確認する。macOSだけで必要な生トレースを必ず取れるとは想定しない。

現在の調査から作った実装案と受入条件は[設計書](design.md)に記載した。印刷を伴う検証は今回の調査・設計作業には含めていない。
