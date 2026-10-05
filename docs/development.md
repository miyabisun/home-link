# 開発ガイド

ソースを取得し、リポジトリへ移動します。以降のコマンドは、特に断りがなければこのディレクトリで実行します。

```sh
git clone https://github.com/miyabisun/home-link.git
cd home-link
```

利用者向けの説明は[README](../README.md)、Androidアプリの画面の規則は[DESIGN.md](../DESIGN.md)にあります。

## APIサーバー

Rustのツールチェーンは `rust-toolchain.toml` の版を `rustup` が自動で選びます。

```sh
cargo run --locked
```

`http://127.0.0.1:3000` で待ち受け、カレントディレクトリの `home-link.db` にデータを保存します。
変更後は次の検証を通してください。CIも同じ内容を実行します。

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

`tests/api.rs` はメモリ上のSQLiteでAPIの要求と応答を検証します。
`src/onboarding.rs` はMatterのQRコード（Base38）と手動ペアリングコード（Verhoeff）の検証、
同じ機器かを判定するキーの取り出しと、そのテストを持ちます。
`src/matter.rs` はmatterjs-serverのWebSocket APIから全nodeを読み、識別子からnodeとendpointを引きます。
照明の選び方（Descriptorのdevice typeとOn/Off、ブリッジ配下の展開、届くかどうか）も同じファイルで単体テストします。
状態APIと照明APIのテストは、`tests/api.rs` の中でmatterjs-serverと同じ形で `get_nodes` と `device_command` に応答する
WebSocketサーバーを立て、全部オン・全部オフ、エラーや無応答の照明、接続断からの再接続を検証します。
無応答の照明のテストは、応答を待つ10秒の期限まで待ちます。
`src/schedule.rs` は自動調整の目標値（日の出・日の入り、22時と5時の境界、照明ごとの範囲への丸め）と、
照明ごとに送るか・送らない理由を決める純粋な処理と、そのテストを持ちます。
自動調整のテストは、同じmockに `read_attribute` と明るさ・色温度の命令を受けさせ、点いている照明だけに書き込むこと、
「全部オフ」の後は再起動をまたいでも「全部オン」まで何も送らないことを検証します。

コンテナは次のように作成し、確認できます。

```sh
docker build -t home-link .
docker run --rm -p 127.0.0.1:3000:3000 home-link
```

## Androidアプリ

アプリは `android/` にあります。JDK 17と[Android SDK Command-line Tools](https://developer.android.com/studio#command-tools)を用意します。
以下のパスは、それぞれの場所に置き換えてください。

```sh
export JAVA_HOME=/path/to/jdk17
export ANDROID_HOME=/path/to/android-sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH"
sdkmanager --licenses
sdkmanager 'platforms;android-37.0' 'build-tools;36.0.0' 'platform-tools'
cd android
./gradlew :app:testDebugUnitTest :app:lintDebug :app:assembleDebug
```

APKは `android/app/build/outputs/apk/debug/app-debug.apk` に出力します。
package IDは `dev.miyabisun.homelink` です。
接続先は既定で `http://homeserver:5011` です。別のURLでビルドするには `-PhomeLinkUrl=http://…` を付けます。
平文HTTPを許可するホストは `app/src/main/res/xml/network_security_config.xml` に書いています。
ホスト名を変える場合は、このファイルも合わせて変更してください。

JVMテストは、JDKの一時HTTPサーバーを相手にAPIクライアントを検証します。入力の状態の扱いも単体で検証します。

### 画面の検証

instrumentationテストは、APIとQRスキャナーをfakeに置き換え、実際の画面部品を操作します。
部屋の作成・選択・登録・成功表示と、登録済み・部屋なし・接続不可・Matter以外のQR・画面の再生成を扱います。
照明のボタンは、送信中・全部成功・一部の照明が残った場合・接続不可の表示と、画面の再生成での結果の保持を扱います。
ホーム画面のウィジェットは、ランチャーへの配置を伴うため、このテストには含みません。
エミュレータで、本物のAPIサーバーとmatterjs-server互換のmockを相手に、ウィジェットを配置して押して確かめます。
アプリは `homeserver` だけに平文HTTPを許可しているため、エミュレータのHTTPプロキシをAPIサーバーへ向けると、
既定の接続先のままのAPKで試せます（APIサーバーはプロキシ形式の要求もそのまま処理します）。

```sh
adb -s emulator-5554 shell settings put global http_proxy 10.0.2.2:PORT
# 確認後
adb -s emulator-5554 shell settings delete global http_proxy
```
fakeの成功は、実際のカメラでの読み取りや、homeserverへの接続を証明しません。

```sh
sdkmanager 'emulator' 'system-images;android-37.0;google_apis;x86_64'
avdmanager create avd -n home-link-api37 -k 'system-images;android-37.0;google_apis;x86_64' -d pixel_9
"$ANDROID_HOME/emulator/emulator" -avd home-link-api37
```

エミュレータの起動後、別のターミナルの `android/` で実行します。`emulator-5554` は `adb devices` で確認した識別子に置き換えます。

```sh
ANDROID_SERIAL=emulator-5554 ./gradlew :app:connectedDebugAndroidTest \
  -Pandroid.testInstrumentationRunnerArguments.capturePrefix=light-
```

画像と文字の配置は、エミュレータの `/data/local/tmp/<capturePrefix><場面>.png` と `.json` に保存されます。
暗色、文字拡大と狭い画面は、試験前に次の設定をしてから同じテストを実行します。

```sh
adb -s emulator-5554 shell cmd uimode night yes
adb -s emulator-5554 shell wm size 840x1680
adb -s emulator-5554 shell wm density 420
adb -s emulator-5554 shell settings put system font_scale 2.0
```

試験前の設定を控え、終了後はその値へ戻してください。既定値へ戻す場合は次のとおりです。

```sh
adb -s emulator-5554 shell cmd uimode night no
adb -s emulator-5554 shell wm size reset
adb -s emulator-5554 shell wm density reset
adb -s emulator-5554 shell settings put system font_scale 1.0
```

## ADBでのインストール

端末の開発者向けオプションで「ワイヤレスデバッグ」をONにし、表示されたIPアドレスとポートでペア設定します。
ペア設定用のポートと接続用のポートは別です。

```sh
adb pair IP:PAIR_PORT
adb connect IP:CONNECT_PORT
adb devices -l
```

[リリース](https://github.com/miyabisun/home-link/releases/latest)からAPKと `SHA256SUMS` を同じフォルダーへ保存します。
`X.Y.Z` は取得した版、`DEVICE_SERIAL` は `adb devices -l` で確認した端末の識別子に置き換えます。

```sh
sha256sum -c SHA256SUMS
adb -s DEVICE_SERIAL install -r home-link-vX.Y.Z.apk
adb -s DEVICE_SERIAL shell am start -n dev.miyabisun.homelink/.MainActivity
```

`-r` は同じ署名のアプリをデータを保持したまま更新します。署名の異なるAPKからは上書きできません。
リリースの `release-info.json` に、ソースのcommit、APKのSHA-256、署名証明書の情報があります。

## リリース

`v` で始まるSemVerのタグをpushすると、`.github/workflows/release.yml` がタグと `Cargo.toml` の版の一致を確認します。
その後、`ghcr.io/miyabisun/home-link:<版>` と `latest` を公開し、GitHub Releaseを作成します。

APKはCIでは作りません。署名鍵をリポジトリやCIへ置かないためです。
同じタグのソースから検証済みのdebug APKを作り、次の3つをそのGitHub Releaseへ追加します。

- `home-link-vX.Y.Z.apk`
- `SHA256SUMS`
- `release-info.json`（ソースのcommit、APKのSHA-256、署名証明書のSHA-256）

`android/app/build.gradle.kts` の `versionName` はCargoの版と揃え、`versionCode` はリリースごとに増やします。
