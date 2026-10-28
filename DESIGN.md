---
version: alpha
name: Sumi / home-link
description: home-linkのWebの管理画面とAndroidアプリの意匠。
colors:
  primary: "#9a6a00"
  accent: "#9a6a00"
  accent-subtle: "rgba(154, 106, 0, 0.10)"
  surface: "#faf6ef"
  surface-raised: "#fffdf8"
  on-surface: "#3a2f28"
  muted: "#6f6257"
  border: "#e3d9c9"
  scrim: "rgba(58, 47, 40, 0.4)"
  link: "#14506e"
  danger: "#9c2b1d"
  danger-subtle: "#f9e9e4"
  wash-base: "#f6efe0"
  wash-raised: "#faf4ea"
  hover-1: "rgba(154, 106, 0, 0.10)"
typography:
  title:
    fontFamily: system-ui
    fontSize: 17px
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: system-ui
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.6
  body-sm:
    fontFamily: system-ui
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: system-ui
    fontSize: 15px
    fontWeight: 500
    lineHeight: 1.2
  caption:
    fontFamily: system-ui
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.4
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  full: 9999px
spacing:
  sp-1: 4px
  sp-2: 8px
  sp-3: 12px
  sp-4: 16px
  sp-5: 24px
components:
  app-header:
    backgroundColor: "{colors.wash-base}"
    textColor: "{colors.on-surface}"
    height: 48px
  sub-header:
    backgroundColor: "{colors.wash-raised}"
    textColor: "{colors.on-surface}"
    height: 40px
  hairline:
    backgroundColor: "{colors.border}"
    height: 1px
  card:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.md}"
    padding: 10px
  button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-hover:
    backgroundColor: "{colors.hover-1}"
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface-raised}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-on:
    backgroundColor: "{colors.accent-subtle}"
    rounded: "{rounded.sm}"
  button-danger:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.danger}"
    rounded: "{rounded.sm}"
  icon-button:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.sm}"
    size: 36px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.on-surface}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: 8px
  modal:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.lg}"
    padding: 16px
  modal-scrim:
    backgroundColor: "{colors.scrim}"
  link:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.link}"
  error-banner:
    backgroundColor: "{colors.danger-subtle}"
    textColor: "{colors.danger}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 8px
  spinner:
    textColor: "{colors.accent}"
    size: 18px
  badge:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.muted}"
    typography: "{typography.caption}"
    rounded: "{rounded.full}"
    padding: 4px
---

# home-link

住まいの機器を、名前とlabelで扱えるようにする。
共通デザイン原本は「rust-svelte-template / DESIGN.md」（2026-10-21参照）。
そのSumi/Kinariと情報密度の規則を、Webの管理画面とAndroidアプリのAndroid標準Viewへ適用した。
frontmatterはKinariの値を持つ。

## Webの管理画面

2026-10-21のuser要求（task ecb6ccb0、「GitHubへのリンクではなく、機器を修正・管理する管理画面を作れと言ったはず」
「ラベル管理・機器管理はアプリではなく、Webサーバのフロントエンドの仕事である」「rust-svelte-templateを使って実装すること」）により、
機器とlabelの管理はhome-linkのサーバーが出すWebの画面で行う。家のLANとtailnetの出先から開く。
2026-10-20のuser要求（「なんで文字を削れってあれだけknowledge等で指示しているのに兎に角文字を増やして画面を埋めれば良いって発想になるのか」）により、
説明文を常設せず、画面の文字は対象の名前・状態・操作に要るものだけにする。

- 原本の画面構成をそのまま使う。全幅48pxのstickyなapp header（左にホームへ戻るアプリ名、右に36pxのメニューボタン）、
  詳細の40pxのsub-header（項目名だけを1行）、本文は中央最大720px・左右12pxの通常の文書フローとする。
  ブレークポイントは768pxで、上下の余白は狭幅16px・広幅24px。幅320px以上でページを横にはみ出させない。
- ホームは機器の一覧、labelの一覧はメニューから開く。メニューの先頭は原本どおりテーマ設定とし、ホームと同じ画面の項目は置かない（担当判断）。
- 一覧はカードの1列とし、名前を主役にする。所属するlabelや設定の要約は注記の大きさとmutedで添え、
  入っている状態と注意の要る状態だけを枠付きのバッジで示す（切れている・接続済みの通常の状態は示さない）。名前の無い機器は機器自身が報告する名前で示す。
- task 14c0f679の要求（2026-10-26）により、登録が外れた照明（fabricからの離脱）は
  応答しない照明・未接続の機器と分け、失敗の色の枠付きバッジで示す。
  task 7c6dd8f8（2026-10-26）でhome-linkが自動で登録し直すようになったので、注記には戻し方の代わりに
  「自動で登録し直し中」と、戻らない理由・試した回数・14日で何回目かを注記の大きさで添える。
  2026-10-26のuserの指摘（「網」「fabric」の言い換えと補足を退けた）により、画面の文は「登録が外れた」と書く。
- 2026-10-28のuser要求（task 6dc7f063、「Discord通知の機能をオミットし、home-linkの画面から状態を確認出来るようにしてください」）により、
  外へ知らせを送らず、userの操作が要る状態はホームの一覧の上に失敗の面の行（1件1行、選ぶと詳しい画面へ）で示す。各画面にもその画面の分を上部に示す。
  文は何が・いつから・userの取る操作だけを書く（task 9f72ded4、2026-10-26のuserの指摘「the ai slop」）。
  仕組みの説明、出来ない理由、分からないことは書かない。home-linkが自分で直している途中の状態は行にせず、カードのバッジと注記に任せる。
- 詳細は上から1列で、注記の大きさのラベルを入力の上に置く。変えた値はその場で送り、保存ボタンは置かない。
  入力の誤りはその入力の直下に失敗の色で示し、それ以外の失敗は本文の先頭に失敗の面で示す。どちらも読み上げる。
- 入り切りの操作は状態を文字で示すボタンとし、入っている間はaccent-subtleの面とaccentの縁で示す。
- 選べる項目が複数のときはチェックボックスの列、1つを選ぶときはセレクトボックスとする。チェックボックスはaccentで塗り、行の高さは44px以上とする。
- 削除は台帳やlabelの設定を失うため、名前を示した確認のモーダルを開いてから行う（担当判断）。
- 塗りつぶす主操作は画面に最大1つとする。削除のボタンは通常のボタンに失敗の色の文字で示す。
- テーマはOSに追従し、メニューのテーマ設定（自動・ライト・ダーク）で固定できる。設定はブラウザのlocalStorageの `home-link:theme` に保存し、初回描画前に適用する。
- 色・寸法・部品・フォーカス・動きは原本の値を使い、client/src/global.sassのCSS変数に集める。アイコンは原本の辞書（client/src/lib/Icon.svelte）から使う。

## 画面の確認（Web）

同じデータで、幅1280と390、明暗の両方を実ブラウザで撮り、ページの横のはみ出しが0、本文に内側の縦スクロールが無いこと、
390pxのheaderの操作対象がアプリ名とメニューの2つだけで、メニューがボタンの右端にそろうことを確かめる。
長い機器名、未接続、空と読込の失敗、入力の誤り、削除の確認を含める。

## Androidアプリ

- 2026-10-21のuser要求（task ecb6ccb0・2059250e、「ラベル管理・機器管理はアプリではなく、Webサーバのフロントエンドの仕事である」
  「ラベル・機器の大項目を削除」）により、アプリはlabelと機器の一覧・labelの画面を持たず、画面は機器の登録だけとする。
  labelの入り切りはウィジェット、管理は上の「Webの管理画面」が受け持つ。
- 同じ要求（「接続先の設定が一番下なのは意味わからん、上のヘッダー部分右上にハンバーガーメニューを作ってそこに入れる」）により、
  原本の画面構成に合わせ、画面の上に48dpのapp header（wash-base、下に1dpの境界）を置く。左にアプリ名、右端にメニューのボタン（48dpの操作領域）を置き、
  接続先の設定はメニューから開く。メニューはボタンの右端にそろえて下へ開き、surface-raised・1dpの境界・角丸12dp・最小幅180dpで、項目は全幅の48dpの行とする。
  開いた画面にはアプリ名（押すと戻る）のheaderと、画面名だけの40dpのsub-header（wash-raised）を置く。
- 同じ要求（「セクションが終わってるんだからこの接続は何も動かないはずだろう」）により、ボタンはそれが働く対象の面の中に置き、面の外に置かない。
  働かない状態のボタンは出さない。接続中は接続できない他の機器の一覧とコードの読み取りを出さない。
- 2026-10-20のuser要求（task 79bec4f7、「WiFiの電球とかThreadの電球とかの区分より真っ先にMatterのQRコードを読むように」
  「読んだその場でhome-linkのサーバーに問い合わせる」「未登録ならその場で登録」「編集画面等で編集出来るように」）により、
  機器の登録はQRの読み取りから始める。読んだらすぐ台帳に入れ、その機器の面を開く。台帳にある機器なら同じ機器の面を開く。
  機器の面ではコード・接続の状態・名称・labelを、上から1列で示し、変えた値はすぐ送る。結果の行と接続・取り消し・Wi-Fiを移す操作も同じ面に置く。
- 同じ要求（「アプリを立ち上げた瞬間にサーバーに登録機器を問い合わせて、未登録機器の一覧を出して、選んで使えるように」）により、
  開くたびに台帳を読み、未接続の機器を読み取った順に一覧で示す。網から外れた機器は一覧の名前にその状態を添え、機器の面では状態を失敗の色とアイコンで示す（task 14c0f679）。選ぶとその機器を一覧のすぐ上の面に開いて接続を始め、
  段階と結果をその面に示す。開いている機器は一覧に重ねて示さない。接続できなかった機器は面に残り、面の接続で試し直せる。
- 2026-10-20のuser要求（task 79cfffb3、「アプリから電球前提という前提を取り払ってください」）により、
  照明の操作以外の文言は電球に限らず「機器」とする。プラグの面には接続の状態の横に入り切りのボタンを置き、
  入っている間はアクセントの面と状態の文言で示す（ボタンは担当判断）。
- 結果は操作のボタンの直前に、アイコンと文で示す。入力に関わる失敗はその入力の面に示す。
  失敗では入力を保持し、直す箇所と再試行の操作を示す。台帳を読めないときと接続先が未設定のときは、画面の最初の面に示し、
  その面に再接続か接続先の設定への入口を添える。
- 画面の最初の面は、止まった理由を分けて1つだけ示す（担当判断、task 6edcfd4d）。順に、設定ファイルを読めない・形が不正・未設定・URLの形の誤り、
  LANへの接続の未許可（「LANへの接続を許可」、Androidが求めなくなったら「設定アプリで許可」）、読み込みの最初の失敗（「再接続」）である。
  接続先の問題には接続先の設定への入口を添える。
- 時間のかかる接続とWi-Fiの移動は、画面を離れても続け、戻った画面の機器の面に結果を示す（担当判断、task 6edcfd4d）。
  アプリが途中で終了したときは、次に開いた画面に「確かめています…」を示し、home-linkから読み直した結果に置き換える。
- 2026-10-20のuser要求により、確定したコードは省略せず、機器のラベルと照合できる形で示す。
  QRはpayloadから求めた11桁の手動ペアリングコード、数字は入力した11桁を、どちらも4-3-4のハイフン区切りで示す。
  11桁に直せないQR（複数機器の連結や独自の手順のもの）はpayloadを全文で示す。
- 2026-10-05のuser要求により、QRの無い機器は同じ面で11桁の数字を入力できる。
  入力欄は面の中で開閉し、数字のキーパッドで4-3-4に区切って表示する。誤りは入力欄の下にその場で示す。
- 2026-10-06のuser要求により、新しいWi-Fi電球はアプリから電話のBluetoothでつなぐ。
- 2026-10-20のuser要求（task 17b9c20c、「スマホではボタン1個だけでWiFi設定が完了するようにして欲しい」）により、
  機器に渡すWi-Fiはhome-linkに一度だけ保存し、登録の列では選ばせず入力させない。未保存の間だけ、登録の列に面と設定の画面への入口を出す。
  保存・選択・削除は設定の画面の面で行う。一覧はセレクトボックスで選び、追加は面の中で開閉し、
  今つながっているWi-Fiの名前を入れてパスワードだけを求める（2026-10-07のuser要求）。
  今のWi-Fiが2.4GHz以外なら、2.4GHzにしかつながらない機器があることを追加の面に示す。
- 同じ要求により（task cedd8851の追記）、接続済みの機器を別のWi-Fiへ移すときも、電話でWi-Fiを選ばせたり入力させたりしない。
  機器の面の主操作のボタン1つで、保存した機器用のWi-Fiへ移す。結果は接続と同じ結果の行に示す。
- 2026-10-17のuser要求（task b156e585、「接続先アドレスを…アプリ内で設定出来るようにしてください」）により、
  接続先は別の設定の画面で変える。入力の誤りは入力欄の下に示し、保存しない。
- 時間のかかる接続は、機器の面の結果の行に進行中の段階をアイコンと文で示す。
  2026-10-20のuser要求（task a1c0ea0b、「キャンセルボタン作って欲しい」）により、接続中は接続ボタンの代わりに取り消しの操作を出す。
  取り消すと待たずに接続の前へ戻り、後から届いた結果は示さない。
  画面を作り直しても同じ機器の面と、接続の段階と結果を示す。

## 色と部品

OSの明暗設定に追従する。独自のテーマ切替は置かない。値はAndroid resourceへ集約する。

| 用途 | Kinari（明） | Sumi（暗） |
| --- | --- | --- |
| 背景・入力欄 | #faf6ef | #191919 |
| 面・通常ボタン・メニュー | #fffdf8 | #232323 |
| app header | #f6efe0 | #232323 |
| sub-header | #faf4ea | #191919 |
| 本文 | #3a2f28 | #e6e6e6 |
| 補助文字・無効 | #6f6257 | #9a9a9a |
| 境界 | #e3d9c9 | #333333 |
| 主操作・成功 | #9a6a00 | #e0a800 |
| 主操作上の文字 | #fffdf8 | #191919 |
| 失敗 | #9c2b1d | #ff6b6b |
| 失敗の面 | #f9e9e4 | #3a1a1a |

- 本文のコントラストは4.5:1以上。成功・失敗は色だけに頼らずアイコンと文で示す。
- 説明文と補助文字を常設しない。面の見出し・入力の上の注記の大きさの名前・状態・結果・操作だけを示す
  （2026-10-20のuser要求、「なんで文字を削れってあれだけknowledge等で指示しているのに兎に角文字を増やして画面を埋めれば良いって発想になるのか」）。
- Android標準のButton/EditText/Spinnerとシステム書体。本文16sp、注記14sp、面の見出しとアプリ名18sp、ウィジェットの設定の画面の見出し28sp。
- 余白は4/8/12/16/24dp。面と部品は角丸8dp・境界1dp。影や説明帯は加えない。
- 塗りつぶす主ボタンは画面ごとに1つ（接続、設定の画面とウィジェットの設定の保存）とし、押せない間は通常ボタンと同じ面にする。
- セレクトボックスは入力欄と同じ面に開閉の矢印を付け、ボタンと区別する。
- アイコンは24×24、2dpの線、丸い端と角。絵文字や文字記号で代用しない。
- 操作領域は48dp以上。文字拡大で高さが増え、全操作へスクロールで到達できる。
  system insetとキーボードを避け、システムバーのアイコンも明暗へ追従する。
- 結果と入力の失敗はlive regionで読み上げる。装飾のアイコンは読み上げない。

## ウィジェット

- アプリと同じtoken（背景・面・本文・境界・アクセント・失敗）とアイコンを使い、OSの明暗に追従する。
  文字色と面はresourceで指定し、OSの明暗の切替に描き直しなしで追従させる。
- 2026-10-21のuser要求（task 9aca0242、「ラベルに対して個別にON/OFFをWidget登録出来るものとする」「ON/OFFボタン 1x2」「ONボタン 1x1」
  「OFFボタン 1x1」「トグルボタン 1x1」「作業用のWidgetは削除する」）により、照明のウィジェットをlabelごとの4種類に置き換える。
  ON/OFFは2つのボタンを横に並べた横2マス、ほかは1マスとし、操作領域を48dp以上にする。
- 同じタスクの追記（「こういうWidgetをホーム画面に追加する時に名前を設定出来る機能ありましたよね？」）により、
  置くときにlabelと名前を選ぶ画面を出す。名前の既定はlabelの名前とし、置いた後もウィジェットの設定から選び直せる。
- エアコンの自動調整は、1×1の切り替えのウィジェット1つで止めて戻す。
  2026-10-23のuser要求（task 50697706、「シンプルにON/OFFの切り替えにしてください」）による。形と状態の示し方はlabelの切り替えと同じ。
- 玄関の解錠と施錠はlabelのウィジェットに並べず、それぞれ1×1の別のウィジェットにする（担当判断、task 1c3a12b8・ca52b49e。押し間違いで玄関が開くのを避ける）。
- 2026-10-14のuser要求（task 65bf3399、「radiusが一致してなくて解錠のradiusが汚い」「照明オン・オフ系が謎に細い」
  「解錠の間延びして間抜けに見える」）により、すべてのウィジェットのボタンを同じ部品で作る。
  - 外枠は背景の面だけで境界を持たず、角丸はシステムのウィジェットの値、内側の余白は8dpとする。
  - ボタンは外枠の高さいっぱいに広げ、面・境界1dp・角丸（外枠の角丸から余白を引いたシステムの内側の値）を共有する。
  - アイコンを文字の上に置き、2つをまとめてボタンの中央に置く。文字は1行で、収まらなければ縮める。
- 同じ要求（「作業中・仕事中と見分ける状態が何処にもなく」）により、切り替えのボタンは入っている間、
  アクセントの面・別のアイコン・状態を示す文言で示し、色だけに頼らない。サーバーの状態を正とし、読めないときは入っていると示さない。
- 切り替えの状態は入・切・不明の3つで持つ（担当判断、task bf74e018）。読み込みに失敗しても前の状態を残し、
  一度も読めていないときと、押した要求に応答が無く結果の分からないときだけ不明とする。
  不明は「?」のアイコンと、読み上げの「（状態不明）」で示し、切と同じ見た目にしない。不明を切と見て押すと、逆に切れるためである。
- 押したボタンの中で進行を短く示す。切り替えは押した後に新しい状態を示し、それ以外の結果と失敗は短く示して少し後にボタンの表示へ戻す。
  縦に広げたときは、結果をボタンの下に示す。
- 失敗の短い表示は、止まった所で分ける（担当判断、task bf74e018。一覧はREADMEの「ボタンの表示」）。
  分け方は、スマホ・home-linkまで・送ったが応答なし・home-linkの手前・home-linkの奥・照明である。
  すべての画面とウィジェットで1つの変換を使う。
  最も狭いボタン（横2マスの半分）でも縮めずに読めるよう、全角5文字以内にする。6文字の「照明応答なし」は端が切れた（emulatorで観測）。
  読み上げには理由と次にすることの文を付ける。
- 接続先がLANのアドレスだけでAndroid 17のLANへの接続の許可が無いときは、送っても止められるので、ボタンを「LAN未許可」の失敗の表示にし、
  押すとアプリを開いて許可を求める（担当判断、task 6edcfd4d）。許可されたらボタンへ戻す。

## 画面での確認

同じ機器のデータで、明暗、文字2倍の狭い画面、長い機器名・label名、
読込失敗、QRからの登録、未接続の一覧、Bluetoothでの接続の段階・成功・各失敗、画面の再生成を実画面と操作で確認する。
登録の画面のどの場面でもボタンが面の外に無いこと、ヘッダーの右端のメニューから接続先の設定が開くことを確かめる。
