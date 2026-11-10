# tepra-cli

エージェントが生成した画像をTEPRA SR-MK2から印刷するためのRust CLI構想です。現在は調査・設計段階で、実行可能なCLIはまだありません。

- [印刷仕様・公式API・既存実装の調査](docs/research.md)
- [CLIと実機検証の設計案](docs/design.md)

SR-MK2はUSBおよびBluetooth GATTで印刷できます。Windowsでは公式WebAPI、macOSではUSB/BLE直接制御を検証候補としています。独自プロトコルによるSR-MK2実機印刷は未検証です。
