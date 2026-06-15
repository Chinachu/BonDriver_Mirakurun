# BonDriver_Mirakurun (Rust)

TVTest から [Mirakurun](https://github.com/Chinachu/Mirakurun) を利用する為の BonDriver です。

本リポジトリはオリジナルの C++ 実装を **Rust に全面移植** したものです。
ホスト(TVTest 等)とは IBonDriver2 互換の vtable を持つ DLL としてABI互換を保っています。

## 必要環境

- [rustup](https://rustup.rs/)(ツールチェーン管理)
- リンカとして次のいずれか
  - **LLVM-MinGW**(既定。`x86_64-pc-windows-gnullvm` 用)
  - もしくは **MSVC Build Tools**(C++ ワークロード。`x86_64-pc-windows-msvc` 用)
- 対応アーキテクチャは **x64** のみ
  - x86(32bit)はメンバ関数が `__thiscall` 規約になるため未対応です
    (`src/lib.rs` で `compile_error!` によりビルドを禁止しています)。

## ビルド

付属の `build.bat` を実行してください。

```bat
build.bat
```

- 既定ターゲットは `x86_64-pc-windows-gnullvm`(LLVM-MinGW)です。
- 必要なツールチェーン/ターゲットは `rustup` で自動的に追加します。
- 生成物は `dist\BonDriver_Mirakurun.dll` と `dist\BonDriver_Mirakurun.ini` に出力されます。
- 別ターゲットを指定する場合は引数で渡せます。MSVC 環境なら:

```bat
build.bat x86_64-pc-windows-msvc
```

手動でビルドする場合(LLVM-MinGW 利用時):

```bat
rustup toolchain install stable-x86_64-pc-windows-gnullvm
cargo +stable-x86_64-pc-windows-gnullvm build --release --target x86_64-pc-windows-gnullvm
```

出力先は `target\<target>\release\BonDriver_Mirakurun.dll` です。

## 設定

`BonDriver_Mirakurun.ini` を DLL と同じフォルダに配置してください。
項目はオリジナル版と互換です。

| キー | 説明 |
| --- | --- |
| `SERVER_HOST` | Mirakurun のホスト名 |
| `SERVER_PORT` | Mirakurun のポート番号 |
| `DECODE_B25` | B25デコード(1=有効) |
| `PRIORITY` | Mirakurun の優先度 |
| `SERVICE_SPLIT` | サービス単位で分割(1=有効) |

## 実装メモ

- JSON 解析は `serde_json` を使用(オリジナルの picojson 置き換え)。
- TS 受信はオリジナルの Push/Pop 2スレッド + オーバーラップドI/O を、
  1本の受信スレッド + `Mutex`/`Condvar` キューという等価構成へ整理しています。

## License

This software is released under the MIT License, see LICENSE.
