# InstanceHarbor Windows 1.1.0

新候選使用 `build-product.ps1` 建置並核對內嵌 Vite 資產；`launch-product.ps1` 只開啟本次已核對的隔離 Agent 工作區。
舊的 `preview/` 與無框架 `index.html` 保留為純模擬對照，不會自動切換到真實控制。

本專案是 Windows 第一版功能候選。Vue／Tauri 與 PowerShell 呼叫的 Rust CLI 使用相同的 Rust 管理入口。分開執行的 Rust Host 持有 Java 的標準輸入與日誌。GUI 或 CLI 退出後，Host 仍能持有原程序。重新核對使用 run ID、PID、Windows 程序建立時間和本機 TCP 監聽者。正常停止經有期限的 Named Pipe，驗證雙方程序身分。重啟需先確認原 Java 和 Host 退出、正常結束且埠已釋放。

## 建置與隔離驗證

`build-product.ps1` 經 `resolve-npm.ps1` 在 PATH 上尋找 npm，`npm.cmd`（官方安裝）與 `npm.exe`（mise 等版本管理工具的 shim）都可用；找不到時會提示安裝 Node.js 或檢查 PATH，腳本不會修改 PATH。解析邏輯的測試：`Invoke-Pester ./tests/resolve-npm.Tests.ps1`。

在 repository 根目錄執行下列命令。驗證腳本使用 SHA-256 核對 Paper 26.3 build 205 和 Mojang 依賴，唯讀取用既有 JAR 快取。所有新工作區和證據寫入根目錄的 .evidence。

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

- 系統匣圖示屬於常駐 Manager（`ihctl.exe WORKSPACE manager-serve`），不屬於 GUI：Manager 在，圖示就在；Manager 結束，圖示跟著消失。
  GUI、CLI 或 Agent 啟動 Manager 時都會出現圖示，但不會開啟 GUI 視窗。
- GUI 啟動後每 5 秒向自己工作區的 Manager 登記（`gui-attach`，只接受同一安裝的 GUI 程序），Manager 不在時會先啟動它。
  所以 GUI 開著時一定有圖示；Manager 中途被結束，下一次登記會再啟動它。
- 一個工作區最多一個圖示。開發版的提示文字以「InstanceHarbor Dev — <工作區資料夾名稱>」開頭，同時有多個工作區時可以分辨。
- 圖示註冊時不帶提示文字，Manager 隨即補上。Windows 11 會把註冊時的提示文字當成圖示名稱，提示文字改變後
  名稱會變成「註冊時的文字 目前的文字」，產品名稱因此重複。現在名稱開頭多一個空白，不影響顯示。
- 滑鼠停在圖示上會顯示執行中的伺服器台數，右鍵選單第一行也會顯示（每 5 秒更新，算法與清單的「可停止」相同）。
  選單文字依「語言與外觀」的語言設定。
- 左鍵點圖示，或右鍵選「開啟 InstanceHarbor」，會以這個工作區開啟 GUI；GUI 已在執行時則叫回它的視窗。
- 右鍵選「結束」會結束 Manager，並關閉已登記的 GUI 視窗。伺服器與 Host 是獨立程序，會繼續執行。
  還有批次工作或核心更新在進行時，Manager 不結束，改為跳出說明，請等工作完成後再結束；這與 `manager-exit` 回報
  `ManagerHasActiveWork` 的條件相同。
  只要有伺服器不是「已停止」（包含啟動中、工作階段未定，或狀態讀不到），會先跳出確認（預設為「否」）。
  其中開啟自動重啟的伺服器會另外說明：自動重啟由 Manager 執行，
  結束後這些伺服器當機或意外結束不會自動重啟，要等 Manager 下次啟動才恢復。按「是」後會再核對一次批次工作。
  CLI 的 `manager-exit` 不詢問，只套用批次工作的條件。
  CLI 的 `app-exit` 與「結束」同一套流程（結束 Manager 並關閉已登記的 GUI 視窗），同樣不詢問、只套用批次工作的條件；Setup 更新時使用它。
- 「結束」只關閉仍是原登記程序的 GUI：`gui-attach` 記下 PID、Windows 程序建立時間與 GUI 執行檔路徑，
  送出關閉訊息前重新核對三者。GUI 已結束、PID 被其他程序重用，或執行檔不同時略過並清掉紀錄，避免把關閉訊息送給無關程式。
- 「結束」失敗時，對話框顯示翻譯後的說明與 GUI 診斷紀錄資料夾；原始錯誤只以代碼寫入診斷紀錄（`manager-quit-failed`），不含路徑。
- 按 GUI 視窗的 X 會結束 GUI 程序，Manager 與圖示留著。原本的「關閉視窗時縮到系統匣」設定已移除。
- 結束看門狗：主視窗關閉後，GUI 暫停向 Manager 登記，避免重新啟動使用者剛結束的 Manager。
  10 秒後程序仍在、且已沒有任何可見視窗時，記錄 `gui-exit-watchdog` 並直接結束程序；仍有可見視窗代表關閉被取消，看門狗解除。
  開發版可設定環境變數 `IH_DEV_HANG_ON_EXIT=1` 啟動 GUI，讓程序在結束時卡住，用來驗證看門狗。
- 回退相容：`preferences.json` 仍保留舊的 `closeToTray` 值但不再使用。值為預設時不寫入檔案；若曾在 1.0.0 關閉此設定，
  回退到 `1.0.0-rc2` 前請先在 1.0.0 重新開啟並儲存，否則 rc2 無法讀取外觀設定。
- 圖示由 `tray-icon` 繪製，使用 tauri-build 嵌入每個執行檔的應用程式圖示（資源 32512；正式版藍色、開發版紅色，見「開發版紅色圖示」）。`muda` 需要 comctl32 第 6 版，
  執行檔已帶對應 manifest；程式庫的單元測試執行檔沒有，所以系統匣程式碼不編進測試建置。

## 視窗開啟大小

- 主視窗預設 1380×860（最小 980×640），主控台視窗預設 1020×820（最小 460×280），單位是套用 Windows 顯示縮放前的邏輯像素。
- 首次開啟時，視窗連同標題列與邊框限制在所在螢幕的可用區域（扣掉工作列）內，並在可用區域中置中。可用區域足夠時維持預設大小。
- 可用區域小於最小尺寸時，最小尺寸一併縮小到可用區域，版面依既有斷點顯示並可捲動。
- 計算在 `src-tauri/src/window_placement.rs` 的 `fit_to_work_area`，單元測試涵蓋 1366×768@100%、1920×1080@125%、可用區域小於最小尺寸與副螢幕座標。
  `Monitor::work_area()` 是實體像素，先扣掉框線再除以縮放比例換算成邏輯像素。
- 不記住上次的視窗大小與位置。

## GUI 冷啟動計時

GUI 不常駐：每次從系統匣或捷徑開啟都是冷啟動（新程序、新的 WebView2）。每次啟動在 GUI 診斷紀錄
（`<應用程式資料夾>\logs\gui-*.jsonl`，開發版為 `InstanceHarbor Dev\logs\<工作區名稱>\gui-*.jsonl`）寫入一筆 `gui-startup` 事件。數值是從 Windows 建立程序起算的毫秒，
包含 `main` 之前的載入時間。

| 標記 | 位置 | 意義 |
|---|---|---|
| `main` … `diagnostics-open` | `native` | `main` 開始、安裝租約、WebView2 檢查、工作區、單一實例鎖、診斷紀錄開啟 |
| `setup`、`window-built`、`setup-done` | `native` | Tauri setup 開始、`WebviewWindowBuilder::build` 回傳（含建立 WebView2）、系統匣建立完成 |
| `present-window`、`window-shown` | `native` | 頁面第一次呼叫 `present_window`、視窗實際顯示 |
| `page-navigation-start` | `page` | WebView2 開始載入頁面 |
| `page-script-start`、`page-mounted` | `page` | 主程式包執行完畢、Vue 掛載完成 |
| `page-preferences-read`、`page-window-presented` | `page` | 收到外觀設定、`present_window` 回傳 |
| `page-first-list-read`、`page-first-list-rendered`、`page-first-list-painted` | `page` | 收到第一份實例清單、DOM 更新、第一次繪製 |

量測方式：建置開發版（`./build-product.ps1 -Development`，`dev-channel` 繼承 release 最佳化），再執行

```powershell
./tests/measure-gui-cold-start.ps1 -Workspace (Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev/workspaces/agent-cold-start') -Runs 10
```

腳本先連線該工作區的 Manager（模擬 Manager 常駐、點系統匣開 GUI），逐次啟動 GUI、讀出 `gui-startup`，
再以 PID 結束自己啟動的 GUI，最後請該工作區的 Manager 結束。同一工作區的開發版 GUI 開著時，腳本會等待，不會關閉它；其他工作區的 GUI 不影響量測。
`-BinaryDirectory` 可指定另一份建置，`-BrowserArguments` 可在不重建的情況下比較 WebView2 參數。

冷啟動的時間大多花在建立 WebView2（`setup` 到 `window-built` 約 580 ms），這段由 WebView2 自己決定。
為了不讓頁面載入後再等讀取，GUI 綁定工作區後立刻在背景執行緒讀外觀設定（`preferences-snapshot`）與實例清單
（`instance-snapshot`），和建立 WebView2 同時進行（`src-tauri/src/startup_prefetch.rs`）。讀取走與頁面呼叫相同、
會寫診斷紀錄的路徑；頁面第一次以背景方式呼叫同一動作時直接拿到結果，不再寫一次紀錄。結果在完成後超過 1.5 秒
（清單輪詢間隔）才被取用時捨棄，頁面改為重新讀取，所以畫面上的清單不會比一次輪詢更舊。

2026-11 在開發機量測（中位數，各 10 次以上）：第一份清單畫出從約 1006 ms 降到約 880 ms，視窗顯示從約 1009 ms
降到約 950 ms。試過但沒有超出雜訊的做法，因此未採用：設定頁與操作紀錄頁延遲載入（主程式包 274 kB → 157 kB，
掛載時間沒有可量測的差異）、把診斷紀錄寫入移到 blocking 執行緒池、WebView2 參數
`--no-proxy-server --disable-background-networking --disable-component-update`。

## 介面語言

- 「應用程式設定」→「介面語言」的選項依序為「跟隨系統」、English、繁體中文。預設為「跟隨系統」，沒有語言設定的舊偏好設定也視為跟隨系統。
- 跟隨系統時，Windows 顯示語言為繁體中文（台灣、香港、澳門）才使用繁體中文，其他語言一律使用英文。前端 `resolveLanguage` 與後端 `presentation.rs` 的後台視窗標題使用同一規則。
- 介面文字放在 `src/i18n.ts`（共用字串）與 `src/messages/*.ts`（依區域分檔）。每筆為 `[繁體中文, English]`，兩種語言的 `{參數}` 必須相同。
- 元件用 `useAppLanguage()` 的 `t()`；元件外的程式（例如 `manager.ts`）用 `tr()`，同樣跟隨目前語言。
- `src/hardcoded-text.test.mjs` 會在 `.vue`、`.ts` 出現寫死的中文時失敗。資料用途的中文（語言名稱、後端送來的分類 ID）列在測試的允許清單。開發者用的 `console` 訊息使用英文。
- 「初始化預設」與世界設定的欄位資料由後端 `policy_catalog.rs` 提供。後端回傳的中文 `label`、`reason`、`source` 保持不變，另附 `reasonCode`、`sourceKind`、`sourceKey`；前端依欄位 key 與這些代碼翻譯（`src/messages/policy.ts`）。新增欄位或原因時，`policy-catalog.test.mjs` 會要求補上翻譯，且中文必須與後端一致。

## 焦點框與捲動區

- 停用（`disabled`）或暫時唯讀（`readonly`）的輸入欄位一律變淡、顯示禁止游標，並隱藏輸入游標，避免看起來仍可輸入。規則在 `src/app.css`。
- 焦點框的寬度與外擴距離由 `src/app.css` 的 `--focus-ring-width`、`--focus-ring-offset` 定義，合計為 `--focus-ring-extent`。
- 對話框裡的 `OverlayScroll` 捲動區在上緣與左緣保留 `--focus-ring-extent` 的內距，並以相同的負外距抵銷，內容位置與寬度不變；右緣與下緣原本的內距已大於焦點框外擴量。
- 覆寫 `.overlay-viewport` 內距時，每一邊都不得小於 `--focus-ring-extent`，否則靠邊輸入框的焦點框會被裁切。`src/focus-ring.test.mjs` 會檢查這項規則。
- 首次讀取的進度條 `InitialQueryProgress` 有兩種版面。實例清單與操作紀錄這類整頁清單加 `centered`：進度條以絕對定位
  對齊最近的定位祖先，在它前一個元素（例如表格標題列）以下的區域水平與垂直置中。在 `OverlayScroll` 裡，這個祖先是不會捲動的
  外框，所以表格比可見區域寬、出現橫向捲軸時，進度條仍停在可見區域中間。操作紀錄頁讀取時讓卡片填滿可見高度，進度條在卡片中間。
  對話框內維持預設的靠左版面。`src/initial-query-progress.test.mjs` 會檢查這項規則。
- `OverlayScroll` 的捲動區只在內容需要捲動時才能用 Tab 選到，讓鍵盤可以捲動；不需要捲動時，Tab 直接跳到裡面的控制項，不會停在整個容器上。

## 用 DevTools 檢查 GUI 畫面

GUI 是 WebView2 裡的網頁。開發版 GUI 啟動時若帶有 WebView2 的除錯參數，就能用 Chrome DevTools Protocol（CDP）連進去，
讀取 DOM、無障礙樹與 CSS，截圖，或在頁面執行檢查程式。不需要改程式碼，也不需要重新建置。

1. 建置開發版（`./build-product.ps1 -Development`），並準備隔離的 Agent 工作區。
2. 以環境變數啟動 GUI，除錯 Port 只綁定本機：

   ```powershell
   $bin = 'src-tauri/target/x86_64-pc-windows-msvc/dev-channel'
   $workspace = Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev/workspaces/agent-devtools'
   & "$bin/ihctl.exe" $workspace init
   $start = [Diagnostics.ProcessStartInfo]::new((Resolve-Path "$bin/InstanceHarborDev.exe").Path)
   $start.UseShellExecute = $false
   $start.Environment['IH_WORKSPACE'] = $workspace
   $start.Environment['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = '--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222'
   [Diagnostics.Process]::Start($start)
   ```

3. 確認 `http://127.0.0.1:9222/json/version` 有回應，頁面是 `http://tauri.localhost/`。
4. 用任何 CDP 用戶端連線。Agent 可用 [chrome-devtools-mcp](https://github.com/ChromeDevTools/chrome-devtools-mcp)，
   以 `--browser-url=http://127.0.0.1:9222` 連到既有的瀏覽器，不另外啟動 Chrome。常用工具：`take_snapshot`（無障礙樹）、
   `take_screenshot`、`evaluate_script`（例如找出超出容器的元素）、`get_css_styles`、`emulate`（切換深淺色）。
5. 檢查完成後執行 `ihctl <工作區> app-exit`，結束 GUI 與該工作區的 Manager，除錯 Port 隨之關閉。

注意事項：

- 只在開發版與隔離工作區使用。除錯 Port 能控制整個頁面，包括呼叫管理指令；不要綁定 `127.0.0.1` 以外的位址。
- 同一時間只有一個 GUI 能使用 9222。同時檢查多個工作區時，改用不同 Port，並讓用戶端連到對應的位址。
- `tests/measure-gui-cold-start.ps1 -BrowserArguments` 與 `tests/verify-native-file-access.mjs` 也用同一個環境變數傳入 WebView2 參數。

## Windows 檔案存取與部分清單

- `ihctl WORKSPACE list` 保留 JSON 陣列，個別登記無法讀取時會把原因寫到標準錯誤。
  自動化可使用 `list --details`，從同一份 JSON 取得 `rows` 與 `issues`。
- 清單優先向已啟動的 Manager 查詢。沒有可用的 Manager 時，唯讀核對已提交的登記與掃描規則。
  清單不建立或寫入 `manager.lock`。如果兩次檢查間資料仍變動，短暫重讀後回報清單尚未核對。
- 清單有個別異常時，GUI 仍顯示其餘可核對的實例，並列出檔名和原因。
  若無法排除登記衝突，啟動會被拒絕。已核對的運行中實例仍可正常停止。
- 每個 `issues` 項目帶 `stage`：`registration` 是登記檔本身無法讀取，`observation` 是登記可讀、但讀取狀態、
  核對管理範圍或解析實例資料夾失敗。`observation` 另帶 `name`、`code`（錯誤開頭的穩定代碼，例如
  `PathRedirected`、`FileAccess`、`OsError`、`InvalidStateFile`）與 `osError`，原因 `reason` 可能是
  `FileInUse`、`AccessDenied`、`PathRedirected`、`InvalidStateFile` 或 `ObservationUnconfirmed`。原始錯誤可能含使用者路徑，
  所以只保留代碼、結尾的 OS 錯誤碼與 `FileAccess` 的階段和檔案類別。
- 狀態無法讀取的實例仍留在 `rows`，`state` 為 `StateUnconfirmed`，`stateIssue` 帶同樣的原因。GUI 在清單列與詳細資料
  顯示原因，這台實例在狀態核對前不能啟動、停止或移除；系統匣「結束」把它算成不是已停止。只有實例資料夾無法解析時，
  列保留讀到的狀態，只少了路徑資料，橫幅仍列出原因。
- 橫幅的「複製診斷資料」複製上述欄位的 JSON（`registry-issues.ts` 的 `registryDiagnosticText`），不含路徑。
- 「核心維護」的「更新核心」停用時，對話框在按鈕上方直接顯示 `core-state` 的 `reason`（`core-diagnostics.ts` 的
  `coreBlockedMessage`），不只放在滑鼠提示。
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

### 伺服器核心 JAR

核心 JAR 不限檔名。登記時依下列順序找出核心，並記下實際檔名：

1. 有 `server.jar` 時使用它。
2. 否則使用資料夾最上層唯一一個自稱為同類核心的 JAR。代理資料夾只接受 Velocity，伺服器資料夾不接受 Velocity。
3. 伺服器資料夾沒有自稱核心的 JAR、但最上層只有一個 JAR 時（例如 Spigot），使用那一個。

同類核心超過一個時回報 `CoreArtifactAmbiguous`。插件放在 `plugins` 裡，不影響判斷。

核心自述的內容在本機讀取，不需要網路，也不會執行核心：

| 核心 | 讀取位置 | 取得的資訊 |
|---|---|---|
| Paperclip 核心（Paper、Folia、Purpur 等 Paper 分支） | `META-INF/patches.list`、`version.json` | 家族、Minecraft 版本、Java 版本 |
| Velocity | `MANIFEST.MF` | 版本 |
| Spigot bundler（新版） | `META-INF/versions.list` | 家族、Minecraft 版本 |
| 舊版 Spigot／CraftBukkit | `META-INF/maven/.../pom.properties` | Minecraft 版本 |

沒有寫明 Java 版本時，讀主類別（`Main-Class`）的 class 檔版本，得到能載入它的最低 Java。

核心分成三級，**都不拒絕**，只有已核對的核心沒有警告：

1. **已核對**：SHA-256 在內建清單，或與 PaperMC 官方清單（`fill.papermc.io`）完全相符。內建清單沒有的 Paper、Folia、Velocity 才會連線查詢；結果記入登記，之後離線也能使用。核心更新與共用核心只接受這一級。
2. **未核對但可讀出版本**：Paper 分支（例如 Purpur）、Spigot、官方清單沒有的雜湊（`CoreNotInCatalog`），或連不上官方清單（`CoreCatalogUnavailable`，一分鐘內不重查）。登記會標記為未核對，Java 依核心自述的 Minecraft 版本自動選擇；預覽與詳細面板顯示警告。
3. **無法辨認**（`CoreUnrecognized`）：讀不出 Minecraft 版本。仍可登記，但啟動前要在實例設定選擇 Java。

Spigot 由 BuildTools 在本機建置，沒有官方檔案可比對，所以內建清單以外的 Spigot 一律是未核對。

實例清單與詳細面板的核心版本（`presentation::core_metadata`）和「核心維護」用相同的來源。
離線辨識（`core_identity`）只讀取 JAR 的 ZIP 目錄與幾個小型中繼資料檔，不啟動 Java：

- Paper 與 Folia：`paper-api` 或 `folia-api` 函式庫的路徑，例如 `dev/folia/folia-api/26.3.build.28-stable/`，可讀出 Minecraft 版本與 build。
  兩者同時出現時無法判斷，視為無法辨識。
- Spigot：BuildTools 產生的 bundler JAR（1.18 起）讀 `META-INF/versions.list`，格式為 `<sha256> *spigot-<版本>-R0.1-SNAPSHOT.jar`；
  較舊的 Spigot（1.8.8～1.17.1，含 1.14 起的 `version.json`）讀 Maven `pom.properties`。只有 Minecraft 版本，沒有 build。
  舊版 JAR 內含整個伺服器，約有 12,000～26,000 個項目，所以核心的 ZIP 目錄上限為 65,535 個項目，大小仍限制在 4 MB。
- 舊版 Paper（1.14、1.14.4）沒有 API 函式庫，只能靠內建清單的雜湊辨識。

清單的判斷與「核心維護」相同：

1. 目前 JAR 的 SHA-256 在內建清單，或與已核對的登記相符（`trusted_profile`）：顯示該版本，為已核對。
2. 否則顯示離線辨識的結果；無法離線辨識時，顯示同雜湊的未核對登記。兩者都標示為離線辨識。
3. 都沒有時顯示「尚未核對」。

清單每隔幾秒刷新，核心檔有 40～90 MB，所以離線辨識成功的核心以「檔案長度與 ZIP 目錄不變」為條件沿用上次的 SHA-256。
ZIP 目錄記錄每個項目的 CRC，任一項目內容改變都會使目錄改變並重新計算。無法離線辨識的核心每次都重新計算。

清單中繼資料以 `offline: true` 標示離線辨識的版本，介面在滑鼠提示與詳細面板說明「版本由核心檔內容讀出，未與內建清單或官方來源比對。」。
離線辨識與網路無關：清單刷新時不連線核對，所以網路正常時也可能出現。
「核心維護」在核心未核對時以 `identified` 回傳離線辨識的版本，「目前版本」只加註「（離線辨識）」；核心更新是否可用仍依 `trusted_profile`，不因離線辨識而改變。

都無法辨識時顯示「尚未核對」，不沿用登記中的舊版本。原因以代碼回傳（`CorePathUnverified`、`CoreUnreadable`、`CoreProfileUnverified`），
由前端 `src/core-display.ts` 翻譯，顯示在清單儲存格的滑鼠提示與詳細面板。新增代碼時 `src/core-display.test.mjs` 會要求補上翻譯。

核心名稱與 build 一律經 `src/core-display.ts` 的 `coreFamilyName`、`coreBuildText`、`coreVersionText` 顯示，例如 `Paper 26.2 · Build 134`。
登記資料仍存 `PAPER`、`FOLIA` 等原始寫法，比對不分大小寫；未知的種類照原樣顯示。`src/core-display.test.mjs` 會在畫面直接插入 `.family` 或手寫 `Build` 時失敗。

每次啟動前，登記的檔名不存在時，會依上述規則重新尋找並更新登記，例如 Velocity 換成檔名帶有新 build 的 JAR。新核心的雜湊與登記不同時，同家族的核心會被接受（離線時以未核對方式接受）；遊戲伺服器另外要求 Minecraft 版本相同，避免世界被意外升級。其他情況維持原本的錯誤，例如 `CoreArtifactChanged`。

登記、匯入預覽與 V1 移交都會用 Windows Restart Manager 查詢哪些程式開著核心 JAR。其他程式（通常是仍在執行的伺服器）開著時回報 `CoreInUse`，不檢查 Port 也不登記。查詢只讀取，不會要求程式關閉。

自動登記需要伺服器核心 JAR 和同層設定；核心未核對時會顯示警告。非 loopback 監聽位址的 Velocity
需在匯入時另外確認位址，自動登記會略過。掃描略過連結及排除路徑，回報名稱、路徑、連接埠及管理權衝突。
達到掃描上限時，請縮小範圍後重新預覽。建立新實例仍由「建立實例」處理。舊版 CLI 匯入命令
及匯入工作紀錄維持相容，桌面版的日常入口是「新增管理資料夾」。

## 移除管理與刪除伺服器資料

詳細面板與實例的右鍵選單都有「移除管理」。伺服器必須已正常停止。對話框有三個選項：

| 選項 | 伺服器資料夾 | InstanceHarbor 設定 | 之後要再管理 |
| --- | --- | --- | --- |
| 移除管理，保留管理設定 | 保留 | 保留 | 「已移除的實例」→「恢復管理」 |
| 移除管理，清除管理設定 | 保留 | 刪除 | 重新匯入；位在管理資料夾內的伺服器改用「恢復管理」 |
| 刪除伺服器資料 | 移到資源回收筒 | 刪除 | 先從資源回收筒還原資料夾，再依上一列處理 |

管理資料夾掃描到的伺服器，以及資料夾位在任一管理資料夾之下的實例（例如建立時加入上層資料夾的實例），清除管理設定後仍保留「不要再自動登記」的紀錄，所以它會出現在「已移除的實例」，下一輪掃描也不會重新登記。
「恢復管理」使用既有的 `adopt`，需要指定 Java，而且 `server.properties` 的 Port 必須與原本登記的相同。

刪除伺服器資料需要輸入實例名稱確認；名稱旁的複製按鈕可以複製實例名稱。下列情況會拒絕刪除，資料不變：

- 實例不是已停止，或核心正被其他程式使用（`CoreInUse`）。
- 資料夾是工作區、工作區內 `instances` 以外的資料夾、`instances` 本身、包含工作區的上層資料夾、
  磁碟根目錄，或底下一層還有其他伺服器資料夾的管理資料夾根目錄。建立或恢復管理的實例，
  自己的資料夾也會列為管理資料夾；這種資料夾底下沒有其他伺服器時可以刪除。
- 資料夾在網路路徑（UNC）上、路徑或內容含有 reparse point（junction、symbolic link 等），
  或資料夾內還有其他受管理的實例。
- 任一其他實例的登記檔讀不到或資料夾路徑無法解析（`DeleteServerOtherInstanceUnconfirmed`）。
  這時無法確認那台實例的資料夾不在要刪除的資料夾裡，所以不略過它，先處理清單上方列出的登記問題再重試。

移到資源回收筒後，程式也會從管理資料夾清單移除這個資料夾的路徑。清單中留著不存在的路徑時，
之後建立實例或變更管理資料夾都會回報「找不到可使用的上層資料夾」。

移動期間不顯示 Windows 的進度視窗，由 InstanceHarbor 顯示進度文字。資料夾太大而無法放進資源回收筒時，Windows 會先詢問是否永久刪除；選「否」會取消，資料與登記都不變。
資料夾移走後才清除登記；清除失敗時回報 `DeleteServerRecycledButRegistered`，可再從右鍵選單移除管理。
「移除管理」不需要伺服器資料夾存在：資料夾已移到資源回收筒或在檔案總管被刪掉時，派送前的核心維護檢查改為核對登記的路徑確實不存在
（連結也算存在），再照常檢查實例已停止與選項；資料夾還在但無法讀取或被重新導向時仍會拒絕。

CLI 對應：

```powershell
ihctl WORKSPACE delete-server NAME NAME
ihctl WORKSPACE removed-instances
ihctl WORKSPACE open-server-folder NAME
ihctl WORKSPACE adopt NAME REQUEST_ID JAVA_EXE
```

`delete-server` 的第二個參數是確認用的實例名稱，必須與第一個完全相同。

## 從 V1 移交伺服器

V1 管理的伺服器可以原地交給 1.0 管理，世界、插件與插件資料都不搬動也不改寫。移交只能用 `ihctl`，每台伺服器各做一次；代理與後端分別移交，啟動順序由使用者決定。

1. 在 V1 正常停止這台伺服器並關閉 V1 的管理器，確認 V1 不會再自動重啟。
2. 預覽。`DIRECTORY` 是伺服器資料夾的絕對路徑，可以在工作區以外：

   ```powershell
   & $ihctl $workspace v1-handoff-preview NAME DIRECTORY
   ```

   預覽是唯讀的。它會確認 V1 已停止，包括 V1 管理器鎖沒有被占用、V1 控制管道不存在、啟動紀錄裡沒有仍在執行的 V1 Host（PID 與啟動時間都要相符）、Port 上沒有監聽者，以及核心 JAR 沒有被其他程式開著。V1 在 `testlab-console` 累積的舊啟動紀錄，以及中斷後沒有 Host 身分的紀錄，不會擋下移交。接著列出要移走的 V1 標記、核心 JAR 的檔名、Port、監聽位址，以及之後要帶入的 `revision`。V1 仍在執行時回報 `V1HostRunning`、`V1ServerListening` 或 `CoreInUse`，資料夾不會變動。
3. 移交。把預覽回傳的 `revision` 原樣帶入。Velocity 監聽非 `127.0.0.1` 時，要再附上監聽位址確認，例如 `0.0.0.0`：

   ```powershell
   & $ihctl $workspace v1-handoff NAME REQUEST JAVA DIRECTORY REVISION [LISTEN_ADDRESS]
   ```

   移交依序做這些事：
   - 寫入 `DIRECTORY\.instanceharbor-v1-handoff\manifest.json`。
   - 把 `workspace.json`、`.testlab-manager.lock` 與 `testlab-console` 移進同一個資料夾。
   - 以與管理資料夾匯入相同的檢查登記這台伺服器，核心保留原本的檔名，例如 `velocity-4.2.1-SNAPSHOT-38.jar`。

   任何一步失敗都會把資料夾還原。資料夾在預覽後有變動時，回報 `V1HandoffPreviewChanged`，需要重新預覽。
4. 回退。伺服器停止時執行：

   ```powershell
   & $ihctl $workspace v1-handoff-rollback NAME
   ```

   回退會取消這台伺服器在 1.0 的登記，依清單把 V1 標記搬回原處，並刪除 `.instanceharbor-v1-handoff`，之後 V1 可以重新接手。

鎖定的 Velocity 核心雜湊 `db6be7e3…aaa` 對應官方的 `velocity-4.2.1-SNAPSHOT-38.jar`（build 38，stable）。檔名標示的 build 與此不同時，以雜湊為準。內建清單沒有的官方 build 會改用 PaperMC 官方清單核對。

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

建立實例時指定名稱、Port、上層資料夾及核心來源。上層資料夾預設帶入「應用程式設定 → 建立實例」的「新伺服器的預設位置」，未設定時是工作區的 `instances`，每次建立仍可修改。
預設位置可選的資料夾與管理資料夾相同：工作區 `instances` 或其子目錄，或工作區以外的本機資料夾；工作區的其他資料夾、包含工作區的上層資料夾、磁碟根目錄與 UNC 路徑會被拒絕並保留原設定。按「使用預設位置」後儲存即清除設定。
變更只影響之後的建立，已存在的伺服器不會搬動。設定存在工作區的 `creation-defaults.json`（例如 `{"parent": "D:\\Servers"}`，清除後為 `{}`）。
舊版（1.1.0 以前）不讀這個檔案，回退後照舊使用 `instances`，重新使用新版時讀回原設定。
名稱限 1 至 64 個小寫英文字母、數字、連字號或句點，開頭使用字母或數字，版本可直接寫入名稱，例如 `paper-1.14.4`。
名稱不能以句點結尾（Windows 會自動去掉結尾的句點），句點前的部分也不能是 Windows 保留名稱（例如 `con.1`）。
同一規則適用於匯入、管理資料夾自動登記與 V1 移交；request ID、run ID 與工作 ID 仍只使用小寫英數與連字號。
確認畫面會顯示完整目的地。目的地尚未被管理資料夾涵蓋時，「新增管理資料夾」預設加入上層資料夾，
可改成實例資料夾本身、更上層的資料夾，或選「不加入管理資料夾」（實例仍受管理，清單維持不變）；其他路徑會被拒絕。
加入的資料夾內若有其他會被自動登記的伺服器，確認畫面以「自動登記為實例」逐列列出實例名稱、核心、Port 與資料夾路徑（沒有時顯示「無」），掃描方式與「新增管理資料夾」相同；
建立前這份清單有變動時，確認會要求重新預覽。切換「新增管理資料夾」只重算管理資料夾與清單（`gui-create-rescope`），不再向官方來源查最新建置；開啟確認畫面後會在背景預先算好每個選項，同一次確認內切換回已算過的選項不再重新掃描。
切換後的重新預覽失敗時，選項改回畫面上那份預覽的選項，並顯示失敗原因；「建立」送出的永遠是畫面上選取的選項。
1.0.x 留下的中斷建立沒有管理資料夾欄位，續做時依當時的方式只把實例資料夾本身加入管理資料夾。
建立完成時，同一個建立工作會立刻登記這些伺服器，不等常駐 Manager 下次掃描；登記失敗只回報 `registrationError`，實例仍已建立，之後由 Manager 掃描補登記。目的地已被涵蓋時不顯示這個選項，也不新增路徑。
「匯入已有伺服器」不會新增管理資料夾。
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

## 測試管理員權限

「應用程式設定 → 測試管理員」可設定最多 10 位測試管理員（Minecraft 名稱與 UUID）。設定只存在
工作區的 `test-administrators.json`，不進 repository。輸入 Minecraft 名稱後，應用程式會向 Mojang
查詢線上 UUID 並填入，同時改用 Mojang 回傳的名稱大小寫。UUID 欄位唯讀但不變暗，只顯示名稱查到的
UUID；改成另一個名稱（不分大小寫）時清空並重新查詢，已儲存的 UUID 屬於已儲存的名稱。新建實例使用
`online-mode=true`，所以 Mojang 的 UUID 就是伺服器使用的 UUID。查詢只在名稱符合 3 到 16 個英數字
或底線時送出，逾時 8 秒。在名稱欄按 Enter 或點旁邊的 ↵ 按鈕會立即查詢，離開名稱欄也會查詢；
UUID 已屬於這個名稱時按鈕停用，查詢失敗後可再按一次重試。管理員動詞為 `test-admin-lookup <名稱>`，
找不到、網路失敗與被限流各有不同的錯誤訊息。

「LuckPerms 權限」與「OP」是兩個獨立的開關，可任意組合；兩個都關閉，或清單留空，都表示停用，
新建實例不授權也不安裝 LuckPerms。舊的設定檔只有 `administrators` 與 `group`，載入時視為
LuckPerms 開、授予所有權限、OP 關。

- **LuckPerms 權限**：授權內容可選「所有權限（`*`）」或「自訂權限節點」。自訂節點每行一個，只能使用
  `a-z`、`0-9` 與 `._*-`，開頭的 `-` 表示該節點設為 `false`，每個節點最長 200 字元、最多 50 個，
  選自訂卻沒有節點會被拒絕。群組名稱選填：留空時節點直接寫在使用者上；填寫時節點寫在該群組，
  成員加入 `group.<名稱>`。群組不可為 `default`，避免所有玩家都得到全部權限。
  關閉「LuckPerms 權限」或改選「所有權限」時，節點與群組會保留但不套用；只有按各欄旁的「清除」
  才會清空（欄位有內容時才能按）。
- **OP**：建立實例時寫入 `ops.json`（`uuid`、`name`、`level` 為 4、`bypassesPlayerLimit` 為 `false`），
  使用同一份管理員清單。關閉時不寫 `ops.json`，由伺服器自行建立空的清單。OP 與 LuckPerms
  無關，只開 OP 時不需要也不會安裝 LuckPerms。Velocity 代理沒有 OP：只開 OP 時，建立 Velocity 的預設是不授權，
  確認畫面說明原因；只有明確選「授權」才會停止建立並回報 `UnsupportedTestPermissionPlatform`。

LuckPerms 使用 `config/test-admin-permissions.json` 固定的版本、官方來源與 SHA-256：

| 平台 | 檔案 | 適用核心 |
|---|---|---|
| Bukkit | `LuckPerms-Bukkit-5.5.85.jar` | Paper、Folia、Spigot 等需要 Java 11 以上的版本 |
| Bukkit（Java 8） | `LuckPerms-Bukkit-5.4.145.jar` | 仍以 Java 8 執行的版本，例如 Spigot 1.8.8、Paper 1.14.4 |
| Velocity | `LuckPerms-Velocity-5.5.85.jar` | Velocity 代理 |

5.5 系列以 Java 11 編譯，所以依核心可用的 Java 主版本選擇。設定頁的「下載 LuckPerms」只下載缺少或
雜湊不符的檔案，核對 SHA-256 後才放入工作區的 `plugin-cache/luckperms/`。建立實例時只讀這份快取，
不會下載；快取缺少或雜湊不符時停止建立並說明原因，不改用其他版本。

建立實例的「測試管理員權限」預設依位置決定：`instances/Agent/` 底下的 Agent 隔離實例不授權，
其他位置授權；但開啟的方式都不適用這個核心時（例如 Velocity 只開 OP），預設改為不授權，預覽回傳的
`testAdministrators.skipped` 帶原因代碼（`ProxyHasNoOp` 或 `UnsupportedCore`）。LuckPerms 尚未下載不算不適用，仍會停止建立。可在建立時改為「授權」或「不授權」，兩個開關都適用；確認畫面列出將套用的內容，
例如「授權 1 位 · LuckPerms · OP」。LuckPerms 開啟時，新實例的 `plugins/` 會有核對過的
LuckPerms JAR，`plugins/LuckPerms/`（Velocity 為 `plugins/luckperms/`）寫入 `storage-method: json` 的
`config.yml`、每位管理員的 `json-storage/users/<uuid>.json`，以及讓 `lp user <名稱>` 在首次登入前
就能查到的 `json-storage/uuidcache.txt`。每台實例的權限資料各自獨立。確認畫面會列出授權人數與檔案。
LuckPerms 首次啟動時會向自己的來源下載執行所需的程式庫，需要網路連線。

既有實例不會自動授權，已有 LuckPerms 資料的資料夾也不會被覆寫。需要時先停止並備份該實例，
啟動後在主控台執行 `lp user <名稱> permission set * true`，再用 `lp user <名稱> permission info` 讀回確認。
只放入 JAR 不算授權完成。



## 候選邊界

### Windows 未整合項目

目前開發檢查點的初始化預設與既有世界設定，請見
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

**新增觀察狀態時的顏色檢查**：清單能顯示的每個狀態都列在 `src/instance-state.ts` 的 `stateTones`，並指定一種狀態色（`ok`、`warn`、`error`、`muted`）。選取列的徽章顏色由 `src/theme.ts` 的 `refreshSelectionTheme()` 依選取顏色與不透明度計算，確保對比至少 4.5:1。新增狀態時：

1. 在 `stateTones` 加上狀態與狀態色。
2. 在 `src/app.css` 加上未選取的 `.badge.<狀態>` 規則（`var(--<狀態色>)`），以及選取列 `.instances tr.selected .badge` 的規則（`var(--row-selection-<狀態色>)`）。
3. 執行 `node --test src/instance-state.test.mjs`。介面有名稱卻沒有狀態色，或缺少任一條 CSS 規則時，測試會失敗。
進階自動重啟可設定每次重試間隔、計算視窗和穩定期，Manager 重開後保留排程及暫停狀態。
手動停止會抑制重啟，包含在 InstanceHarbor 後台輸入的 `stop`、`end`、`shutdown`、`restart`（可加 `/` 或 `minecraft:` 等前綴）及 Ctrl+C。
其他就緒後的正常退出（退出碼 0）與一般模式相同，依「非預期退出時重啟」設定處理。
Paper、Folia 從自己的後台收到 stop 時，輸出會有一行 `]: Stopping the server`（RCON 與玩家則是 `[Rcon: …]`、`[玩家名稱: …]`）；Host 依此判定手動停止，所以用方向鍵叫出歷史指令或 Tab 補全的 `stop` 也不會重啟，RCON、插件造成的正常退出則照常重啟。
Velocity 沒有這行輸出：在後台做過 Host 無法讀回的編輯後按 Enter，若 120 秒內正常退出，視為可能的手動停止，不重啟（原因 `ConsoleStopNotVerified`）。
獨立主控台視窗（tier 3）的退出碼 0、130、143 與 `0xC000013A` 無法區分是否為使用者關窗，不重啟；其他退出碼依當機設定處理。

核心維護支援同核心與同版本線的單台替換、備份還原及中斷核對。
Velocity 可在同一個 Velocity 版本內換 build（例如 4.2.1-SNAPSHOT build 38 → 43），Port 以 `velocity.toml` 的 `bind` 核對。
`-SNAPSHOT` 版本只有 BETA 頻道，所以 Velocity 也接受 BETA build；ALPHA 一律不提供，Paper、Folia 仍只接受 STABLE。核心檔名沿用登記的名稱，不改寫。
已核對的 Velocity 也可以換到較新的 Velocity 版本（例如 4.2.1-SNAPSHOT → 4.3.1），入口是核心維護的「更換 Velocity 版本」，分兩步確認：

1. 選擇目標版本（`core-velocity-versions`）。版本清單取自 fill API 的 `projects/velocity`，依 API 回傳順序判斷新舊，只列出比目前新的版本；SNAPSHOT 照常列出並標示。
2. 摘要頁（`core-velocity-target`）列出「目前版本 → 目標版本」、目標版本最新可用的 build 與發布頻道，以及風險：插件可能需要更新、不備份 `plugins` 與插件資料、會備份 `velocity.toml` 與 `forwarding.secret`。
   勾選「我了解跨版本更新的風險」後才能確認，確認後執行 `core-install-version`。

跨版本更新沿用同版本更新的停止條件與交易，另外把 `velocity.toml` 與 `forwarding.secret` 備份在該請求的 `proxy-files`。
還原跨版本的備份時，先保留目前的這兩個檔案，再放回備份當時的檔案並替換核心；替換沒有完成時，核對會放回目前的檔案。
批次更新只處理同版本的候選，不會包含跨版本更新。
Purpur 以官方 API（`api.purpurmc.org/v2/purpur/<版本>?detailed=true`）檢查同一個 Minecraft 版本線的新 build，只接受 `result` 為 `SUCCESS` 的 build。
API 只提供 MD5，所以目前 JAR 的 build 以 MD5 比對官方清單找出；找不到相符的 build 時不提供更新（`CoreSourceUnverifiedLocalHash`）。
候選以官方 MD5 指定，下載走 HTTPS，內容須與該 MD5 相符；之後計算 SHA-256 登記為已核對，往後的核對都用 SHA-256。
畫面以「核對方式：以 Purpur 官方 MD5 核對」和 SHA-256 核對區分。未核對、只由 JAR 自述為 Purpur 的實例也能更新，更新後改為已核對。
Purpur 的候選在下載前沒有 SHA-256，所以不支援預先下載與批次更新，批次預覽會提示改用單台核心維護。API 不提供檔案大小，下載時只顯示已下載的大小。

替換核心時，目的地 JAR 只要有任何開啟的檔案代碼（例如清單正在計算無法離線辨識的核心雜湊，或防毒軟體掃描）就會失敗。替換最多等待 5 秒讓這類讀取結束，仍失敗才回報錯誤。
來源檢查依核心及版本分組。開發或部署時可用程序環境
`INSTANCEHARBOR_PAPERMC_CONTACT_URL` 指定另一個有效的 HTTPS 聯絡網址。
Spigot 1.8.8 的原生提交距離和同版本發布資訊各自核對。原生版本顯示落後只提供診斷資訊，Spigot 更新仍需使用已核對的本機 JAR。
在「應用程式設定 → 核心更新」可開啟「預先下載核心更新」，預設關閉。開啟後，常駐 Manager
會核對同版本線的 Paper 與 Velocity 更新，並在工作區依 SHA-256 共用候選。準備期間各實例可繼續運行。
確認更新且符合原有的停止條件後，管理端核對本機候選，再為各台分別備份及替換。
最後一台已更新或移除管理，且目前無安裝使用該檔案時，Manager 會回收候選。關閉設定會停止背景下載，
並回收已無安裝使用的預先下載副本。已準備的候選及待更新實例可在 Manager 重開後重新核對。
共用候選位於本工作區的 `staging/core-prefetch`，和共用核心快取分開。
批次更新可從全部操作或目前多選的實例開啟。確認前可檢查逐台計畫。常駐 Manager 保存批次 ID 及逐台派工狀態。
在進度視窗或操作紀錄選擇「取消剩餘」，仍在排隊或準備候選 JAR 的項目會取消。
已進入核心替換交易的項目會完成或保留結果供核對。已更新項目的備份可從單台核心維護還原。
Manager 重開時先核對已進入替換交易的原請求，已取消的項目保持取消。其餘未進入交易的項目依原計畫重新核對登記與核心身分。

單台核心維護與批次更新都標示目前的階段：核對更新來源、下載核心、使用已下載的核心、核對 SHA-256、備份目前的核心、替換核心。
下載時，來源提供檔案大小（PaperMC fill API 的 `size`）就顯示有數值的進度條與「已下載 / 總大小」，例如 `23.4 / 52.7 MB`（1 MB 為 1024² 位元組）。
來源沒有提供大小時退回無限進度條，只顯示已下載的大小，不推算百分比。

- **單台**：GUI 以 `core_maintenance_with_progress` 指令執行 `core-install-latest`、`core-install` 與 `core-restore`，階段經 Tauri Channel 即時回報。
- **批次**：由常駐 Manager 執行。目前項目的階段寫入 `core-batches/<批次 ID>.progress`，`core-batch-list` 讀出後附在執行中項目的 `progress` 欄位。
  這個檔案只供顯示，遺失或過期都不影響批次結果；同一階段的位元組最多每 0.4 秒寫一次。
- **同一核心只下載一次**：批次中相同 SHA-256 的候選共用一份。開啟預先下載時，Paper 使用 `staging/core-prefetch`；其他情況放在
  `staging/core-batch-candidates`。第二台起顯示「使用已下載的核心」，不顯示下載進度。沒有任何進行中的批次項目需要時，Manager 回收該候選。
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
`testAdministrators` 可選 `true` 或 `false`：省略時 Human 授予已設定的測試管理員，Agent 不授權。
授權時依設定的開關寫入 LuckPerms 資料與 `ops.json`，內容與 GUI 建立相同，`prepare.json` 另記錄 `testAdministrators` 人數。
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
RC2 使用系統提供的 Microsoft Visual C++ v14 x64 Runtime。1.0.0 起執行檔靜態連結 Visual C++ 執行階段（`.cargo/config.toml`），不再需要 Visual C++ 可轉散發套件。rc2 Setup 曾把 `vcruntime140.dll` 與 `vcruntime140_1.dll` 放在安裝資料夾；1.0 Setup 更新與解除安裝時會刪除這兩個檔案。GUI 另使用 Windows WebView2。
### Windows x64 發行版封裝（1.0.0-rc2 與正式版）

完整步驟（取得 ConPTY 套件、建置、授權通知、ZIP 與 Setup）見 [docs/windows-distribution-licenses.md](windows-distribution-licenses.md) 的「Build a distribution」。
從 repository 根目錄以 PowerShell 7 執行 `./build-product.ps1 -Release -ConsoleRuntimeSource <ConPTY .nupkg>`，使用已安裝的 MSVC、Windows SDK 與鎖定的前端依賴。
用 `pack-windows.ps1` 將 release 執行檔、已核對的 ConPTY 與授權文件封裝成 ZIP。不要傳入 `-VCRuntimeDirectory`。
`1.0.0-rc2` 與正式版號（例如 `1.0.0`）是發行版：必須附授權通知與啟動說明；正式版另必須包含 `conpty.dll` 與 `OpenConsole.exe`，缺少時拒絕封裝。
`build-product.ps1 -Release` 沒有 `-ConsoleRuntimeSource`（或環境變數 `IH_CONSOLE_RUNTIME_SOURCE`）時會失敗，詳見「伺服器後台」。
接著用 `pack-windows-setup.ps1` 傳入 ZIP、預期 SHA-256 和既有 NSIS 編譯器。
Setup 從通過雜湊核對的 ZIP 取得同一套執行檔。ZIP 使用者請先讀 `WINDOWS-GETTING-STARTED.txt`。
Setup 每次執行都先檢查 WebView2 Runtime（登錄的版本號與該版本的 `msedgewebview2.exe` 都要存在，與程式啟動時找到的相同）；缺少時說明要安裝 Microsoft「Evergreen 獨立安裝程式」的 x64 版，
可直接開啟 Microsoft 下載，並在寫入前停止（靜默安裝回傳退出碼 3）。InstanceHarbor 每次啟動也做同樣檢查，
缺少時提示後結束，不會留下沒有視窗的背景程序。
這個提示不需 GUI 啟動即可決定語言：先唯讀讀取將使用的工作區 `preferences.json` 的語言設定；
設為「跟隨系統」或讀不到時，看 Windows 顯示語言，繁體中文（台灣、香港、澳門）顯示繁中，其他一律英文。
Setup 的對話框用同一條系統語言規則。
已安裝時，Setup 比較版本：較舊時詢問是否更新，相同時詢問是否修復（重新安裝程式檔案），
較新時預設拒絕並詢問是否確定退回舊版。工作區資料一律保留；InstanceHarbor 仍在執行時，Setup 不替換檔案。
使用者取消時退出碼為 4。

已安裝的程式仍在執行時（更新或修復），Setup 會先關閉它，不再要求使用者自行關閉：

- 能否繼續只看安裝標記檔 `instanceharbor.install`：GUI、CLI、Manager 與每台伺服器的 Host 都持有它直到結束，
  Setup 能以不共用的方式開啟它時才替換檔案，並持有到替換完成。伺服器台數只用來說明為何等待。
- Setup 把自己的 `ihctl.exe` 解到暫存資料夾，以 `ihctl setup-probe servers|close ...`（`src-tauri/src/setup_probe.rs`）
  呼叫**已安裝版本**的 `ihctl`：用 `list` 讀取預設正式工作區（`%LOCALAPPDATA%/InstanceHarbor/workspaces/default`）
  不是「已停止」的伺服器台數（規則與系統匣「結束」相同）；台數為 0 時請已安裝版本結束 GUI 與 Manager。
  1.1.0 起用 `ihctl <工作區> app-exit`，與系統匣「結束」同一套流程（不詢問，有批次工作或核心更新時回報 `ManagerHasActiveWork`）。
  1.0.0／1.0.1 沒有這個指令，Setup 依已安裝的版本號改用 `manager-exit`，並請使用者自行關閉 GUI 視窗；不會對舊版送出未知指令，也不以視窗標題送關閉訊息。
  2026-11-25 在 Windows Sandbox 實測 1.0.1：Manager 結束後 GUI 不會自行結束，按視窗關閉後縮到系統匣，仍持有標記檔，所以等待頁請使用者從系統匣結束。
- 互動安裝在歡迎頁後顯示「正在關閉 InstanceHarbor」等待頁，每 1.5 秒重新檢查：有伺服器沒有停止時顯示「還有 N 台伺服器沒有停止」；
  台數為 0 但標記檔仍被占用時，說明可能是其他工作區或已不在管理範圍的伺服器。Setup 不替使用者停止伺服器，
  等待頁只提供「開啟 InstanceHarbor」與「取消」；取消時退出碼為 4，不改動已安裝的檔案。標記檔釋放後自動進入安裝頁。
- 靜默安裝（`/S`）不等待伺服器：有伺服器沒有停止或狀態讀不到時立即失敗，退出碼為 5，不改動已安裝的檔案。
  伺服器都已停止時自動關閉 GUI 與 Manager，最多等 30 秒標記檔釋放，仍被占用時退出碼為 2。
- 等待頁各狀態的文字對應（`windows-setup-close.nsh`）由 `tests/verify-windows-setup-version.ps1` 一併驗證。版本順序與 WebView2 偵測由 `tests/verify-windows-setup-version.ps1 -Compiler <makensis.exe>` 驗證。
Windows WebView2 與伺服器所需的 Java 由電腦提供。

Setup 安裝在 `%LOCALAPPDATA%/Programs/InstanceHarbor`，在「開始」與已安裝的應用程式中登記為 InstanceHarbor。
雙擊便攜版或已安裝的 `InstanceHarbor.exe`，預設使用 `%LOCALAPPDATA%/InstanceHarbor/workspaces/default`。
以腳本直接啟動時，`IH_WORKSPACE` 可指定已初始化的工作區。Setup 建立的「開始」捷徑固定使用正式預設工作區。
需要隔離多個候選時，請明確指定各自的工作區。這個版本第一次啟動會建立預設工作區，不會自動遷移舊版資料。

解除安裝前先從 GUI 正常停止各台伺服器，再從系統匣選「結束」，一併關閉 GUI 與該工作區的 Manager。
也可以改用該安裝的 `ihctl.exe WORKSPACE manager-exit`。
卸載會保留工作區與日誌。要測試回退可先備份工作區，解除安裝 Setup，再用已驗證的舊 ZIP 開啟同一工作區。
便攜安裝可用 `install-windows.ps1 -Action Rollback` 切回已安裝的前一版，包含 GUI 檔名仍為 `instanceharbor-v2.exe` 的候選。保留工作區中的新舊設定，重開後先核對再啟動伺服器。

### 同時使用正式版與開發版

在 repository 根目錄執行 `npm run dev` 或 `./launch-development.ps1`。第一次會建置開發版，
並依 checkout 路徑建立專用工作區。後續會使用同一工作區。從 Windows「開始」的 Setup 捷徑啟動時，
會使用正式版與正式預設工作區。兩版可同時開啟，再次啟動同一版時會定位該版原視窗。

正式版使用 `dist` 與 Rust `release`，開發版使用 `dist-dev` 與 Rust `dev-channel`。
開發版的執行檔為 `InstanceHarborDev.exe`，使用 `%LOCALAPPDATA%/InstanceHarbor Dev`。其工作區、設定、WebView2 與日誌都在該目錄下。

開發版依工作區分開 GUI，讓多個 Agent 可以同時驗收不同工作區。正式版不變，一次只開一個 GUI。

| 項目 | 正式版 | 開發版 |
|---|---|---|
| 單一執行個體鎖 | `Local\InstanceHarbor-GUI` | `Local\InstanceHarbor-GUI-Dev-<工作區鍵>`，同一工作區仍只能開一個 GUI |
| 重複啟動 | 叫出原視窗 | 只叫出同一工作區的視窗（以執行檔與含工作區的視窗標題比對） |
| WebView2 資料 | `InstanceHarbor\webview` | `InstanceHarbor Dev\webviews\<工作區名稱>` |
| 工作列身分（AppUserModelID） | Windows 預設 | `local.instanceharbor.dev.<工作區鍵>`，每個工作區一個工作列按鈕 |
| 工作列右鍵清單（Jump List）的程式名稱 | 安裝捷徑的名稱 | `InstanceHarbor Dev — <工作區名稱>`。主視窗以 `SHGetPropertyStoreForWindow` 設定 `System.AppUserModel.RelaunchDisplayNameResource` 與 `RelaunchCommand`，只存在視窗上，不寫登錄也不建捷徑；從清單點名稱重新開啟時沒有 `IH_WORKSPACE`，會提示改用啟動腳本 |
| 主視窗、工作列、系統匣提示、訊息框 | `InstanceHarbor` | `InstanceHarbor Dev — <工作區名稱>` |
| 伺服器後台視窗 | 原標題 | 原標題後加上 `[InstanceHarbor Dev — <工作區名稱>]` |
| 側欄名稱、「關於」 | `InstanceHarbor` | 側欄第一行 `InstanceHarbor Dev`，第二行工作區名稱；「關於」另列「開發工作區」與 Git commit |
| GUI 診斷紀錄 | `InstanceHarbor\logs` | `InstanceHarbor Dev\logs\<工作區名稱>`，同時開著的 GUI 不會互搶寫入鎖與 16 MiB 容量上限 |
| 應用程式圖示（執行檔、工作列、Alt+Tab、標題列、系統匣、側欄） | 藍色 | 紅色，形狀相同 |

`<工作區名稱>` 是工作區資料夾名稱；`<工作區鍵>` 是名稱轉小寫後的 FNV-1a 64 位元雜湊（16 位十六進位），
讓鎖名稱與 AppUserModelID 長度固定。實作集中在 `src-tauri/src/desktop_channel.rs`。
1.0.1 以前的開發版把所有工作區的 WebView2 資料放在 `InstanceHarbor Dev\webview`，GUI 診斷紀錄直接放在 `InstanceHarbor Dev\logs`；這兩處的舊資料不再使用，可以手動刪除。

需要重新建置開發版，或手動指定另一個開發工作區時，可使用下列入口。

```powershell
./launch-development.ps1 -Build
$devWorkspace = Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev/workspaces/my-feature'
./launch-development.ps1 -Workspace $devWorkspace -Initialize
```

開發工作區必須是 `%LOCALAPPDATA%/InstanceHarbor Dev/workspaces` 的直接子目錄，
名稱可使用英數、連字號和底線。開發版 GUI 缺少 `IH_WORKSPACE` 或工作區不在此目錄時拒絕啟動。
開發版 CLI 也要求明確傳入同範圍的工作區。正式版拒絕開發資料目錄，Manager 仍按工作區路徑區分。

Rust 測試要在兩種設定下各跑一次，開發版專屬行為只在加上 feature 時編譯：

```powershell
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --features development-channel
```

測試夾具把臨時工作區建在系統暫存資料夾。只有本 crate 的單元測試（`cfg(test)`）會讓開發版以正式版規則檢查暫存資料夾內的工作區；
任何執行檔都不編譯這段，所以實際執行時的開發版限制不變。位置規則本身由 `desktop_channel` 的測試以假的 LocalAppData 在兩種設定下驗證，
不會建立或改動真正的 `InstanceHarbor Dev` 資料夾。
兩版同時啟動伺服器時，請為各工作區使用獨立副本，並為每台伺服器指派不同的系統連接埠。

#### 開發版紅色圖示

開發版的圖示由正式版的藍色圖示只轉動色相（+140°）產生，飽和度、亮度、透明度與 ICO 內每個尺寸都不變：

| 正式版（藍） | 開發版（紅） | 使用位置 |
|---|---|---|
| `src-tauri/icons/icon.ico` | `src-tauri/icons/icon-dev.ico` | 執行檔資源 32512：檔案總管、工作列、Alt+Tab、標題列、系統匣 |
| `src/assets/instanceharbor-brand.png` | `src/assets/instanceharbor-brand-dev.png` | 側欄品牌圖 |

建置時自動選用，不需要手動切換：`build.rs` 依 `development-channel` feature 選 ICO（`src-tauri/src/app_icon.rs`），
`build-product.ps1 -Development` 以 Vite mode `development-channel` 建置前端，`vite.config.ts` 據此選側欄圖。
系統匣圖示讀取執行檔資源，所以跟著執行檔切換。

更換藍色圖示後，在 repository 根目錄重新產生紅色版並一起提交（只用 Python 標準函式庫）：

```powershell
python generate-development-icons.py
python generate-development-icons.py --check
```

`--check` 確認紅色檔只改了色相、尺寸與目錄欄位相同，並確認藍色版仍是藍色、紅色版是紅色。
要調整紅色深淺，修改腳本的 `HUE_SHIFT_DEGREES` 後重新產生。

防止正式版誤用紅色圖示的檢查：

- `cargo test` 的 `app_icon` 測試：正式版設定必須嵌入 `icons/icon.ico`，開發版必須嵌入 `icons/icon-dev.ico`，兩檔尺寸相同但內容不同。
- `build-product.ps1` 建置後以 `verify-app-icon.ps1` 讀出 `InstanceHarbor.exe`（或 `InstanceHarborDev.exe`）與 `ihctl.exe` 內嵌的圖示，
  逐位元組比對該版本應使用的 ICO；並確認前端只含該版本的側欄圖。不符時建置失敗。

讀出執行檔內嵌的圖示並另存為 ICO，再產生紅藍對照圖：

```powershell
./verify-app-icon.ps1 -Binary <正式版 exe> -Export $env:TEMP/product.ico
./verify-app-icon.ps1 -Binary <開發版 exe> -Export $env:TEMP/development.ico
python generate-development-icons.py --comparison $env:TEMP/icon-comparison.png --product $env:TEMP/product.ico --development $env:TEMP/development.ico
```

### 版本與 commit

建置時把 Git commit 前七碼寫進 `InstanceHarbor.exe`（開發版為 `InstanceHarborDev.exe`）與 `ihctl.exe`。
工作目錄有未提交的變更（包括未追蹤的檔案）時加上 `-dirty`，例如 `33dfd35-dirty`。
沒有 Git 或不在 Git checkout 時，值為 `unknown`，建置不會失敗。

- `build-product.ps1` 讀取 `git rev-parse HEAD` 與 `git status --porcelain`，把結果放進 `IH_BUILD_COMMIT` 傳給 Rust 建置，
  建置後確認 `ihctl --version` 報告同一個值，並寫入 `build-manifest.json` 的 `commit`。
- 直接執行 `cargo build` 時，`src-tauri/build.rs` 自己向 Git 查詢。它在 commit、切換分支或暫存變更後重新查詢，
  但只修改檔案、沒有暫存時不會重新查詢，所以 `-dirty` 可能沒有更新。需要準確的值時使用 `build-product.ps1`。
- 手動設定 `IH_BUILD_COMMIT` 時，值必須是七位小寫十六進位（可加 `-dirty`）或 `unknown`，否則 Rust 建置失敗。

`ihctl --version` 輸出 `InstanceHarbor CLI 1.0.0 (33dfd35)`，開發版為 `InstanceHarbor Dev CLI 1.0.0 (33dfd35)`。
「應用程式設定」→「關於」顯示版本、Git commit（開發版另顯示開發工作區）、作者、GitHub 儲存庫與回報問題的按鈕，以及授權條款與第三方授權通知的按鈕。這一頁沒有可儲存的設定，所以不顯示「儲存設定」與「還原變更」。
GitHub 按鈕用預設瀏覽器開啟固定的 `https://github.com/Command1264/InstanceHarbor` 與其 `/issues`，頁面只傳送連結名稱，不傳送網址。
授權按鈕用記事本開啟執行檔旁的 `LICENSE` 與 `THIRD-PARTY-NOTICES.txt`。開發版與本機建置沒有這兩個檔案，按鈕會停用。
GUI 診斷紀錄的 `gui-start` 事件也記錄 `commit`。

### 儲存位置

`%LOCALAPPDATA%` 下，正式版使用 `InstanceHarbor`，開發版使用 `InstanceHarbor Dev`（下表以 `<應用程式資料夾>` 表示）。

| 用途 | 位置 | 說明 |
|---|---|---|
| 工作區 | `<應用程式資料夾>\workspaces\default`（正式）、`InstanceHarbor Dev\workspaces\<名稱>`（開發） | 實例登記、設定、工作紀錄與執行狀態 |
| 測試管理員 | 工作區的 `test-administrators.json` 與 `plugin-cache/luckperms/` | 只存在本機；見「測試管理員權限」 |
| GUI 診斷紀錄 | `<應用程式資料夾>\logs\gui-*.jsonl`（正式）、`InstanceHarbor Dev\logs\<名稱>\gui-*.jsonl`（開發） | 1.0 之前寫在 `<應用程式資料夾>\v2\logs`；舊檔留在原處，可以手動刪除。每個資料夾各有 16 MiB 上限 |
| WebView2 資料 | `<應用程式資料夾>\webview`（正式）、`InstanceHarbor Dev\webviews\<名稱>`（開發） | 啟動腳本已設定 `WEBVIEW2_USER_DATA_FOLDER` 時，正式版改用該資料夾；開發版一律使用左欄位置 |
| Setup 安裝位置 | `Programs\InstanceHarbor` | 解除安裝登錄 `InstanceHarbor-Desktop`，安裝租約檔 `instanceharbor.install` |

GUI 診斷紀錄每次啟動寫一組檔案，檔名為 `gui-<GUI 啟動的本機時間 yyyyMMdd-HHmmss>-<PID>-<分段序號>.jsonl`，
例如 `gui-20261125-060415-56824-1.jsonl`。單檔寫到 2 MiB 時，最後一行寫入 `segment-full` 事件與下一段的檔名，
接著在 `-2.jsonl` 繼續記錄。資料夾總量超過 16 MiB 時，從最久沒有寫入、而且不屬於目前這次啟動的 `gui-*.jsonl`
開始刪除，1.0.x 的舊檔名（`gui-<PID>-<奈秒>-<序號>.jsonl`）也一起計算；其他檔案只計入容量，不會被刪除。
目前這次啟動本身寫滿 16 MiB 時，之後的事件不再記錄，GUI 顯示容量上限的提示。
每隔幾秒重複的背景查詢（例如 `core-batch-list`、`jobs`、`core-state`、`world-live-values`）只在第一次與結果改變時寫入；
比較的是筆數與錯誤代碼，不比較回傳值。真的無法寫入（磁碟已滿、沒有存取權）時，GUI 警告並顯示上表的資料夾。

1.0 之前的版本把 WebView2 資料放在 `%LOCALAPPDATA%\local.instanceharbor.v2`（開發版 `local.instanceharbor.v2.dev`）。
裡面只有介面快取，1.0 不再使用，可以手動刪除，或用下面的「清除快取」。`<應用程式資料夾>` 下的 `diagnostics` 與 `lifecycle` 屬於 V1，1.0 不讀寫它們。

#### 清除快取

「應用程式設定 → 清除快取」先列出每一類項目與大小，按「清除快取」並確認後才刪除。只處理下列明確位置：

| 類別 | 位置 | 說明 |
|---|---|---|
| 核心下載暫存 | 工作區 `staging` 內名稱完全符合 `download-<SHA-256>`、`pending-download-<PID>-<時間>`、`core-download-<PID>-<時間>` 的資料夾 | 建立中的工作持有 `download-<SHA-256>.lock`，或寫入暫存的程序仍在執行時，標示為使用中並略過 |
| 舊版介面快取 | `%LOCALAPPDATA%\local.instanceharbor.v2`（開發版 `local.instanceharbor.v2.dev`） | 1.0 以前的 WebView2 資料 |
| 介面快取（WebView2） | 上表「WebView2 資料」的位置 | GUI 執行中無法刪除。清除時在工作區寫入 `webview-cache-clear.json`，下次啟動 GUI、WebView2 開啟前刪除；開發版只影響該工作區自己的資料夾。啟動腳本以 `WEBVIEW2_USER_DATA_FOLDER` 指定其他資料夾時不提供清除 |

清單與刪除時都略過連結與 junction，不刪除其他檔名，也不以萬用字元決定目標；`staging` 內的建立暫存（`gui-create-*`）、續做紀錄、`core-prefetch` 與工作區其他資料都不在範圍內。
確認後若清單已變更（例如有新的下載），會要求重新核對。刪除失敗的項目逐項列出原因（檔案使用中、沒有存取權限等），不影響其他項目。
排定的 WebView2 刪除失敗時保留排定與原因，設定頁顯示原因，下次啟動再試一次；結果也寫入 GUI 診斷紀錄（`webview-cache-cleared`／`webview-cache-clear-failed`）。
不再使用的共用核心待共用核心（#62）完成後再加入。

工作區的 `workspace.json` 記錄格式值。1.0 新建的工作區寫入 `instanceharbor-workspace-1`。
rc2 與開發版建立的工作區保留 `instanceharbor-v2-windows-preview-1`；1.0 照常開啟且不改寫，所以舊版仍能開啟它們。
舊版不認得 1.0 新建的工作區，會回報 `UnsupportedWorkspace`。格式值以 `instanceharbor-<用途>-<N>` 命名，只有不相容的變更才增加 `N`。

### 復原金鑰

`ihctl WORKSPACE recovery-key-status` 可讀取本機 DPAPI 金鑰狀態，
`ihctl WORKSPACE recovery-key-rotate GENERATION` 依目前世代輪替金鑰，
首次建立傳入 `none`。桌面 PowerShell 7 可使用
`manage-recovery-backup.ps1` 透過安全提示輸入密碼，執行匯出、核對或回復金鑰。
備份只供同一個絕對工作區路徑復原，回復前須核對公開金鑰與世代。
應用程式設定的「復原金鑰」可操作同一套保管功能。
已釘住復原金鑰的工作階段可從原生介面或 PowerShell 接管。流程見可信 Host 接管。

復原金鑰正分階段移除（Issue #96）。目前是第 1 階段：

- 啟動伺服器時不再寫入 `runs/<實例>/recovery-pin.json`，工作區有沒有金鑰都一樣。新的 run 一律使用無金鑰的控制條件：
  同一個 Windows 帳戶，且執行檔是安裝資料夾的 GUI 或 `ihctl.exe`。Manager 重啟或更新後都能直接接手。
- 舊版啟動、已釘住金鑰而且仍在執行的 Host 照舊驗證：同一個安裝的新 Manager 用 `Rebind` 接手；安裝改變時，
  用 `ihctl WORKSPACE host-recover NAME RUN`（或設定頁「執行中工作階段」）以金鑰簽名的 `RecoverChallenge`／`RecoverProof` 接手。
  正常停止再啟動後，這台伺服器就不再依賴金鑰。
- 設定頁的「復原金鑰」分頁只在工作區已有金鑰時顯示，並說明此功能即將移除。沒有金鑰的工作區看不到這個分頁，
  `recovery-key-*` 指令與 `manage-recovery-backup.ps1` 仍可使用，第 2 階段才移除。
- 金鑰檔（工作區根目錄的 `recovery-key.dpapi.json`）與匯出的 `.ihrecovery` 備份檔不會被自動刪除。
  確認沒有仍在執行、已釘住的 run（`runs/<實例>/recovery-pin.json` 所在的實例仍在執行）後，可以手動刪除；
  刪除金鑰檔後設定頁不再顯示「復原金鑰」分頁。

`verify-recovery-pin.ps1` 用測試替身（`tests/controlled-server.rs`）驗證這些行為：無金鑰工作區、有金鑰時新啟動的 run 沒有 pin，
以及搭配 `-LegacyBinaryDirectory`（指向 1.1.0 或更早的建置）時，舊版釘住的 run 仍能由重啟後的 Manager 接管與停止。
`-Evidence` 必須是新的資料夾。開發版建置只開啟 `InstanceHarbor Dev\workspaces\<名稱>`，所以另以 `-Workspace` 指定新的隔離工作區。

工作區由明確路徑建立。掃描允許與排除名單支援絕對及相對路徑。管理資料夾、匯入與新建可以使用工作區 instances 底下或工作區以外的任意本機資料夾；工作區的內部資料夾、包含工作區的上層資料夾、磁碟根目錄與 UNC 路徑一律拒絕，登記後仍依掃描白名單決定是否納管。測試使用 IPv4 回環監聽。GUI 新建沿用全域 Java 設定，並自動核對需要的 Mojang JAR。1.0 遷移入口已移出正式 GUI；V1 伺服器改用「從 V1 移交伺服器」的 CLI 指令。

Windows 一般匯入支援放有已核對 Velocity JAR（任意檔名）與
`velocity.toml` 的代理目錄。預設接受 `127.0.0.1:PORT`。要匯入綁定其他介面的代理，
請在 GUI 的可選欄位輸入與頂層 `bind` 相符的 IPv4 位址。
匯入確認使用同一組位址、Port、核心與設定檔預覽版本。啟動時再次核對登記的位址及 Port。
更新監聽位址後須重新核對匯入。代理可監聽所有介面（`0.0.0.0`），但要輸入 `0.0.0.0` 明確確認；詳細面板會警告代理對區網與外部開放。廣播與群播位址一律拒絕，後端路由仍須是明確位址。

PowerShell 可將 IPv4 附在 `import-preview NAME JAVA DIRECTORY IPV4`。
將預覽回傳的 Port 及 revision 原樣帶入 `import NAME REQUEST JAVA DIRECTORY PORT REVISION IPV4`。
匯入只寫管理登記，Velocity 的路由、轉送與玩家登入設定由伺服器資料夾提供。
正常停止時 Host 對 Velocity 送出 `shutdown`，對遊戲後端送出 `stop`。

伺服器停止時會先關閉 port，再結束 Java，最後 Host 才退出，所以只依程序與監聽者推算的狀態會短暫經過「啟動中」與「工作階段未定」。只要有停止要求，GUI 一律顯示「停止中」，直到確認已停止。停止要求包括：GUI 送出的單台停止、批次停止，以及 run 資料夾中的 `stop-queued`、`stop-sent` 與 `console-stop-typed`；最後一個標記會以 `stopPhase: "Typed"` 回報。port 被其他程序占用時仍顯示「Port 被占用」。
此次範圍為一般匯入與啟停，代理 Port 和路由由 `velocity.toml` 管理。

Vue 與 CLI 共用 Rust 管理入口，批次啟動、正常停止及重啟由工作區的常駐 Manager 保存。
關窗後可在操作紀錄查回原工作，行為與隔離驗證見批次生命週期。

操作失敗只顯示在失敗對話框，不寫進實例清單上方的狀態文字。從本視窗送出的批次中，每台失敗或無法核對的實例一出現就列進同一個對話框，其他實例繼續排隊與啟動；對話框開著時後續失敗會併入，清單過長時在對話框內捲動。批次啟動建立時先檢查每台登記的 Java 檔案是否存在，找不到的實例立即記為失敗，不等前面的實例就緒。批次紀錄只保存穩定代碼：`JavaExecutableMissing`、`JavaExecutableUnreadable`、`PortOccupied` 與 `StartIntentChanged` 原樣保留，其他啟動錯誤記為 `StartRejected`，不保存原始系統訊息。
Host 的控制管線限制同 Windows 帳戶及本候選執行檔。強制停止須由使用者明確確認。


### 啟動中停止與逾時後重試

Host 收到停止要求後先排隊，等 Paper 的啟動日誌出現 Done 才送出 stop。觀察資料分開標示已排隊和已送出。管理端的停止等待逾時後，原工作紀錄保留結果未確認。GUI 可對仍是同一個 run 的程序提出新的正常停止，CLI 則用新請求 ID 與原 run ID。已確認停止時，重新啟動會直接啟動；啟動中或正在監聽時，會先正常停止。重啟等待停止時逾時就結束這次重啟工作，後續不會自動啟動新 run。

啟動在啟動前檢查（preflight，指尚未改動 run 資料夾、也尚未啟動 Host 或 Java 的階段）失敗時，管理端移除這次預留的工作紀錄，同一請求 ID 可以直接重送，不會變成 `ResultUnconfirmed`。已開始建立 run 之後的失敗，以及重啟中已完成停止的失敗，仍保留結果未確認。登記的 Java 執行檔不存在時回報 `JavaExecutableMissing`，無法讀取時回報 `JavaExecutableUnreadable`；GUI 會請使用者到「實例設定」→「Java 與 JVM」重新選擇 Java。

隔離驗證會縮短管理端的等待期限，重現 Paper 尚未到 Done 時發生 StopTimeout。這個測試設定只適用 Debug 驗證器及其 .evidence 工作區。它不會加速或變更 Java 的停止行為。驗收時可在狀態為啟動中時按正常停止，確認先看到等待提示，隨後回到已停止。不要對既有測服施加負載。


### 緊急停止

正常停止送出後，同一位置顯示紅字「緊急停止」。取消確認或按 Esc 會返回原畫面，讓原本的正常停止繼續。確認視窗的預設焦點是「取消」，紅色「強制停止」會再次核對實例的 run。Host 只終止自己持有的 Java 子程序；原工作階段若已退出，直接回報退出。強制停止可能遺失未存檔的世界變更，完成後須確認 Host 和 Java 退出且連接埠已釋放。強制退出不符合正常重啟的啟動條件。

隔離驗證會暫停自己建立的 Paper Java 程序，讓正常停止保持等待，以便確認焦點、取消、深淺色及強制停止。驗收時可在隔離 Paper 啟動中按「正常停止」，再點「緊急停止」，先取消一次，再重新確認。若 Paper 在確認前已正常停止，應回到已停止。


## Vue 瀏覽器預覽與對照

使用已安裝的 Node.js，從 repository 根目錄執行：

```powershell
npm ci --offline --no-audit --no-fund
npm run preview:vue -- --port 4176 --strictPort
```

- Vue 版位於 `http://127.0.0.1:4176/preview/`。
- 無框架版位於 `http://127.0.0.1:4176/preview/legacy.html`。來源是原有的 `docs/design/wireframe/index.html`，原檔維持不變。
  這個頁面與其他審查參考（例如 `activity-review.html`、`settings-inventory.html`）只保留在維護者的私人封存，公開 repository 沒有收錄。
  預覽伺服器與 `npm run build:preview` 只提供 `docs/design/wireframe/` 中實際存在的檔案，缺少的頁面連結會無法開啟。
- `npm run build:preview` 將兩個入口輸出至 `dist-preview`，可用本機靜態 HTTP 伺服器開啟。既有 Tauri 建置入口仍是 `index.html`。

Vue 元件負責預覽殼、實例表格與詳情、設定版面、操作紀錄、共用對話框、Java 選擇與核心維護。複雜表單、精靈的部分內容與模擬控制器仍以既有預覽流程為基礎，並經由共用的頁面與對話框邊界接入。生命週期佇列獨立於 Vue 元件。這些保留的相容區域可逐步替換為有型別的欄位與元件。

所有資料都是模擬。使用者主動匯出時，瀏覽器只下載紀錄摘要。操作紀錄會保留在同一分頁的工作階段儲存空間，重開頁面會將當時尚未確認的工作保留為待核對紀錄。前次工作不會重送。兩版的參考操作包含選取、設定草稿、建立與匯入、移除後再匯入、核心維護、批次操作與停止。新版單台正常停止直接送出模擬要求，等待期間可用紅字緊急停止並另行確認。Play 的示範埠對齊已整合的登記表。保留期限和 CSV 的完整流程仍由操作紀錄的補充示意頁提供。

Agent 瀏覽器檢查從 repository 根目錄執行：

```powershell
node docs/design/wireframe/verify-browser.cjs http://127.0.0.1:4176/preview/
```

測試使用獨立 Chrome 設定檔，報告與畫面輸出至 `artifacts/v2-vue-preview-validation`。Windows 原生標題列、DPI、輔助技術與真實服務行為屬後續驗證。

### 伺服器後台（預設開啟）

所有伺服器核心每次啟動都使用後台，包括 Spigot、Paper、Folia、Purpur 等 Paper 分支與 Velocity，不再依核心家族限制，也不需要工作區開關檔（舊的 `native-console.enabled` 已無作用）。後台把按鍵交給伺服器的標準輸入，不論核心用 JLine 編輯指令列或一般逐行讀取都適用。
Host 在啟動時依序選擇，並把結果與跳過原因寫入 `runs/<實例>/console-tier.json`；實例詳情的「後台」欄顯示本次使用哪一層及原因。

1. **InstanceHarbor 後台（tier 1）**：執行檔旁、雜湊固定的 `conpty.dll` 與 `OpenConsole.exe`，在 InstanceHarbor 自己的後台視窗顯示。
   優先使用，可避開 Windows Terminal 的顯示問題。
2. **Windows 內建主控台元件（tier 2）**：tier 1 檔案缺少、雜湊不符或建立失敗時，改用 Windows 內建的 pseudoconsole（kernel32 `CreatePseudoConsole`），仍在 InstanceHarbor 後台視窗顯示。
3. **獨立主控台視窗（tier 3）**：只有 tier 1、2 都不可用，且使用者在 GUI 的警告對話框選「以主控台視窗啟動」時才使用。
   Windows 依預設終端機設定開啟 conhost 或 Windows Terminal。**關閉該視窗會立即停止伺服器。**
   Host 以主控台輸入事件送出 stop 與世界設定指令；輸出由 `logs/latest.log` 複製到 `stdout.log`。
   批次啟動、自動重啟與 `restart` 永不進入 tier 3。
4. **一般模式（plain）**：沒有後台。只有 Java／伺服器目錄／核心路徑 260 個 UTF-16 字元以上，或 tier 1、2 都無法建立且使用者未確認 tier 3 時使用。

舊版 Java 由核心 JAR 的相對檔名（預設 `server.jar`）啟動，避開絕對路徑造成的 ZipFS 問題。

**手動輸入與世界設定指令並存**：Host 追蹤使用者在後台正在編輯的那一行。受管指令（世界規則、遊戲規則）只在該行為空時送出；
使用者有未送出的輸入時，指令等到使用者按 Enter 或到期限為止。到期仍未送出時回報 `WorldConsoleInputBusy`（未寫入，可重試）。
在「世界設定」儲存的遊戲規則若因此未送出，狀態為「後台那一行輸入完成後即時套用」：Host 在同一次執行中每 2 秒重試，該行清空後立即套用並讀回；停止前仍未送出時，下次啟動就緒後套用。

**停止後寫入存檔**：遊戲規則到停止前都沒有送出時，InstanceHarbor 在確認正常停止後，直接把它寫進世界存檔，
世界設定頁顯示「已寫入世界存檔，下次啟動時核對」。條件與步驟：

- 只在「InstanceHarbor 送出停止指令、Java 以代碼 0 結束」並取得世界的執行期租約後寫入。由 GUI 或 CLI 停止、Manager 背景掃描，
  或下次啟動前的檢查執行，伺服器執行中不碰存檔。遊戲內 `/stop`、當機或強制停止不寫入，維持下次啟動套用。
- 只寫主世界，與後台 `gamerule` 指令及讀回核對的範圍一致：26.x 為 `dimensions/minecraft/overworld/data/minecraft/game_rules.dat`
  （Spigot 為 `data/minecraft/game_rules.dat`），舊版為 `level.dat` 的 `Data/GameRules`。
- 只替換規則值本身，規則必須已經以該版本的型別存在。寫入前把原檔複製到工作區
  `history/world-save-backups-<實例>/`（保留最近 20 份），以暫存檔原子替換後重新讀回；讀回不符就還原原檔。
- 無法寫入時原檔不變，畫面說明原因，規則改回下次啟動套用。下次啟動就緒後仍以後台讀回核對；存檔已是目標值時只讀回、不重送。
- 世界時間不在此範圍，維持下次啟動套用。
方向鍵、Tab 補全等無法讀回的編輯會讓該行維持「非空」直到按 Enter。

**tier 1 runtime 來源**：`console-runtime.json` 固定 Microsoft ConPTY NuGet `1.25.260930003` 的 x64 檔案雜湊與簽署者，
`native_console.rs`、`pack-windows.ps1`、`install-windows.ps1` 的雜湊由測試核對一致。
`provision-console-runtime.ps1 -Source <資料夾或本機 .nupkg> -Destination <執行檔資料夾>` 核對 SHA-256、x64 與 Microsoft Authenticode 簽章後複製，不下載任何檔案；
目的地已有不同內容的同名檔時拒絕覆寫。`build-product.ps1` 以 `-ConsoleRuntimeSource` 呼叫它；開發版缺少來源時只警告（改用 tier 2），`-Release` 會失敗。

**測試各層**：開發版 Host 讀取環境變數 `IH_CONSOLE_TIER_LIMIT=system|window|plain`，跳過較高的層（原因 `ConsoleTierLimited`）。正式版忽略此變數。
`ihctl WORKSPACE console-plan <實例>` 回傳下次啟動預計的層、跳過原因與是否需要確認。
開發版另有 `ihctl WORKSPACE console-io <實例> <run> input <JSON 字串>`（或 `read <offset>`），經與後台視窗相同的 Named Pipe 送出輸入，用於不開 GUI 的自動驗證；正式版沒有此指令。

**日誌**：「查看啟動紀錄」對話框把 pseudoconsole 寫入的 VT 控制碼轉成純文字行；`stdout.log` 原檔不變。
伺服器啟動中或監聽中時，對話框每 2 秒在背景重讀；內容沒變不重繪。捲到底部時會跟著新輸出，往上捲時保留位置。背景讀取失敗時保留最後內容。

### Windows 原生後台視窗

主程式首次建立視窗時置中。重複執行只聚焦已開啟的主視窗，保留其位置。

後台使用 Windows 原生標題列與邊框。可拖邊框調整視窗、最大化，或從最大化狀態拖曳標題列還原。
視窗標題與分頁、貼上對話框文字跟隨介面語言。終端在可用空間變動時核對目前輸出，讀到尾端後向同一次執行的 ConPTY 送出尺寸。連續拖曳時合併中間尺寸，沒有輸出需要解析的過時尺寸不觸發重排。已有輸出的資料仍按 Host 回報的格線解析。
捲動使用貼近終端視窗右緣的細版懸浮捲軸，保留滾輪、鍵盤及拖曳操作。終端內層的原生捲軸隱藏，懸浮捲軸顯示時不改變文字寬度。手動捲過即時輸出底部時，最多將最後有內容的一行移到視窗頂端。留白只存在畫面上，開始輸入時回到即時提示列。隱藏後台時該次終端仍持續接收輸出。
尺寸同步需要當次 Host 支援。舊 Host 維持本來的終端尺寸，視窗操作仍由 Windows 處理。

後台有選取文字時，可用 Ctrl+C、Ctrl+Shift+C 或 Ctrl+Insert 複製。未選取文字時，Ctrl+C 在後台攔下。貼上支援 Ctrl+V、Ctrl+Shift+V 和 Shift+Insert。單行貼上只帶入文字。含換行時先顯示確認，焦點預設在「仍要貼上」，按 Esc 可取消。
