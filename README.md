# home-link

住まいの機器を、部屋と名前で記録するサービスです。
家じゅうの照明を、Androidのボタン1つで全部オン・全部オフにできます。
電球などに印刷されたMatterのQRコードをAndroidアプリで読み取り、部屋と名前を付けて登録します。
QRコードが無い機器は、印字された11桁の数字（Matterの手動ペアリングコード）を入力して登録します。
新しいWi-Fi電球とThread電球（Aqara T2など）は、電話のBluetoothで直接つなぎ、家のWi-FiかThread網とmatterjs-serverへ入れてから台帳に登録できます。
登録した内容は、自宅サーバーで動くhome-linkのAPIがSQLiteに保存します。
機器自身の識別子（ベンダー名とシリアル番号、Wi-Fi機器はMACアドレス）も保存でき、
Matterのcontroller（[matterjs-server](https://github.com/matter-js/matterjs-server)）を作り直しても、
台帳の機器が今どのnodeにいるかを識別子から引き直します。

構成は、Rust（axum）のAPIサーバーと、Kotlinで作ったAndroidアプリです。
照明の操作は、matterjs-serverに登録済みの照明を全部オン・全部オフにするものだけです。
照明ごと・部屋ごとの操作は、現在のhome-linkにはありません。
Matterの機器登録（commissioning）はmatterjs-serverが行い、home-linkのアプリは電話のBluetoothを貸すだけです。

## 公開範囲と認証

APIに認証はありません。家庭LANとTailscaleのtailnetなど、信頼できる経路にだけ公開してください。
インターネットへ直接公開しないでください。
MatterのQRコードと手動ペアリングコードには、機器のsetup passcodeが含まれます。
そのためコードの全文は機器1台の取得APIだけが返し、一覧APIとログには出しません。

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
| `MATTER_SERVER_URL` | なし | matterjs-serverのWebSocket API（例: `ws://192.168.1.100:5580/ws`）。未設定なら状態API・照明API・機器のBluetooth登録は503を返します。 |

## Androidアプリ

Android 16以降の端末と、Tailscaleへの接続が必要です。
アプリは `http://homeserver:5011` へ接続します。
`homeserver` はtailnetのMagicDNS名なので、Tailscaleが有効なら家の外からも使えます。
QRの読み取りにはGoogle Playサービスのコードスキャナーを使うため、カメラの権限は求めません。
位置情報の許可は、電球に渡すWi-Fiを追加するときに、今つながっているWi-FiのSSIDを読むためだけに求めます。

[リリース](https://github.com/miyabisun/home-link/releases/latest)のAssetsから
`home-link-vX.Y.Z.apk` をダウンロードし、端末で開いてインストールします。
APKは開発用のdebug署名です。PCからADBで導入する方法と、ファイルの検証方法は
[開発ガイド](docs/development.md#adbでのインストール)を参照してください。

### 照明を全部オン・全部オフにする

アプリの上部にある「照明オン」「照明オフ」を押すと、matterjs-serverに登録された照明を全部切り替えます。
結果は切り替えた台数と、応答のなかった照明・失敗した照明・台帳にあるのに見つからない機器の台数で示します。
1台でも切り替えられなかった照明があれば、失敗の色で表示します。
無視の対象にした照明（下の「二度と応答しない照明を無視する」）は、切り替えず、台数にも数えません。

「ホーム画面にボタンを置く」を押すと、同じ2つのボタンに「仕事」「作業」を加えた4つのボタンのウィジェットをホーム画面へ置けます。
「仕事」はlabel「仕事用デスク」、「作業」はlabel「作業」の照明だけを昼の明るさで点け、もう一度押すと元に戻します
（下の「labelの照明を明るくする」）。labelはAPIで作って照明に割り当てます。
ウィジェットは横4マス・縦1段で置かれ、横は約240dp（電話で3マス程度）まで縮められます。
1段では、押したボタンの文字が数秒だけ結果（「オン完了」「解除完了」「一部失敗」「接続不可」「ラベルなし」「照明なし」など）に変わります。
縦に広げると、ボタンの下に直前の結果を表示します。ウィジェットは台数を示さず、成功か、切り替えられなかった照明があるかだけを示します。

### Wi-Fi電球をBluetoothでつなぐ

新品や初期化した、ペアリング待ちのMatter over Wi-Fiの電球（Tapo、BEAMTECなど）を、Aqara Homeを使わずに登録できます。
前のルーター向けに設定されて今つながらない電球も、初期化すれば同じ手順で今のWi-Fiへ入れられます。
Matter over Threadの電球（Aqara T2など）は、下の「Thread電球をBluetoothでつなぐ」で登録します。
Zigbeeの電球は対象外です（親機が要るため）。Aqara Homeで追加してから、下の「記録だけ」で登録してください。
登録の画面は「Wi-Fiの電球」と「Threadの電球（T2など）」のタブに分かれています。この節はWi-Fiのタブの手順です。

電話がBluetoothでmatterjs-serverの[BLE Proxy](https://github.com/matter-js/matterjs-server/blob/main/docs/ble-proxy-protocol.md)
（`ws://192.168.1.100:5580/ble`）の中継を務め、Matterの手順（暗号化、Wi-Fi情報の送付、fabricへの参加）はmatterjs-serverが行います。
そのため、登録するときは電話を家のWi-Fiにつなぎ、電球の近くに置きます。
matterjs-serverはBluetoothを有効にして（BLE Proxyを受け付けて）起動しておく必要があります。

1. 「つなぎ方」で「新しいWi-Fi電球をBluetoothでつなぐ」を選びます（既定）。
2. 「電球に渡すWi-Fi」のセレクトボックスで、電球をつなぐWi-Fiを選びます。最後に選んだWi-Fiが選ばれています。
   初回は「Wi-Fiを追加」を押します。今つながっているWi-FiのSSIDが入るので、パスワードを入力して保存します。
   SSIDを読むために、Androidの仕様で位置情報（正確な位置）の許可を求めます。許可しない場合はSSIDを入力します。
   電球は2.4GHzのWi-Fiにしかつながりません。今のWi-Fiが5GHzなら、追加の画面にそう表示されます。
   Wi-Fiは複数保存でき、要らなくなったものは「このWi-Fiを削除」で消せます。
   パスワードはこの電話の中だけに、Android Keystoreの鍵で暗号化して保存します。
   登録のたびにhome-link経由でmatterjs-serverへ渡し、home-linkのデータベースやログには残しません。
3. 電球をペアリング待ちにします。BEAMTECは電源のオフ・オンを5回くり返します。ほかはメーカーの手順に従います。
4. QRコードを読み取るか数字を入力し、部屋と名前（任意）を決めて「登録」を押します。
   初回は「付近のデバイス」の許可を求めます。Android 17では、家のネットワークにあるmatterjs-serverへの接続もこの許可に含まれます。
   Bluetoothがオフならオンにするよう求めます。

登録中は、電球を探している・接続している・コードとWi-Fiの設定を送っている・電球がWi-Fiにつながるのを待っている、の段階を表示します。
電球が見つからない、コードが違う、Wi-Fiにつながらない、時間切れ、matterjs-serverに接続できない、はそれぞれ理由と次にすることを表示し、入力は残ります。
台帳に同じ機器（同じ識別子かコード）がある場合は新しく登録せず、既存の登録を示します。

### Thread電球をBluetoothでつなぐ

Thread電球はWi-Fiの無線を持たないため、Bluetoothで渡すのはWi-Fiではなく、自前のThread網の資格情報（dataset）です。
datasetはmatterjs-serverに `set_thread_dataset` で登録したものを使い、アプリやhome-linkには持ちません。
Thread網（OpenThread Border Router）が無いうちは、Threadのタブに「Thread網が未準備です」と表示され、登録できません。
matterjs-serverにdatasetが入ると、アプリを変えずに登録できるようになります（「もう一度確かめる」で表示を更新します）。

1. 「Threadの電球（T2など）」のタブを選びます。「つなぎ方」は「新しいThread電球をBluetoothでつなぐ」（既定）のままにします。
2. 電球をペアリング待ちにします。Aqara T2は電源のオフ・オンを1秒間隔で10回くり返すと初期化されます（初期はThreadで動きます）。
3. QRコードを読み取るか数字を入力し、部屋と名前（任意）を決めて「登録」を押します。Wi-Fiは選びません。

段階の表示、失敗の表示、台帳に同じ機器がある場合の扱いはWi-Fi電球と同じです。電球がThread網に参加できない場合はその旨を表示します。

### 機器を登録する（記録だけ）

すでにmatterjs-serverにつながっている機器は、どちらのタブでも「つなぎ方」で「記録だけ」を選んで台帳にだけ登録します。

1. 「QRを読み取る」で機器のQRコードを読み取ります。
   QRコードが無い機器は「数字で入力」を押し、機器に印字された11桁の数字を入力します。
   11桁そろうと確定し、数字の誤りはその場で表示します。
2. 部屋を選びます。部屋が無ければ「部屋を追加」で作成します。
3. 必要なら機器名を入力し、「登録」を押します。機器名は空欄でも登録できます。

登録すると部屋の選択はそのまま残るので、同じ部屋の次の機器はQRを読み取るだけで登録できます。
同じ機器の二重登録（QRコードと数字の組み合わせを含む）、存在しない部屋、サーバーへの接続失敗は、それぞれ理由を表示します。
失敗した場合も入力内容は消えないので、原因を直してから「登録」を押し直してください。

## API

すべてJSONです。失敗時は `{"error": "コード", "message": "説明"}` を返します。

| メソッドとパス | 内容 | 主な失敗 |
| --- | --- | --- |
| `GET /api/rooms` | 部屋の一覧（登録された機器の台数つき） | |
| `POST /api/rooms` | 部屋の作成。本文は `{"name": "寝室"}` | 400 `invalid_room_name`、409 `duplicate_room_name` |
| `PATCH /api/rooms/{id}` | 部屋の名前の変更。本文は作成と同じ | 404 `room_not_found`、409 `duplicate_room_name` |
| `DELETE /api/rooms/{id}` | 部屋の削除。機器が残っている部屋は削除しません | 404 `room_not_found`、409 `room_has_devices` |
| `GET /api/devices` | 機器の一覧。コードは含みません | |
| `POST /api/devices` | 機器の登録 | 400 `invalid_qr_payload`・`invalid_manual_code`・`missing_setup_code`・`invalid_identifier`・`invalid_mac`・`invalid_device_name`、404 `room_not_found`、409 `duplicate_qr_payload`・`duplicate_identifier` |
| `GET /api/devices/{id}` | 機器1台。登録したコードの全文 `qr_payload` または `manual_code` を含みます | 404 `device_not_found` |
| `PATCH /api/devices/{id}` | labelの割り当てと無視の設定。本文は送った項目だけを変えます。`{"label_ids": [1, 2]}`（割り当てを置き換え、`[]` で外す）、`{"ignored": true}`（`false` で戻す） | 400 `invalid_device_update`・`conflicting_labels`、404 `device_not_found`・`label_not_found` |
| `DELETE /api/devices/{id}` | 機器の削除 | 404 `device_not_found` |
| `GET /api/labels` | labelの一覧（値と割り当てた機器の台数、明るくしている最中か `boosted` つき） | |
| `POST /api/labels` | labelの作成。本文は `{"name": "キッチン", "night_level": 76, "warm_kelvin": 2700}` | 400 `invalid_label_name`・`invalid_label`、409 `duplicate_label_name` |
| `PATCH /api/labels/{id}` | labelの名前・値のうち、送った項目だけを変更します。`null` の値は全体の設定に戻ります | 400 `invalid_label_name`・`invalid_label`・`conflicting_labels`、404 `label_not_found`、409 `duplicate_label_name` |
| `DELETE /api/labels/{id}` | labelの削除。割り当てた機器はlabel無しに戻ります | 404 `label_not_found` |
| `POST /api/commission` | 機器をmatterjs-serverへBluetoothで登録（commissioning）し、台帳に登録 | 400 `missing_setup_code`・`invalid_qr_payload`・`invalid_manual_code`・`invalid_network`・`invalid_wifi`・`invalid_device_name`、404 `room_not_found`、422 `device_not_found`・`wrong_code`・`wifi_failed`・`thread_failed`・`commission_failed`、504 `commission_timeout`、502 `matter_server_unreachable`、503 `matter_server_not_configured`・`bluetooth_unavailable`・`thread_not_ready` |
| `GET /api/thread` | Thread電球を登録できるか（matterjs-serverがThread網のdatasetを持つか）を `{"ready": true}` で返します | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/status` | 台帳の機器ごとに、matterjs-serverで見えるか（`visible`）と今の `node_id`・`endpoint` | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/lights` | 照明ごとの今の状態（`on`・`off`・`no_response`・`ignored`）と、無視の照明を除いた件数 | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/on` | 無視の照明を除く全部の照明をオンにし、照明ごとの結果（`switched`・`no_response`・`failed`・`ignored`）と、無視の照明を除いた件数を返します | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/off` | 全部の照明をオフにし、すべてのlabelの明るくする状態を切ります。応答は `on` と同じ形です | 同上 |
| `POST /api/lights/boost` | 本文 `{"label": "作業"}` のlabelの照明を明るくする状態を切り替えます。応答は `on` と同じ件数に、`label`・`action`（`boost` か `release`）・`boosted` を加えた形です | 400 `invalid_boost`、404 `label_not_found`、422 `label_has_no_lights`、503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/lights/schedule` | 明るさと色温度の自動調整の設定（`settings`）、labelごとの値と割り当てた機器（`labels`）、最後に押された全部オン・全部オフ（`intent`）、直近の調整の記録（`runs`、新しい順に144回分） | |
| `PUT /api/lights/schedule` | 自動調整の設定のうち、送った項目だけを変更します。応答は `GET` と同じ形です | 400 `invalid_schedule` |
| `GET /api/health` | 稼働確認と版（`{"status":"ok","version":"0.1.4"}`） | |
| `GET /healthz` | 稼働確認（`ok`） | |

部屋の名前は前後の空白を除いて1〜100文字、機器名は0〜100文字です。
機器の登録では、QRコード `qr_payload` と手動ペアリングコード `manual_code` のどちらか一方、
または機器の識別子 `vendor` と `serial_number` の組を送ります。コードと識別子は両方送っても構いません。
QRコードは `MT:` で始まるMatterのセットアップコードとして検証し、前後の空白は除いて保存します。
手動ペアリングコードは空白とハイフンを除いた11桁の数字にして、チェック数字とsetup passcodeを検証して保存します。
QRコードと手動ペアリングコードは、setup passcodeとdiscriminatorの上位4ビットが一致すれば同じ機器として扱い、
二重登録を `duplicate_qr_payload` で拒否します。

識別子は、matterjs-serverが機器から読んだBasic Information（ブリッジ配下の機器はBridged Device Basic Information）の
VendorNameとSerialNumberです。Aqara Hub M3配下のT2ではZigbeeのIEEEアドレス、TapoではWi-FiのMACがSerialNumberになります。
`vendor` と `serial_number` は両方を前後の空白を除いて1〜100文字で送り、`mac` は任意で16進数12桁（`:` と `-` は除きます）を送ります。
同じ `vendor` と `serial_number` の組、または同じ `mac` の二重登録は `duplicate_identifier` で拒否します。

`POST /api/commission` は、本文の `room_id`、`qr_payload` か `manual_code` のどちらか一方、`name`（任意）、
`network`（`wifi` か `thread`、既定は `wifi`）を受け取ります。`wifi` では `wifi_ssid`（1〜32バイト）と `wifi_password`（1〜64文字）も受け取り、
matterjs-serverへ `set_wifi_credentials` でWi-Fi情報を渡してから、`commission_with_code`（`network_only: false`）でBluetooth経由の登録を行います。
`thread` ではWi-Fi情報を渡さず、matterjs-serverが持つThread網のdatasetで登録します。
matterjs-serverの `server_info` の `thread_credentials_set` がtrueでなければ、登録を始めずに `thread_not_ready` を返します。
Bluetoothの中継は、要求の前からmatterjs-serverの `/ble` に接続している電話などのBLE Proxyが務めます。
登録できたら、新しいnodeのBasic InformationのVendorNameとSerialNumber、Wi-Fi機器はGeneral DiagnosticsのWi-Fiインターフェースの
MACアドレスを読み、コードとともに台帳へ登録して201を返します。
同じ識別子かコードの機器が台帳にあれば新しく登録せず、その機器に欠けていた識別子を補って200で返します（`registered: false`）。
1回の登録は5分で打ち切ります。Wi-Fiのパスワードはmatterjs-serverへ渡すだけで、データベース・ログ・応答には含めません。
`device_not_found` はペアリング待ちの機器がBluetoothで見つからない、`wrong_code` は見つかったがコードを受け付けない、
`wifi_failed` は機器がWi-Fiにつながらない、`thread_failed` は機器がThread網に参加できない、`commission_failed` はそれ以外（matterjs-serverの説明を `message` に含みます）です。

`GET /api/status` は、そのたびにmatterjs-serverの全nodeを読み、識別子から機器の今の `node_id` と `endpoint` を引きます。
node IDとendpointはmatterjs-serverが振る番号なので台帳には保存しません。
`visible: false` の機器は、matterjs-serverに見えないか識別子が無い機器で、matterjs-serverへの登録し直しが必要です。

照明APIは、要求のたびにmatterjs-serverの全nodeを読み、照明の種類（On/Off・Dimmable・Color Temperature・Extended Colorの各Light）を持ち、
On/Offの属性があるendpointを照明として扱います。Aqara Hub M3などのブリッジ配下の照明も、endpointごとに1台と数えます。
nodeが使えない照明と、ブリッジが届かないと報告している照明には命令を送らず、`no_response` とします。
命令を受け付けた照明だけを `switched` に数え、エラーが返った照明は `failed`、10秒以内に応答の無かった照明は `no_response` です。
応答の無い照明へは、2秒待ってから全nodeを読み直し、届くようになった照明も含めて同じ要求の中で送り直します（最大3回）。
途中で接続が切れた場合は、つなぎ直して全部の照明へ送り直します（オン・オフは状態に依らない指定なので、二重に届いても結果は同じです）。
照明の `name` は台帳の機器名で、台帳に無い照明は製品名です。台帳の機器のうちmatterjs-serverに見えないものは `missing_devices` に並び、
`missing` に数えます。状態の `GET /api/lights` は、matterjs-serverが最後に読んだOn/Offの値を返します。

#### 二度と応答しない照明を無視する

ブリッジ配下に残った電球など、二度と応答しない照明は、台帳に登録して無視の対象にできます。
無視の照明には、全部オン・全部オフと自動調整の命令を送りません。全部オン・全部オフの件数（`switched`・`no_response`・`failed`・`missing`）と、
`GET /api/lights` の件数にも数えません。照明ごとの項目には、`result` または `state` が `ignored` として残ります。
応答の無い照明を自動で無視にはしません。

```sh
# ブリッジ配下のT2を、VendorNameとSerialNumberで登録してから無視にする
curl -X POST http://homeserver:5011/api/devices -H 'content-type: application/json' \
  -d '{"room_id":1,"vendor":"Aqara","serial_number":"54ef44100126e0a6","name":"外したT2"}'
curl -X PATCH http://homeserver:5011/api/devices/7 -H 'content-type: application/json' -d '{"ignored":true}'
# {"id":7,…,"ignored":true,…}
```

### 明るさと色温度の自動調整

有効にすると、10分ごと（毎時0分・10分・…）に、点いている照明の明るさと色温度をその時刻の値へ合わせます。既定では無効です。
明るさの割合はLevel Controlの254を100%とします（80%は203、40%は102）。

| 時間帯 | 明るさ | 色温度 |
| --- | --- | --- |
| 朝: 日の出（5時より前なら5時）→ `morning_end_minute` | `night_level` → `day_level` へ線形に上げる | `warm_kelvin` → `cool_kelvin` へ線形に上げる |
| 昼: `morning_end_minute` → 日の入り | `day_level` | `cool_kelvin` |
| 夕: 日の入り → 22時 | `day_level` → `night_level` へ線形に下げる | `cool_kelvin` → `warm_kelvin` へ線形に下げる |
| 夜: 22時 → 0時 | `night_level` | `warm_kelvin` |
| 深夜: 0時 → 翌朝の開始 | `night_level` の `late_night_percent`%（既定は半分、1以上） | `warm_kelvin` |

- 朝の上昇は、深夜の値ではなく通常の `night_level` から始まります。
- 日の出・日の入りは `latitude`・`longitude` とAsia/Tokyoの時刻で計算します。既定は東京（新宿）です。
- 台帳の機器にlabelを割り当てると、その照明はlabelの値（`day_level`・`night_level`・`cool_kelvin`・`warm_kelvin`）で同じ曲線をたどります。
  labelの値が `null` の項目と、時刻（朝の終わり・22時・日の出と日の入りの地点）は全体の設定を使います。
  1台に複数のlabelを割り当てられ、それぞれの値を重ねます。部屋とは別に持ちます。
  同じ照明の2つのlabelが同じ項目に値を持つと、どちらを使うか決まらないため、その割り当てとlabelの変更は `conflicting_labels` で拒否します。
  値を持たないlabel（「作業」など）は、ほかのlabelと重ねられます。
  labelの値の範囲は全体の設定と同じで、全体の設定と重ねた結果が `warm_kelvin` ≤ `cool_kelvin` でなければ `invalid_label` で拒否します。
- 色温度と明るさは、照明ごとに報告された範囲へ収めます。色温度に対応しない照明へは色温度を、調光しない照明へは明るさを送りません。
- 1回の変化は30秒（`transitionTime` 300）かけて移します。
- 前回送った値と同じ値は送りません。送った値はメモリだけに持ち、再起動後は改めて送ります。

| 設定 | 既定値 | 内容 |
| --- | --- | --- |
| `enabled` | `false` | 自動調整を行うか |
| `latitude`・`longitude` | `35.6895`・`139.6917` | 日の出・日の入りを計算する地点 |
| `morning_end_minute` | `600`（10:00） | 朝の上昇が昼の値に達する時刻（0時からの分、301〜1319） |
| `day_level`・`night_level` | `203`・`102` | 昼と夜の明るさ（1〜254） |
| `warm_kelvin`・`cool_kelvin` | `3000`・`5000` | 夜と昼の色温度（K）。`warm_kelvin` は `cool_kelvin` 以下 |
| `late_night_percent` | `50` | 0時から朝の開始までの明るさを、各照明の `night_level`（labelの値を重ねた後）の何%にするか（1〜100、四捨五入）。`100` で減らしません |

点灯を伴う書き込みはしません。

- 調整のたびに、送る直前に各照明のOn/Offを照明から読み直し、点いている照明にだけ送ります。
- 消えている照明、届かない照明には何も送りません。
- 命令はLevel Controlの `MoveToLevel` とColor Controlの `MoveToColorTemperature` で、`ExecuteIfOff` を立てません。
  読んだ後に消された照明は、命令を無視して消えたままです。
- 「全部オフ」（`POST /api/lights/off`）が押されると、次に「全部オン」が押されるまで、
  ほかの手段で点けられた照明も含めて何も送りません。
  押された操作と時刻は保存し、再起動しても保ちます。照明ごとのオン・オフは保存せず、毎回matterjs-serverから読みます。
- 有効なときに「全部オン」を押すと、点けた後にその時刻の値へ合わせます。
  オンを受け付けた照明は、点灯の報告が遅れるブリッジ配下の照明もあるため読み直さず、前回と同じ値でも、
  `MoveToLevelWithOnOff` と `ExecuteIfOff` 付きの `MoveToColorTemperature` ですぐに（`transitionTime` 0）送ります。

`runs` は調整ごとに、時刻（`at`）、きっかけ（`trigger`: `scheduled` または `lights_on`）、全体の設定の目標値（`level`・`kelvin`）、
日の出・日の入り、送った命令の数（`commands`）、照明ごとの判断（`decision`）を持ちます。
照明ごとに、割り当てたlabelの名前（`labels`、無ければ `[]`）と、その照明の目標値（`target_level`・`target_kelvin`）も持ちます。
判断は、送った `sent`、全部オフ中の `all_off`、無効の `disabled`、消えていた `off`、届かない `no_response`、
エラーが返った `failed`、調光にも色温度にも対応しない `unsupported`、前回と同じ値の `unchanged`、無視の照明の `ignored` です。
`sent` の照明の `level`・`mireds` は、前回から変わって送った値だけを持ちます。
記録はメモリだけに持ち、再起動で消えます。

```sh
curl -X PUT http://homeserver:5011/api/lights/schedule -H 'content-type: application/json' -d '{"enabled":true}'
# {"settings":{"enabled":true,"latitude":35.6895,"longitude":139.6917,"morning_end_minute":600,"day_level":203,"night_level":102,"warm_kelvin":3000,"cool_kelvin":5000,"late_night_percent":50},
#  "labels":[{"id":1,"name":"キッチン","day_level":null,"night_level":76,"cool_kelvin":null,"warm_kelvin":2700,"boosted":false,
#             "devices":[{"id":5,"name":"キッチン1","room_name":"リビング"},…]},…],"intent":{"action":"on","at":"2026-10-05T21:10:00+09:00"},"runs":[…]}

curl http://homeserver:5011/api/lights/schedule
# {…,"runs":[{"at":"2026-10-05T23:00:00+09:00","trigger":"scheduled","sunrise":"05:39","sunset":"17:23","level":102,"kelvin":3000,"commands":2,
#   "lights":[{"node_id":1,"endpoint":3,"name":"キッチン1","room_name":"リビング","labels":["キッチン"],"target_level":76,"target_kelvin":2700,
#              "decision":"sent","level":76,"mireds":370},
#             {"node_id":5,"endpoint":1,"name":"読書灯","room_name":"寝室","labels":[],"target_level":102,"target_kelvin":3000,"decision":"off"},…]},…]}

curl -X POST http://homeserver:5011/api/labels -H 'content-type: application/json' -d '{"name":"キッチン","night_level":76,"warm_kelvin":2700}'
# 201 {"id":1,"name":"キッチン","day_level":null,"night_level":76,"cool_kelvin":null,"warm_kelvin":2700,"device_count":0,"boosted":false}

curl -X PATCH http://homeserver:5011/api/devices/5 -H 'content-type: application/json' -d '{"label_ids":[1]}'
# {"id":5,…,"labels":[{"id":1,"name":"キッチン"}],…}
```

0.1.19までの1台1つのlabel（`label_id`）は、更新後の最初の起動でそのまま `labels` へ移ります。
`PATCH /api/devices/{id}` の `label_id` は廃止し、送ると400 `invalid_device_update` です。

#### labelの照明を明るくする

`POST /api/lights/boost` は、labelの照明を明るくする状態を、押すたびに入れる・切るで切り替えます。
ウィジェットの「仕事」「作業」がこれを使います。状態はlabelごとにDBへ保存し、再起動しても保ちます。

- 入れると、そのlabelの照明（無視の照明を除く）を、消えていた照明も含めて点け、昼の値にします。
  昼の値は、その照明のlabelの `day_level`・`cool_kelvin` を重ねた全体の昼の値です。
  全部オンと同じく、オンの後に `MoveToLevelWithOnOff` と `ExecuteIfOff` 付きの `MoveToColorTemperature` をすぐに送ります。
  自動調整が無効でも働きます。
- 入れている間は、自動調整（0時以降の半減を含む）と全部オンが、その照明を昼の値のまま保ちます。
- もう一度押すと切ります。入れる前に消えていた照明は消し、点いていた照明はその時刻の目標値へ戻します。
  別の明るくしているlabelにも属する照明は、そのlabelを切るまで昼の値のままです。
- 全部オフは、すべてのlabelの状態を切ります。
- 入れたときに1台も点けられなければ、状態は入れません（`boosted` が `false`）。
- labelが無ければ404 `label_not_found`、無視でない照明が1台も割り当てられていなければ422 `label_has_no_lights` です。

```sh
curl -X POST http://homeserver:5011/api/labels -H 'content-type: application/json' -d '{"name":"作業"}'
curl -X PATCH http://homeserver:5011/api/devices/5 -H 'content-type: application/json' -d '{"label_ids":[1,2]}'
curl -X POST http://homeserver:5011/api/lights/boost -H 'content-type: application/json' -d '{"label":"作業"}'
# {"label":"作業","action":"boost","boosted":true,"switched":6,"no_response":0,"failed":0,"missing":0,
#  "lights":[{"node_id":1,"endpoint":3,"name":"キッチン1","room_name":"リビング","result":"switched"},…],"missing_devices":[]}
```

以前の版の色温度の下限（`min_kelvin`）は、更新後の最初の起動で、値ごとに `warm_kelvin` だけを持つlabel（`夜3500K` など）へ移り、
その照明に割り当てられます。下限は日の入り〜22時の途中でも効いていましたが、labelの `warm_kelvin` は夜の値なので、
その間は全体の `cool_kelvin` からlabelの値へ線形に下がります。`PATCH /api/devices/{id}` に `min_kelvin` を送ると400です。

### 例

```sh
curl -X POST http://homeserver:5011/api/rooms -H 'content-type: application/json' -d '{"name":"寝室"}'
# {"id":1,"name":"寝室","device_count":0}

curl -X POST http://homeserver:5011/api/devices -H 'content-type: application/json' \
  -d '{"room_id":1,"qr_payload":"MT:Y.K9042C00KA0648G00","name":"天井灯"}'
# {"id":1,"room_id":1,"room_name":"寝室","name":"天井灯","created_at":"2026-10-05T09:46:07Z"}

curl -X POST http://homeserver:5011/api/devices -H 'content-type: application/json' \
  -d '{"room_id":1,"manual_code":"3497 011 2332","name":"天井灯"}'
# {"error":"duplicate_qr_payload","message":"この機器は登録済みです"}

curl -X POST http://homeserver:5011/api/devices -H 'content-type: application/json' \
  -d '{"room_id":1,"vendor":"Tapo","serial_number":"CCBABDE0C244","mac":"CC:BA:BD:E0:C2:44","name":"読書灯"}'
# {"id":2,"room_id":1,"room_name":"寝室","name":"読書灯","vendor":"Tapo","serial_number":"CCBABDE0C244","mac":"CCBABDE0C244","created_at":"2026-10-05T12:30:00Z"}

curl -X POST http://homeserver:5011/api/commission -H 'content-type: application/json' \
  -d '{"room_id":2,"qr_payload":"MT:Y.K9042C00KA0648G00","name":"押入れ1","wifi_ssid":"home-2g","wifi_password":"…"}'
# 201 {"node_id":17,"registered":true,"device":{"id":9,"room_id":2,"room_name":"押入れ","name":"押入れ1","vendor":"Tapo",
#      "serial_number":"CCBABDE0C244","mac":"CCBABDE0C244","labels":[],"created_at":"2026-10-06T03:30:00Z"}}
# 422 {"error":"wifi_failed","message":"機器がWi-Fi「home-2g」に接続できませんでした"}

curl http://homeserver:5011/api/status
# {"devices":[{"endpoint":null,"id":1,"name":"天井灯","node_id":null,"room_name":"寝室","visible":false},{"endpoint":0,"id":2,"name":"読書灯","node_id":5,"room_name":"寝室","visible":true}],"version":"0.1.4"}

curl -X POST http://homeserver:5011/api/lights/off
# {"action":"off","switched":10,"no_response":1,"failed":0,"missing":1,
#  "lights":[{"node_id":1,"endpoint":2,"name":"Aqara LED Bulb T2","room_name":null,"result":"no_response"},
#            {"node_id":5,"endpoint":1,"name":"読書灯","room_name":"寝室","result":"switched"},…],
#  "missing_devices":[{"id":1,"name":"天井灯","room_name":"寝室"}]}

curl http://homeserver:5011/api/lights
# {"on":0,"off":10,"no_response":1,"missing":1,"lights":[{"node_id":5,"endpoint":1,"name":"読書灯","room_name":"寝室","state":"off"},…],"missing_devices":[…]}

curl -X DELETE http://homeserver:5011/api/rooms/1
# {"error":"room_has_devices","message":"この部屋には機器が1台登録されています。先に機器を削除してください"}
```

## 開発

ビルド、テスト、リリースの手順は[開発ガイド](docs/development.md)、
Androidアプリの画面の規則は[DESIGN.md](DESIGN.md)にあります。

## ライセンス

[MIT License](LICENSE)
