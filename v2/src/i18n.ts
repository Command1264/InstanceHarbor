import { computed, inject, onMounted, onUnmounted, provide, ref, shallowRef, watch, type InjectionKey } from 'vue'

export type Locale = 'zh-TW' | 'en'
export type LanguagePreference = Locale | 'system'
export type TimeFormat = 'local24' | 'local12' | 'utc'

import consoleTitleTokens from './console-title-tokens.json' with { type: 'json' }
import { appMessages } from './messages/app.ts'
import { instancesMessages } from './messages/instances.ts'
import { settingsMessages } from './messages/settings.ts'
import { managerMessages } from './messages/manager.ts'
import { componentsMessages } from './messages/components.ts'
import { policyMessages } from './messages/policy.ts'
import { consoleMessages } from './messages/console.ts'

export { consoleTitleTokens }
export const defaultConsoleTitleTemplate = '{instance}'

const consoleTitleKeys = new Set(consoleTitleTokens.map(([key]) => key))
const controlCharacter = /[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u2028-\u202e\u2066-\u2069]/u
const titleParts = /\{([^{}]+)\}|[{}]/g

export function validateConsoleTitle(template: string): 'invalid' | 'placeholder' | undefined {
  if (!template.trim() || template.length > 256 || controlCharacter.test(template)) return 'invalid'
  for (const [, token] of template.matchAll(titleParts)) {
    if (!token || !consoleTitleKeys.has(token)) return 'placeholder'
  }
}

export function renderConsoleTitle(template: string, values: Record<string, string>) {
  if (validateConsoleTitle(template)) return undefined
  const title = template.replace(titleParts, (_, key: string) => {
    const value = Object.hasOwn(values, key) ? values[key] : ''
    return Array.from(value).filter(character => !controlCharacter.test(character)).join('').trim() || 'N/A'
  })
  return Array.from(title).slice(0, 1024).join('')
}

const timeFormatters = new Map<string, Intl.DateTimeFormat>()

export function formatDisplayTime(value: number, locale: Locale, format: TimeFormat = 'local24'): string {
  const key = locale + ':' + format
  let formatter = timeFormatters.get(key)
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, {
      dateStyle: 'short',
      timeStyle: 'medium',
      hourCycle: format === 'local12' ? 'h12' : 'h23',
      timeZone: format === 'utc' ? 'UTC' : undefined,
    })
    timeFormatters.set(key, formatter)
  }
  return formatter.format(value) + (format === 'utc' ? ' UTC' : '')
}

const baseMessages = {
  'skip.main': ['跳至主要內容', 'Skip to main content'],
  'nav.main': ['主要導覽', 'Main navigation'],
  'nav.instances': ['實例', 'Instances'],
  'nav.activity': ['操作紀錄', 'Activity'],
  'nav.settings': ['應用程式設定', 'Application settings'],
  'connection.connected': ['管理服務：已連線', 'Management service: Connected'],
  'connection.browser': ['管理服務：需要桌面程式', 'Management service: Desktop app required'],
  'connection.unknown': ['管理服務：尚未核對', 'Management service: Unconfirmed'],
  'connection.disconnected': ['管理服務：無法連線', 'Management service: Disconnected'],
  'connection.preferencesStale': ['無法重新讀取外觀設定，保留上次設定', 'Could not reload appearance settings. Keeping the last settings.'],
  'connection.preferencesMissing': ['外觀設定尚未讀取', 'Appearance settings are unavailable'],
  'connection.desktopRequired': ['請使用 InstanceHarbor 桌面程式。設計預覽位於 v2/preview。', 'Use the InstanceHarbor desktop app. The design preview is available in v2/preview.'],
  'creation.checking': ['正在核對建立條件', 'Checking creation requirements'],
  'creation.downloadingCore': ['正在下載伺服器核心', 'Downloading the server core'],
  'creation.downloadingMojang': ['正在取得 Mojang 伺服器檔案', 'Fetching Mojang server files'],
  'creation.creating': ['正在建立並登記實例', 'Creating and registering the instance'],
  'diagnostics.title': ['路由與插件診斷', 'Route and plugin diagnostics'],
  'diagnostics.proxy': ['Velocity 實例', 'Velocity instance'],
  'diagnostics.automatic': ['自動辨識', 'Detect automatically'],
  'diagnostics.route': ['路由名稱（選填）', 'Route name (optional)'],
  'diagnostics.jar': ['插件 JAR 檔名（選填）', 'Plugin JAR filename (optional)'],
  'diagnostics.jarHelp': ['留空會核對建服時已登記的插件。其他插件請填完整檔名。', 'Leave blank to check the plugin registered during setup. Enter the full filename for another plugin.'],
  'diagnostics.refresh': ['核對診斷', 'Check diagnostics'],
  'diagnostics.checking': ['正在核對診斷', 'Checking diagnostics'],
  'diagnostics.readError': ['無法讀取診斷。請核對輸入與管理服務連線後再試。', 'Could not read diagnostics. Check the inputs and management connection, then try again.'],
  'diagnostics.runChanged': ['查詢時工作階段有變更。請重新核對診斷。', 'A run changed during the check. Check diagnostics again.'],
  'diagnostics.invalidRoute': ['路由名稱須以英文字母或數字開頭，可包含句點、底線及連字號。', 'Start with an English letter or digit. Periods, underscores, and hyphens are allowed.'],
  'diagnostics.invalidJar': ['請輸入完整的 .jar 檔名，無須填資料夾路徑。', 'Enter the full .jar filename without a folder path.'],
  'diagnostics.checkedAt': ['查詢時間：{time}', 'Checked at: {time}'],
  'diagnostics.results': ['診斷結果', 'Diagnostic results'],
  'diagnostics.configuration': ['Velocity 路由設定', 'Velocity route configuration'],
  'diagnostics.runtime': ['當次路由運作', 'Current route runtime'],
  'diagnostics.plugin': ['插件載入', 'Plugin loading'],
  'diagnostics.player': ['玩家連線', 'Player connection'],
  'diagnostics.source': ['證據來源：{source}', 'Evidence source: {source}'],
  'diagnostics.velocityConfig': ['Velocity 設定檔', 'Velocity configuration file'],
  'diagnostics.managedRun': ['管理服務工作階段', 'Managed run'],
  'diagnostics.currentRunLog': ['當次工作階段的日誌', 'Current run log'],
  'diagnostics.pluginJar': ['插件 JAR', 'Plugin JAR'],
  'diagnostics.noSource': ['無', 'None'],
  'diagnostics.playerDetail': ['需使用 Minecraft 客戶端實際登入才能證實。', 'Verification requires signing in with a Minecraft client.'],
  'diagnostics.proxyName': ['代理：{name}', 'Proxy: {name}'],
  'diagnostics.routeName': ['路由：{name}', 'Route: {name}'],
  'diagnostics.proxyRun': ['代理 Run ID：{id}', 'Proxy Run ID: {id}'],
  'diagnostics.backendRun': ['後端 Run ID：{id}', 'Backend Run ID: {id}'],
  'diagnostics.forwarding': ['轉送方式：{state}', 'Forwarding: {state}'],
  'diagnostics.match': ['設定一致', 'Configuration matches'],
  'diagnostics.mismatch': ['設定不一致', 'Configuration mismatch'],
  'diagnostics.notConfigured': ['未設定', 'Not configured'],
  'diagnostics.notApplicable': ['不適用', 'Not applicable'],
  'diagnostics.unconfirmed': ['未確認', 'Unconfirmed'],
  'diagnostics.ready': ['當次程序已就緒', 'Current run ready'],
  'diagnostics.stopped': ['已停止', 'Stopped'],
  'diagnostics.starting': ['啟動中', 'Starting'],
  'diagnostics.present': ['已找到插件檔案', 'Plugin file found'],
  'diagnostics.enabling': ['已看到啟用訊息', 'Enable message observed'],
  'diagnostics.enabled': ['已核對啟用', 'Enablement confirmed'],
  'diagnostics.failed': ['載入失敗', 'Loading failed'],
  'diagnostics.disabled': ['已停用', 'Disabled'],
  'diagnostics.observed': ['已觀察', 'Observed'],
  'diagnostics.unverified': ['未驗證', 'Unverified'],
  'diagnostics.reason.selectBackend': ['請選取 Velocity 連接的後端實例。', 'Select the backend instance connected to Velocity.'],
  'diagnostics.reason.selectProxy': ['找到多台 Velocity，請選擇要核對的實例。', 'Multiple Velocity instances were found. Choose one to check.'],
  'diagnostics.reason.proxyUnavailable': ['無法核對已登記的 Velocity 實例。', 'Could not verify a registered Velocity instance.'],
  'diagnostics.reason.noProxy': ['工作區內沒有已登記的 Velocity。', 'No Velocity instance is registered in this workspace.'],
  'diagnostics.reason.backendEndpointUnconfirmed': ['無法從後端設定檔核對監聽位址及 Port。', 'Could not verify the listen address and port in the backend configuration.'],
  'diagnostics.reason.backendBindingChanged': ['後端設定檔的監聽位址或 Port 與登記值不同。', 'The backend listen address or port differs from the registered value.'],
  'diagnostics.reason.velocityConfigUnreadable': ['無法讀取或解析 Velocity 設定檔。', 'Could not read or parse the Velocity configuration file.'],
  'diagnostics.reason.proxyBindingUnconfirmed': ['無法從 Velocity 設定檔核對監聽位址及 Port。', 'Could not verify the listen address and port in the Velocity configuration.'],
  'diagnostics.reason.proxyBindingChanged': ['Velocity 設定檔的監聽位址或 Port 與登記值不同。', 'The Velocity listen address or port differs from the registered value.'],
  'diagnostics.reason.routeMissing': ['Velocity 設定檔找不到這條路由。', 'The route was not found in the Velocity configuration.'],
  'diagnostics.reason.routeMatches': ['路由指向後端已登記的監聽位址與 Port。', 'The route points to the backend’s registered listen address and port.'],
  'diagnostics.reason.routeEndpointMismatch': ['路由指向的位址或 Port 與後端登記值不同。', 'The address or port in the route differs from the registered backend.'],
  'diagnostics.reason.routeEndpointUnsupported': ['此版只核對明確的 IPv4 位址與 Port。', 'This version checks explicit IPv4 addresses and ports.'],
  'diagnostics.reason.selectRoute': ['無法自動確定路由。請輸入 Velocity 設定中的路由名稱。', 'Could not identify a route automatically. Enter its name from the Velocity configuration.'],
  'diagnostics.reason.noCurrentPluginRun': ['目前只有插件檔案可核對。', 'The plugin file is the available evidence.'],
  'diagnostics.reason.pluginIdentityChanged': ['插件檔案已變更，無法用這份檔案核對當次載入。', 'The plugin file changed. Its current copy cannot confirm loading in this run.'],
  'diagnostics.reason.runLogUnconfirmed': ['無法從當次工作階段取得可核對的日誌。', 'No verifiable log was available for the current run.'],
  'diagnostics.reason.pluginLifecycleObserved': ['當次日誌有對應的插件載入訊息。', 'The current run log contains a matching plugin lifecycle event.'],
  'diagnostics.reason.pluginEventUnconfirmed': ['當次日誌未找到足夠的插件載入證據。', 'The current run log does not contain enough evidence to verify plugin loading.'],
  'diagnostics.reason.pluginNotApplicable': ['此版核對後端伺服器的插件。', 'This version checks plugins on backend servers.'],
  'diagnostics.reason.selectPlugin': ['請輸入要核對的插件 JAR 檔名。', 'Enter the filename of the plugin JAR to check.'],
  'diagnostics.reason.pluginDescriptorUnconfirmed': ['無法從 JAR 核對 Bukkit 或 Paper 的插件名稱。', 'Could not verify a Bukkit or Paper plugin name in the JAR.'],
  'diagnostics.reason.pluginJarUnavailable': ['找不到插件 JAR，或無法安全地讀取它。', 'Could not find the plugin JAR or read it safely.'],
  'diagnostics.reason.evidenceChanged': ['查詢時證據有變更。請重新核對診斷。', 'Evidence changed during the check. Check diagnostics again.'],
  'diagnostics.reason.bothRunsReady': ['Velocity 和後端都由當次已就緒的程序監聽。', 'Velocity and the backend are listening on their ready processes for the current runs.'],
  'diagnostics.reason.runUnconfirmed': ['無法核對其中一台當次的工作階段或監聽程序。', 'Could not confirm a current run or listening process for one of the instances.'],
  'diagnostics.reason.runStarting': ['Velocity 或後端仍在啟動。', 'Velocity or the backend is still starting.'],
  'diagnostics.reason.runStopped': ['Velocity 或後端已停止。', 'Velocity or the backend is stopped.'],
  'appearance.title': ['語言與外觀', 'Language and appearance'],
  'appearance.language': ['介面語言', 'Interface language'],
  'appearance.systemLanguage': ['跟隨系統', 'Follow system'],
  'appearance.theme': ['主題', 'Theme'],
  'appearance.systemTheme': ['跟隨系統', 'Follow system'],
  'appearance.light': ['淺色', 'Light'],
  'appearance.dark': ['深色', 'Dark'],
  'appearance.selectionColor': ['清單選取色', 'List selection color'],
  'appearance.selectionOpacity': ['清單選取底色不透明度', 'List selection opacity'],
  'appearance.selectionOpacityValue': ['不透明度百分比', 'Opacity percentage'],
  'appearance.closeToTray': ['關閉視窗時縮到系統匣', 'Keep running in the notification area when the window is closed'],
  'appearance.closeToTrayHelp': ['兩種方式都不會停止伺服器。', 'Servers keep running either way.'],
  'settings.coreUpdates': ['核心更新', 'Core updates'],
  'settings.corePrefetch': ['預先下載核心更新', 'Download core updates in advance'],
  'settings.corePrefetchUnavailable': ['目前無法核對已儲存的核心更新設定。', 'Could not verify the saved core update settings.'],
  'core.source.prepared': ['已下載並核對', 'Downloaded and verified'],
  'appearance.selectionOpacityInvalid': ['請輸入 0 到 100 的整數。', 'Enter a whole number from 0 to 100.'],
  'appearance.productColor': ['InstanceHarbor 配色', 'InstanceHarbor colors'],
  'appearance.blue': ['藍色', 'Blue'],
  'appearance.red': ['紅色', 'Red'],
  'appearance.green': ['綠色', 'Green'],
  'appearance.magenta': ['洋紅色', 'Magenta'],
  'appearance.yellow': ['黃色', 'Yellow'],
  'appearance.cyan': ['青色', 'Cyan'],
  'appearance.purple': ['紫色', 'Purple'],
  'appearance.selectionSample': ['已選取的實例', 'Selected instance'],
  'appearance.pathSeparator': ['Windows 路徑分隔符', 'Windows path separator'],
  'appearance.forwardSlash': ['正斜線 /', 'Forward slash /'],
  'appearance.backslash': ['反斜線 \\', 'Backslash \\'],
  'appearance.unavailable': ['目前無法核對已儲存的外觀設定。請回實例頁按「重新核對」。', 'Could not verify saved appearance settings. Return to Instances and select Refresh.'],
  'appearance.timeFormat': ['時間格式', 'Time format'],
  'appearance.local24': ['本地時間（24 小時制）', 'Local time (24-hour clock)'],
  'appearance.local12': ['本地時間（12 小時制）', 'Local time (12-hour clock)'],
  'appearance.utc': ['世界協調時間（UTC）', 'Coordinated Universal Time (UTC)'],
  'appearance.preview': ['預覽', 'Preview'],
  'appearance.consoleTitle': ['預設後台標題', 'Default console title'],
  'appearance.instanceConsoleTitle': ['實例後台標題', 'Instance console title'],
  'appearance.inheritedTitleHelp': ['留空可繼承預設後台標題。下次開啟後台時套用。', 'Leave blank to use the default title. Changes apply next time this console opens.'],
  'appearance.titleLength': ['請輸入 1 至 256 個字元的範本，不含換行或控制字元。', 'Enter a template with 1–256 characters and no line breaks or control characters.'],
  'appearance.titlePlaceholder': ['請使用清單中的佔位符，並以成對的大括號括住名稱。', 'Use a placeholder from the list and enclose its name in matching braces.'],
  'appearance.titleVariables': ['可用的標題佔位符', 'Console title placeholders'],
  'appearance.insertTitleToken': ['插入佔位符', 'Insert placeholder'],
  'appearance.titleSample': ['範例實例', 'Example instance'],
  'activity.heading': ['操作紀錄', 'Activity'],
  'activity.all': ['全部紀錄', 'All Activity'],
  'activity.core': ['近期核心更新', 'Recent core updates'],
  'activity.reload': ['重新讀取', 'Reload'],
  'activity.loading': ['讀取中', 'Loading'],
  'activity.loaded': ['已讀取', 'Loaded'],
  'activity.result': ['結果', 'Result'],
  'activity.allResults': ['全部', 'All'],
  'activity.older': ['顯示 90 天前的已結案紀錄', 'Show closed jobs older than 90 days'],
  'activity.exportFormat': ['匯出格式', 'Export format'],
  'activity.export': ['匯出目前結果', 'Export results'],
  'activity.exportRequested': ['已送出匯出請求', 'Export requested'],
  'activity.empty': ['目前沒有符合條件的操作紀錄。', 'No activity matches these filters.'],
  'activity.missingTime': ['時間未提供', 'Time unavailable'],
  'activity.cancelRemaining': ['取消後續操作', 'Cancel remaining work'],
  'activity.checkDiagnostics': ['請核對診斷紀錄', 'Review the diagnostics log'],
  'velocity.listenAddress': ['Velocity 監聽 IPv4 位址', 'Velocity IPv4 listen address'],
  'velocity.listenAddressOptional': ['Velocity 監聽 IPv4 位址（選填）', 'Velocity IPv4 listen address (optional)'],
  'velocity.listenAddressRequired': ['此代理使用對外位址。請輸入 velocity.toml 的 bind 位址以確認。', 'This proxy listens on an external address. Enter the bind address from velocity.toml to confirm it.'],
  'velocity.listenAddressChanged': ['確認的位址與 velocity.toml 的 bind 不符。請核對位址後重新預覽。', 'The confirmed address differs from the bind address in velocity.toml. Check the address and preview again.'],
  'velocity.requiresSpecificAddress': ['請在 velocity.toml 指定單一 IPv4 位址。不能使用 0.0.0.0。', 'Specify one IPv4 address in velocity.toml. The address cannot be 0.0.0.0.'],
  'velocity.listenAddressNotApplicable': ['只有 Velocity 對外匯入需要填寫此位址。請清空欄位後重新核對。', 'This address applies to external Velocity imports. Clear the field and check again.'],
  'activity.worldCheck': ['核對世界工作結果', 'Verify world job result'],
  'activity.worldChecking': ['正在核對世界工作', 'Verifying world job'],
  'activity.worldCheckFailed': ['無法確認這筆世界工作。請保留工作 ID 並重新核對。', 'Could not verify this world job. Keep its job ID and check again.'],
  'activity.worldSaved': ['已核對世界存檔', 'World save verified'],
  'activity.worldNoChange': ['已核對，未變更世界', 'Verified. No world change was needed.'],
  'activity.state.Completed': ['已完成', 'Completed'],
  'activity.state.CompletedWithIssues': ['已完成，有項目待處理', 'Completed with issues'],
  'activity.state.Running': ['處理中', 'In progress'],
  'activity.state.Sending': ['正在送出', 'Submitting'],
  'activity.state.AwaitingNormalStop': ['已讀回，保存待驗證', 'Read back, save unverified'],
  'activity.state.VerifiedLive': ['已核對生效', 'Verified in game'],
  'activity.state.Unconfirmed': ['結果未確認', 'Outcome unconfirmed'],
  'activity.state.Failed': ['失敗', 'Failed'],
  'activity.state.Pending': ['處理中', 'In progress'],
  'activity.state.Queued': ['排隊中', 'Queued'],
  'activity.state.Dispatching': ['正在派工', 'Dispatching'],
  'activity.state.Preparing': ['準備核心', 'Preparing core'],
  'activity.state.Starting': ['啟動中', 'Starting'],
  'activity.state.Cancelled': ['已取消', 'Canceled'],
  'activity.state.Skipped': ['已略過', 'Skipped'],
  'activity.state.StopQueued': ['等待停止派工', 'Stop queued'],
  'activity.state.StopDispatching': ['正在派出停止', 'Dispatching stop'],
  'activity.state.Stopping': ['等待正常退出', 'Waiting for graceful exit'],
  'activity.state.StopUnconfirmed': ['停止結果未確認', 'Stop unconfirmed'],
  'activity.action.create': ['建立', 'Create'],
  'activity.action.import': ['匯入', 'Import'],
  'activity.action.import-batch': ['批次匯入', 'Batch import'],
  'management.add': ['新增管理資料夾', 'Add managed folder'],
  'management.paths': ['管理資料夾', 'Managed folders'],
  'management.browse': ['瀏覽', 'Browse'],
  'management.allow': ['納入資料夾', 'Allowed folders'],
  'management.exclude': ['排除資料夾', 'Excluded folders'],
  'management.allowRow': ['納入資料夾 {index}', 'Allowed folder {index}'],
  'management.excludeRow': ['排除資料夾 {index}', 'Excluded folder {index}'],
  'management.addPath': ['新增路徑', 'Add path'],
  'management.removePath': ['刪除路徑', 'Remove path'],
  'management.removePathDetail': ['這列會從草稿移除。儲存時才會變更管理範圍。', 'This removes the row from the draft. The managed scope changes when you save.'],
  'management.emptyPath': ['空白路徑', 'Empty path'],
  'management.pickFolder': ['選擇資料夾', 'Choose folder'],
  'management.pickFile': ['選擇檔案', 'Choose file'],
  'management.removeRow': ['刪除第 {index} 列路徑', 'Remove path {index}'],
  'management.pickFailed': ['無法開啟資料夾選取器，請再試一次。', 'Could not open the folder picker. Try again.'],
  'management.selectedFolder': ['選取的資料夾', 'Selected folder'],
  'management.changeFolder': ['更換資料夾', 'Choose another folder'],
  'management.preview': ['掃描預覽', 'Scan preview'],
  'management.summary': ['可登記 {selected} 台 · 排除 {excluded} 台 · 待核對 {blocked} 台', '{selected} to register · {excluded} to exclude · {blocked} to review'],
  'management.blocked': ['待核對', 'Review required'],
  'management.alreadyManaged': ['已在管理中', 'Already managed'],
  'management.noCandidates': ['沒有可顯示的伺服器。仍可儲存資料夾供後續掃描。', 'No servers were found. You can still save the folder for future scans.'],
  'management.rootLocked': ['選取的資料夾本身就是這台伺服器。若不想登記它，請取消或更換資料夾。', 'The selected folder is this server. Cancel or select a different folder to leave it out.'],
  'management.saveDone': ['已儲存管理資料夾', 'Managed folder saved'],
  'management.result': ['登記結果', 'Registration results'],
  'management.resultCounts': ['已登記 {registered} 台 · 待核對 {pending} 台', '{registered} registered · {pending} to review'],
  'management.registered': ['已登記', 'Registered'],
  'management.registrationPending': ['管理資料夾已儲存，部分登記尚未核對。請在實例清單重新核對。', 'The managed folder was saved. Some registrations remain unconfirmed. Refresh the instance list to check.'],
  'management.previewFailed': ['無法預覽這個資料夾。請核對路徑後重新讀取。', 'Could not preview the folder. Check the path and reload the preview.'],
  'management.saveFailed': ['無法確認儲存結果。請在設定中核對管理資料夾，再重新預覽。', 'Could not confirm the save. Check Managed Folders in Settings before previewing again.'],
  'management.reload': ['重新讀取', 'Reload'],
  'management.scanning': ['正在讀取候選伺服器', 'Reading server candidates'],
  'management.scanBusy': ['掃描中', 'Scanning'],
  'management.saving': ['正在儲存並核對登記', 'Saving and checking registrations'],
  'management.pathRequired': ['請輸入資料夾路徑。', 'Enter a folder path.'],
  'management.pathInvalid': ['找不到可用的資料夾。請核對路徑與存取權限。', 'Could not find a usable folder. Check the path and access permissions.'],
  'management.pathDuplicate': ['這個資料夾已在同一份名單中。', 'This folder is already in this list.'],
  'management.excludeOutside': ['排除資料夾須為某個納入資料夾的子資料夾。', 'The excluded path must be a subfolder of an allowed folder.'],
  'management.javaAutomatic': ['啟動時自動選擇', 'Choose automatically at startup'],
  'management.javaInherited': ['使用全域 Java 設定', 'Use global Java settings'],
  'management.rediscoverJava': ['重新探索', 'Scan again'],
  'management.discoveringJava': ['探索中', 'Scanning'],
  'management.chooseInstalledJava': ['從已安裝的 Java 選擇', 'Choose installed Java'],
  'management.javaScanFolderUnavailable': ['這個已儲存的資料夾目前無法讀取。', 'This saved folder is currently unavailable.'],
  'management.javaSearchScope': ['Java 搜尋範圍', 'Java search scope'],
  'management.javaSearchHelp': ['最多往下搜尋六層子資料夾。', 'Searches up to six levels of subfolders.'],
  'management.javaSearchResults': ['查看搜尋結果', 'View search results'],
  'management.javaPickerTitle': ['選擇 Java 執行檔', 'Choose a Java executable'],
  'management.javaResultsTitle': ['Java 搜尋結果', 'Java search results'],
  'management.javaPickerUse': ['使用此版本', 'Use this version'],
  'management.javaPickerEmpty': ['未找到可選的 Java 安裝。可返回設定頁輸入完整路徑。', 'No Java installation found. Enter the full path in Settings.'],
  'management.javaResultsEmpty': ['未找到 Java 安裝。', 'No Java installation found.'],
  'management.javaPickerPreview': ['目前顯示搜尋範圍草稿的結果。', 'These results use the draft search scope.'],
  'management.javaSearchSources': ['預設來源', 'Default sources'],
  'management.javaSourceWindows': ['Windows 常見安裝位置', 'Standard Windows locations'],
  'management.javaSourceMise': ['mise', 'mise'],
  'management.javaSourceHome': ['JAVA_HOME', 'JAVA_HOME'],
  'management.javaSourcePath': ['PATH', 'PATH'],
  'management.javaSearchDirectories': ['搜尋資料夾', 'Search folders'],
  'management.javaSearchDirectoryRow': ['第 {index} 個 Java 搜尋資料夾', 'Java search folder {index}'],
  'management.javaSearchRemoveDetail': ['這列會從草稿移除。儲存後才會變更 Java 搜尋範圍。', 'This removes the row from the draft. The Java search scope changes when you save.'],
  'management.javaSearchPathLimit': ['最多 {count} 個搜尋資料夾。', 'Up to {count} search folders.'],
  'management.javaSearchDefaults': ['恢復預設搜尋範圍', 'Restore default search scope'],
  'management.javaSearchReviewRows': ['請修正標示的路徑後再儲存。', 'Correct the highlighted paths before saving.'],
  'management.javaSearchPathInvalid': ['請輸入已存在的本機完整資料夾路徑。', 'Enter the full path of an existing local folder.'],
  'management.javaSearchConflict': ['搜尋規則已有新版本。草稿仍保留，請先複製要保留的路徑，再重新讀取。', 'The search rules changed elsewhere. Your draft is preserved. Copy any paths you need before reloading.'],
  'management.javaSearchDiscardReload': ['捨棄草稿並重新讀取', 'Discard draft and reload'],
  'management.javaCustomSource': ['額外搜尋資料夾', 'Extra search folder'],
  'management.globalJava': ['全域 Java', 'Global Java'],
  'management.javaSelection': ['Java 選用方式', 'Java selection'],
  'management.javaAuto': ['自動選擇相容版本', 'Automatically select a compatible version'],
  'management.javaManual': ['手動指定 Java', 'Specify Java manually'],
  'management.javaManualPathRequired': ['請選擇手動模式要使用的 Java 執行檔，或切換成自動。', 'Choose a Java executable for Manual mode, or switch to Automatic.'],
  'management.scopeConfirm': ['確認管理範圍變更', 'Confirm managed scope changes'],
  'management.scopeNoChanges': ['這次規則變更沒有影響既有實例，也沒有新增候選。', 'These rule changes have no effect on existing instances and found no new candidates.'],
  'management.scopeAffected': ['已登記的實例', 'Registered instances'],
  'management.scopeIncluded': ['列入管理', 'Included'],
  'management.scopeExcluded': ['移出範圍', 'Out of scope'],
  'management.scopeActive': ['仍可在實例清單正常停止', 'Graceful stop remains available in the instance list'],
  'management.scopeCandidates': ['掃描到的伺服器', 'Servers found'],
  'management.scopeRegister': ['儲存時登記', 'Register on save'],
  'management.scopeConflict': ['掃描候選有衝突。請調整管理範圍或以排除資料夾排除其中一個資料夾。', 'Server candidates conflict. Adjust the managed scope or exclude one of the folders.'],
  'batchImport.title': ['批次匯入', 'Batch import'],
  'batchImport.content': ['批次匯入內容', 'Batch import contents'],
  'batchImport.directory': ['掃描目錄', 'Scan directory'],
  'batchImport.java': ['共用 Java 執行檔', 'Shared Java executable'],
  'batchImport.preview': ['候選預覽', 'Candidate preview'],
  'batchImport.selection': ['已選 {count}／{total} 台，每批最多 {max} 台', '{count} of {total} selected. Up to {max} per batch.'],
  'batchImport.incomplete': ['掃描結果不完整。請縮小範圍或檢查目錄存取權後重新核對。', 'The scan is incomplete. Narrow the scope or check folder access, then preview again.'],
  'batchImport.stale': ['輸入已變更。請重新預覽，再勾選要匯入的實例。', 'Inputs changed. Preview again, then select the instances to import.'],
  'batchImport.noCandidates': ['這個範圍沒有候選伺服器。', 'No candidate servers were found in this scope.'],
  'batchImport.Ready': ['可匯入', 'Ready to import'],
  'batchImport.Blocked': ['不可匯入', 'Import blocked'],
  'batchImport.Minecraft': ['Minecraft', 'Minecraft'],
  'batchImport.Velocity': ['Velocity', 'Velocity'],
  'batchImport.Unknown': ['核心未辨識', 'Core unidentified'],
  'batchImport.port': ['Port {port}', 'Port {port}'],
  'batchImport.rowSettings': ['調整此台登記', 'Edit this registration'],
  'batchImport.name': ['管理名稱', 'Registration name'],
  'batchImport.rowJava': ['此台 Java 執行檔', 'Java executable for this server'],
  'batchImport.listenAddress': ['確認 Velocity 監聽位址', 'Confirm Velocity listen address'],
  'batchImport.hasConflicts': ['與 {names} 衝突，請擇一勾選。', 'Conflicts with {names}. Select one of the conflicting candidates.'],
  'batchImport.conflictsSelected': ['與已選取的 {name} 衝突。', 'Conflicts with the selected candidate {name}.'],
  'batchImport.close': ['關閉', 'Close'],
  'batchImport.check': ['預覽候選', 'Preview candidates'],
  'batchImport.confirm': ['匯入所選', 'Import selected'],
  'batchImport.again': ['回到預覽', 'Back to preview'],
  'batchImport.previewFailed': ['無法預覽候選。請檢查掃描目錄和管理服務連線。', 'Could not preview candidates. Check the scan directory and management connection.'],
  'batchImport.lookupFailed': ['無法查詢這筆匯入。請確認工作 ID 和管理服務連線。', 'Could not look up this import. Check the work ID and management connection.'],
  'batchImport.result': ['逐台結果', 'Results by instance'],
  'batchImport.workId': ['工作 ID', 'Work ID'],
  'batchImport.resultCounts': ['已匯入 {imported} 台，已拒絕 {rejected} 台，待重新確認 {pending} 台', '{imported} imported, {rejected} rejected, {pending} need confirmation'],
  'batchImport.interrupted': ['作業曾中斷。待確認的項目須重新預覽並勾選。', 'The job was interrupted. Preview and select entries that still need confirmation.'],
  'batchImport.Imported': ['已匯入', 'Imported'],
  'batchImport.Rejected': ['已拒絕', 'Rejected'],
  'batchImport.NeedsConfirmation': ['待重新確認', 'Needs confirmation'],
  'batchImport.lookup': ['查詢匯入紀錄', 'Look up an import'],
  'batchImport.lookupButton': ['查詢', 'Look up'],
  'batchImport.pending.preview': ['正在讀取候選', 'Reading candidates'],
  'batchImport.pending.apply': ['正在逐台登記並核對結果', 'Registering instances and checking the results'],
  'batchImport.pending.status': ['正在讀取匯入紀錄', 'Reading the import record'],
  'batchImport.reason.unknown': ['證據不足，請核對來源與診斷紀錄。', 'Evidence is incomplete. Check the source and diagnostic log.'],
  'batchImport.reason.BatchImportEvidenceUnavailable': ['無法取得足夠的來源或執行狀態證據。', 'Source or runtime evidence is unavailable.'],
  'batchImport.reason.BatchImportAmbiguousCore': ['目錄同時有後端與 Velocity 設定，無法判斷匯入類型。', 'The folder has both backend and Velocity configuration, so its import type cannot be confirmed.'],
  'batchImport.reason.BatchImportConfigUnavailable': ['找不到可讀取的伺服器設定檔。', 'No readable server configuration file was found.'],
  'batchImport.reason.BatchImportPreviewChanged': ['來源或登記條件已變更。請重新預覽再確認。', 'The source or registration conditions changed. Preview again before confirming.'],
  'batchImport.reason.BatchImportSelectionConflict': ['所選項目有衝突。請重新預覽並調整勾選。', 'Selected candidates conflict. Preview again and adjust the selection.'],
  'batchImport.reason.BatchImportResultUnconfirmed': ['匯入結果未確認。請用上方工作 ID 查詢原匯入紀錄。', 'The import result is unconfirmed. Look up the original import using the work ID above.'],
  'batchImport.reason.BatchImportDirectoryOutsideScope': ['只能在工作區 instances 或其下的目錄掃描。', 'Scan within the workspace’s instances directory.'],
  'batchImport.reason.BatchImportJavaRequired': ['請填入共用或此台的 Java 執行檔。', 'Enter the shared Java executable or the one for this server.'],
  'batchImport.reason.BatchImportPreviewIncomplete': ['請縮小掃描範圍後再核對。', 'Narrow the scan scope and preview again.'],
  'batchImport.reason.InvalidJavaExecutable': ['無法核對 Java 執行檔，請檢查路徑。', 'Could not verify the Java executable. Check its path.'],
  'batchImport.reason.BatchImportNotFound': ['找不到此工作 ID 的匯入紀錄。', 'No import record matches this work ID.'],
  'batchImport.reason.RequestConflict': ['工作 ID 已用於不同內容。請使用原工作 ID 查詢紀錄，或重新預覽。', 'This work ID belongs to different inputs. Look up the original record or preview again.'],
  'batchImport.reason.ManagerBusy': ['管理服務正在處理其他工作。請查詢結果，並在確認後重新預覽。', 'The management service is busy. Look up the result before starting another preview.'],
  'batchImport.reason.InvalidInstanceName': ['管理名稱請使用小寫英文字母、數字及連字號，並以英文字母或數字開頭。', 'Start the registration name with a lowercase letter or digit. Use lowercase letters, digits, and hyphens.'],
  'batchImport.reason.AlreadyManaged': ['已有同名實例登記。', 'An instance with this name is already registered.'],
  'batchImport.reason.InstanceMustBeStopped': ['請先正常停止這台伺服器。', 'Stop this server gracefully before importing.'],
  'batchImport.reason.ImportPathAlreadyRegistered': ['這個目錄與既有登記的目錄重疊。', 'This folder overlaps a registered folder.'],
  'batchImport.reason.ImportRegistrationPathChanged': ['這個目錄與保留的登記位置不同。', 'This folder differs from the location in the retained registration.'],
  'batchImport.reason.ImportOutsideInstanceRoot': ['伺服器目錄超出工作區 instances 範圍。', 'The server folder is outside the workspace’s instances directory.'],
  'batchImport.reason.InstanceExcludedOrInvalid': ['目前掃描 policy 排除此實例，或無法核對目錄。', 'Scan policy excludes this instance or its directory could not be verified.'],
  'batchImport.reason.MissingCoreArtifact': ['找不到 server.jar。', 'server.jar is missing.'],
  'batchImport.reason.UnverifiedVelocityCore': ['無法核對 Velocity 核心。', 'Could not verify the Velocity core.'],
  'batchImport.reason.InvalidVelocityBind': ['無法核對 Velocity 監聽設定。', 'Could not verify the Velocity listener configuration.'],
  'batchImport.reason.VelocityListenAddressConfirmationRequired': ['請在此台登記中確認 Velocity 的監聽位址。', 'Confirm the Velocity listen address in this registration.'],
  'batchImport.reason.VelocityListenAddressChanged': ['輸入位址與 Velocity 設定的監聽位址不一致。', 'The entered address differs from the Velocity listen address.'],
  'batchImport.reason.VelocityRequiresSpecificAddress': ['Velocity 須使用明確的 IPv4 監聽位址。', 'Velocity must listen on a specific IPv4 address.'],
  'batchImport.reason.VelocityListenAddressNotApplicable': ['只有 Velocity 可確認外部監聽位址。', 'Only Velocity supports confirming a non-loopback listen address.'],
  'batchImport.reason.ExpectedLoopbackServerAddress': ['後端伺服器須監聽 127.0.0.1。', 'The backend server must listen on 127.0.0.1.'],
  'batchImport.reason.InvalidServerPort': ['伺服器設定的 Port 無效。', 'The configured server port is invalid.'],
  'batchImport.reason.PortAlreadyRegistered': ['已有其他實例登記此 Port。', 'Another instance already has this port registered.'],
  'batchImport.reason.UnverifiedManagementCore': ['無法核對伺服器核心或其 Java 版本需求。', 'The server core or its Java requirements could not be verified.'],
  'batchImport.reason.ManagementRegistrationRemoved': ['這個資料夾的管理登記先前已移除。', 'Management was previously removed for this folder.'],
  'batchImport.reason.ImportManagementUnconfirmed': ['找到舊管理標記或無法確認伺服器目錄已停止使用。', 'A legacy management marker was found or the server folder could not be confirmed idle.'],
  'batchImport.reason.ImportTreeTooLarge': ['伺服器目錄超出可核對上限。', 'The server folder exceeds the verification limit.'],
  'batchImport.reason.DirectoryRedirected': ['目錄或檔案經由連結重新導向。', 'A folder or file is redirected through a link.'],
  'batchImport.reason.PathRedirected': ['目錄或檔案經由連結重新導向。', 'A folder or file is redirected through a link.'],
  'activity.action.start': ['啟動', 'Start'],
  'activity.action.batch-start': ['批次啟動', 'Batch start'],
  'activity.action.batch-stop': ['批次正常停止', 'Batch graceful stop'],
  'activity.action.batch-restart': ['批次重新啟動', 'Batch restart'],
  'activity.action.start-if-current': ['啟動', 'Start'],
  'activity.action.stop': ['正常停止', 'Graceful stop'],
  'activity.action.restart': ['重新啟動', 'Restart'],
  'activity.action.force-stop': ['強制停止', 'Force stop'],
  'activity.action.core-install': ['更新核心', 'Update core'],
  'activity.action.core-restore': ['還原核心', 'Restore core'],
  'activity.action.core-reconcile': ['核對核心維護', 'Reconcile core maintenance'],
  'activity.action.core-batch-install': ['批次核心更新', 'Batch core update'],
  'activity.action.world-live-rule': ['套用遊戲規則', 'Apply game rule'],
  'core.source.spigotRefsUnverified': ['已查到同版本的發布資訊。請從備份與診斷核對本機核心的來源。', 'Release information is available. Verify the installed JAR under Backups and Diagnostics.'],
  'core.source.spigotNativeFailed': ['原生版本核對暫時失敗。請檢查連線，再重新檢查更新。', 'The native version check failed. Check your connection, then check for updates again.'],
  'core.source.nativeBehind': ['原生版本檢查顯示落後 {count} 個提交', 'Native version check reports {count} commits behind'],
  'core.source.updateAvailable': ['有更新可用', 'Update available'],
  'core.source.current': ['已是最新', 'Up to date'],
  'core.source.noStableBuild': ['沒有穩定版', 'No stable build'],
  'core.diagnostics.show': ['查看可複製診斷', 'View shareable diagnostics'],
  'core.diagnostics.copy': ['複製診斷', 'Copy diagnostics'],
  'core.diagnostics.copied': ['已複製診斷', 'Diagnostics copied'],
  'core.diagnostics.copyFailed': ['無法寫入剪貼簿。請選取上方內容後手動複製。', 'Could not access the clipboard. Select the text above to copy it manually.'],
  'core.batch.all': ['全部更新核心', 'Update all cores'],
  'core.batch.selected': ['更新所選核心', 'Update selected cores'],
  'core.batch.heading': ['批次核心更新', 'Batch core update'],
  'core.batch.scope': ['共 {count} 台實例。確認後依序更新已停止且有同版候選的實例。', '{count} instances in scope. After confirmation, stopped instances with an update for the same Minecraft version are updated in order.'],
  'core.batch.reviewChanged': ['實例或核心版本已變更，請重新檢查再確認。若選取範圍已變更，請關閉後重新開啟。', 'An instance or core version changed. Check again before confirming. If your selection changed, close and reopen the dialog.'],
  'core.batch.checkFailed': ['無法核對更新。請確認管理服務和更新來源可以連線，再重試。', 'Could not check for updates. Check the management service and update source connections, then retry.'],
  'core.batch.submitFailed': ['無法確認批次已送出。可重試同一批次，或到操作紀錄核對。', 'Could not confirm the batch submission. Retry the same batch or check Activity.'],
  'core.batch.cancelFailed': ['無法確認剩餘項目是否已取消。請檢查管理服務連線，再核對操作紀錄。', 'Could not confirm cancellation. Check the management service connection and review Activity.'],
  'core.batch.limit': ['一次最多核對 {count} 台，請縮小選取範圍。', 'A batch can check up to {count} instances. Select fewer instances.'],
  'core.batch.noTargets': ['沒有可以核對的實例。若清單已變更，請關閉後重新開啟。', 'There are no instances to check. If the list changed, close and reopen the dialog.'],
  'core.batch.noCandidate': ['沒有可安裝的同版核心', 'No installable core for the same Minecraft version'],
  'core.batch.notStopped': ['已略過，請先正常停止', 'Skipped. Stop the server gracefully first.'],
  'core.batch.operationPending': ['已略過，請等候目前的實例操作完成', 'Skipped. Wait for the current instance operation to finish.'],
  'core.batch.updateTo': ['可更新至 Build {build}', 'Can update to build {build}'],
  'core.batch.check': ['檢查更新', 'Check for updates'],
  'core.batch.checking': ['正在核對更新', 'Checking for updates'],
  'core.batch.confirm': ['確認更新', 'Confirm update'],
  'core.batch.retrySubmit': ['重試送出', 'Retry submission'],
  'core.batch.submitting': ['正在送出批次', 'Submitting batch'],
  'core.batch.cancel': ['取消剩餘', 'Cancel remaining'],
  'core.batch.cancelling': ['正在取消剩餘項目', 'Canceling remaining items'],
  'core.batch.cancelHelp': ['可取消尚未開始替換的項目。已開始替換的項目會繼續核對結果。', 'Items can be canceled until replacement starts. Any replacement already in progress continues through verification.'],
  'core.batch.progress': ['已處理 {finished}／{total} 台', 'Processed {finished} of {total}'],
  'core.batch.results': ['逐台結果', 'Results by instance'],
  'core.batch.Queued': ['排隊中', 'Queued'],
  'core.batch.Preparing': ['準備核心', 'Preparing core'],
  'core.batch.Sending': ['正在替換核心', 'Replacing core'],
  'core.batch.Completed': ['已更新', 'Updated'],
  'core.batch.Failed': ['更新失敗', 'Update failed'],
  'core.batch.Unconfirmed': ['結果未確認，請至核心維護核對', 'Outcome unconfirmed. Check Core Maintenance.'],
  'core.batch.Cancelled': ['已取消', 'Canceled'],
  'core.batch.close': ['關閉', 'Close'],
  'activity.action.world-first-ready': ['套用首次啟動規則', 'Apply first start rules'],
  'activity.action.world-live-time': ['設定世界時間', 'Set world time'],
  'activity.action.remove-management': ['移除管理', 'Remove from management'],
  'activity.action.adopt': ['重新接入', 'Readopt'],
  'activity.reason.RunUnconfirmed': ['工作階段無法核對', 'Could not verify the session'],
  'activity.reason.DispatchUnconfirmed': ['派工結果未確認', 'Dispatch result unconfirmed'],
  'activity.reason.HostUnconfirmed': ['宿主身分未確認', 'Host identity unconfirmed'],
  'activity.reason.HostProbeTimeout': ['宿主核對逾時', 'Host verification timed out'],
  'activity.reason.HostPipeUnavailable': ['宿主控制通道無法連線', 'Host control pipe unavailable'],
  'activity.reason.ReadinessUnconfirmed': ['就緒逾時', 'Readiness timed out'],
  'activity.reason.StartIntentChanged': ['啟動條件已變更', 'Start conditions changed'],
  'activity.reason.StartRejected': ['啟動要求被拒絕', 'Start request rejected'],
  'activity.reason.AutoJavaUnavailable': ['找不到相容的 Java，請在實例設定指定已安裝的版本。', 'No compatible Java was found. Select an installed version in the instance settings.'],
  'activity.reason.ExitedBeforeReady': ['就緒前已退出', 'Exited before becoming ready'],
  'activity.reason.StopTimeout': ['停止逾時，請核對目前狀態', 'Stop timed out. Check the current status.'],
  'activity.reason.StopResultUnconfirmed': ['未核對到本次正常停止', 'Could not verify graceful stop'],
  'activity.reason.StopIntentUnconfirmed': ['無法核對停止意圖', 'Could not verify the stop request'],
  'activity.reason.StopIntentChanged': ['停止意圖或原工作階段已變更', 'The stop request or original session changed'],
  'activity.reason.RegistrationChanged': ['實例登記已變更', 'Instance registration changed'],
  'activity.reason.PortOccupied': ['Port 被其他程序占用', 'Another process is using the port'],
  'activity.reason.PortOccupiedOrUnavailable': ['Port 尚未釋放', 'The port is still unavailable'],
  'activity.reason.SupersededByStop': ['已由較新的停止要求取代', 'Replaced by a later stop request'],
  'activity.reason.RunMismatch': ['宿主無法核對停止要求', 'The host could not verify the stop request'],
  'activity.reason.ManagerBusy': ['實例有其他作業，請重新核對', 'Another operation is active. Refresh the instance status.'],
  'activity.reason.BatchTargetPending': ['實例已有派工中的操作', 'The instance already has a pending job'],
  'activity.reason.CoreSourceMissingContact': ['更新來源需要聯絡網址，請核對啟動設定', 'The update source requires a contact URL. Check the launcher configuration.'],
  'activity.reason.CoreSourceInvalidContact': ['更新來源聯絡網址無效，請核對啟動設定', 'The update source’s contact URL is invalid. Check the launcher configuration.'],
  'activity.reason.CoreArtifactChanged': ['核心版本已變更，請重新檢查更新', 'The core changed. Check for updates again.'],
  'activity.reason.CoreIdentityChanged': ['核心身分已變更，請重新檢查更新', 'The core identity changed. Check for updates again.'],
  'activity.reason.CoreTargetUnavailable': ['找不到已登記實例，請重新核對實例清單', 'The registered instance could not be found. Refresh the instance list.'],
  'activity.reason.InstanceNotConfirmedStopped': ['實例尚未確認停止，請重新核對狀態', 'The instance has not been confirmed stopped. Refresh its status.'],
  'activity.reason.CoreCandidateUnverified': ['無法核對候選核心，請重新檢查更新', 'The update candidate could not be verified. Check for updates again.'],
  'console.scrollback': ['終端輸出捲動', 'Terminal scrollback'],
  'console.tabs': ['伺服器後台', 'Server consoles'],
  'console.closeTab': ['關閉 {name} 的後台分頁', 'Close the console tab for {name}'],
  'console.closeFailed': ['無法關閉這個後台分頁，請再試一次。', 'Could not close this console tab. Try again.'],
  'console.loadFailed': ['無法載入終端元件。', 'Could not load the terminal.'],
  'console.openFailed': ['無法開啟後台分頁，請從實例清單再試一次。', 'Could not open the console tab. Try again from Instances.'],
  'console.inputNotSent': ['輸入未送出，請縮短貼上的文字或稍後再試。', 'Input was not sent. Paste less text or try again shortly.'],
  'console.inputUnconfirmed': ['無法確認輸入結果，請核對終端輸出。', 'Could not confirm that input was received. Check the terminal output.'],
  'console.copyFailed': ['無法複製到剪貼簿，請再試一次。', 'Could not copy to the clipboard. Try again.'],
  'console.pasteFailed': ['無法讀取剪貼簿，請再試一次。', 'Could not read the clipboard. Try again.'],
  'console.pasteInvalid': ['剪貼簿文字過長或含有控制字元，請修改後再貼上。', 'The text is too long or contains control characters. Edit it before pasting.'],
  'console.pasteTitle': ['確認貼上', 'Confirm paste'],
  'console.pasteDescription': ['剪貼簿文字含有換行，貼上可能執行命令。要貼到 {name} 的後台嗎？', 'The clipboard text contains line breaks and may execute commands. Paste it into the console for {name}?'],
  'console.pasteCancel': ['取消', 'Cancel'],
  'console.pasteConfirm': ['仍要貼上', 'Paste anyway'],
  'console.disconnected': ['終端連線中斷，正在等待原工作階段。', 'Terminal disconnected. Waiting for the original session.'],
  'recovery.heading': ['復原金鑰', 'Recovery key'],
  'recovery.noConnection': ['連線後可管理復原金鑰。', 'Connect to the management service to manage the recovery key.'],
  'recovery.reading': ['正在核對金鑰', 'Checking the key'],
  'recovery.none': ['目前工作區尚未登記復原金鑰。', 'This workspace has no registered recovery key.'],
  'recovery.generation': ['金鑰世代', 'Key generation'],
  'recovery.publicKey': ['公開金鑰', 'Public key'],
  'recovery.details': ['檢視金鑰身分', 'View key identity'],
  'recovery.create': ['建立金鑰', 'Create key'],
  'recovery.rotate': ['輪替金鑰', 'Rotate key'],
  'recovery.createConfirm': ['為此工作區建立新的復原金鑰。', 'Create a recovery key for this workspace.'],
  'recovery.rotateConfirm': ['輪替會建立新一代金鑰。完成後請為新金鑰匯出備份。', 'Rotation creates a new key generation. Export a backup for the new key when it is ready.'],
  'recovery.created': ['已建立金鑰。', 'Key created.'],
  'recovery.rotated': ['已輪替金鑰。', 'Key rotated.'],
  'recovery.export': ['匯出加密備份', 'Export encrypted backup'],
  'recovery.inspect': ['檢查備份', 'Inspect backup'],
  'recovery.restore': ['確認復原', 'Confirm restore'],
  'recovery.backupPath': ['備份檔案位置', 'Backup file path'],
  'recovery.pathHelp': ['輸入副檔名為 .ihrecovery 的完整絕對路徑。', 'Enter the full path to a file ending in .ihrecovery.'],
  'recovery.password': ['備份密碼', 'Backup password'],
  'recovery.passwordHelp': ['請輸入至少 12 個字元。', 'Enter at least 12 characters.'],
  'recovery.exportHelp': ['復原需要同一個工作區路徑與此密碼。請分開保存備份與密碼。', 'Restore requires the same workspace path and this password. Keep them separately.'],
  'recovery.inspectHelp': ['先檢查備份的金鑰身分，再確認是否復原。', 'Inspect the key identity in the backup before deciding whether to restore it.'],
  'recovery.restoreHelp': ['請核對下方金鑰身分並重新輸入密碼。只有工作區未登記金鑰，或已有相同金鑰時才能確認復原。', 'Review the key identity below and enter the password again. You can restore to a workspace with no registered key or the same key.'],
  'recovery.exported': ['已匯出加密備份。', 'Encrypted backup exported.'],
  'recovery.restored': ['已確認復原金鑰。', 'Recovery key verified.'],
  'recovery.passwordInvalid': ['請確認密碼至少有 12 個字元，且未超過 1024 個 UTF 8 位元組。', 'Enter at least 12 characters and at most 1024 UTF 8 bytes.'],
  'recovery.authenticationFailed': ['無法驗證備份。請確認密碼和檔案是否正確。', 'Could not verify the backup. Check the password and file.'],
  'recovery.workspaceMismatch': ['備份屬於不同的工作區路徑。請使用目前工作區匯出的備份。', 'This backup belongs to a different workspace path. Use a backup exported for this workspace.'],
  'recovery.keyChanged': ['工作區金鑰已變更。請重新核對金鑰身分。', 'The workspace key changed. Check the key identity again.'],
  'recovery.otherKey': ['此工作區已登記不同的金鑰。請檢查相符的備份。', 'This workspace already has a different key. Inspect its matching backup.'],
  'recovery.backupChanged': ['備份金鑰身分已變更。請重新檢查備份。', 'The backup key identity changed. Inspect the backup again.'],
  'recovery.invalidPath': ['無法使用此路徑。請選擇現有本機資料夾，並核對檔案位置。', 'This path is unavailable. Choose an existing local folder and check the file path.'],
  'recovery.destinationExists': ['這個檔案已存在。請指定新的檔案名稱。', 'This file already exists. Choose a new filename.'],
  'recovery.readFailed': ['無法核對金鑰。請檢查工作區存取權限並重試。', 'Could not check the key. Check workspace permissions and try again.'],
  'recovery.operationFailed': ['無法完成金鑰操作。請核對檔案位置與存取權限後重試。', 'Could not complete the key operation. Check the file location and permissions before retrying.'],
  'recovery.working': ['正在處理金鑰', 'Processing the key'],
  'recovery.hostActive': ['仍有使用此金鑰的工作階段。請先正常停止相關實例，再輪替金鑰。', 'A running session still uses this key. Stop the affected instance normally before rotating the key.'],
  'recovery.targetsTitle': ['執行中工作階段', 'Running sessions'],
  'recovery.targetsEmpty': ['目前沒有符合條件的執行中工作階段。', 'No running session currently meets the recovery requirements.'],
  'recovery.targetsReadFailed': ['無法核對工作階段清單。請檢查管理服務連線，再手動重新核對。', 'Could not check the session list. Check the management connection, then refresh manually.'],
  'recovery.target': ['實例', 'Instance'],
  'recovery.chooseTarget': ['選擇執行中實例', 'Choose a running instance'],
  'recovery.runId': ['工作階段 ID', 'Run ID'],
  'recovery.refreshTargets': ['重新核對工作階段', 'Refresh sessions'],
  'recovery.readingTargets': ['正在核對工作階段', 'Checking sessions'],
  'recovery.openRecover': ['接管工作階段', 'Take over session'],
  'recovery.confirmTitle': ['確認接管工作階段', 'Confirm session takeover'],
  'recovery.confirmHelp': ['將核對同一個執行中工作階段與管理程序身分，成功後由目前的管理程序接手。這不會重啟伺服器。', 'The app will verify this running session and the manager identity before takeover. The server will not restart.'],
  'recovery.confirmRecover': ['確認接管', 'Confirm takeover'],
  'recovery.recovering': ['正在核對接管', 'Checking takeover'],
  'recovery.recovered': ['已核對接管同一個工作階段。', 'Takeover of the same session was verified.'],
  'recovery.managerActive': ['原管理程序仍在執行，無需接管。請核對目前工作階段。', 'The original manager is still running. Check the current session before taking further action.'],
  'recovery.challengeExpired': ['這次接管驗證已逾時。請重新核對工作階段，再決定是否重新發起。', 'The takeover challenge expired. Refresh the session before deciding whether to start a new attempt.'],
  'recovery.runChanged': ['工作階段已變更或無法確認。請重新核對實例與工作階段 ID。', 'The session changed or could not be verified. Refresh the instance and run ID.'],
  'recovery.candidateUnconfirmed': ['無法確認目前管理程序的身分。請核對管理服務及程式檔案。', 'The current manager identity could not be verified. Check the management service and executable.'],
  'recovery.pending': ['已有一筆接管驗證正在進行。請先核對工作階段，不要重複送出。', 'A takeover challenge is already pending. Check the session before sending another request.'],
  'recovery.resultUnconfirmed': ['無法確認接管結果。請先核對實例與工作階段，不要直接重送。', 'The takeover result is unconfirmed. Check the instance and session before sending another request.'],
  'recovery.recoverFailed': ['無法完成接管。請核對管理服務、工作階段與診斷紀錄。', 'Could not complete takeover. Check the management service, session, and diagnostic logs.'],
  'settings.category': ['設定分類', 'Settings categories'],
  'settings.content': ['設定內容', 'Settings content'],
  'settings.back': ['返回實例', 'Back to instances'],
  'settings.instanceTitle': ['{name} · 設定', '{name} · Settings'],
  'settings.java': ['Java 與 JVM', 'Java and JVM'],
  'settings.restart': ['停止與重啟', 'Stop and restart'],
  'settings.autoRestart': ['自動重啟', 'Automatic restart'],
  'settings.javaCheck': ['核對 Java 與 JVM 參數', 'Verify Java and JVM options'],
  'settings.javaCheckHelp': ['不會儲存設定。', 'Does not save the settings.'],
  'settings.javaChecking': ['正在核對 Java 與 JVM 參數', 'Verifying Java and JVM options'],
  'settings.javaVerified': ['Java 與 JVM 參數已通過核對', 'Java and JVM options verified'],
  'settings.javaRestoreField': ['恢復預設值（{field}）', 'Restore Default ({field})'],
  'settings.javaRestoreFieldHelp': ['將此欄位改為適用的預設值。儲存後生效。', 'Use the applicable default for this field. Save to apply it.'],
  'settings.javaUndoField': ['還原這次變更（{field}）', 'Undo Draft Change ({field})'],
  'settings.javaUndoFieldHelp': ['將此欄位還原為上次儲存的值。', 'Restore the last saved value for this field.'],
  'settings.restartAdvanced': ['使用進階重試策略', 'Use advanced retry policy'],
  'settings.restartDelay': ['等待時間（秒）', 'Delay (seconds)'],
  'settings.restartLimit': ['重試上限', 'Retry limit'],
  'settings.restartSchedule': ['各次重試等待（秒）', 'Delay per attempt (seconds)'],
  'settings.restartScheduleHelp': ['以半形逗號分隔，例如 2, 5, 10。每次 1 到 300 秒，數量須等於重試上限。', 'Separate delays with commas, for example 2, 5, 10. Use 1 to 300 seconds per attempt and match the retry limit.'],
  'settings.restartWindow': ['重試計數時間窗（秒）', 'Retry count window (seconds)'],
  'settings.restartStable': ['穩定運作判定（秒）', 'Stable runtime (seconds)'],
  'settings.restartInvalid': ['請核對等待時間與重試上限。時間窗與穩定運作須為 60 到 3600 秒。', 'Check the delays and retry limit. The retry window and stable runtime must be 60 to 3600 seconds.'],
  'settings.restartState.Disabled': ['自動重啟未啟用', 'Automatic restart is disabled'],
  'settings.restartState.Scheduled': ['已排程自動重啟', 'Automatic restart scheduled'],
  'settings.restartState.Watching': ['正在監看伺服器退出狀態', 'Watching for server exit'],
  'settings.restartState.Paused': ['自動重啟已暫停。請先核對實例，再手動啟動。', 'Automatic restart is paused. Check the instance, then start it manually.'],
  'settings.restartState.Suppressed': ['目前工作階段不會自動重啟', 'Automatic restart suppressed for this session'],
  'settings.restartState.Unconfirmed': ['自動重啟狀態未確認', 'Automatic restart status unconfirmed'],
  'settings.restartAttemptCount': ['目前時間窗已重試 {count} 次', '{count} retries in the current window'],
  'settings.restartDue': ['預計重試時間：{time}', 'Scheduled retry: {time}'],
  'settings.terminal': ['後台', 'Console'],
  'settings.world': ['世界設定', 'World settings'],
  'settings.core': ['核心維護', 'Server core maintenance'],
  'settings.initialization': ['初始化預設', 'Initialization defaults'],
  'settings.scan': ['管理資料夾', 'Managed folders'],
  'settings.saved': ['設定已儲存', 'Settings saved'],
  'settings.saveFailed': ['儲存失敗', 'Could not save settings'],
  'settings.unsaved': ['目前設定有尚未儲存的變更。', 'There are unsaved settings changes.'],
  'settings.restoreTitle': ['回到此頁上次儲存的設定', 'Restore the last saved settings on this page'],
  'action.cancel': ['取消', 'Cancel'],
  'action.close': ['關閉', 'Close'],
  'action.checking': ['核對中', 'Checking'],
  'action.checkingUpdates': ['檢查中', 'Checking'],
  'action.submitting': ['送出中', 'Submitting'],
  'action.confirm': ['確認', 'Confirm'],
  'action.discard': ['放棄變更', 'Discard changes'],
  'action.restore': ['還原變更', 'Restore changes'],
  'action.cancelChanges': ['取消變更', 'Cancel changes'],
  'action.save': ['儲存設定', 'Save settings'],
  'action.saveShort': ['儲存', 'Save'],
  'action.apply': ['套用', 'Apply'],
  'action.refresh': ['重新核對', 'Refresh'],
  'action.retryRead': ['重試讀取', 'Try reading again'],
  'action.working': ['處理中', 'Working'],
  'feedback.failed': ['無法完成操作', 'Could not complete the operation'],
  'instance.state.stopped': ['已停止', 'Stopped'],
  'instance.state.listening': ['正在監聽', 'Listening'],
  'instance.state.starting': ['啟動中', 'Starting'],
  'instance.state.portOccupied': ['Port 已被占用', 'Port in use'],
  'instance.state.runUnconfirmed': ['工作階段未確認', 'Session unconfirmed'],
} as const satisfies Record<string, readonly [string, string]>

const messageGroups = [baseMessages, appMessages, instancesMessages, settingsMessages, managerMessages, componentsMessages, policyMessages, consoleMessages] as const
const messages = {
  ...baseMessages,
  ...appMessages,
  ...instancesMessages,
  ...settingsMessages,
  ...managerMessages,
  ...componentsMessages,
  ...policyMessages,
  ...consoleMessages,
}

export function duplicateMessageIds(): string[] {
  const seen = new Set<string>()
  const duplicates: string[] = []
  for (const group of messageGroups) for (const id of Object.keys(group)) {
    if (seen.has(id)) duplicates.push(id)
    seen.add(id)
  }
  return duplicates
}

export type MessageId = keyof typeof messages
type MessageValues = Record<string, string | number>

export function resolveLanguage(
  preference: LanguagePreference | undefined,
  systemLanguages: readonly string[],
): Locale {
  if (preference !== 'system') return preference === 'en' ? 'en' : 'zh-TW'

  for (const candidate of systemLanguages) {
    let locale: Intl.Locale
    try { locale = new Intl.Locale(candidate) }
    catch { continue }
    if (locale.language === 'en') return 'en'
    if (locale.language === 'zh' && (
      locale.script === 'Hant'
      || (!locale.script && ['TW', 'HK', 'MO'].includes(locale.region ?? ''))
    )) return 'zh-TW'
  }
  return 'zh-TW'
}

export function translate(locale: Locale, id: MessageId, values: MessageValues = {}): string {
  const message = messages[id][locale === 'en' ? 1 : 0]
  return message.replace(/\{(\w+)\}/g, (token, key: string) =>
    Object.hasOwn(values, key) ? String(values[key]) : token)
}

export function isMessageId(id: string): id is MessageId {
  return Object.hasOwn(messages, id)
}

export function messageParameterMismatches(): MessageId[] {
  const parameters = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort().join(',')
  return (Object.keys(messages) as MessageId[]).filter(id => {
    const [chinese, english] = messages[id]
    return !chinese.trim() || !english.trim() || parameters(chinese) !== parameters(english)
  })
}

function systemLanguages() {
  if (typeof navigator === 'undefined') return []
  return navigator.languages?.length ? navigator.languages : [navigator.language]
}

function translator(locale: Readonly<{ value: Locale }>) {
  return {
    locale,
    t: (id: MessageId, values?: MessageValues) => translate(locale.value, id, values),
  }
}

// Hold a getter, not the computed itself: ref() returns a ref argument unchanged, so a computed stored
// here would become the read-only source and later assignments would be ignored.
const activeLocale = shallowRef<() => Locale>(() => 'zh-TW')

/** Translates outside components (for example manager.ts) with the app's current language. Reactive inside computed and templates. */
export function tr(id: MessageId, values?: MessageValues): string {
  return translate(activeLocale.value(), id, values)
}

const appLanguage: InjectionKey<ReturnType<typeof translator>> = Symbol('app-language')
const fallback = translator(computed((): Locale => 'zh-TW'))

export function provideAppLanguage(
  saved: () => LanguagePreference | undefined,
  preview: () => LanguagePreference | undefined,
) {
  const languages = ref(systemLanguages())
  const locale = computed(() => resolveLanguage(preview() ?? saved(), languages.value))
  activeLocale.value = () => locale.value
  provide(appLanguage, translator(locale))

  function refreshSystemLanguage() {
    const next = systemLanguages()
    if (next.join(',') !== languages.value.join(',')) languages.value = next
  }
  watch(() => preview() ?? saved(), preference => {
    if (preference === 'system') refreshSystemLanguage()
  }, { flush: 'sync' })
  watch(locale, current => {
    if (typeof document !== 'undefined') document.documentElement.lang = current
  }, { immediate: true, flush: 'sync' })
  onMounted(() => {
    refreshSystemLanguage()
    window.addEventListener('languagechange', refreshSystemLanguage)
    window.addEventListener('focus', refreshSystemLanguage)
  })
  onUnmounted(() => {
    window.removeEventListener('languagechange', refreshSystemLanguage)
    window.removeEventListener('focus', refreshSystemLanguage)
  })
  return translator(locale)
}

export function useAppLanguage() {
  return inject(appLanguage, fallback)
}
