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
`src/commission.rs` はmatterjs-serverへWi-Fi情報を渡して `commission_with_code` で機器を登録する処理と、
失敗の文言（matter.jsの例外の文）を見つからない・コード違い・Wi-Fi・時間切れに分ける処理、
登録した機器のBasic InformationとWi-FiのMACアドレスの取り出しと、それらの単体テストを持ちます。
機器登録APIのテストは、`tests/api.rs` の中で `set_wifi_credentials`・`commission_with_code`・`read_attribute` に応答するmockを立て、
成功・各失敗・時間切れ・台帳の重複・入力の検証と、データベースのファイルと応答にWi-Fiのパスワードが残らないことを検証します。
`src/schedule.rs` は自動調整の目標値（日の出・日の入り、朝・昼・夕・夜の境界と線形の補間、照明ごとの下限と範囲への丸め）と、
照明ごとに送るか・送らない理由（前回と同じ値を含む）を決める純粋な処理と、そのテストを持ちます。
自動調整のテストは、同じmockに `read_attribute` と明るさ・色温度の命令を受けさせ、点いている照明だけに書き込むこと、
台帳の下限が照明へ届くこと、「全部オフ」の後は再起動をまたいで10分ごとに1日分実行しても「全部オン」まで何も送らないことを検証します。

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
Bluetoothで機器を登録するときのmatterjs-serverのBLE Proxyは既定で `ws://192.168.1.100:5580/ble` で、`-PbleProxyUrl=ws://…` で変えられます。
平文HTTPを許可するホストは `app/src/main/res/xml/network_security_config.xml` に書いています。
ホスト名を変える場合は、このファイルも合わせて変更してください。

JVMテストは、JDKの一時HTTPサーバーを相手にAPIクライアントを検証します。入力の状態の扱いも単体で検証します。
`BleProxyTest` は、OkHttpのMockWebServerでmatterjs-serverの `/ble` を、fakeでBLEの層を置き換え、
BLE Proxy Protocol v1のhello、各命令とその失敗、`device_discovered` などのevent、binary frame（`WRITE_DATA` と `NOTIFICATION`）を検証します。
pingへのpongはOkHttpが自動で返します（実サーバーへ75秒接続して切られないことを確認済み）。
実際のBluetoothを使う `AndroidBle` と、Keystoreに保存するWi-Fi情報は、エミュレータと実機で確かめます。

### 画面の検証

instrumentationテストは、APIとQRスキャナーをfakeに置き換え、実際の画面部品を操作します。
部屋の作成・選択・登録・成功表示と、登録済み・部屋なし・接続不可・Matter以外のQR・画面の再生成を扱います。
Bluetoothでの登録は、BLEの中継をfakeに置き換え、Wi-Fiの設定、段階の表示、画面の再生成をまたいだ進行、成功、
各失敗（電球が見つからない・コード違い・Wi-Fi・時間切れ・matterjs-serverに接続できないなど）での入力の保持を扱います。
保存したWi-Fiの一覧は、今つながっているWi-Fiの読み取りをfakeにして、選択・最後に選んだものの保持・追加・同じ名前の上書き・削除、
今のSSIDの自動入力、5GHzの表示、Wi-Fiにつながっていない・SSIDを読めない場合の表示を扱います。
Keystoreに暗号化して保存したWi-Fi情報を読み戻せること、平文で残らないことも同じテストで確かめます。
一覧の操作と保存形式（0.1.13までの1件の形式の読み込みを含む）、2.4GHzの判定は `SavedWifiTest` で検証します。
実際のSSIDの読み取り（位置情報の許可ダイアログを含む）は、エミュレータの実APKで「AndroidWifi」が入ることで確かめます。
照明のボタンは、送信中・全部成功・一部の照明が残った場合・接続不可の表示と、画面の再生成での結果の保持を扱います。
ホーム画面のウィジェットは、ランチャーへの配置を伴うため、このテストには含みません。
エミュレータで、本物のAPIサーバーとmatterjs-server互換のmockを相手に、ウィジェットを配置して押して確かめます。
1段と縦に広げた表示、横4マスと2マスの幅で、電話とタブレット（Lenovo Y700相当の1600×2560・320dpi）のランチャーに置いて確かめます。
1段の短い結果の文言は `LightsMessageTest` で検証します。
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

アプリはAndroid 16（API 36）以降に対応します。Android 16でも同じテストを実行するには、
`system-images;android-36;google_apis;x86_64` で `home-link-api36` を作って起動します。

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
