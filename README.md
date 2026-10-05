# home-link

住まいの機器を、部屋と名前で記録するサービスです。
家じゅうの照明を、Androidのボタン1つで全部オン・全部オフにできます。
電球などに印刷されたMatterのQRコードをAndroidアプリで読み取り、部屋と名前を付けて登録します。
QRコードが無い機器は、印字された11桁の数字（Matterの手動ペアリングコード）を入力して登録します。
登録した内容は、自宅サーバーで動くhome-linkのAPIがSQLiteに保存します。
機器自身の識別子（ベンダー名とシリアル番号、Wi-Fi機器はMACアドレス）も保存でき、
Matterのcontroller（[matterjs-server](https://github.com/matter-js/matterjs-server)）を作り直しても、
台帳の機器が今どのnodeにいるかを識別子から引き直します。

構成は、Rust（axum）のAPIサーバーと、Kotlinで作ったAndroidアプリです。
照明の操作は、matterjs-serverに登録済みの照明を全部オン・全部オフにするものだけです。
照明ごと・部屋ごとの操作と、Matterの機器登録（commissioning）は、現在のhome-linkにはありません。

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
| `MATTER_SERVER_URL` | なし | matterjs-serverのWebSocket API（例: `ws://192.168.1.100:5580/ws`）。未設定なら状態APIと照明APIは503を返します。 |

## Androidアプリ

Android 17以降の端末と、Tailscaleへの接続が必要です。
アプリは `http://homeserver:5011` へ接続します。
`homeserver` はtailnetのMagicDNS名なので、Tailscaleが有効なら家の外からも使えます。
QRの読み取りにはGoogle Playサービスのコードスキャナーを使うため、カメラの権限は求めません。

[リリース](https://github.com/miyabisun/home-link/releases/latest)のAssetsから
`home-link-vX.Y.Z.apk` をダウンロードし、端末で開いてインストールします。
APKは開発用のdebug署名です。PCからADBで導入する方法と、ファイルの検証方法は
[開発ガイド](docs/development.md#adbでのインストール)を参照してください。

### 照明を全部オン・全部オフにする

アプリの上部にある「照明オン」「照明オフ」を押すと、matterjs-serverに登録された照明を全部切り替えます。
結果は切り替えた台数と、応答のなかった照明・失敗した照明・台帳にあるのに見つからない機器の台数で示します。
1台でも切り替えられなかった照明があれば、失敗の色で表示します。

「ホーム画面にボタンを置く」を押すと、同じ2つのボタンのウィジェットをホーム画面へ置けます。
ウィジェットはボタンの下に、直前の結果を短く表示します。

### 機器を登録する

1. アプリを開き、「QRを読み取る」で機器のQRコードを読み取ります。
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
| `DELETE /api/devices/{id}` | 機器の削除 | 404 `device_not_found` |
| `GET /api/status` | 台帳の機器ごとに、matterjs-serverで見えるか（`visible`）と今の `node_id`・`endpoint` | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `GET /api/lights` | 照明ごとの今の状態（`on`・`off`・`no_response`）と、その件数 | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/on` | 全部の照明をオンにし、照明ごとの結果（`switched`・`no_response`・`failed`）と件数を返します | 503 `matter_server_not_configured`、502 `matter_server_unreachable` |
| `POST /api/lights/off` | 全部の照明をオフにします。応答は `on` と同じ形です | 同上 |
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

`GET /api/status` は、そのたびにmatterjs-serverの全nodeを読み、識別子から機器の今の `node_id` と `endpoint` を引きます。
node IDとendpointはmatterjs-serverが振る番号なので台帳には保存しません。
`visible: false` の機器は、matterjs-serverに見えないか識別子が無い機器で、matterjs-serverへの登録し直しが必要です。

照明APIは、要求のたびにmatterjs-serverの全nodeを読み、照明の種類（On/Off・Dimmable・Color Temperature・Extended Colorの各Light）を持ち、
On/Offの属性があるendpointを照明として扱います。Aqara Hub M3などのブリッジ配下の照明も、endpointごとに1台と数えます。
nodeが使えない照明と、ブリッジが届かないと報告している照明には命令を送らず、`no_response` とします。
命令を受け付けた照明だけを `switched` に数え、エラーが返った照明は `failed`、10秒以内に応答の無かった照明は `no_response` です。
途中で接続が切れた場合は、つなぎ直して全部の照明へ送り直します（オン・オフは状態に依らない指定なので、二重に届いても結果は同じです）。
照明の `name` は台帳の機器名で、台帳に無い照明は製品名です。台帳の機器のうちmatterjs-serverに見えないものは `missing_devices` に並び、
`missing` に数えます。状態の `GET /api/lights` は、matterjs-serverが最後に読んだOn/Offの値を返します。

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
