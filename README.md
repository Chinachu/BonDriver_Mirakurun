# BonDriver_Mirakurun (Rust)

TVTest から [Mirakurun](https://github.com/Chinachu/Mirakurun) を利用する為の BonDriver です。

本リポジトリはオリジナルの C++ 実装を **Rust に全面移植** したものです。
ホスト(TVTest 等)とは IBonDriver2 互換の vtable を持つ DLL として ABI 互換を保っています。

## 必要環境

- [rustup](https://rustup.rs/)(ツールチェーン管理)
- リンカとして次のいずれか
  - **MSVC Build Tools**(既定。C++ ワークロード + Windows SDK。`x86_64-pc-windows-msvc` 用)
  - もしくは **LLVM-MinGW**(`x86_64-pc-windows-gnu` 用)
- 対応アーキテクチャは **x64** のみ
  - x86(32bit)はメンバ関数が `__thiscall` 規約になるため未対応です
    (`src/lib.rs` で `compile_error!` によりビルドを禁止しています)。

## 環境構築(任意)

MSVC ビルド環境を一括で用意したい場合は、付属の `setup-env.bat` を実行してください。
[winget configure](https://learn.microsoft.com/windows/package-manager/configuration/)
を利用して `configuration.dsc.yaml` に定義された以下をインストールします。

- Visual Studio 2022 Build Tools(VCTools ワークロード + Windows 11 SDK)
- Rustup(Rust ツールチェーン管理)

```bat
setup-env.bat
```

- `winget`(App Installer)が必要です。Microsoft Store から入手できます。
- 適用時に UAC(管理者昇格)と構成内容の確認を求められます。
- 完了後、`x86_64-pc-windows-msvc` ツールチェーンが既定として設定されます。

## ビルド

付属の `build.bat` を実行してください。

```bat
build.bat
```

- 既定ターゲットは `x86_64-pc-windows-msvc`(MSVC)です。
- 必要なツールチェーン/ターゲットは `rustup` で自動的に追加します。
- 生成物は `dist\BonDriver_Mirakurun.dll` と `dist\BonDriver_Mirakurun.ini` に出力されます。
- 別ターゲットを指定する場合は引数で渡せます。LLVM-MinGW 環境なら:

```bat
build.bat x86_64-pc-windows-gnu
```

手動でビルドする場合(MSVC 利用時):

```bat
rustup toolchain install stable-x86_64-pc-windows-msvc
cargo +stable-x86_64-pc-windows-msvc build --release --target x86_64-pc-windows-msvc
```

出力先は `target\<target>\release\BonDriver_Mirakurun.dll` です。

## 設定

`BonDriver_Mirakurun.ini` を DLL と同じフォルダに配置してください。
項目はオリジナル版と互換です。すべて `[GLOBAL]` セクションに記述します。

| キー | 既定値 | 説明 |
| --- | --- | --- |
| `SERVER_HOST` | `localhost` | Mirakurun のホスト名 |
| `SERVER_PORT` | `8888` | Mirakurun のポート番号(付属の ini では `40772` を設定) |
| `DECODE_B25` | `0` | B25 デコード(1=有効) |
| `PRIORITY` | `0` | Mirakurun の優先度 |
| `SERVICE_SPLIT` | `0` | サービス単位で分割(1=有効) |

## 実装メモ

- vtable は MSVC(cl.exe x64)が実際に生成するスロット順に完全一致させています
  (同名オーバーロードの逆順配置、`IBonDriver2` で再宣言される `Release` の
  基底スロット共有を含む全17スロット)。
- TVTest(LibISDB)は `CreateBonDriver` の戻り値に `dynamic_cast<IBonDriver2*>`
  を実行するため、MSVC 互換の RTTI 構造体(Complete Object Locator 等)を
  実行時に構築し vtable に付加しています(`src/rtti.rs`)。
- TS 受信はオリジナルの Push/Pop 2スレッド + オーバーラップド I/O を、
  1本の受信スレッド + `Mutex`/`Condvar` キューという等価構成へ整理しています。
  ストリーム先頭の HTTP レスポンスヘッダは TS データに混入させず読み飛ばします。
- JSON 解析は `serde_json` を使用(オリジナルの picojson 置き換え)。
- FFI(`extern "C"`)境界を Rust の unwind が越えないよう、リリースビルドでは
  `panic = "abort"` を指定しています。

## License

This software is released under the MIT License, see LICENSE.
