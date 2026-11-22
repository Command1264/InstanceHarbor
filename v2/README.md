# InstanceHarbor Windows 1.0.0-rc2

新候選使用 `build-product.ps1` 建置並核對內嵌 Vite 資產；`launch-product.ps1` 只開啟本次已核對的隔離 Agent 工作區。
舊的 `preview/` 與無框架 `index.html` 保留為純模擬對照，不會自動切換到真實控制。

本目錄是 Windows 第一版功能候選。Vue／Tauri 與 PowerShell 呼叫的 Rust CLI 使用相同的 Rust 管理入口。分開執行的 Rust Host 持有 Java 的標準輸入與日誌。GUI 或 CLI 退出後，Host 仍能持有原程序。重新核對使用 run ID、PID、Windows 程序建立時間和本機 TCP 監聽者。正常停止經有期限的 Named Pipe，驗證雙方程序身分。重啟需先確認原 Java 和 Host 退出、正常結束且埠已釋放。

## 建置與隔離驗證

在本目錄執行下列命令。驗證腳本使用 SHA-256 核對 Paper 26.3 build 205 和 Mojang 依賴，唯讀取用既有 JAR 快取。所有新工作區和證據寫入本目錄的 .evidence。

```powershell
npm ci --offline
npm run build
cargo build --offline --locked --manifest-path src-tauri/Cargo.toml
./launch-acceptance.ps1
```

驗證器建立一台實例，驗證重試，移除管理登記，再重新匯入原目錄。它模擬其他程序占用埠，啟動 Paper 至 Done，關閉與重開原生 Tauri，從 GUI 發出重啟，最後經 CLI 正常停止並確認埠釋放。已驗證的工作區可直接用 launch-acceptance.ps1 開啟。啟動 GUI 本身只讀取清單。

## 驗收步驟

1. 關閉可能仍開著的 2.0 驗收視窗，執行 launch-acceptance.ps1。選取 paper-agent，應顯示已停止。
2. 點啟動，應先看到原位處理回饋，接著出現 run 和監聽 PID。關閉並重新開啟 GUI，核對同一個 run。
3. 點重新啟動。正常停止並啟動成功後，核對 run 已變更。點工作紀錄確認可查到作業。
4. 最後點正常停止，確認清單回到已停止。如果出現工作階段未確認或停止逾時，保留現場並回報畫面與工作 ID。

## 系統匣

- 主程式在 Windows 通知區域（右下角）顯示圖示。滑鼠停在圖示上會顯示執行中的伺服器台數。
- 左鍵點圖示，或右鍵選「顯示應用程式」，可叫回主視窗。再次執行 `InstanceHarbor.exe` 也會叫回隱藏的視窗。
- 「語言與外觀」的「關閉視窗時縮到系統匣」預設開啟：按 X 只隱藏視窗。關閉此設定後，按 X 會結束介面。
- 右鍵選「結束」才會結束 GUI 程序。伺服器、Host 與 Manager 在任何情況下都不隨 GUI 結束。
- 回退相容：此設定維持預設時不寫入 `preferences.json`。若已關閉此設定，回退到 `1.0.0-rc2` 前請先重新開啟並儲存，否則舊版無法讀取外觀設定。

## 介面語言

- 介面文字放在 `src/i18n.ts`（共用字串）與 `src/messages/*.ts`（依區域分檔）。每筆為 `[繁體中文, English]`，兩種語言的 `{參數}` 必須相同。
- 元件用 `useAppLanguage()` 的 `t()`；元件外的程式（例如 `manager.ts`）用 `tr()`，同樣跟隨目前語言。
- `src/hardcoded-text.test.mjs` 會在 `.vue`、`.ts` 出現寫死的中文時失敗。資料用途的中文（語言名稱、後端送來的分類 ID）列在測試的允許清單。開發者用的 `console` 訊息使用英文。
- 「初始化預設」與世界設定的欄位資料由後端 `policy_catalog.rs` 提供。後端回傳的中文 `label`、`reason`、`source` 保持不變，另附 `reasonCode`、`sourceKind`、`sourceKey`；前端依欄位 key 與這些代碼翻譯（`src/messages/policy.ts`）。新增欄位或原因時，`policy-catalog.test.mjs` 會要求補上翻譯，且中文必須與後端一致。

## Windows 檔案存取與部分清單

- `ihctl WORKSPACE list` 保留 JSON 陣列，個別登記無法讀取時會把原因寫到標準錯誤。
  自動化可使用 `list --details`，從同一份 JSON 取得 `rows` 與 `issues`。
- 清單優先向已啟動的 Manager 查詢。沒有可用的 Manager 時，唯讀核對已提交的登記與掃描規則。
  清單不建立或寫入 `manager.lock`。如果兩次檢查間資料仍變動，短暫重讀後回報清單尚未核對。
- 清單有個別異常時，GUI 仍顯示其餘可核對的實例，並列出檔名和原因。
  若無法排除登記衝突，啟動會被拒絕。已核對的運行中實例仍可正常停止。
- 外觀設定、全域 Java 與掃描範圍的短暫共用違規會有限重試。
  無法讀取外觀設定時，GUI 保留最後讀到的設定，若沒有讀取紀錄則顯示尚未讀取。
  無法讀取或核對原檔時，設定儲存會保留原檔。
  外觀草稿若與磁碟上的版本衝突，重試讀取後可核對草稿，再明確儲存。
- `preferences-snapshot` 回傳 `preferences` 和 `revision`，供編輯器呼叫
  `save-preferences JSON REVISION` 時拒絕舊草稿。已存在的單參數 CLI 呼叫仍相容。
- 可用 `tests/verify-file-access.ps1` 在全新隔離目錄驗證 CLI，接著執行
  `tests/verify-native-file-access.mjs` 驗證原生 GUI。測試使用合成登記，無須啟動 Java 或 Minecraft。

## 代理與插件唯讀診斷

選取已登記的後端實例，展開「工作階段與診斷資料」後手動核對。可指定已登記的 Velocity、路由別名，
以及後端 `plugins/` 內的 JAR 檔名。留空時，診斷會嘗試找出唯一符合已登記端點的路由，
並以建服收據上的插件雜湊辨識插件。無法唯一辨識時可手動指定。

```powershell
$options = @{ proxy = 'diagnostics-velocity'; route = 'paper'; plugin = 'LuckPerms-Bukkit-5.5.85.jar' } | ConvertTo-Json -Compress
& $ihctl $workspace instance-diagnostics diagnostics-paper $options
```

配置結果取自磁碟上的 `velocity.toml`、後端 `server.properties` 與實例登記。首版只比對明確的 IPv4 與連接埠。
運作結果核對兩端當次 Run ID、程序身分、監聽程序和就緒證據。只有當次日誌配對、仍可核對兩端的本機連線，
而且代理路由配置沒有在啟動後修改時，才顯示已觀察到轉送。插件 JAR 存在、開始啟用、啟用成功、失敗和停用各有狀態，
依當次 Host 日誌判定。格式不明、證據過期或讀取期間變動時，回報未確認。

這項查詢不寫入工作區或應用程式日誌，不送後台指令，也不進行 DNS 或遠端探測。
玩家連線在 GUI 與 CLI 一律顯示未驗證，實際登入證據記錄在隔離驗收結果。

## 新增管理資料夾

在實例頁選擇「新增管理資料夾」，用 Windows 資料夾選取器指定目前工作區的 `instances`
或其子目錄。選取後會自動唯讀掃描，單台和多台使用同一個入口。可登記的伺服器預設勾選；
取消勾選子資料夾會在儲存時加入黑名單。儲存新增的白名單後，程式在原地登記符合條件的伺服器，
並回報逐台結果。登記不會啟動伺服器或修改來源檔案。首次啟動時核對本機 Java；
若無相容版本，保留登記並可從實例設定指定 Java。
管理服務持續核對已儲存的白名單，在後續發現符合條件的伺服器時也會登記。沒有白名單時停止自動探索。

白名單與黑名單的每列都能手動輸入、用資料夾圖示開啟選取器，或用垃圾桶圖示確認移除草稿。
已有路徑的列從該路徑的父資料夾開啟選取器。空白列從程式所在資料夾開始。
編輯時逐列核對資料夾，黑名單每列必須位於某個白名單資料夾底下，且排除優先。
按「儲存」時核對管理範圍的影響並顯示確認。取消確認會保留草稿。
規則無效時保留前一次已儲存的名單。從管理範圍排除運行中的實例後，仍可在原工作階段正常停止。
由管理資料夾登記的實例在停服後才離開清單。以往已登記和透過「建立實例」新增的實例會在白名單調整後保留，
明確加入黑名單時則遵循相同的排除及停服流程。

自動登記需要可核對版本與雜湊的 `server.jar` 和同層設定。需另外確認非 loopback 監聽位址的
Velocity 保留待處理。掃描略過連結及排除路徑，回報名稱、路徑、連接埠及管理權衝突。
達到掃描上限時，請縮小範圍後重新預覽。建立新實例仍由「建立實例」處理。舊版 CLI 匯入命令
及匯入工作紀錄維持相容，桌面版的日常入口是「新增管理資料夾」。

## 全域 Java 與建立實例

「應用程式設定」的 Java 預設自動挑選與已核對核心相容的本機版本，也可以儲存指定的 `java.exe`。
模式與手動路徑各自保留。切到自動時，路徑欄仍可查看，切回手動時才能編輯或選擇檔案。
新建實例及管理資料夾自動登記的實例會繼承全域設定。已儲存的單台 Java 路徑優先，
清空可繼承實例的 Java 路徑會恢復繼承。變更於下次啟動生效。

全域手動模式與實例的「從已安裝的 Java 選擇」使用同一個可重新掃描的清單。清單顯示已核對版本、來源與執行檔路徑，
有 JDK release 資訊時也顯示架構與供應商。選取列的版本、供應商及路徑共用選取配色的前景色。
手動重新探索時，按鈕將「探索中」與後方的進度指示一起置中，並保留現有清單。減少動態效果模式使用靜態指示。
全域「Java 搜尋範圍」可分別開關 Windows 常見安裝位置、mise、`JAVA_HOME` 與 `PATH`。
搜尋資料夾最多可新增 16 筆。自訂搜尋從指定目錄依深度往下找 JDK，包括同一根目錄的直接安裝與 mise 安裝。
最多深入六層，每個根目錄走訪最多 32,768 筆目錄項目，單一目錄最多檢查 1,024 筆，且略過連結目錄。每次最多探測 32 個自訂候選，
並依實際執行檔路徑去重。
全域 Java 的「查看搜尋結果」在兩種模式都可使用。清單以唯讀方式顯示目前搜尋範圍，包含未儲存草稿。
「從已安裝的 Java 選擇」在自動模式可見並停用。切到手動模式即可從目前搜尋範圍挑選版本並填入草稿。
搜尋範圍變更後會在背景預先探索。兩個視窗共用符合目前草稿的結果，快速連續修改時合併為最後一次搜尋。
開啟視窗時若目前範圍還未有結果，清單位置會顯示不定進度長條。「重新探索」可手動更新。
伺服器自動配對依已儲存的搜尋設定。Java 模式、手動路徑與搜尋範圍有變更時，頁尾「儲存設定」可用。
按下後核對必填欄位及搜尋範圍，有錯誤會保留草稿並提示修正。

新版工作區將搜尋範圍儲存在 `java-discovery-scope.json`。首次載入時，舊的
`java-discovery.json` 單一路徑會顯示為第一筆搜尋資料夾，舊檔保留供舊版回退使用。
回退時舊版使用原單一路徑與原預設來源。重新啟用新版後會讀回新版儲存的多路徑設定。
先前驗收版儲存的排除值只供讀取舊格式，新版 Java 搜尋會忽略其內容。

自動模式依已核對的核心類型與 Minecraft 版本，從已安裝的相容 Java 主版本挑選較新的穩定版本。
建立預覽會顯示建議主版本。官方下載建立在找不到相容 Java 時保留草稿，並提示檢查全域 Java。一般登記後缺少相容 Java 的實例可在實例設定手選。
啟動前重新核對核心和 Java，每次成功派工會在工作階段記錄使用的執行檔。已儲存的實例手動路徑優先。
Windows 預設 mise 搜尋使用帳戶本身的 Local AppData。隔離日誌的位置可獨立指定。明確設定的 `MISE_DATA_DIR` 仍優先。

建立實例時指定名稱、Port、工作區 `instances/` 內的父資料夾及核心來源。
名稱限 1 至 64 個小寫英文字母、數字或連字號，開頭使用字母或數字。版本寫入名稱時可使用 `test-paper-26-3`。
確認畫面會顯示完整目的地。如果目的地尚未被白名單涵蓋，會一併列出要加入的實例資料夾。
本機模式只指定核心 JAR，需要的 Mojang JAR 會在核心旁及已知的工作區快取中以 SHA-256 核對。
資料夾與檔案欄使用各自的 Windows 選取器，預設位置依相鄰欄位決定。

Paper 與 Folia 各自列出官方 Minecraft 正式版本。選版後核對該版本最新的穩定建置。
確認畫面固定顯示要取得的 Build。確認後會重新核對該 Build 仍在官方穩定清單，
下載核心與配套 Mojang JAR，驗證 SHA-256 後才登記。無穩定建置時可另選版本。
這個版本若有已核對的初始化對應，會沿用工作區預設。其他版本使用核心預設，保留 loopback、
指定連接埠、正版驗證及停用 RCON 的基本設定。

開啟 GUI 時會載入 WebView 中最後成功取得的公開版本資料，再查 Paper 與 Folia，
每 30 分鐘刷新版本與當前選取的建置。等待時保留同一選擇的快取。當選擇器可見且尚未取得
結果時，慢速請求才顯示不定進度條。版本快取供顯示，建立時仍須與官方來源核對。

Spigot 使用 [官方 BuildTools](https://www.spigotmc.org/wiki/buildtools/) 在應用程式外建置。
本機 JAR 建立入口仍接受已核對的核心雜湊。若自行建置的 JAR 與既有核對清單不同，
可先完成伺服器資料夾與連接埠設定，再由 CLI 選擇明確的 Java 匯入。這條路徑適用於
已確定來源的本機成品，日常的管理資料夾自動登記仍依原有核心核對規則。



## 候選邊界

### Windows 未整合項目

目前開發檢查點的初始化政策與既有世界設定，請見
Windows 初始化與世界設定。
下載請求預設使用 [InstanceHarbor](https://github.com/Command1264/InstanceHarbor) 作為 PaperMC 聯絡網址。
已驗證的下載檔案可在同一工作區重新使用。官方下載建立仍會向官方核對當次確認的建置。
可寫欄位包含已確認的 `server.properties`、新版地獄維度設定，以及已核對版本的 Spigot 數值型 YAML。
新建時已選取的遊戲規則和初始時間在第一個可核對的就緒工作階段套用一次。結果未確認時保留原工作記錄，後續啟動不重送。
既有世界的設定檔與遊戲規則可逐列編輯，由右下角「儲存」提交。規則在下一次已核對的啟動套用，
Manager 在正常停止後自動比對存檔。非預期退出時保留尚未核對的規則目標。
調整目前世界時間是獨立操作，確認後由 Host 送往同一個就緒工作階段並立即讀回。它不加入下次啟動佇列。
世界時間仍在流動時，以即時讀回為準。
執行期間提交的檔案與實例設定會記錄在工作區，待原程序退出、資料目錄與連接埠核對完成後寫入。
Manager 重開可接續待寫入的設定，新啟動在啟動 Java 前完成相同核對。偵測到檔案衝突時保留變更供重審。

Java 與 JVM 頁面顯示有效預設值。欄位可還原上次儲存值，有可核對的預設則也可恢復預設。兩個動作只更新草稿。進階旗標預設摺疊，已有旗標或驗證出錯時會展開。可用有時限的 Java 探測核對草稿，例如 `-XX:ActiveProcessorCount=2`，再手動儲存。
外觀可預覽 Windows 強調色或自選色票。清單選取底色預設不透明度為 25%，可用滑桿或直接輸入 0 到 100 的整數。輸入無效時保留上次有效預覽並停止滑桿同步。觀察狀態盡可能使用原色，必要時使用獨立底色維持對比。較濃的底色優先使用可讀的白字。取消會回復上次設定。
進階自動重啟可設定每次重試間隔、計算視窗和穩定期，Manager 重開後保留排程及暫停狀態。
手動停止或 ConPTY 無法辨識原因的正常退出會抑制重啟。

核心維護支援同核心與同版本線的單台替換、備份還原及中斷核對。
來源檢查依核心及版本分組。開發或部署時可用程序環境
`INSTANCEHARBOR_PAPERMC_CONTACT_URL` 指定另一個有效的 HTTPS 聯絡網址。
Spigot 1.8.8 的原生提交距離和同版本發布資訊各自核對。原生版本顯示落後只提供診斷資訊，Spigot 更新仍需使用已核對的本機 JAR。
在「應用程式設定 → 核心更新」可開啟「預先下載核心更新」，預設關閉。開啟後，常駐 Manager
會核對同版本線的 Paper 更新，並在工作區依 SHA-256 共用候選。準備期間各實例可繼續運行。
確認更新且符合原有的停止條件後，管理端核對本機候選，再為各台分別備份及替換。
最後一台已更新或移除管理，且目前無安裝使用該檔案時，Manager 會回收候選。關閉設定會停止背景下載，
並回收已無安裝使用的預先下載副本。已準備的候選及待更新實例可在 Manager 重開後重新核對。
共用候選位於本工作區的 `staging/core-prefetch`，和共用核心快取分開。
批次更新可從全部操作或目前多選的實例開啟。確認前可檢查逐台計畫。常駐 Manager 保存批次 ID 及逐台派工狀態。
在進度視窗或操作紀錄選擇「取消剩餘」，仍在排隊或準備候選 JAR 的項目會取消。
已進入核心替換交易的項目會完成或保留結果供核對。已更新項目的備份可從單台核心維護還原。
Manager 重開時先核對已進入替換交易的原請求，已取消的項目保持取消。其餘未進入交易的項目依原計畫重新核對登記與核心身分。
操作紀錄預設隱藏 90 天前已結案資料，
保留原始紀錄及結果未確認的工作。

### 從 PowerShell 準備測服

`ihctl.exe <workspace> prepare <managed-id> <request-id> <request-json>` 在已初始化的工作區準備一台測服並保持停止。
請求 JSON 包含 `audience`（`Human` 或 `Agent`）、`project`、`instance`、`java`、`port`、`coreJar`、
`permissionJar`、視核心而定的 `mojangJar`，以及專案插件的 `pluginJar` 與 `pluginSha256`。
`worldType` 可選 `Flat` 或 `Normal`，預設 `Flat`。共用 Lobby 省略專案插件欄位。
`managed-id` 是 2.0 管理清單識別碼，`instance` 是實際目錄名稱及 Human 的預定路由名稱。
Human 新建核對已登記的名稱及連接埠，並保存在 `instances/Human/<project>/<instance>`。
Agent 建立要求獨立指定用途及空閒連接埠，使用 `instances/Agent/<project>/<instance>`。
既有 `create` 及一般匯入保留原入口。

使用同一個 `managed-id`、Agent 專案、實例名稱和連接埠，並在 `prepare` 請求加入
`"replace": true`，可明確重建由 2.0 `prepare` 建立的 Agent 測服。逐台正常停止後才可重建。
程式先核對舊實例與新素材，再封存原資料夾、登記與最後工作階段。完成時新副本保持停止。
封存位於工作區的 `agent-rebuild-archives/<request-id>`，回覆包含封存路徑。Human、一般匯入和
無法核對原身分的實例不能使用此入口。

重建發布途中中斷時，請以原請求 ID 和完全相同的 JSON 重試。待完成的交易會阻擋這台實例的啟動及相關寫入。
若重建尚未進入發布就失敗，原目錄保持原狀。若收到 `ResultUnconfirmed` 且沒有待發布紀錄，
可保留原 staging 供調查，在確認原實例仍完整後用新請求 ID 再試。

成功回傳原工作 ID。重送相同請求時沿用原 ID，不同內容使用相同請求 ID 會被拒絕。
有未完成紀錄時回報 `ResultUnconfirmed` 並保留 staging 供核對。新建流程不配置 Velocity 路由。

#### Agent 建服設定檔

新建 Agent 實例可以傳入 `profile`，讓 `prepare` 選擇已安裝的 Java 並核對工作區內的固定素材。不填 `port` 或填 `0` 時，管理端選擇空閒的 loopback 埠，並在工作區的埠登記鎖內再次核對。回覆包含實際埠、Java 路徑、核心雜湊、設定檔與原工作 ID。

| `profile` | 核心 | Java | `<workspace>/profile-assets/<profile>/` |
| --- | --- | --- | --- |
| `paper-26.3` | Paper 26.3 build 205 | 25 | `server.jar`、同版 `mojang.jar`、LuckPerms Bukkit 5.5.85 命名為 `luckperms.jar` |
| `spigot-1.8.8` | Spigot 1.8.8 已核對 BuildTools 產物 | 8 | `server.jar`、LuckPerms Bukkit 5.4.145 命名為 `luckperms.jar` |

素材目錄由呼叫端事先放入已核對的檔案。設定檔不下載核心或安裝 Java；找不到素材或相符 Java 時拒絕。此入口適用新建 Agent，Human、重建及手動指定 JAR 仍用既有契約。非 Lobby 的實例仍須帶入已核對的 `pluginJar` 和 `pluginSha256`。

```powershell
$spec = '{"audience":"Agent","project":"Lobby","instance":"paper-agent","profile":"paper-26.3"}'
& $ihctl $workspace prepare paper-agent prepare-paper-agent $spec
& $ihctl $workspace agent-list
```

新實例保持停止。用相同請求 ID 和原 JSON 重試，可讀回首次解析的結果。發布資料夾或登記中斷時，原請求會核對當時保存的素材與身分後繼續。尚未完成暫存的請求回報 `ResultUnconfirmed`，保留暫存供核對。若外部程序已占用保留的埠，待埠釋放後用原請求重試。

#### Agent 資料夾清冊

`ihctl WORKSPACE agent-list [nextCursor]` 透過常駐 Manager 唯讀列出本工作區 `instances/Agent/<project>/<instance>`。未登記或已移除管理的 Agent 資料夾仍會出現在清冊。查詢不改動 GUI 的實例設定清單。

回應包含版本化的 `schemaVersion`、`items` 與 `nextCursor`。每列提供可供後續檔案管理查詢的資料夾 ID、相對路徑及 `logicalBytes`。大小是檔案長度合計，並非磁碟已配置空間或可回收估計。掃描略過重新解析點，`sizeComplete: false` 表示本次結果僅為已觀察大小。當 `nextCursor` 有值時，繼續傳入該值可讀取下一頁。清冊不提供寫入、移動或刪除操作。

`doctor [managed-id]` 核對工作區、政策與已登記的核心、Java 路徑和連接埠。
`defaults-diff <managed-id>` 回傳預設差異和證據狀態。
核心更新後會核對目前 JAR 是否符合已登記的 SHA-256，再依該核心版本讀回設定。若雜湊不符，回傳 `CoreIdentityChanged`。
`compatibility-probe-check <manifest-json>` 可唯讀核對隔離的 Velocity／Spigot／Paper 轉送配置和固定 Via JAR。
探針輸出包含是否已核對設定、程序啟動和客戶端連線三項證據。

共用核心 JAR 驗證以隔離工作區啟用。隔離測試中，
兩台 Paper 可使用同一份已核對的 JAR，同時各自寫入世界與設定。
Paper 26.3 的共用插件 JAR 隔離實驗也可使用同一份唯讀 JAR，各自保存插件資料。
共用插件尚未接入管理器與 GUI。

安裝腳本接受有預先核對 SHA-256 的便攜包，可驗證版本切換、
還原與解除安裝，安裝或解除安裝時須先退出該安裝位置的程序。
RC2 使用系統提供的 Microsoft Visual C++ v14 x64 Runtime。GUI 另使用 Windows WebView2。
### Windows x64 1.0.0-rc2 封裝

從本目錄以 PowerShell 7 執行 `./build-product.ps1 -Release`，使用已安裝的 MSVC、Windows SDK 與鎖定的前端依賴。
用 `pack-windows.ps1` 將 release 執行檔、已核對的 ConPTY 與授權文件封裝成 ZIP。不要傳入 `-VCRuntimeDirectory`。
接著用 `pack-windows-setup.ps1` 傳入 ZIP、預期 SHA-256 和既有 NSIS 編譯器。
Setup 從通過雜湊核對的 ZIP 取得同一套執行檔。Setup 要求系統有 Visual C++ x64 Runtime 14.44.35211.0 或更新版本。
沒有符合版本時，Setup 會在寫入前提示使用 [Microsoft 官方 x64 Runtime 安裝程式](https://aka.ms/vc14/vc_redist.x64.exe)，
靜默安裝則回傳退出碼 3。ZIP 使用者請先讀 `WINDOWS-GETTING-STARTED.txt`。
Setup 解除安裝會保留共用的 Runtime。Windows WebView2 與伺服器所需的 Java 由電腦提供。
若乾淨 Windows 或 Windows Sandbox 提示找不到 WebView2，先安裝
[Microsoft WebView2 Evergreen Standalone Installer 的 x64 版](https://developer.microsoft.com/microsoft-edge/webview2/#download-section)，再重開 InstanceHarbor。

Setup 安裝在 `%LOCALAPPDATA%/Programs/InstanceHarbor`，在「開始」與已安裝的應用程式中登記為 InstanceHarbor。
雙擊便攜版或已安裝的 `InstanceHarbor.exe`，預設使用 `%LOCALAPPDATA%/InstanceHarbor/workspaces/default`。
以腳本直接啟動時，`IH_V2_WORKSPACE` 可指定已初始化的工作區。Setup 建立的「開始」捷徑固定使用正式預設工作區。
需要隔離多個候選時，請明確指定各自的工作區。這個版本第一次啟動會建立預設工作區，不會自動遷移舊版資料。

解除安裝前先從 GUI 正常停止各台伺服器並關閉 GUI，再以該安裝的 `ihctl.exe WORKSPACE manager-exit` 關閉該工作區的 Manager。
卸載會保留工作區與日誌。要測試回退可先備份工作區，解除安裝 Setup，再用已驗證的舊 ZIP 開啟同一工作區。
便攜安裝可用 `install-windows.ps1 -Action Rollback` 切回已安裝的前一版，包含 GUI 檔名仍為 `instanceharbor-v2.exe` 的候選。保留工作區中的新舊設定，重開後先核對再啟動伺服器。

### 同時使用正式版與開發版

在專案的 `v2` 目錄執行 `npm run dev` 或 `./launch-development.ps1`。第一次會建置開發版，
並依 checkout 路徑建立專用工作區。後續會使用同一工作區。從 Windows「開始」的 Setup 捷徑啟動時，
會使用正式版與正式預設工作區。兩版可同時開啟，再次啟動同一版時會定位該版原視窗。

正式版使用 `dist` 與 Rust `release`，開發版使用 `dist-dev` 與 Rust `dev-channel`。
開發版的執行檔為 `InstanceHarborDev.exe`，視窗名稱為「InstanceHarbor Dev」，使用自己的 Windows 工作列身分和
`%LOCALAPPDATA%/InstanceHarbor Dev`。其工作區、設定、WebView2 與日誌都在該目錄下。

需要重新建置開發版，或手動指定另一個開發工作區時，可使用下列入口。

```powershell
./launch-development.ps1 -Build
$devWorkspace = Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev/workspaces/my-feature'
./launch-development.ps1 -Workspace $devWorkspace -Initialize
```

開發工作區必須是 `%LOCALAPPDATA%/InstanceHarbor Dev/workspaces` 的直接子目錄，
名稱可使用英數、連字號和底線。開發版 GUI 缺少 `IH_V2_WORKSPACE` 或工作區不在此目錄時拒絕啟動。
開發版 CLI 也要求明確傳入同範圍的工作區。正式版拒絕開發資料目錄，Manager 仍按工作區路徑區分。
兩版同時啟動伺服器時，請為各工作區使用獨立副本，並為每台伺服器指派不同的系統連接埠。

### 復原金鑰

`ihctl WORKSPACE recovery-key-status` 可讀取本機 DPAPI 金鑰狀態，
`ihctl WORKSPACE recovery-key-rotate GENERATION` 依目前世代輪替金鑰，
首次建立傳入 `none`。桌面 PowerShell 7 可使用
`manage-recovery-backup.ps1` 透過安全提示輸入密碼，執行匯出、核對或回復金鑰。
備份只供同一個絕對工作區路徑復原，回復前須核對公開金鑰與世代。
應用程式設定的「復原金鑰」可操作同一套保管功能。
已釘住復原金鑰的工作階段可從原生介面或 PowerShell 接管。流程見可信 Host 接管。

工作區由明確路徑建立。掃描允許與排除名單支援絕對及相對路徑，一般匯入限定於工作區的 instances 之下。測試使用 IPv4 回環監聽。GUI 新建沿用全域 Java 設定，並自動核對需要的 Mojang JAR。1.0 遷移入口已移出正式 GUI。

Windows 一般匯入支援放有已核對 Velocity JAR（`server.jar`）與
`velocity.toml` 的代理目錄。預設接受 `127.0.0.1:PORT`。要匯入綁定其他介面的代理，
請在 GUI 的可選欄位輸入與頂層 `bind` 相符的明確 IPv4 位址。
匯入確認使用同一組位址、Port、核心與設定檔預覽版本。啟動時再次核對登記的位址及 Port。
更新監聽位址後須重新核對匯入。管理器拒絕所有介面的萬用字元位址。

PowerShell 可將 IPv4 附在 `import-preview NAME JAVA DIRECTORY IPV4`。
將預覽回傳的 Port 及 revision 原樣帶入 `import NAME REQUEST JAVA DIRECTORY PORT REVISION IPV4`。
匯入只寫管理登記，Velocity 的路由、轉送與玩家登入設定由伺服器資料夾提供。
正常停止時 Host 對 Velocity 送出 `shutdown`，對遊戲後端送出 `stop`。
此次範圍為一般匯入與啟停，代理 Port 和路由由 `velocity.toml` 管理。

Vue 與 CLI 共用 Rust 管理入口，批次啟動、正常停止及重啟由工作區的常駐 Manager 保存。
關窗後可在操作紀錄查回原工作，行為與隔離驗證見批次生命週期。
Host 的控制管線限制同 Windows 帳戶及本候選執行檔。強制停止須由使用者明確確認。


### 啟動中停止與逾時後重試

Host 收到停止要求後先排隊，等 Paper 的啟動日誌出現 Done 才送出 stop。觀察資料分開標示已排隊和已送出。管理端的停止等待逾時後，原工作紀錄保留結果未確認。GUI 可對仍是同一個 run 的程序提出新的正常停止，CLI 則用新請求 ID 與原 run ID。已確認停止時，重新啟動會直接啟動；啟動中或正在監聽時，會先正常停止。重啟等待停止時逾時就結束這次重啟工作，後續不會自動啟動新 run。

隔離驗證會縮短管理端的等待期限，重現 Paper 尚未到 Done 時發生 StopTimeout。這個測試設定只適用 Debug 驗證器及其 .evidence 工作區。它不會加速或變更 Java 的停止行為。驗收時可在狀態為啟動中時按正常停止，確認先看到等待提示，隨後回到已停止。不要對既有測服施加負載。


### 緊急停止

正常停止送出後，同一位置顯示紅字「緊急停止」。取消確認或按 Esc 會返回原畫面，讓原本的正常停止繼續。確認視窗的預設焦點是「取消」，紅色「強制停止」會再次核對實例的 run。Host 只終止自己持有的 Java 子程序；原工作階段若已退出，直接回報退出。強制停止可能遺失未存檔的世界變更，完成後須確認 Host 和 Java 退出且連接埠已釋放。強制退出不符合正常重啟的啟動條件。

隔離驗證會暫停自己建立的 Paper Java 程序，讓正常停止保持等待，以便確認焦點、取消、深淺色及強制停止。驗收時可在隔離 Paper 啟動中按「正常停止」，再點「緊急停止」，先取消一次，再重新確認。若 Paper 在確認前已正常停止，應回到已停止。


## Vue 瀏覽器預覽與對照

使用已安裝的 Node.js，從本目錄執行：

```powershell
npm ci --offline --no-audit --no-fund
npm run preview:vue -- --port 4176 --strictPort
```

- Vue 版位於 `http://127.0.0.1:4176/preview/`。
- 無框架版位於 `http://127.0.0.1:4176/preview/legacy.html`。來源是原有的 `docs/design/v2-wireframe/index.html`，原檔維持不變。
- `npm run build:preview` 將兩個入口輸出至 `v2/dist-preview`，可用本機靜態 HTTP 伺服器開啟。既有 Tauri 建置入口仍是 `v2/index.html`。

Vue 元件負責預覽殼、實例表格與詳情、設定版面、操作紀錄、共用對話框、Java 選擇與核心維護。複雜表單、精靈的部分內容與模擬控制器仍以既有預覽流程為基礎，並經由共用的頁面與對話框邊界接入。生命週期佇列獨立於 Vue 元件。這些保留的相容區域可逐步替換為有型別的欄位與元件。

所有資料都是模擬。使用者主動匯出時，瀏覽器只下載紀錄摘要。操作紀錄會保留在同一分頁的工作階段儲存空間，重開頁面會將當時尚未確認的工作保留為待核對紀錄。前次工作不會重送。兩版的參考操作包含選取、設定草稿、建立與匯入、移除後再匯入、核心維護、批次操作與停止。新版單台正常停止直接送出模擬要求，等待期間可用紅字緊急停止並另行確認。Play 的示範埠對齊已整合的登記表。保留期限和 CSV 的完整流程仍由操作紀錄的補充示意頁提供。

Agent 瀏覽器檢查從 repository 根目錄執行：

```powershell
node docs/design/v2-wireframe/verify-browser.cjs http://127.0.0.1:4176/preview/
```

測試使用獨立 Chrome 設定檔，報告與畫面輸出至 `artifacts/v2-vue-preview-validation`。Windows 原生標題列、DPI、輔助技術與真實服務行為屬後續驗證。

### Windows 原生後台視窗

主程式首次建立視窗時置中。重複執行只聚焦已開啟的主視窗，保留其位置。

後台使用 Windows 原生標題列與邊框。可拖邊框調整視窗、最大化，或從最大化狀態拖曳標題列還原。
終端在可用空間變動時核對目前輸出，讀到尾端後向同一次執行的 ConPTY 送出尺寸。連續拖曳時合併中間尺寸，沒有輸出需要解析的過時尺寸不觸發重排。已有輸出的資料仍按 Host 回報的格線解析。
捲動使用貼近終端視窗右緣的細版懸浮捲軸，保留滾輪、鍵盤及拖曳操作。終端內層的原生捲軸隱藏，懸浮捲軸顯示時不改變文字寬度。手動捲過即時輸出底部時，最多將最後有內容的一行移到視窗頂端。留白只存在畫面上，開始輸入時回到即時提示列。隱藏後台時該次終端仍持續接收輸出。
尺寸同步需要當次 Host 支援。舊 Host 維持本來的終端尺寸，視窗操作仍由 Windows 處理。

後台有選取文字時，可用 Ctrl+C、Ctrl+Shift+C 或 Ctrl+Insert 複製。未選取文字時，Ctrl+C 在後台攔下。貼上支援 Ctrl+V、Ctrl+Shift+V 和 Shift+Insert。單行貼上只帶入文字。含換行時先顯示確認，焦點預設在「仍要貼上」，按 Esc 可取消。
