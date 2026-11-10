# SR-MK2 USB プロトコル調査

## 実装後の確認結果

同日、このMacのSR-MK2とユーザーが24mmと確認した装着テープで、709×256dot（内容長指定50mm）の校正画像を送信した。最初の試行はCLIの状態判定が厳しすぎてcode 7となったが、ユーザーはラベルの印字と自動カットを確認した。ST=3/4/5を待機扱いに修正した後、別の明示的な試行でCLIは`state=completed`、終了コード0を返した。完了待ちで観測したST遷移は`[2, 0]`で、最終応答は下記の通常時の22 byteと同一だった。結果不明になったジョブをCLIが自動再送したわけではない。

送信したCのデータは`01 01 01 01`（公式UIのカットON/ハーフカットOFF）、Tは141dot。成功結果は`completion_evidence=usb_busy_then_idle`として保存する。ラベルの正確な全長・前後余白、他のテープ品種やファームウェア、長い連続ジョブは未検証。初期CLIは試験的なUSB経路として、観測したカートリッジ状態と1枚・内容長100mm以下に限定する。

以下の静的解析メモは、実機試験の前に得た情報と未確認点の記録である。

2026-11-10。実機での列挙・状態取得および下記の試験印刷を除き、この Mac にある TEPRA LINK 2 1.4.0 (build 83) の x86_64 バイナリを静的に解析した結果である。実行バイナリの SHA-256: `42a58c5677aa7f5e03f926db893e8a42ba926b7d3d6a89a0662fca3ed75085f5`。

## USB と状態取得の実測

macOS 15.7.3 で VID `0d8a`、PID `0103`、configuration 1、interface 0、class 7/subclass 1/protocol 2、bulk IN `81`、bulk OUT `02`、最大パケット 64 byte を確認した。IEEE 1284 device ID は `MFG:KING JIM;CMD:ESCPL2;MDL:TEPRA PRO SR-MK2;CLS:PRINTER;`。

静的解析で確認した vendor/interface IN control request `bmRequestType=c1, bRequest=01, wValue=0000, wIndex=0000, wLength=22` を一度実行し、22 byte の応答を得た。bulk 出力は行っていない。

```text
16 00 00 00 00 00 05 00 05 00 00 01 00 00 00 00 00 00 00 01 00 00
```

これは一台・一時点の値であり、個々の状態の意味や条件別応答を確認したものではない。

この後の初回の独自 USB 試験印刷について、ユーザーが印字と自動カットの実行を目視確認した。CLI は印刷後の未知の状態値により終了コード7を返し、その際の生値は記録されなかった。直後の状態取得は ST=0、ER=0 だった。この試験だけでは方向、寸法、余白、各状態遷移や印刷完了の判定方法は確定していない。

## 応答の解析（静的）

`AccessManagerUSB.printerStatus` は control request の結果を `AccessManager.parseStatusFromLwStatus:` に渡す。製品情報辞書は SR-MK2 の `Status Length` を22としており、22 byte 用分岐のキーと入力位置は以下のとおり。単一 byte は符号付き `char` に変換される。複数 byte はビッグエンディアン。

| キー | オフセット（0始まり） | 今回の値 |
| --- | --- | --- |
| ST | 1 | 0 |
| ER、ER2 | 2 | 0 |
| EI2 | 4 | 0 |
| EJ2 | 5 | 0 |
| TW | 6 | 5 |
| TR | 7 | 0 |
| TS | 8 | 5 |
| T8S | 9 | 0 |
| T8O | 10 | 0 |
| T8T | 11 | 1 |
| PP | 12–13 | 0 |
| RB | 14–17 | 0 |
| WA | 18 | 0 |
| VER | 19 | 1 |
| RES | 20–21 | 0 |

オフセット3はこの分岐の上記キーに使われない。TW=5 を変換する内部関数は `StatusKeyTapeWidth=6` を返す。製品情報の `printableSizeFromTape:resolution:modelNumber:` は tape enum 6 かつ SR-MK2 の model number 21 なら描画サイズ 256.0 を返す。他の状態値やテープ幅への一般的な対応は未確認。TS 等の意味も未確認である。24 mm テープ時の最大印刷幅18.1 mm、ヘッド256 dot、360 dpi は[公式製品仕様](https://www.kingjim.co.jp/products/tepra/sr-mk2.html)の値で、ユーザーが装着テープは24 mmと確認したため、今回の実機では raw TW=5 と24 mmが対応する。ただし他のカートリッジ品種や firmware に一般化した検証は行っていない。

## コマンド生成（静的解析）

`PrintController` の USB 対応経路とコマンド生成を確認した。SR-MK2 の製品辞書にある `Command Level` は5、`Model Number` は21。次のバイト列は静的な生成結果であり、本機での正常印刷を保証しない。数値引数と実際の印刷条件の対応が未確認の箇所が残る。

ジョブ環境は次の順に連結される。`C` と `D` は内部設定に依存する。

```text
1b 7b 03 40 40 7d
1b 7b 07 7b 00 00 53 54 22 7d
1b 7b 07 43 [4 data bytes] [checksum] 7d
1b 7b 04 44 [density] [checksum] 7d
1b 7b 03 47 47 7d
```

公式アプリの印刷パラメータ構築 (0x1000f0a5d〜) は `Tape Cut` スイッチ ON を `ParameterKeyTapeCut=0`、OFF を `ParameterKeyTapeCut=2` に変換する。スイッチ ON、`ParameterKeyHalfCut=0` の `C` は `1b 7b 07 43 01 01 01 01 47 7d`。別の内部 `ParameterKeyTapeCut=1` と `ParameterKeyHalfCut=0` の組み合わせでは `C` は `1b 7b 07 43 01 00 01 01 46 7d`。ただし内部列挙値1が実際にどのカット設定を表すかは未確認。これらの物理的なカット動作は未実測。`ParameterKeyDensity=0` の場合 `D` は `1b 7b 04 44 05 49 7d`。密度は設定値に5を足し、0〜10の範囲外なら0にする。共通 checksum はコマンド種とデータの各 byte の和の下位8 bit。

ページ設定は `imageHeight`、`imageWidth`、`contentMargin` を引数にする。Command Level 5 の場合、`L` と `T` の順に生成し、条件により `y` を追加する。

```text
1b 7b 07 4c [imageHeight u32 LE] [checksum] 7d
1b 7b 05 54 [contentMargin u16 LE] [checksum] 7d
# ParameterKeyPriorityPrintSetting=1 のときのみ追加
1b 7b 03 79 79 7d
```

`TepraPrintFormDraw` は `ObjectDataTopFrame.marginsValue` を切り捨てて margin に渡す。初期化では `TepraMarginsMinimum` を resolution × (1/25.4)、`TepraMarginsLarge` を resolution × (21/25.4)、既定を resolution × (10/25.4) としている。360 dpi の場合はそれぞれ14、297、141 dot 相当の整数になる。物理余白は未実測。

`imageWidth` はこの level の L/T フレームには入らない。内部の `imageHeight` / `imageWidth` と実ラベルの送り方向・ヘッド方向の対応、margin の物理的意味は実測が必要。

各画像行の先頭に `1b 2e 00 00 00 01 [imageWidth u16 LE]` を置き、続けて画素を8個ずつ MSB first にまとめる。内部画像の画素値が180未満なら1、そうでなければ0。256画素の場合は8 byte のヘッダーと32 byte のデータで1行40 byte。公式アプリは描画前に DeviceGray の8 bitコンテキストを作り、startRow の平行移動と `(a,b,c,d,tx,ty)=(0,-1,1,0,0,rowCount)` の変換を結合する。ただし描画側の座標系と物理送り方向を含めた最終的な向きは未確認。実装中の画像変換がこの入力形式と一致するとは限らない。

公式アプリの印刷処理では、SR-MK2 の製品辞書上のモデル番号に対応する経路は最大1024行をまとめて送信し、chunk ごとに状態確認する。正常経路では各ページのラスタ送信後に単独の `0c` を送り、全ページ後に `1b 7b 03 40 40 7d` を送る。条件分岐・エラー処理を含む全ジョブの再現は未完了。

## 印刷後状態（静的）

公式アプリの `checkStatusForAfterPrint:` は初回0.5秒待ち、その後おおむね0.05秒間隔で状態を取得する。通常経路では状態辞書の `ST`（生応答 byte[1]）が1または2になったことを記録し、後続で `ST=0` かつ `ER=0` に戻ると成功へ分岐する。`ST=3`、`4`、`5` で `ER=0` の場合はこのループ内で即異常終了せず待機するが、それらのみでは1/2観測フラグは立たない。最初から `ST=0, ER=0` のみを観測した場合を直ちに完了としていない。中止・エラーなどの別分岐も存在するため、この条件だけで全動作を定義しない。応答の条件別の実測はまだない。

印刷中の `checkStatusForPrinting:10` では、SR-MK2 のモデル番号21に対応するテーブルを参照する。`ER=0` かつ raw `ST=3`、`4`、`5` はそれぞれ内部状態3、1、2に変換され、いずれも処理継続の結果0となる。これは印刷後に完了を確定する条件とは異なる。
