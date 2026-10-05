# home-link

住まいの機器を、部屋と名前で記録するサービスです。
電球などに印刷されたMatterのQRコードをAndroidアプリで読み取り、部屋と名前を付けて登録します。
登録した内容は、自宅サーバーで動くhome-linkのAPIがSQLiteに保存します。

構成は、Rust（axum）のAPIサーバーと、Kotlinで作ったAndroidアプリです。
Matterの機器登録（commissioning）と照明の操作は、現在のhome-linkにはありません。

## 公開範囲と認証

APIに認証はありません。家庭LANとTailscaleのtailnetなど、信頼できる経路にだけ公開してください。
インターネットへ直接公開しないでください。
MatterのQRコードには機器のsetup passcodeが含まれます。
そのためQRコードの全文は機器1台の取得APIだけが返し、一覧APIとログには出しません。

## サーバーの起動

[リリース](https://github.com/miyabisun/home-link/releases/latest)ごとに、
コンテナイメージ `ghcr.io/miyabisun/home-link` を公開しています。
データベースはコンテナ内の `/data` に置かれるので、ボリュームを割り当てます。

```sh
docker run -d --name home-link -p 5009:3000 -v home-link-data:/data ghcr.io/miyabisun/home-link:latest
curl http://127.0.0.1:5009/healthz
```

`ok` が返れば起動しています。
イメージは uid 10001 で動きます。ボリュームの代わりにホストのディレクトリを割り当てる場合は、
`--user` で実行ユーザーを指定し、そのユーザーが書き込めるディレクトリを使ってください。

| 環境変数 | 既定値 | 内容 |
| --- | --- | --- |
| `PORT` | `3000` | 待ち受けるTCPポート。1〜65535の10進数で、不正な値では起動しません。 |
| `DATABASE_PATH` | `home-link.db`（コンテナでは `/data/home-link.db`） | SQLiteデータベースのファイル。無ければ作成します。 |
| `LOG_LEVEL` | `info` | `off`・`error`・`warn`・`info`・`debug`・`trace` のいずれか。 |

## Androidアプリ

Android 17以降の端末と、Tailscaleへの接続が必要です。
アプリは `http://homeserver:5009` へ接続します。
`homeserver` はtailnetのMagicDNS名なので、Tailscaleが有効なら家の外からも使えます。
QRの読み取りにはGoogle Playサービスのコードスキャナーを使うため、カメラの権限は求めません。

[リリース](https://github.com/miyabisun/home-link/releases/latest)のAssetsから
`home-link-vX.Y.Z.apk` をダウンロードし、端末で開いてインストールします。
APKは開発用のdebug署名です。PCからADBで導入する方法と、ファイルの検証方法は
[開発ガイド](docs/development.md#adbでのインストール)を参照してください。

### 機器を登録する

1. アプリを開き、「QRを読み取る」で機器のQRコードを読み取ります。
2. 部屋を選びます。部屋が無ければ「部屋を追加」で作成します。
3. 必要なら機器名を入力し、「登録」を押します。機器名は空欄でも登録できます。

登録すると部屋の選択はそのまま残るので、同じ部屋の次の機器はQRを読み取るだけで登録できます。
同じQRコードの二重登録、存在しない部屋、サーバーへの接続失敗は、それぞれ理由を表示します。
失敗した場合も入力内容は消えないので、原因を直してから「登録」を押し直してください。

## API

すべてJSONです。失敗時は `{"error": "コード", "message": "説明"}` を返します。

| メソッドとパス | 内容 | 主な失敗 |
| --- | --- | --- |
| `GET /api/rooms` | 部屋の一覧（登録された機器の台数つき） | |
| `POST /api/rooms` | 部屋の作成。本文は `{"name": "寝室"}` | 400 `invalid_room_name`、409 `duplicate_room_name` |
| `PATCH /api/rooms/{id}` | 部屋の名前の変更。本文は作成と同じ | 404 `room_not_found`、409 `duplicate_room_name` |
| `DELETE /api/rooms/{id}` | 部屋の削除。機器が残っている部屋は削除しません | 404 `room_not_found`、409 `room_has_devices` |
| `GET /api/devices` | 機器の一覧。QRコードは含みません | |
| `POST /api/devices` | 機器の登録 | 400 `invalid_qr_payload`・`invalid_device_name`、404 `room_not_found`、409 `duplicate_qr_payload` |
| `GET /api/devices/{id}` | 機器1台。QRコードの全文 `qr_payload` を含みます | 404 `device_not_found` |
| `DELETE /api/devices/{id}` | 機器の削除 | 404 `device_not_found` |
| `GET /api/health`、`GET /healthz` | 稼働確認 | |

部屋の名前は前後の空白を除いて1〜100文字、機器名は0〜100文字です。
QRコードは `MT:` で始まるMatterのセットアップコードとして検証し、前後の空白は除いて保存します。

```sh
curl -X POST http://homeserver:5009/api/rooms -H 'content-type: application/json' -d '{"name":"寝室"}'
# {"id":1,"name":"寝室","device_count":0}

curl -X POST http://homeserver:5009/api/devices -H 'content-type: application/json' \
  -d '{"room_id":1,"qr_payload":"MT:Y.K9042C00KA0648G00","name":"天井灯"}'
# {"id":1,"room_id":1,"room_name":"寝室","name":"天井灯","created_at":"2026-10-05T09:46:07Z"}

curl -X DELETE http://homeserver:5009/api/rooms/1
# {"error":"room_has_devices","message":"この部屋には機器が1台登録されています。先に機器を削除してください"}
```

## 開発

ビルド、テスト、リリースの手順は[開発ガイド](docs/development.md)、
Androidアプリの画面の規則は[DESIGN.md](DESIGN.md)にあります。

## ライセンス

[MIT License](LICENSE)
