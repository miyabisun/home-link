# home-link

住まいの機器を、名前とlabelで記録するサービスです。
照明とスマートプラグにlabel（「ambient」「work」など）を付け、labelごとにON・OFF・切り替え、
明るさと色温度の自動調整への連動、ONの間の明るさの固定、毎日の時刻でのON・OFFを設定できます。
機器とlabelの管理（名前の変更、labelの作成と割り当て、自動調光連動・固定の値・スケジュールの設定、台帳からの削除）は、
home-linkのサーバーが出すWebの管理画面で行います。labelの操作と設定は、APIのほかMCP（AIエージェント向けの道具）からも行えます。
スマートプラグなど照明以外のMatterの機器は、1台ずつ入り切りもできます。
部屋のSwitchBot CO2センサー（温湿度計）の温度・湿度・CO2を、サーバーのBluetoothで受けて画面とAPIで示します。
同じ部屋のMatterのエアコン（Room Air Conditioner）へは、home-linkが運転モードと設定温度を送ります。
温度計が目標の温度（既定25℃）の前後になるように、登録した住所の外気温も見て決めます。
長く家を空けるときは、ホーム画面のボタンで切り、帰る前に入れ直せます。
玄関のSESAME 5なども、ホーム画面のボタン1つで家の外から解錠・施錠できます（CANDY HOUSEのWeb API経由）。
機器に印刷されたMatterのQRコードをAndroidアプリで読み取り、名前とlabelを付けて登録します。
QRコードが無い機器は、印字された11桁の数字（Matterの手動ペアリングコード）を入力して登録します。
新しいWi-FiとThreadの機器（Aqara T2など）は、電話のBluetoothで直接つなぎ、家のWi-FiかThread網とmatterjs-serverへ入れてから台帳に登録できます。
登録した内容は、自宅サーバーで動くhome-linkのAPIがSQLiteに保存します。
機器自身の識別子（ベンダー名とシリアル番号、Wi-Fi機器はMACアドレス）も保存でき、
Matterのcontroller（[matterjs-server](https://github.com/matter-js/matterjs-server)）を作り直しても、
台帳の機器が今どのnodeにいるかを識別子から引き直します。

構成は、Rust（axum）のAPIサーバーと、それに組み込んだSvelteのWebの管理画面、Kotlinで作ったAndroidアプリです。
照明とプラグはlabelの単位で操作します（下の「label」）。
機器の種類（照明・プラグ・エアコン・その他）は、Matterのdevice typeからhome-linkが求めます。
プラグは、そのプラグを入れたlabelの操作と、1台ずつの入り切りでだけ切り替わります。
照明の全部オン・全部オフと自動調整は、冷蔵庫などをつないだプラグを切りません。
Matterの機器登録（commissioning）はmatterjs-serverが行い、home-linkのアプリは電話のBluetoothを貸すだけです。

## 公開範囲と認証

APIに認証はありません。家庭LANやVPNなど、信頼できる経路にだけ公開してください。
Webの管理画面（`/`）とMCP（`/mcp`）もAPIと同じポートにあり、同じく認証はありません。
インターネットへ直接公開しないでください。
MatterのQRコードと手動ペアリングコードには、機器のsetup passcodeが含まれます。
そのためコードの全文は機器1台の取得APIだけが返し、一覧APIとログには出しません。

解錠と施錠のAPI（`POST /api/door/unlock`・`/lock`）も、ほかのAPIと同じく送信元を問わず受け付けます。
APIに届く経路にいる機器なら、どれからでも玄関を開けられます。公開する範囲はこれを踏まえて決めてください。
CANDY HOUSEのAPIキーとSESAMEの秘密鍵は、データベース・ログ・応答には出しません。

機器に渡すWi-FiのSSIDとパスワードは、home-linkのデータベースに平文で保存します。
データベースファイルとそのバックアップ（R2などへのコピーを含む）には、Wi-Fiのパスワードが入ります。
APIはパスワードを書き込むだけで、一覧・応答・ログには出しません。
matterjs-serverへは登録と差し替えのときだけ渡します。終わったら、matterjs-serverが保存した分を `remove_wifi_credentials` で消します。

## サーバーの起動

[リリース](https://github.com/miyabisun/home-link/releases/latest)ごとに、
コンテナイメージ `ghcr.io/miyabisun/home-link` を公開しています。
データベースはコンテナ内の `/data` に置かれるので、ボリュームを割り当てます。

```sh
docker run -d --name home-link -p 5011:3000 -v home-link-data:/data ghcr.io/miyabisun/home-link:latest
curl http://127.0.0.1:5011/healthz
```

`ok` が返れば起動しています。
イメージは uid 10001 で動きます。ボリュームの代わりにホストのディレクトリを割り当てる場合は、
`--user` で実行ユーザーを指定し、そのユーザーが書き込めるディレクトリを使ってください。

| 環境変数 | 既定値 | 内容 |
| --- | --- | --- |
| `PORT` | `3000` | 待ち受けるTCPポート。1〜65535の10進数で、不正な値では起動しません。 |
| `DATABASE_PATH` | `home-link.db`（コンテナでは `/data/home-link.db`） | SQLiteデータベースのファイル。無ければ作成します。 |
| `LOG_LEVEL` | `info` | `off`・`error`・`warn`・`info`・`debug`・`trace` のいずれか。 |
| `MATTER_SERVER_URL` | なし | matterjs-serverのWebSocket API（例: `ws://192.0.2.10:5580/ws`）。未設定なら状態API・照明API・機器のBluetooth登録は503を返します。 |
| `SESAME_API_KEY` | なし | CANDY HOUSEのWeb APIのAPIキー（[CANDY HOUSEのパートナー向けページ](https://partners.candyhouse.co/)で発行）。 |
| `SESAME_UUID` | なし | 解錠するSESAMEのUUID（SESAMEアプリの機器の画面にあります）。 |
| `SESAME_SECRET_KEY` | なし | SESAMEの秘密鍵（32桁の16進数）。SESAMEアプリで共有する鍵のQRコードを上のページで読み取ると得られます。 |
| `SWITCHBOT_METER_ADDRESS` | なし | 受信するSwitchBot CO2センサー（W4900010）のBluetoothアドレス（例: `AA:BB:CC:DD:EE:01`）。未設定なら受信しません。 |
| `GSI_URL` | `https://msearch.gsi.go.jp/address-search/AddressSearch` | 住所から座標を引く国土地理院の住所検索API。試験で差し替えるときだけ設定します。 |
| `OPEN_METEO_URL` | `https://api.open-meteo.com/v1/forecast` | 外気温を取るOpen-MeteoのAPI。試験で差し替えるときだけ設定します。 |

`SESAME_` の3つは、解錠と施錠のAPIだけが使います。どれかが無いか形式が違うと、この2つは503 `sesame_not_configured` を返し、ほかの機能はそのまま動きます。
SESAMEはWi-Fiモジュールでインターネットにつながっている必要があります。

温度計は、センサーが流し続けるBluetoothの広告を、接続せずにホストのBlueZ（システムのD-Bus）から受けます。
コンテナにはホストのD-Busのsocketを渡します。BluetoothのアダプタはBlueZが扱い、コンテナへ渡す必要はありません。

```sh
docker run -d --name home-link -p 5011:3000 -v home-link-data:/data \
  -v /run/dbus/system_bus_socket:/run/dbus/system_bus_socket \
  -e SWITCHBOT_METER_ADDRESS=AA:BB:CC:DD:EE:01 ghcr.io/miyabisun/home-link:latest
```

- 受けた値は5分間だけ今の値として扱い、それより古い値は返しません（受けた時刻は残します）。
- アダプタが外れたり、BlueZやD-Busに届かなかったりしても、ほかの機能はそのまま動き、1分ごとに受信をやり直します。
- 機器の登録（`POST /api/commission` など）の間は受信を止め、BluetoothをBLE Proxyに譲ります。終わると受信に戻ります。

## Webの管理画面

サーバーのポートをブラウザで開きます。上の例なら `http://127.0.0.1:5011/`、家のLANやVPN（tailnetなど）の出先からは
サーバーに届く名前で `http://home-link.example:5011/` のように開きます。画面はサーバーのbinaryに組み込まれていて、別に配置するものはありません。

| 画面 | できること |
| --- | --- |
| 機器（最初の画面） | 台帳の機器の一覧。名前、付いているlabel、入っているプラグと、未接続・応答なし・無視の機器はその状態を示します。選ぶと機器の画面を開きます |
| 機器の画面 | 名前の変更、labelの割り当て（チェックで付け外し）、無視の設定、プラグの入り切り、台帳からの削除（確認してから） |
| ラベル（右上のメニュー） | labelの一覧と作成。一覧は固定・自動調光の別、機器の台数、入っているかを示します。名前を入れて「追加」で作り、そのlabelの画面を開きます |
| labelの画面 | 名前の変更、入り切り、調光（なし・自動調光・固定）、固定の明るさ（%）と色温度（K）、毎日の時刻での入・切、機器の割り当て、削除（確認してから） |
| 温度計（右上のメニュー） | 温度計の今の温度・湿度・CO2・電池と受けた時刻。受信が途絶えたときは、最後に受けた時刻だけを示します |
| エアコン（右上のメニュー） | 目標の温度（18〜30℃）の変更、自動調整の入・切、室温・湿度・外気温とその時刻、エアコンの今の運転（「冷房 25℃」「切」など）、直近の判断の理由。「設定」に家の住所の登録と夜の時間帯 |

変えた値はその場でAPIへ送り、保存ボタンはありません。値の誤りはその入力の下に、サーバーが受け付けなかった理由は画面の上部に示します。
テーマは右上のメニューの「テーマ設定」で、自動（OSに従う）・ライト・ダークから選べます。
機器の登録（QRコードの読み取りとBluetoothでの接続）は、電話のカメラとBluetoothを使うため、Androidアプリで行います。

## Androidアプリ

Android 16以降の端末で動きます。
アプリの画面は機器の登録だけを受け持ちます。機器とlabelの管理は上の「Webの管理画面」で行い、labelの入り切りはウィジェットで行います。
画面の上のヘッダーの右端のメニューから「接続先の設定」を開きます。
0.1.39までのアプリは0.1.40で無くなった部屋のAPIを読むため、起動すると接続できないと示します。この版へ更新してください。
接続先（home-linkのURLと、Bluetoothでの登録に使うmatterjs-serverのBLE ProxyのURL）は、アプリに埋め込まず端末に保存します。
家の外から使うなら、電話から届くURL（VPNの中だけで引ける名前など）を設定します。
QRの読み取りにはGoogle Playサービスのコードスキャナーを使うため、カメラの権限は求めません。
位置情報の許可は、接続先の設定の画面で機器に渡すWi-Fiを追加するときに、今つながっているWi-FiのSSIDを読むためだけに求めます。

[リリース](https://github.com/miyabisun/home-link/releases/latest)のAssetsから
`home-link-vX.Y.Z.apk` をダウンロードし、端末で開いてインストールします。
APKは開発用のdebug署名です。PCからADBで導入する方法と、ファイルの検証方法は
[開発ガイド](docs/development.md#adbでのインストール)を参照してください。

### 接続先を設定する

接続先が未設定のあいだ、アプリは何も送らず、画面の上部に「接続先が未設定です」と示します。
その横の「接続先を設定」か、ヘッダーの右端のメニューの「接続先の設定」を押し、URLを入れて「保存」を押します。
URLの形が正しくないと保存しません。保存した接続先は、アプリとウィジェットの次の要求から使われます。
Android 17では、家のLANのアドレスへ送るために、保存のときに付近のデバイスへの接続の許可を求めます。

設定は、端末の次のYAMLファイルに保存します。PCからADBで書き込むこともできます（root不要）。

```text
/sdcard/Android/data/dev.miyabisun.homelink/files/home-link.yaml
```

```yaml
home_link_url: "http://home-link.example:5011"
ble_proxy_url: "ws://192.0.2.10:5580/ble"
```

| キー | 内容 |
| --- | --- |
| `home_link_url` | home-linkのURL。`http://` か `https://` で始めます。照明・登録・玄関の要求を送ります |
| `ble_proxy_url` | matterjs-serverのBLE ProxyのURL。`ws://` か `wss://` で始めます。Bluetoothで登録しないなら空でかまいません |

```sh
adb -s DEVICE_SERIAL push home-link.yaml /sdcard/Android/data/dev.miyabisun.homelink/files/home-link.yaml
```

アプリは要求を送るたびにこのファイルを読むので、書き換えた後に再起動は要りません。
アプリを入れ直すとファイルも消えます。

### labelのウィジェットを置く

ホーム画面のウィジェットの一覧から、home-linkの次の4つを置けます。置くときに、送り先のlabelとウィジェットに出す名前を決める画面が開きます。
名前の既定はlabelの名前です。空欄にするとlabelの名前を使います。置いた後も、ウィジェットの長押しの設定から変えられます。

| ウィジェット | 大きさ | 押すと |
| --- | --- | --- |
| オン/オフ | 横2マス | 左の「<名前>オン」でlabelをON、右の「<名前>オフ」でOFF |
| オン | 1マス | labelをON |
| オフ | 1マス | labelをOFF |
| 切り替え | 1マス | labelを切り替え（`/toggle`）。labelがONの間は「<名前>中」とアクセントの面で示します |

例えば、label「work」の切り替えに「仕事」、label「game」の切り替えに「ゲーム」と名前を付けて置きます。
押したボタンの文字は「送信中…」のあと、数秒だけ結果（「オン完了」「オフ完了」「一部失敗」「失敗」「接続不可」「ラベルなし」「機器なし」など）に変わります。
切り替えは、すべての機器を切り替えられたときは新しい状態をすぐに示します。切り替えの状態は、置いたとき、押した後、30分ごとにhome-linkから読み直します。

0.1.37までの照明のウィジェット（照明オン・照明オフ・仕事・作業）は、アプリを更新するとホーム画面から消えます。上のウィジェットを置き直してください。

### エアコンを止めて戻す

ホーム画面のウィジェットの一覧から、home-linkの「エアコン」（1×1マス）を置きます。
押すたびに、エアコンの自動調整の入・切を切り替えます。

- 切: エアコンを切り、入れ直すまで自動調整を止めます。長く家を空ける間に使います。
- 入: 自動調整を再開し、その時の室温に合わせてエアコンを入れます。入っている間は「エアコン中」とアクセントの面で示します。

入・切はhome-linkが保存し、サーバーを再起動しても保ちます。状態は、置いたとき、押した後、30分ごとにhome-linkから読み直します。
押してhome-linkに届かなかったときは、数秒だけ「接続不可」などを示してから元の表示に戻ります。

### 玄関を解錠・施錠する

ホーム画面のウィジェットの一覧から、home-linkの「玄関の解錠」と「玄関の施錠」（どちらも1×1マス）を置きます。
labelのウィジェットとは別にしてあり、labelのボタンを押し間違えて玄関が開くことはありません。
ボタンを押すと、home-linkを通じてCANDY HOUSEのWeb APIへ解錠または施錠を送ります。
SESAMEアプリの履歴には `home-link` の名前で記録されます。

押したあとの送信は、短いフォアグラウンドサービスで行います。
データセーバーなどで裏の通信が止められていても、モバイル回線から届きます。
そのとき、通知欄に送信中の通知が一瞬出ることがあります。

ボタンの文字は「送信中…」のあと、数秒だけ次の結果に変わります。

| 表示 | 意味 |
| --- | --- |
| 受付済み | CANDY HOUSEが指示を受け付けました。扉が開いた・閉まったことの確認ではありません |
| 接続先未設定 | アプリに接続先がありません。上の「接続先を設定する」で設定してください |
| 接続不可 | home-linkに届きません。接続先の設定と通信を確かめてください |
| 時間切れ | 15秒以内にhome-linkへつながらないか、20秒以内に応答がありませんでした。接続先の設定と通信を確かめてください |
| 未設定 | サーバーに `SESAME_` の環境変数がありません |
| 拒否 | CANDY HOUSEが受け付けませんでした。APIキー・UUID・秘密鍵を確かめてください |
| API不通 | home-linkからCANDY HOUSEに届きません |

### 機器を登録してBluetoothでつなぐ

新品や初期化した、ペアリング待ちのMatterの機器（Wi-FiのTapo・BEAMTEC、ThreadのAqara T2など）を、Aqara Homeを使わずに登録できます。
前のルーター向けに設定されて今つながらない機器も、初期化すれば同じ手順で今のWi-Fiへ入れられます。
Zigbeeの機器は対象外です（親機が要るため）。

電話がBluetoothでmatterjs-serverの[BLE Proxy](https://github.com/matter-js/matterjs-server/blob/main/docs/ble-proxy-protocol.md)
（接続先の `ble_proxy_url`）の中継を務め、Matterの手順（暗号化、Wi-FiかThread網の情報の送付、fabricへの参加）はmatterjs-serverが行います。
Wi-FiかThreadかは機器の申告からmatterjs-serverが決めるので、アプリでは選びません。
Thread網の資格情報（dataset）は、matterjs-serverに `set_thread_dataset` で登録したものを使います。
接続するときは電話を家のWi-Fiにつなぎ、機器の近くに置きます。matterjs-serverはBluetoothを有効にして（BLE Proxyを受け付けて）起動しておく必要があります。

1. 初回だけ、機器に渡すWi-Fiをhome-linkに保存します。画面に「機器に渡すWi-Fi」が出ていたら「Wi-Fiを設定」を押し、
   接続先の設定の画面で「Wi-Fiを追加」を押します。今つながっているWi-FiのSSIDが入るので、パスワードを入力して保存します。
   SSIDを読むために、Androidの仕様で位置情報（正確な位置）の許可を求めます。許可しない場合はSSIDを入力します。
   2.4GHzのWi-Fiにしかつながらない機器も多くあります。今のWi-Fiが5GHzなら、追加の画面にそう表示されます。
   Wi-Fiは複数保存でき、セレクトボックスで選んだものを機器に渡します。要らなくなったものは「このWi-Fiを削除」で消せます。
   0.1.31までの版がこの電話に保存したWi-Fiは、更新後の初回起動で一度だけhome-linkへ移し、電話からは消します。
   保存したWi-Fiは、どの電話から接続しても使われます。登録の画面ではWi-Fiを選んだり入力したりしません。
2. 「QRを読み取る」で機器のQRコードを読みます。QRコードが無い機器は「数字で入力」で、機器に印字された11桁の数字を入力します。
   読んだ時点で機器はhome-linkの台帳に入り、その機器の面（コード・接続の状態・機器名・ラベル・「接続」）が開きます。
   台帳にある機器のコードを読むと、新しく入れずにその機器の面を開きます。
   機器名・ラベルは、変えるとすぐhome-linkに送ります。
3. 機器をペアリング待ちにし（BEAMTECは電源のオフ・オンを5回、Aqara T2は1秒間隔で10回くり返します。ほかはメーカーの手順に従います）、「接続」を押します。
   初回は「付近のデバイス」の許可を求めます。Android 17では、家のネットワークにあるmatterjs-serverへの接続もこの許可に含まれます。
   Bluetoothがオフならオンにするよう求めます。

アプリを開くと、台帳のうち接続していない機器が「未接続」に並びます（名前が無い機器は印字されたコードで示します）。
押した機器は一覧のすぐ上の機器の面に開き、そのまま接続を始めます。
例えば3台の機器のQRコードを先にすべて読んでおき、取り付けてから上から1台ずつ押して接続できます。接続できた機器は一覧から消えます。

接続の進み具合・結果・操作は、すべてその機器の面の中に出ます。
接続中は、機器を探している・接続している・コードと設定を送っている、の段階を表示し、「キャンセル」だけを出します。
「キャンセル」を押すと、Bluetoothの中継を閉じてすぐに接続の前へ戻ります。後から届いた結果は表示しません。
機器が見つからない、コードが違う、Wi-FiやThread網につながらない、時間切れ、matterjs-serverに接続できない、はそれぞれ理由と次にすることを表示します。
接続できなかった機器は面に残り、機器の電源を入れ直して面の「接続」を押せば、アプリの再起動やQRの読み直しをせずに試し直せます。

### プラグを入り切りする

接続済みのプラグのコードを読むと、機器の面の接続の状態の横に入り切りのボタンが出ます。
今の状態（「入」「切」）を示し、押すと反対に切り替えます。機器が応答しないときは「応答なし」と示し、押すと試し直します。

### 接続済みの機器を別のWi-Fiへ移す

接続済みのWi-Fiの機器は、初期化せずに、機器に渡すWi-Fi（接続先の設定の画面で選んだもの）へ移せます。
機器のコードを読んでその機器の面を開き、「「Wi-Fiの名前」へ移す」を押します。
このボタンは、機器が今つながっているWi-Fiが機器に渡すWi-Fiと違うときだけ出ます。
移すのには数分かかることがあります。
移した先のWi-Fiにつながらなかった機器は、自分で元のWi-Fiへ戻ります。
アプリは、元のWi-Fiに戻ったか、まだ応答が無いかを示し、機器の接続の状態を読み直します。
移した先のWi-Fiからmatterjs-serverへ届かない（家のネットワークから切り離したIoT用のWi-Fiなど）場合も、機器は元のWi-Fiへ戻ります。

## API

すべてJSONです。失敗時は `{"error": "コード", "message": "説明"}` を返します。

| メソッドとパス | 内容 | 主な失敗 |
| --- | --- | --- |
| `GET /api/devices` | 機器の一覧。接続の状態 `state`（`unconnected`・`connected`・`no_response`）と製品の情報を含み、コードは含みません。接続したことのある機器には、Matterのdevice typeから求めた種類 `kind`（`light`・`plug`・`aircon`・`other`）を付けます。照明とエアコン以外で入り切りできる機器には、最後に読んだ入・切 `on`（`true`・`false`）を付けます | |
| `POST /api/devices` | QRコードか11桁のコードだけで機器を台帳に入れます。本文は `{"qr_payload": "MT:…"}` か `{"manual_code": "…"}`。新しい行は201、同じ機器の行があれば作らずに200で返します（`registered`） | 400 `missing_setup_code`・`invalid_qr_payload`・`invalid_manual_code` |
| `GET /api/devices/{id}` | 機器1台。一覧の項目に、登録したコードの全文 `qr_payload` または `manual_code` を加えます | 404 `device_not_found` |
| `PATCH /api/devices/{id}` | 機器の名前・labelの割り当て・無視の設定。本文は送った項目だけを変えます。`{"name": "押入れ1"}`、`{"label_ids": [1, 2]}`（割り当てを置き換え、`[]` で外す）、`{"ignored": true}`（`false` で戻す） | 400 `invalid_device_update`・`invalid_device_name`、404 `device_not_found`・`label_not_found` |
| `POST /api/devices/{id}/commission` | 台帳の機器を、記録したコードでmatterjs-serverへBluetoothで登録（commissioning）し、その行に識別子を補います。本文は要りません | 404 `device_not_found`、409 `no_setup_code`、422 `device_not_found`・`wrong_code`・`wifi_failed`・`thread_failed`・`commission_failed`、504 `commission_timeout`、502 `matter_server_unreachable`、503 `matter_server_not_configured`・`bluetooth_unavailable` |
| `POST /api/devices/{id}/wifi` | 接続済みのWi-Fiの機器を、初期化せずに機器に渡すWi-Fi（`for_devices`）へ移します。本文は要りません。既にそのWi-Fiにいれば何も送りません。成功すると `{"device": {…}}` を200で返します | 404 `device_not_found`、409 `wifi_not_set`・`not_on_wifi`、422 `wifi_reverted`、504 `wifi_reverting`、502 `matter_server_unreachable`、503 `matter_server_not_configured` |
| `POST /api/devices/{id}/on`・`/off` | プラグなど、照明以外で入り切りできる機器1台を入・切にします。応答は `{"state": "on"}`・`"off"`・`"no_response"`（応答しない・時間内に答えない機器） | 404 `device_not_found`、409 `not_connected`・`not_switchable`（照明とエアコンと、入り切りの無い機器）、502 `switch_failed`・`matter_server_unreachable`、503 `matter_server_not_configured` |
| `DELETE /api/devices/{id}` | 機器の削除 | 404 `device_not_found` |
| `GET /api/labels` | labelの一覧。性質（`auto`・`override_level`・`override_kelvin`）、毎日の切り替え `schedule`、最後のON・OFF `on`（`true`・`false`・未操作の `null`）、割り当てた機器の台数 `device_count`、上書き中か `boosted` つき | |
| `POST /api/labels` | labelの作成。本文は `{"name": "work", "override_level": 254, "override_kelvin": 5000}`、`{"name": "ambient", "auto": true}`、`{"name": "razor", "schedule": [{"time": "00:00", "on": true}, {"time": "05:00", "on": false}]}` など（下の「label」） | 400 `invalid_label_name`・`invalid_label`、409 `duplicate_label_name` |
| `PATCH /api/labels/{id}` | labelの名前・性質・`schedule` のうち、送った項目だけを変更します。`override_level`・`override_kelvin` を `null` にすると上書きの値を外し、`schedule` は送ったもので置き換えます（`[]` で消す） | 400 `invalid_label_name`・`invalid_label`、404 `label_not_found`、409 `duplicate_label_name` |
| `DELETE /api/labels/{id}` | labelの削除。割り当てた機器は、ほかのlabelだけになります | 404 `label_not_found` |
| `POST /api/labels/{id}/on`・`/off`・`/toggle` | labelの機器（照明とプラグ、無視の機器を除く）をON・OFF・切り替えにします（下の「label」）。応答は、行った操作 `action`（`on`・`off`・`release`）、操作後の `on`・`boosted`、件数（`switched`・`no_response`・`failed`・`missing`）、機器ごとの結果 `devices` と、見つからない機器 `missing_devices` です | 404 `label_not_found`、422 `label_has_no_devices`、503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/wifi` | 保存したWi-Fiの一覧（`id`・`ssid`・機器に渡すものか `for_devices`）。パスワードは返しません | |
| `POST /api/wifi` | Wi-Fiの保存。本文は `{"ssid": "home-2g", "password": "…"}`。同じSSIDならパスワードを置き換えます。最初の1つ、または `"for_devices": true` で送ったものを機器に渡します | 400 `invalid_wifi` |
| `PUT /api/wifi/devices` | 機器に渡すWi-Fiの指定。本文は `{"id": 2}` | 404 `wifi_not_found` |
| `DELETE /api/wifi/{id}` | Wi-Fiの削除。機器に渡すものを消すと、残りの最初のものを渡します | 404 `wifi_not_found` |
| `POST /api/commission` | 0.1.32までのアプリの登録。Wi-FiかThreadかを指定して機器をmatterjs-serverへBluetoothで登録（commissioning）し、台帳に登録 | 400 `missing_setup_code`・`invalid_qr_payload`・`invalid_manual_code`・`invalid_network`・`invalid_device_name`、409 `wifi_not_set`、422 `device_not_found`・`wrong_code`・`wifi_failed`・`thread_failed`・`commission_failed`、504 `commission_timeout`、502 `matter_server_unreachable`、503 `matter_server_not_configured`・`bluetooth_unavailable`・`thread_not_ready` |
| `GET /api/meter` | 温度計の今の値。`{"address":"AA:BB:CC:DD:EE:01","reading":{"temperature":26.7,"humidity":57,"co2":717,"battery":74},"received_at":1792143307,"fresh_for":300}`。`received_at` はUnix秒で、それから `fresh_for` 秒を過ぎると `reading` は `null` になります。未設定なら `address` も `null` です | |
| `GET /api/aircon` | エアコンの自動調整。入・切 `on`、目標の温度 `target`、夜の時間帯 `night_from`・`night_until`（0時からの分）、台帳のエアコンの今の状態 `device`（`{"id","name","state","on","mode","setpoint"}`。`mode` は `cool`・`heat`・`dry`・`fan_only`・`auto`・`off`、`setpoint` は今の運転の設定温度℃。matterjs-serverから読めないかエアコンが無ければ `null`）、温度計の値 `reading`（`GET /api/meter` と同じ、古ければ `null`）、外気温 `outdoor`（`{"temperature","at","fetched_at","fresh"}`。`at` はその値の時刻、`fresh` は2時間以内か。まだ取っていなければ `null`）、家の位置 `location`（`{"name","latitude","longitude","registered"}`）、最後にエアコンが受け付けた命令 `sent` とその設定温度 `setpoint`（補正後）、直近の判断の理由 `reason`、直近の実行 `runs`（新しい順に144回分） | |
| `PUT /api/aircon` | 送った項目だけを変えます。本文は `{"on": false}`、`{"target": 24}`（18〜30の整数）、`{"night_from": 1320, "night_until": 420}`（0〜1439）など。入・切と目標はその場でエアコンへ反映します。応答は `GET /api/aircon` と同じです | 400 `invalid_aircon` |
| `GET /api/aircon/runs` | 14日分の実行の記録（古い順）。`?since=`（Unix秒）でそれ以降に絞れます。各回は時刻 `at`、きっかけ `trigger`、室温 `temperature`・湿度 `humidity`・外気温 `outdoor`、決めた命令 `command`、結果 `result`、理由 `reason` を持ちます | |
| `GET /api/location` | 家の位置。登録した住所 `address`、見つかった場所の名前 `name`、`latitude`・`longitude`、登録済みか `registered`。未登録なら東京（新宿）の座標です | |
| `PUT /api/location` | 本文 `{"address": "東京都新宿区西新宿2-8-1"}` の住所を国土地理院の住所検索で引き、最初の候補を家の位置にします。見つからないときと届かないときは前の位置のままです。応答は `GET /api/location` と同じです | 400 `invalid_location`、404 `address_not_found`、502 `address_search_unreachable` |
| `POST /api/aircon/toggle` | 入・切を切り替えます（ウィジェットのボタン）。保存してすぐ `{"on": true}` を返し、エアコンへはその後に送ります | |
| `GET /api/thread` | Threadの機器を登録できるか（matterjs-serverがThread網のdatasetを持つか）を `{"ready": true}` で返します | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/status` | 台帳の機器ごとに、matterjs-serverで見えるか（`visible`）と今の `node_id`・`endpoint` | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/lights` | 照明ごとの今の状態（`on`・`off`・`no_response`・`ignored`）と、無視の照明を除いた件数 | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/on` | 0.1.37までのアプリの照明オン。上書きの値を持たないlabelをすべてONにし、無視の照明と、上書きの値を持つlabelだけが付いた照明を除く全部の照明をオンにします。照明ごとの結果（`switched`・`no_response`・`failed`・`ignored`・`override`）と、それらを除いた件数を返します。プラグは切り替えません | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/off` | 0.1.37までのアプリの照明オフ。すべてのlabelをOFFにし（上書きもやめます）、全部の照明をオフにします。応答は `on` と同じ形です | 同上 |
| `POST /api/lights/boost` | 0.1.37までのウィジェットの「仕事」「作業」。本文 `{"label": "作業"}` の名前のlabelを `/toggle` と同じく切り替えます。応答は `/toggle` と同じ形で、`action` はONにしたとき `boost`、それ以外は `release` です | 400 `invalid_boost`、404 `label_not_found`、422 `label_has_no_lights`、503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/lights/schedule` | 明るさと色温度の自動調整の設定（`settings`）、labelと割り当てた機器（`labels`）、直近の調整の記録（`runs`、新しい順に144回分） | |
| `PUT /api/lights/schedule` | 自動調整の設定のうち、送った項目だけを変更します。応答は `GET` と同じ形です | 400 `invalid_schedule` |
| `POST /api/door/unlock` | 玄関のSESAMEの解錠をCANDY HOUSEのWeb APIへ送り、受け付けられたら `{"status":"accepted"}` を返します | 503 `sesame_not_configured`、502 `sesame_rejected`（CANDY HOUSEのHTTPステータスを `message` に含みます）・`sesame_unreachable` |
| `POST /api/door/lock` | 玄関のSESAMEの施錠をCANDY HOUSEのWeb APIへ送り、受け付けられたら `{"status":"accepted"}` を返します | 503 `sesame_not_configured`、502 `sesame_rejected`（CANDY HOUSEのHTTPステータスを `message` に含みます）・`sesame_unreachable` |
| `POST /mcp` | MCP（Streamable HTTP、JSONの応答だけ）。下の「MCP」 | |
| `GET /api/health` | 稼働確認と版（`{"status":"ok","version":"0.1.4"}`） | |
| `GET /healthz` | 稼働確認（`ok`） | |

機器名は0〜100文字です。

### QRコードを読んだ時点で台帳に入れ、後から接続する

機器は、QRコード（または11桁のコード）を読んだ時点で `POST /api/devices` で台帳に入ります。名前は無くてよく、
後から `PATCH /api/devices/{id}` で付けます。QRコードからはVendor ID・Product ID（`vendor_id`・`product_id`）を読んで持ちます。
11桁のコードには入っていないので、その場合は `null` です。製品名（`product_name`）は、接続した機器がmatterjs-serverへ報告したものです。
同じ機器かどうかは、登録と同じくsetup passcodeとdiscriminatorの上位4ビットで判定します。
例えば3台の機器のQRコードを先に読んでおき、後から1台ずつ接続できます。

機器の一覧と1台の取得は、そのたびにmatterjs-serverの全nodeを読み、識別子から接続の状態 `state` を求めます。
`unconnected` は接続していない（またはmatterjs-serverにもう無い）、`connected` は接続済み、`no_response` はnodeがあるが応答しない機器です。
matterjs-serverの接続先が無い・接続できないときは `null` です。人が直す状態の欄はありません。
`wifi_ssid` は、Wi-Fiでつながっている機器が今いるWi-Fiの名前です（Network Commissioningの接続中のネットワーク）。それ以外の機器は `null` です。
`network` は、機器がつながるネットワーク（`wifi`・`thread`・`ethernet`、ブリッジ配下はブリッジのもの）で、分からなければ `null` です。
接続していない機器は識別子が無いので、照明の `missing_devices` には入りません。

`POST /api/devices/{id}/commission` は、行に記録したコードで `commission_with_code`（`network_only: false`）を送ります。
機器に渡すWi-Fi（`for_devices`）を保存していれば、先に `set_wifi_credentials` で渡します。保存が無ければWi-Fiは渡しません。
Wi-FiかThreadかは指定せず、機器の申告からmatterjs-serverが決めます（Thread網のdatasetはmatterjs-serverが持つものを使います）。
接続できたら、その行にVendorName・SerialNumber・Wi-FiのMACアドレスを補い、`{"node_id": …, "device": {…}}` を200で返します。
失敗の分類と打ち切りの時間は `POST /api/commission` と同じです。失敗した行はそのまま残り、`unconnected` のままです。
同じ識別子の機器が台帳の別の行にあれば（初期化して読み直した機器など）、その行を残し、読んだ行のコード、
その行に無い名前を移して、読んだ行を消します。labelと無視の設定は残した行のものです。

`POST /api/devices/{id}/wifi` は、Matterの仕様の手順でWi-Fiを差し替えます。
まず `device_command` で、機器のGeneral CommissioningにArmFailSafe（2分）を送ります。
続いてNetwork Commissioningで、今のネットワークをRemoveNetworkで外します。
AddOrUpdateWiFiNetworkで新しいWi-Fiを入れ、ConnectNetworkでつながせます。
ConnectNetworkの応答は、機器が元のネットワークを離れるため届かないことが多く、届かなくても先へ進みます。
新しいWi-Fiの上でCommissioningCompleteを受け付けられたら確定し、200を返します。
確定できないときは、ArmFailSafe（0秒）でfail-safeを終えます。ConnectNetworkが新しいWi-Fiを拒んだときと、fail-safeが切れるまで確定できなかったときです。
fail-safeが終わると、機器は元のネットワークに戻ります（Matterの仕様）。
その後に機器から接続中のネットワークを読みます。元のWi-Fiで応答すれば422 `wifi_reverted`、1分待っても応答が無ければ504 `wifi_reverting` です。
Wi-Fiのパスワードは機器への命令だけに入れ、応答とログには含めません。

`POST /api/commission` は、0.1.32までのアプリのために残しています。台帳の行を作らずに登録し、登録できたら行を作ります。
QRコードは `MT:` で始まるMatterのセットアップコードとして検証し、前後の空白は除いて保存します。
手動ペアリングコードは空白とハイフンを除いた11桁の数字にして、チェック数字とsetup passcodeを検証して保存します。
QRコードと手動ペアリングコードは、setup passcodeとdiscriminatorの上位4ビットが一致すれば同じ機器として扱います。

識別子は、matterjs-serverが機器から読んだBasic Information（ブリッジ配下の機器はBridged Device Basic Information）の
VendorNameとSerialNumberです。Aqara Hub M3配下のT2ではZigbeeのIEEEアドレス、TapoではWi-FiのMACがSerialNumberになります。

`POST /api/commission` は、本文の `qr_payload` か `manual_code` のどちらか一方、`name`（任意）、
`network`（`wifi` か `thread`、既定は `wifi`）を受け取ります。`wifi` では、保存したWi-Fiのうち機器に渡すもの（`for_devices`）を
matterjs-serverへ `set_wifi_credentials` で渡してから、`commission_with_code`（`network_only: false`）でBluetooth経由の登録を行います。
機器に渡すWi-Fiが無ければ、登録を始めずに409 `wifi_not_set` を返します。本文のWi-Fiの項目は使いません。
`thread` ではWi-Fi情報を渡さず、matterjs-serverが持つThread網のdatasetで登録します。
matterjs-serverの `server_info` の `thread_credentials_set` がtrueでなければ、登録を始めずに `thread_not_ready` を返します。
Bluetoothの中継は、要求の前からmatterjs-serverの `/ble` に接続している電話などのBLE Proxyが務めます。
登録できたら、新しいnodeのBasic InformationのVendorNameとSerialNumber、Wi-Fi機器はGeneral DiagnosticsのWi-Fiインターフェースの
MACアドレスを読み、コードとともに台帳へ登録して201を返します。
同じ識別子かコードの機器が台帳にあれば新しく登録せず、その機器に欠けていた識別子を補って200で返します（`registered: false`）。
1回の登録は5分で打ち切ります。Wi-Fiのパスワードは応答とログには含めません。
`device_not_found` はペアリング待ちの機器がBluetoothで見つからない、`wrong_code` は見つかったがコードを受け付けない、
`wifi_failed` は機器がWi-Fiにつながらない、`thread_failed` は機器がThread網に参加できない、`commission_failed` はそれ以外（matterjs-serverの説明を `message` に含みます）です。

`GET /api/status` は、そのたびにmatterjs-serverの全nodeを読み、識別子から機器の今の `node_id` と `endpoint` を引きます。
node IDとendpointはmatterjs-serverが振る番号なので台帳には保存しません。
`visible: false` の機器は、matterjs-serverに見えないか識別子が無い機器で、matterjs-serverへの登録し直しが必要です。

照明APIは、要求のたびにmatterjs-serverの全nodeを読み、照明の種類（On/Off・Dimmable・Color Temperature・Extended Colorの各Light）を持ち、
On/Offの属性があるendpointを照明として扱います。Aqara Hub M3などのブリッジ配下の照明も、endpointごとに1台と数えます。
nodeが使えない照明と、ブリッジが届かないと報告している照明には命令を送らず、`no_response` とします。
命令は全部の照明へ一度に送るので、応答しない照明が他の照明を待たせません。
命令を受け付けた照明だけを `switched` に数え、エラーが返った照明は `failed`、2.5秒以内に応答の無かった照明は `no_response` です。
応答の無い照明へは、1秒後に全nodeを読み直し、届くようになった照明も含めて同じ要求の中でもう一度送ります。どちらの命令への応答も数えます。
途中で接続が切れた場合は、2.5秒の間につなぎ直して、まだ応答の無い照明へ送り直します（オン・オフは状態に依らない指定なので、二重に届いても結果は同じです）。
期限までに受けた結果は、接続が戻らなくても返します。照明ごとの項目の `network` は、その機器がつながるネットワーク（`wifi`・`thread`・`ethernet`、ブリッジ配下はブリッジのもの）です。
照明の `name` は台帳の機器名で、台帳に無い照明は製品名です。台帳の機器のうちmatterjs-serverに見えないものは `missing_devices` に並び、
`missing` に数えます。状態の `GET /api/lights` は、matterjs-serverが最後に読んだOn/Offの値を返します。

#### 二度と応答しない照明を無視する

ブリッジ配下に残った電球など、二度と応答しない照明は、台帳の機器なら無視の対象にできます。
無視の照明には、全部オン・全部オフと自動調整の命令を送りません。全部オン・全部オフの件数（`switched`・`no_response`・`failed`・`missing`）と、
`GET /api/lights` の件数にも数えません。照明ごとの項目には、`result` または `state` が `ignored` として残ります。
応答の無い照明を自動で無視にはしません。

```sh
curl -X PATCH http://home-link.example:5011/api/devices/7 -H 'content-type: application/json' -d '{"ignored":true}'
# {"id":7,…,"ignored":true,…}
```

### label

labelは、照明とプラグ（On/Offを持つMatterの機器）をまとめて操作する単位です。1台に複数のlabelを付けられます（`PATCH /api/devices/{id}` の `label_ids`）。
SESAMEなどの錠はlabelの対象ではありません。

labelには、次の性質を持たせられます。

| 性質 | 内容 |
| --- | --- |
| `auto` | 自動調光連動。`true` のlabelの照明は、下の「明るさと色温度の自動調整」に従います |
| `override_level`・`override_kelvin` | 上書きの値（明るさ1〜254、色温度1000〜10000K）。2つは両方を指定するか、両方を `null` にします。ONの間、labelの照明をこの値に固定し、自動調整しません。`auto` とは同じlabelに持てません |
| `schedule` | 毎日の切り替え。`[{"time": "00:00", "on": true}, {"time": "05:00", "on": false}]` の形で、時刻（Asia/Tokyo、`00:00`〜`23:59`）ごとにONかOFFを1回送ります。間に手で切り替えた状態は、次の時刻まで保ちます |

labelの操作（`POST /api/labels/{id}/on`・`/off`・`/toggle`）は、labelの機器のうち無視の機器を除く照明とプラグに送ります。

- **ON**: 機器をオンにします。上書きの値を持つlabelなら、その照明を上書きの値ですぐに点けます（`MoveToLevelWithOnOff` と `ExecuteIfOff` 付きの `MoveToColorTemperature`、`transitionTime` 0）。
  自動調整の対象になった照明があり、自動調整が有効なら、続けて自動調整を行います（`runs` の `trigger` は `label_on`）。
  1台もオンにできなかったときは、labelの `on` を変えません。
- **OFF**: 機器をオフにします。その機器に付いた上書きのlabelも、上書きをやめます。
- **切り替え（toggle）**: labelの `on` が `true` でなければONにします。`true` なら、上書きの値を持たないlabelはOFFにし、
  上書きの値を持つlabelは上書きをやめます。上書きをやめた照明は、ほかのlabelで決まる状態に戻ります。
  ほかの上書き中のlabelがあればその値のまま、ONの（OFFでない）自動調光連動のlabelがあればその時刻の目標値へすぐ戻り（`ExecuteIfOff` なし、`transitionTime` 0）、
  どちらでもなければ消えます。プラグも同じく、自動調光連動のlabelに入っていなければ切ります。

1台の照明に性質の違うlabelが付いているときは、次の順に決めます。labelの順はidの小さい順です。

1. 無視の照明には何も送りません。
2. 上書き中（上書きの値を持ち、`on` が `true`）のlabelがあれば、最初のlabelの値に固定し、自動調整しません。
3. 自動調光連動のlabelがあり、そのどれかの `on` が `false` でなければ（未操作の `null` を含みます）、自動調整に従います。
4. 自動調光連動のlabelがすべてOFFなら、自動調整は何も送りません（`decision` は `all_off`）。
5. 自動調光連動のlabelが無い照明は、自動調整しません（`decision` は `manual`）。台帳に無い照明も同じです。

例えば次のように使います（`254` が100%、`203` が80%です）。

| label | 性質 | 使い方 |
| --- | --- | --- |
| `ambient` | `auto: true` | 普段の照明。ON/OFFで点け消しし、点いている間は自動調整に従います |
| `work` | 上書き 254・5000K | 仕事用のデスクライト。切り替えで100%・5000Kで点け、もう一度押すと消えます |
| `game`・`kitchen` | 上書き 203・5000K | `ambient` の照明の一部。切り替えで昼の明るさにし、もう一度押すと `ambient` の自動調整に戻ります |
| `razor` | `schedule` 00:00 ON・05:00 OFF | 電気シェーバーをつないだプラグ。夜中だけ電源を入れます |

```sh
curl -X POST http://home-link.example:5011/api/labels -H 'content-type: application/json' -d '{"name":"ambient","auto":true}'
# 201 {"id":1,"name":"ambient","auto":true,"override_level":null,"override_kelvin":null,"on":null,"schedule":[],"device_count":0,"boosted":false}

curl -X PATCH http://home-link.example:5011/api/devices/5 -H 'content-type: application/json' -d '{"label_ids":[1,2]}'
# {"id":5,…,"labels":[{"id":1,"name":"ambient"},{"id":2,"name":"game"}],…}

curl -X POST http://home-link.example:5011/api/labels/2/toggle
# {"label":"game","action":"on","on":true,"boosted":true,"switched":3,"no_response":0,"failed":0,"missing":0,
#  "devices":[{"node_id":1,"endpoint":3,"name":"リビング1","result":"switched"},…],"missing_devices":[]}
```

### エアコンの自動調整

10分ごと（毎時0分・10分・…）に、温度計の値と外気温から台帳のエアコン（無視していない最初の1台）へ送る状態を決めます。
外気温は、Webの管理画面で登録した住所の座標で、Open-Meteoから取ります（未登録なら東京・新宿）。
住所はhome-linkのデータベースにだけ保存し、ログには出しません。

外気温で、エアコンが無いとき部屋がどちらへ動くかを見て、運転を選びます（目標25℃の場合）。

| 外気温 | 運転 |
| --- | --- |
| 目標+2℃以上（27℃以上） | 暖房は入れない。冷房は目標に届いても切らず、保持はエアコンの温度調節に任せる |
| 目標以上（25〜27℃） | 暖房は入れない。冷房は目標に届いたら切る |
| 目標−5℃より上（20〜25℃） | 室温が帯（目標±1℃）を出た側の運転で入れ、目標に届いたら切る |
| 目標−5℃以下（20℃以下） | 冷房・除湿は入れない。暖房は目標に届いても切らない |

- 帯の中で湿度が65%以上なら除湿します（55%以下に下がるまで）。外気温が目標−5℃以下のときは除湿しません。
- 同じ運転が30分続き、温度計の30分の平均と今の値が目標から0.5℃以上離れていれば、設定温度を1℃動かします（目標±3℃まで）。
  補正は運転ごとに保存し、再起動しても保ちます。
- 運転・停止はそれぞれ30分以上続けます。自動の送信は30分に1回までです。入・切と目標の変更はすぐ送ります。
- 夜（既定22:00〜翌7:00、`night_from`・`night_until` で変更）は、室温が帯を出たときの切り替えと送り直しだけを送ります。
- 入れた直後は、外気温が目標+2℃以上か目標−5℃以下なら、帯の中でもすぐその側の運転で入れます。
- 帯の外で冷房（暖房）を送って30分たっても室温が下がらない（上がらない）ときは、同じ状態を1回だけ送り直します。
  それでも動かなければ、理由に「エアコンが信号を受けていない可能性」を示し、それ以上は送りません。
- 外気温が古い（2時間以上前の値）か取れないときは、室温だけで決め、自分では切りません。
- 風量は自動です。設定温度はエアコンの範囲（16〜32℃）に収まるよう、使わない側の設定温度を2℃離して送ります。
- エアコンは信号のたびに鳴るので、前にエアコンが受け付けた状態と同じなら送りません。受け付けた状態は保存し、再起動しても送り直しません。
- 温度計の値が古い（5分以上受けていない）か温度計が無いときは、エアコンを変えません。
- 自動調整を切ると、エアコンを切って止めます。切っている間は、室温にかかわらず送りません。
- エアコンは、labelの操作、照明の全部オン・全部オフ、`POST /api/devices/{id}/on`・`/off` では切り替わりません。

```sh
curl -s -X PUT localhost:5011/api/aircon -H 'content-type: application/json' -d '{"target":24}'
# {"on":true,"target":24,"night_from":1320,"night_until":420,
#  "device":{"id":23,"name":"エアコン","state":"connected","on":true,"mode":"cool","setpoint":24.0},
#  "reading":{"temperature":26.7,"humidity":57,"co2":717,"battery":74},
#  "outdoor":{"temperature":30.2,"at":"2026-10-23T14:00:00+09:00","fetched_at":"2026-10-23T14:00:12+09:00","fresh":true},
#  "location":{"name":"東京都新宿区西新宿二丁目８番","latitude":35.689629,"longitude":139.691772,"registered":true},
#  "sent":{"power":"on","mode":"cool","celsius":24},"setpoint":24,"reason":"室温26.7℃が24℃より1℃以上高いため、冷房にする",
#  "runs":[{"at":"2026-10-23T14:00:12+09:00","trigger":"target","temperature":26.7,"humidity":57,"outdoor":30.2,
#           "command":{"power":"on","mode":"cool","celsius":24},"reason":"室温26.7℃が24℃より1℃以上高いため、冷房にする","result":"sent"},…]}
```

`runs` の `result` は次のどれかです。

| `result` | 意味 |
| --- | --- |
| `sent` | エアコンが受け付けた |
| `unchanged` | 送る必要が無かった |
| `held` | 変える必要はあるが、30分の間隔・夜・受けていない可能性のため送らなかった（`reason` に理由） |
| `no_reading` | 温度計の値が無い |
| `no_aircon` | エアコンが見つからない |
| `no_response`・`refused`・`failed` | 届かなかった・断られた・matterjs-serverに届かない。次の回に送り直します |

### 明るさと色温度の自動調整

有効にすると、10分ごと（毎時0分・10分・…）に、自動調光連動のlabelの照明のうち点いているものの明るさと色温度を、その時刻の値へ合わせます。既定では無効です。
明るさの割合はLevel Controlの254を100%とします（80%は203、40%は102）。

| 時間帯 | 明るさ | 色温度 |
| --- | --- | --- |
| 朝: 日の出（5時より前なら5時）→ `morning_end_minute` | `night_level` → `day_level` へ線形に上げる | `warm_kelvin` → `cool_kelvin` へ線形に上げる |
| 昼: `morning_end_minute` → 日の入り | `day_level` | `cool_kelvin` |
| 夕: 日の入り → 22時 | `day_level` → `night_level` へ線形に下げる | `cool_kelvin` → `warm_kelvin` へ線形に下げる |
| 夜: 22時 → 0時 | `night_level` | `warm_kelvin` |
| 深夜: 0時 → 翌朝の開始 | `night_level` の `late_night_percent`%（既定は半分、1以上） | `warm_kelvin` |

- 朝の上昇は、深夜の値ではなく通常の `night_level` から始まります。
- 日の出・日の入りは、家の位置（`PUT /api/location`、未登録なら東京・新宿）とAsia/Tokyoの時刻で計算します。
- 値は全体の設定だけを使います。labelごとの値はありません。
- 色温度と明るさは、照明ごとに報告された範囲へ収めます。色温度に対応しない照明へは色温度を、調光しない照明へは明るさを送りません。
- 定時の1回の変化は30秒（`transitionTime` 300）かけて移します。
- 前回送った値と同じ値は送りません。送った値はメモリだけに持ち、再起動後は改めて送ります。

| 設定 | 既定値 | 内容 |
| --- | --- | --- |
| `enabled` | `false` | 自動調整を行うか |
| `morning_end_minute` | `600`（10:00） | 朝の上昇が昼の値に達する時刻（0時からの分、301〜1319） |
| `day_level`・`night_level` | `203`・`102` | 昼と夜の明るさ（1〜254） |
| `warm_kelvin`・`cool_kelvin` | `3000`・`5000` | 夜と昼の色温度（K）。`warm_kelvin` は `cool_kelvin` 以下 |
| `late_night_percent` | `50` | 0時から朝の開始までの明るさを、`night_level` の何%にするか（1〜100、四捨五入）。`100` で減らしません |

点灯を伴う書き込みはしません。

- 調整のたびに、送る直前に各照明のOn/Offを照明から読み直し、点いている照明にだけ送ります。
- 消えている照明、届かない照明には何も送りません。
- 命令はLevel Controlの `MoveToLevel` とColor Controlの `MoveToColorTemperature` で、`ExecuteIfOff` を立てません。
  読んだ後に消された照明は、命令を無視して消えたままです。
- 自動調光連動のlabelをOFFにすると、次にONにするまで、ほかの手段で点けられた照明も含めて、そのlabelの照明へ何も送りません。
  labelのON・OFFはDBに保存し、再起動しても保ちます。照明ごとのオン・オフは保存せず、毎回matterjs-serverから読みます。
- labelのONや照明オンで点けた照明は、点灯の報告が遅れるブリッジ配下の照明もあるため読み直さず、前回と同じ値でも、
  `MoveToLevelWithOnOff` と `ExecuteIfOff` 付きの `MoveToColorTemperature` ですぐに（`transitionTime` 0）送ります。

`runs` は調整ごとに、時刻（`at`）、きっかけ（`trigger`: `scheduled`・`label_on`・`lights_on`）、全体の設定の目標値（`level`・`kelvin`）、
日の出・日の入り、送った命令の数（`commands`）、照明ごとの判断（`decision`）を持ちます。
照明ごとに、割り当てたlabelの名前（`labels`、無ければ `[]`）と、その照明の目標値（`target_level`・`target_kelvin`、上書き中は上書きの値）も持ちます。
判断は、送った `sent`、自動調光連動のlabelがすべてOFFの `all_off`、自動調光連動のlabelが無い `manual`、上書き中の `override`、無効の `disabled`、消えていた `off`、届かない `no_response`、
エラーが返った `failed`、調光にも色温度にも対応しない `unsupported`、前回と同じ値の `unchanged`、無視の照明の `ignored` です。
`sent` の照明の `level`・`mireds` は、前回から変わって送った値だけを持ちます。
記録はメモリだけに持ち、再起動で消えます。

```sh
curl -X PUT http://home-link.example:5011/api/lights/schedule -H 'content-type: application/json' -d '{"enabled":true}'
# {"settings":{"enabled":true,"morning_end_minute":600,"day_level":203,"night_level":102,"warm_kelvin":3000,"cool_kelvin":5000,"late_night_percent":50},
#  "labels":[{"id":1,"name":"ambient","auto":true,…,"devices":[{"id":5,"name":"リビング1"},…]},…],"runs":[…]}

curl http://home-link.example:5011/api/lights/schedule
# {…,"runs":[{"at":"2026-10-05T23:00:00+09:00","trigger":"scheduled","sunrise":"05:39","sunset":"17:23","level":102,"kelvin":3000,"commands":2,
#   "lights":[{"node_id":1,"endpoint":3,"name":"リビング1","labels":["ambient"],"target_level":102,"target_kelvin":3000,
#              "decision":"sent","level":102,"mireds":333},
#             {"node_id":5,"endpoint":1,"name":"デスク","labels":["work"],"target_level":102,"target_kelvin":3000,"decision":"manual"},…]},…]}
```

### 0.1.36からの移行

0.1.36までの「照明オン」「照明オフ」、labelの「明るくする」（boost）、専用のlabel（`dedicated`）、labelごとの値（`day_level`・`night_level`・`cool_kelvin`・`warm_kelvin`）は、labelの性質に置き換わりました。
更新後の最初の起動で、照明の動きが変わらないように次のとおり移ります。labelの名前・idと機器への割り当ては残ります。

- それぞれのlabelは、上書きの値を持つlabelになります。値はそのlabelの `day_level`・`cool_kelvin`（無ければ全体の昼の値）です。
  明るくしている最中だったlabelはONのまま（上書き中）です。専用のlabel（仕事用のデスクライトなど）も、その値（例えば254・5000K）の上書きになります。
- 専用のlabelが付いていない台帳の機器は、すべて新しい自動調光連動のlabel「自動調光」に入ります。これまで自動調整されていた照明が、続けて自動調整されます。
  「自動調光」は、最後に押されたのが「照明オフ」ならOFF、「照明オン」ならONで始まります。
  プラグもこのlabelに入るため、「自動調光」のON・OFFはプラグも切り替えます。labelを付け替えたら、このlabelは消してかまいません。
- labelごとの値は使われなくなり、全体の設定だけで調整します。台帳に無い照明は自動調整されなくなります。
- `GET /api/lights/schedule` の `intent` はなくなりました。0.1.37までのアプリが読む `labels` の `name`・`boosted` は残ります。
- `POST /api/labels`・`PATCH /api/labels/{id}` に `day_level` などの値や `dedicated` を送ると400 `invalid_label` です。

### MCP

`POST /mcp` は、[MCP](https://modelcontextprotocol.io/)のStreamable HTTPのサーバーです。JSON-RPCの1件に、JSONで1件を返します（SSEは使いません）。
APIと同じポートで、認証はありません。道具はAPIへの要求に置き換えて実行し、APIの応答の本文を `content` の文字で返します。APIが失敗を返すと `isError` が `true` です。

| 道具 | 引数 | 対応するAPI |
| --- | --- | --- |
| `list_labels` | なし | `GET /api/labels` |
| `create_label` | `name`、`auto`・`override_level`・`override_kelvin`・`schedule`（任意） | `POST /api/labels` |
| `update_label` | `id` と、変える項目 | `PATCH /api/labels/{id}` |
| `delete_label` | `id` | `DELETE /api/labels/{id}` |
| `list_devices` | なし | `GET /api/devices` |
| `set_device_labels` | 機器の `id`、`label_ids` | `PATCH /api/devices/{id}` |
| `label_on`・`label_off`・`label_toggle` | `id` | `POST /api/labels/{id}/on`・`/off`・`/toggle` |

```sh
claude mcp add --transport http home-link http://home-link.example:5011/mcp
```

### 例

```sh
curl -X POST http://home-link.example:5011/api/devices -H 'content-type: application/json' \
  -d '{"qr_payload":"MT:Y.K9042C00KA0648G00"}'
# 201 {"registered":true,"device":{"id":9,"name":"","vendor":null,"serial_number":null,"mac":null,
#      "vendor_id":65521,"product_id":32768,"state":"unconnected","product_name":null,"labels":[],"ignored":false,"created_at":"2026-10-21T03:30:00Z"}}

curl -X POST http://home-link.example:5011/api/devices/9/commission
# 200 {"node_id":17,"device":{"id":9,…,"vendor":"Tapo","serial_number":"AABBCCDDEEFF","mac":"AABBCCDDEEFF","state":"connected","product_name":"Smart Bulb",…}}
# 422 {"error":"wifi_failed","message":"機器がWi-Fi「home-2g」に接続できませんでした"}

curl -X PATCH http://home-link.example:5011/api/devices/9 -H 'content-type: application/json' -d '{"name":"押入れ1"}'

curl http://home-link.example:5011/api/status
# {"devices":[{"endpoint":null,"id":1,"name":"天井灯","node_id":null,"visible":false},{"endpoint":0,"id":2,"name":"読書灯","node_id":5,"visible":true}],"version":"0.1.4"}

curl -X POST http://home-link.example:5011/api/lights/off
# {"action":"off","switched":10,"no_response":1,"failed":0,"missing":1,
#  "lights":[{"node_id":1,"endpoint":2,"name":"Aqara LED Bulb T2","result":"no_response"},
#            {"node_id":5,"endpoint":1,"name":"読書灯","result":"switched"},…],
#  "missing_devices":[{"id":1,"name":"天井灯"}]}

curl http://home-link.example:5011/api/lights
# {"on":0,"off":10,"no_response":1,"missing":1,"lights":[{"node_id":5,"endpoint":1,"name":"読書灯","state":"off"},…],"missing_devices":[…]}
```

## 開発

ビルド、テスト、リリースの手順は[開発ガイド](docs/development.md)、
Androidアプリの画面の規則は[DESIGN.md](DESIGN.md)にあります。

## ライセンス

[MIT License](LICENSE)
