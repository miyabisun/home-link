# 開発ガイド

ソースを取得し、リポジトリへ移動します。以降のコマンドは、特に断りがなければこのディレクトリで実行します。

```sh
git clone https://github.com/miyabisun/home-link.git
cd home-link
```

利用者向けの説明は[README](../README.md)、Webの管理画面とAndroidアプリの画面の規則は[DESIGN.md](../DESIGN.md)にあります。

## APIサーバー

Rustのツールチェーンは `rust-toolchain.toml` の版を `rustup` が自動で選びます。
サーバーはWebの管理画面（`client/dist`）を組み込んでビルドするので、先に画面をビルドします（Node.js 24）。
`client/dist` が無いと `build.rs` がビルドを止めます。

```sh
npm --prefix client ci
npm --prefix client run build
cargo run --locked
```

`http://127.0.0.1:3000` で待ち受け、カレントディレクトリの `home-link.db` にデータを保存します。
変更後は、commitの前に次の検証を手元で通してください。CIは検証を実行しません。

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked
scripts/smoke.sh
```

`scripts/smoke.sh` は、ビルドした `target/debug/home-link` を一時ディレクトリへ写して起動し、
`/healthz`・`/api/health`（`Cargo.toml` の版）・labelの作成・404・組み込んだ画面とJSが返ることをHTTPで確かめます。

`tests/api.rs` はメモリ上のSQLiteでAPIの要求と応答を検証します。
照明の接続の見張りは、判断（いつ切れたと数え、つなぎ直し、知らせるか）を `src/watch.rs` の副作用の無い `Watch::step` に分け、
1分ずつ時刻を進める列の単体テストで確かめます。matterjs-serverへの問い合わせ・Discordへの送信・履歴の書き込みは `src/watcher.rs` が行い、
`tests/watch.rs` が、mockのmatterjs-serverとmockのDiscordのWebhookを相手に `Home::check_lights` を時刻を進めて呼び、
見張り→つなぎ直し→知らせ→履歴のAPIを通して確かめます。
`src/onboarding.rs` はMatterのQRコード（Base38）と手動ペアリングコード（Verhoeff）の検証、
同じ機器かを判定するキーの取り出しと、そのテストを持ちます。
`src/link.rs` はmatterjs-serverとの通信の層です。`Link` は接続・送信・応答待ち・切断のすべてに期限を持ちます。
機器への要求の応答は、1つの関数 `Outcome::of` で分けます。
分類は「受け付けた」「送っていない」「答えが無い」「matterjs-serverのエラー」「機器の拒否」です。
matterjs-serverと話す処理（照明・プラグ・エアコン・自動調光・登録・Wi-Fiの移動）はすべてこの層を使い、`tokio_tungstenite` を直接使いません。
`src/matter.rs` はこの層でmatterjs-serverのWebSocket APIから全nodeを読み、識別子からnodeとendpointを引きます。
照明の選び方（Descriptorのdevice typeとOn/Off、ブリッジ配下の展開、届くかどうか）も同じファイルで単体テストします。
状態APIと照明APIのテストは、`tests/api.rs` の中でmatterjs-serverと同じ形で `get_nodes` と `device_command` に応答する
WebSocketサーバーを立て、全部オン・全部オフ、エラーや無応答の照明、最初のオンに応答しない照明への送り直し、接続断からの再接続、
遅れて届く応答と、無応答の照明があっても他の照明の結果が3秒以内に返ることを検証します。
無応答の照明のテストは、応答を待つ2.5秒の期限まで待ちます。
通信の層の期限は、TCPは受けるがWebSocketのupgradeに答えないmock（`hanging`）で確かめます。
全部オン・label・プラグ・登録・エアコン・Wi-Fiの移動と、1分ごとのlabelの予定・10分ごとの調整が、期限内に終わります。
全部の応答を2.9秒遅らせるmock（`slow_ms`）では、labelのONと全部オンが15秒の予算の内に返ります（約15秒かかります）。
結果の分け方は、次のmockで応答の項目とwarnのログを確かめます。

- `get_nodes` には答えて、命令に答えないmock
- `Node 5 is not connected` を返すmock（matterjs-serverのエラー）
- `status` 付きのエラーを返すmock（機器の拒否）
- 3秒遅れて答えるmock

部分の失敗で状態を壊さないことは、次のmockで確かめます。

- 最初の接続だけを通し、次の接続を止めるmock（`gated`）: labelのONで切り替えは届き、上書きの値を送れなくても、labelは `on` で保存され、照明ごとの結果が返る。
  matterjs-serverに届かないときは、全部オン・全部オフ・labelのOFF・解除のどれもlabelの `on` を変えない。
- 1台が答えないmock（`silent`）: その照明しか持たないlabelは、全部オフの後も元の `on` のまま。
- 照明の明るさ・色温度を外から変えるmock（`edits`、壁のスイッチでの入れ直しに当たる）: 次の調整で送り直し、自動調光連動でない照明には送らない。
  mockは送られた明るさと色温度を `8/0`・`768/7` に反映します。
- 最初の何回かの命令に答えないmock（`silent_for`）と `Home::with_schedule_retry`: 抜けた分の予定をまとめて実行し、答えなかった機器だけへ送り直す。
  抜けた分の求め方（`schedule::minutes_due`）は単体テストで確かめます。
- ファイルのDBを開き直す `Home` で、再起動をまたいだ予定を確かめます。止めている間の分は起動（`Home::start_schedules`）で実行し、11分以上なら実行しない。
  最初の送信の途中で止めたら起動後に送り直し、送り直しを待つ間に止めたら重ねて送らない（`tokio::spawn` した実行を `abort` して止める）。
- `Home::supervise` にpanicする処理を渡し、起こし直されて `/healthz` に出ること、終わった処理で503になることを確かめます。
- エアコンが答えないmockで、`POST /api/aircon/toggle` が `delivered: false` を返すことを確かめます。
- CANDY HOUSEの代わりのHTTPサーバーで、ChromeのUser-Agentと、答えないときに10秒で打ち切ることを確かめます。

要求のログは `capture_logs` で、起動時に温度計のアドレスを出さないことは `tests/port.rs` で実行ファイルを起動して確かめます。
機器の種類とプラグの入り切りは、同じmockに照明・プラグ（0x010A・0x010B）・ブリッジ配下のプラグ・センサーを並べ、
種類と入・切の値、そのnodeとendpointへのOn/Off、応答しない・使えないプラグ、照明と入り切りの無い機器の拒否、
全部オン・全部オフと自動調整がプラグへ命令を送らないことを検証します。
`src/commission.rs` はmatterjs-serverへ保存した機器用のWi-Fi情報を渡して（Threadでは渡さず、Thread網の有無を確かめて）
`commission_with_code` で機器を登録する処理と、
失敗をmatterjs-server（1.5.0から）の `reason` と、それが無い古い版では文言（matter.jsの例外の文）で、見つからない・コード違い・Wi-Fi・Thread・時間切れに分ける処理、
登録した機器のBasic InformationとWi-FiのMACアドレスの取り出し、接続済みの機器のWi-Fiの差し替え（fail-safeの下での付け替えと確定・戻り）と、それらの単体テストを持ちます。
機器登録APIのテストは、`tests/api.rs` の中で `set_wifi_credentials`・`commission_with_code`・`read_attribute` に応答するmockを立て、
成功・各失敗・時間切れ・台帳の重複・入力の検証を検証します。
Wi-FiのAPIは、保存・一覧・機器に渡すものの指定・削除と、一覧・応答・ログにパスワードが出ないこと、登録が機器に渡すWi-Fiを `set_wifi_credentials` で送ることを検証します。
mockの `server_info` の `thread_credentials_set` を切り替え、Threadでの登録の成功（Wi-Fi情報を送らないこと）、
Thread網が未準備のときの拒否と `GET /api/thread` も検証します。
Wi-Fiの差し替えは、`device_command` を機器のように受けるmockで、成功・ConnectNetworkの応答が届かない場合・新しいWi-Fiを拒んだ場合・
fail-safeの後に元のWi-Fiで応答する場合・応答が無い場合を検証し、パスワードが応答・ログ・Wi-Fiの表以外のDBに残らないことも確かめます。
`src/meter.rs` はSwitchBot CO2センサーの広告（サービスデータ `fd3d` とメーカーデータ）の読み取り、
受けた値を5分で古いとする扱い、機器の登録の間に受信を止めてBluetoothを譲る判断と、そのテストを持ちます。
BlueZのD-Busから受ける部分はテストしません。実際のセンサーとアダプタで、受信・登録の間の停止と再開を確かめます。
`src/aircon.rs` は、エアコンへ送る状態を決める処理と、書き込む順を持ちます。
送る状態は、温度計の温度・湿度、外気温、前に送った状態と覚えている値（補正・送った時刻・30分の室温・送った後の確かめ）から、
切か、冷房・暖房・除湿と設定温度のどれかに決めます。判断は副作用の無い関数で、時刻を10分ずつ進める列のテストで確かめます
（外気温ごとの入り切り、補正、30分の間隔、夜、送り直し、外気温が無いとき）。
書き込む順は、2つの設定温度を離したまま動かし、運転モード・風量・Onの順に続けます。
`src/outside.rs` は、国土地理院の住所検索とOpen-Meteoの応答の読み取りを、実際の応答の例でテストします。
エアコンのテストは、同じmockに `write_attribute` を受けさせ、Room Air Conditioner（0x0072）のnodeを置いて次を検証します。

- 種類と状態を読めること
- 温度計の値に応じた書き込みが、endpoint 1のThermostat・Fan Control・On/Offに届くこと
- 同じ状態を二度送らず、温度計の値が古いと送らないこと
- OFFが自動調整より優先し、再起動をまたいで保たれること。ONで入り、自動調整が再開すること
- labelの操作、照明の全部オン・全部オフ、自動調整、機器の入り切りがエアコンに届かないこと
- 国土地理院とOpen-Meteoの代わりをループバックに立て（`Home::with_outside`）、住所の登録・見つからない・届かない、
  登録した座標での外気温の取得とUser-Agent、外気温30℃で24.5℃でも暖房にしないこと、外気温が取れないときの室温だけの判断、
  14日分の記録の読み出し、照明の日の出・日の入りが登録した座標になること
- 0.1.47（`user_version` 10）のDBを開くと、変えてあった照明の座標が家の位置へ移ること

`src/schedule.rs` は自動調整の目標値（日の出・日の入り、朝・昼・夕・夜の境界と線形の補間、範囲への丸め）、
照明ごとに送るか・送らない理由（前回と同じ値を含む）、1台に付いたlabelから上書き・自動調整・対象外を決める規則、
スケジュールの時刻の読み取りを決める純粋な処理と、そのテストを持ちます。
自動調整のテストは、同じmockに `read_attribute` と明るさ・色温度の命令を受けさせ、自動調光連動のlabelの点いている照明だけに書き込むこと、
そのlabelのOFFの後は再起動をまたいで10分ごとに1日分実行してもONまで何も送らないこと、
labelのONや「全部オン」で受け付けた照明には、オンの報告が遅れていても読み直さずにその時刻の値を書くことを検証します。
labelの操作のテストは、照明とプラグ（ブリッジ配下を含む）へのON・OFF・切り替え、上書きの値での点灯と自動調整からの除外、
上書きをやめた照明がほかのlabelの状態（自動調整か消灯）へ戻ること、スケジュールが時計を差し替えた各時刻（0時をまたぐ場合を含む）にONとOFFを送ることを検証します。
v0.1.36（`user_version` 8）の形のDBを開き、labelと割り当てが残ることと、0.1.36のウィジェットとアプリが使う照明オン・照明オフ・「仕事」「作業」と自動調整が以前と同じ命令を送ることも確かめます。
MCPのテストは、APIサーバーをループバックで立て、HTTPのclientとしてJSON-RPCで初期化・道具の一覧・各道具の呼び出しと失敗を確かめます。

`tests/ui.rs` は、組み込んだ画面が `/` と画面のURLで返り、JSとCSSが正しい種類で返ること、`/api` と `/mcp` が画面へ流れないことを確かめます。

## Webの管理画面

画面は `client/` にあり、[rust-svelte-template](https://github.com/miyabi-sunny-side/rust-svelte-template) のSvelte 5（runes）とViteの構成から作りました。
APIを `cargo run` で3000番に立てたまま、Viteの開発サーバーで画面を開けます（`/api` は3000番へ中継します）。

```sh
npm --prefix client run dev
```

変更後は、commitの前に次の検証とE2Eテストを手元で通してください。

```sh
npm --prefix client run format:check
npm --prefix client run check
npm --prefix client test
npm --prefix client run lint:design
```

`client/src/lib/admin.ts` は、入力の検査（名前の長さ、固定の明るさ・色温度、スケジュールの時刻）と、
表示の組み立て（名前の無い機器の呼び名、状態、labelの要約、明るさの%と254段階の変換、入り切りの結果）を持ち、
`admin.test.ts` で単体テストします。

E2Eテストは、ChromiumとPlaywrightで、ビルドしたhome-link（`target/debug/home-link`）とmockのmatterjs-serverを相手に動きます。
`client/e2e/serve.mjs` が一時ディレクトリの新しいDBでhome-linkを起動し、照明とプラグを1台ずつ返すmockを立て、台帳に機器を入れます。
テストは画面を操作し、APIを通じてSQLiteのファイルに入った値を読んで確かめます（機器の名前・label・無視・削除、labelの作成・名前・調光・固定の値・スケジュール・機器・削除・入り切り）。
エアコンの画面は、mockのエアコンを相手に、入・切と目標の温度の変更がSQLiteに入ること、範囲外の目標を示して送らないことを確かめます。
`serve.mjs` は国土地理院とOpen-Meteoの代わりも立て（`GSI_URL`・`OPEN_METEO_URL`）、住所の登録の成功と失敗、外気温と理由の表示、夜の時刻の保存を確かめます。
照明の接続の画面は、`GET /api/lights/watch` と `GET /api/lights/outages` の応答をPlaywrightで差し替え、切れた照明・網から外れた照明・長い名前・matterjs-serverに届かない状態と14日の履歴を、幅1280と390、明暗の両方で確かめます。
温度計の画面は、E2Eのhome-linkに温度計が無いため、`GET /api/meter` の応答をPlaywrightで差し替えて、今の値・途絶え・未受信を確かめます。
応答しない機器の表示は、`GET /api/devices` とlabelの入り切りの応答をPlaywrightで差し替え、Threadの機器だけが応答しないときと、Wi-Fiの機器も応答しないときを、幅1280と390、明暗の両方で確かめます。
`screens.spec.ts` は幅1280と390の明暗で全画面を撮り（`client/test-results/screens/`）、横のはみ出しと本文の内側の縦スクロールが無いことを確かめます。

```sh
npm --prefix client run build
cargo build --locked
npx --prefix client playwright install chromium
npm --prefix client run test:e2e
```

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
接続先はAPKに埋め込みません。端末の `home-link.yaml`（[README](../README.md#接続先を設定する)）か、アプリの「接続先の設定」で決めます。
接続先のホストは決まっていないため、平文HTTPはすべてのホストに許可しています（`app/src/main/res/xml/network_security_config.xml`）。

JVMテストは、JDKの一時HTTPサーバーを相手にAPIクライアントを検証します。
接続先のYAMLの読み書きとURLの検査は `EndpointsTest` で検証します。予備のURLの一覧の3つの書き方と、予備の無い0.1.50までの形も扱います。
ファイルが無い・読めない・形が不正・正しいの4つの読み方、BOM、一時ファイルを経た保存と、保存に失敗したときに元のファイルが残ることも扱います。
要求の送り先の決め方は `LocalNetworkTest` で検証します。LANのアドレスの判定、Android 17のLANへの接続の許可が無いときにLANのURLを除くこと、
除いた結果どこにも届かなかったときに許可の無さとして示すことを扱います。
接続とWi-Fiの移動の進め方（1つずつ・取り消し・記録・プロセスの終了後の読み直し）は `DeviceWorkTest` と `RegisterFormTest`、
画面を離れても変更を最後まで送り、結果を次の画面へ渡すことは `OutboxTest` で検証します。
接続先が無いときに何も送らないことは `HttpHomeLinkApiTest` で検証します。入力の状態の扱いも単体で検証します。

要求の送り方は `Reach` が決め、`ReachTest` が時計と回線をfakeにして次を検証します。

- 送る前の失敗（名前解決・接続の拒否・接続の時間切れ）だけを送り直し、応答待ちの時間切れと送った後の切断は送り直さない
- 15秒を過ぎたら、止まった所（回線なし・通信制限・接続不可・時間切れ）を返す
- 主のURLが1秒で接続できないか先に失敗したら予備へも接続を始め、先に接続できた1つだけが送る

`HttpHomeLinkApiTest` は、実際のHTTPで次を検証します。

- 接続を断られた主と、受付待ちの列が埋まって接続できない主から、予備へ1回だけ送る
- 要求を受けて応答しない主から、予備へ送らない
- 前の要求の接続を使い回さない
- 401・403・404・502〜504・JSONでない応答を区別し、失敗をログに残す

ボタンの短い表示と読み上げの文は `ButtonFaceTest`・`LabelsTest`・`DoorMessageTest` で検証します。入・切・不明の状態の扱いは `LabelsTest` で検証します。
`BleProxyTest` は、OkHttpのMockWebServerでmatterjs-serverの `/ble` を、fakeでBLEの層を置き換え、
BLE Proxy Protocol v1のhello、各命令とその失敗、`device_discovered` などのevent、binary frame（`WRITE_DATA` と `NOTIFICATION`）を検証します。
pingへのpongはOkHttpが自動で返します（実サーバーへ75秒接続して切られないことを確認済み）。
実際のBluetoothを使う `AndroidBle` と、0.1.31までの版がKeystoreに保存したWi-Fi情報の移行は、エミュレータと実機で確かめます。

### 画面の検証

instrumentationテストは、APIとQRスキャナーをfakeに置き換え、実際の画面部品を操作します。
QRコードや11桁の数字を読むと台帳に入り機器の面が開くこと、同じコードで同じ機器の面が開くこと、Matter以外のQR、
3台を読んだ後の起動で未接続の一覧に3台が並び、上から1台ずつ接続でき、接続した機器が一覧から消えること、
名前・ラベルの変更がAPIへ届き、画面の再生成で同じ機器の面へ戻ることを扱います。
home-linkの応答を待つあいだに画面を作り直しても、名前・ラベルの変更が届いて新しい画面に結果が出ることも扱います。
接続先のファイルが無い・読めない・形が不正・URLの形の誤り・BOM付きの表示と、つながらない接続先で最初の失敗をすぐに示すこと（実際のHTTPで、25秒以内）も扱います。
Bluetoothでの接続は、BLEの中継をfakeに置き換え、機器に渡すWi-Fiが無いときの設定への入口、段階の表示、画面の再生成をまたいだ進行、成功、
各失敗（機器が見つからない・コード違い・Wi-Fi・Thread網・時間切れ・matterjs-serverに接続できないなど）の後も機器が面に残り、
アプリを再起動せずにもう一度押して接続できること、取り消しと、取り消した後に届いた結果を表示しないことを扱います。
一覧から選んだ機器の段階・結果・取り消しがその機器の面に出ること、接続中は一覧とQRの読み取りが出ないことも確かめます。
プラグの入り切りは、コードで開いた機器の面で、送信中・応答なし・失敗・画面の再生成と、照明には入り切りが出ないことを扱います。
登録の画面にlabelと接続済みの機器の一覧が無いこと、ヘッダーの右端のメニューから接続先の設定が開くことも扱います。
どの場面の画像でも、画面の文字に「電球」が無いこと、登録の画面ではどのボタンも面の中にあることを確かめます。
接続済みの機器のWi-Fiの差し替えは、機器の面のボタン、画面の再生成をまたいだ進行、成功、元のWi-Fiに戻った場合と応答が無い場合を扱います。
PATCHがAndroidのOkHttpから送られることは、ローカルのfakeのhome-linkで確かめます。
機器に渡すWi-Fiは、接続先の設定の画面で、今つながっているWi-Fiの読み取りをfakeにして、選択・追加・同じ名前の上書き・削除・保存と読み込みの失敗、
今のSSIDの自動入力、5GHzの表示、Wi-Fiにつながっていない・SSIDを読めない場合の表示を扱います。
電話に残ったWi-Fiがhome-linkへ一度だけ移り電話から消えること、移せなかったときは次の起動まで残すこと、登録の画面にWi-Fiの入力が出ないことも扱います。
保存形式の読み込み（0.1.13までの1件の形式を含む）と移す順序、2.4GHzの判定は `SavedWifiTest` で検証します。
実際のSSIDの読み取り（位置情報の許可ダイアログを含む）は、エミュレータの実APKで「AndroidWifi」が入ることで確かめます。
Keystoreからの移行は、0.1.31のAPKでWi-Fiを保存してから新しいAPKを上書きインストールし、fakeのhome-linkに一度だけ届くことで確かめます。
ウィジェットの設定の画面は、テストの中でAppWidgetHostからウィジェットのIDを取り（`appwidget grantbind`）、labelの選択と名前の既定、保存、
設定し直し、labelが無い場合、ほかのアプリのIDを断ることを扱います。
テストは開始時に架空の接続先を `home-link.yaml` へ書き、文言のホスト名をそこから読みます。
接続先が無いときの案内と送らないこと、設定の画面での入力の検査、YAMLへの保存、保存した接続先への次の要求（ループバックのfakeのhome-link）も扱います。
ホーム画面のウィジェットは、ランチャーへの配置を伴うため、このテストには含みません。
エミュレータで、fakeのhome-linkか本物のAPIサーバーとmatterjs-server互換のmockを相手に、ウィジェットを配置して押して確かめます。
labelのウィジェットは、4種類を置いて設定の画面でlabelと名前を選び、各ボタンの要求が届くこと、切り替えの状態の表示、
0.1.37までの照明のウィジェットを置いた端末へ上書きインストールしても落ちないことを確かめます。
すべてのウィジェットの押下は、短いフォアグラウンドサービス（`PressService`）から送ります。
Wi-Fiを切ってモバイル回線だけにし、データセーバーを入れます。

```sh
adb shell svc wifi disable
adb shell cmd netpolicy set restrict-background true
```

この状態で、各ウィジェットの要求がfakeのhome-linkへ1回だけ届くことを確かめます。
broadcast receiverから送ると、名前解決から止められて「接続不可」になります（0.1.50までの照明とエアコンのウィジェット）。
充電中のバッテリーセーバーは入らないため、充電を外してから入れます。
`dumpsys power` の「Battery Saver is currently: ON」を確かめてから押します。

```sh
adb shell dumpsys battery unplug
adb shell settings put global low_power 1
```
予備のURLは、主を応答の無いアドレス（`http://192.0.2.1:PORT`）に、予備をfakeにします。押してから約1秒で、予備へ1回だけ届くことを確かめます。
ほかに次も確かめます。

- 回線が無い状態（`svc data disable`）で押し、途中で戻すと1回だけ届く
- fakeが応答しないと「結果不明」になり、送り直さない
- 読み直しが失敗すると `dumpsys jobscheduler` の `WidgetRefresh` が回線を待ち、Wi-Fiが戻ると状態を読み直す
電話とタブレット（Lenovo Y700相当の1600×2560・320dpi）のランチャーに置いて確かめます。
ボタンの表示と短い結果の文言は `LabelsTest` と `ButtonFaceTest` で検証します。
エアコンのウィジェットは、テストの中でAppWidgetHostに置いて画面に描き、fakeのAPIを相手に確かめます。
読んだ状態の表示と、押して入と切が切り替わることを扱います。届かないときに短い結果を示し、元の表示へ戻ることも扱います。
入・切は保存されたがエアコンへ届かなかった応答（`delivered` が false）では「届かず」と理由の読み上げを示し、その後に新しい状態へ戻ることも扱います。
接続とWi-Fiの移動は、ホストのfakeのhome-linkが応答を40秒遅らせ、押した直後にホームへ戻って別のアプリを開き、画面を消して確かめます。
モバイル回線だけ＋データセーバー＋充電を外して `dumpsys deviceidle force-idle` の状態で、要求が最後まで届き、アプリへ戻ると結果が出ることを確かめます。
待つあいだ `dumpsys activity services dev.miyabisun.homelink` で `DeviceWorkService` が `isForeground=true`（connectedDevice）になります。
0.1.55まではこの条件で、home-linkが移動を終えても「時間内に応答がありませんでした」になりました。
待つあいだに `run-as dev.miyabisun.homelink kill -9` でプロセスを終わらせ、開き直すと「確かめています…」から結果へ変わることも確かめます。
Android 17では、LANのアドレスの接続先をadb pushで入れた端末で、画面の「LANへの接続を許可」とウィジェットの「LAN未許可」が出ること、
許可の前はfakeへ何も届かず、許可の後に届くことを確かめます（許可は `pm revoke … android.permission.ACCESS_LOCAL_NETWORK` で戻せます）。
エミュレータでは、ホストのfakeのhome-linkやAPIサーバーを接続先にします（エミュレータからホストは `10.0.2.2`）。

```sh
printf 'home_link_url: "http://10.0.2.2:PORT"\n' > home-link.yaml
adb -s emulator-5554 push home-link.yaml /sdcard/Android/data/dev.miyabisun.homelink/files/home-link.yaml
```

Android 17は、LANのアドレス（`10.0.2.2` を含む）への接続に付近のデバイスへの接続の許可を要します。
アプリの「接続先の設定」で保存すると求められます。エミュレータに大域のHTTPプロキシ（`global_http_proxy_host`）が残っていると、
接続先にかかわらずプロキシへ送られるので、試験の前に `adb shell settings list global | grep proxy` で確かめてください。
fakeの成功は、実際のカメラでの読み取りや、実際のhome-linkへの接続を証明しません。

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

アプリの `versionName` と `versionCode` は、`android/app/build.gradle.kts` が `Cargo.toml` の版から作ります。
`versionCode` は 主×1000000＋副×1000＋patch です。リリースで書き換えるのは `Cargo.toml` だけです。
