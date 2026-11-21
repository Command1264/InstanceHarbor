# InstanceHarbor 1.0.0-rc2 本機候選

Windows x64 ZIP 請完整解壓後執行 InstanceHarbor.exe。Setup 為目前使用者安裝並建立開始選單捷徑。
Windows 需有 WebView2 Runtime 與 [Microsoft Visual C++ v14 x64 Runtime](https://aka.ms/vc14/vc_redist.x64.exe)。Setup 會檢查 x64 Runtime 版本，便攜版的安裝說明在 `WINDOWS-GETTING-STARTED.txt`。啟動 Minecraft 實例需準備相容 Java。Setup 尚未簽章。

ZIP 與 Setup 共用本機正式工作區。設定與實例資料應先備份。回退前正常停服、關閉視窗並退出該工作區的 Manager。
Setup 解除安裝會保留工作區。便攜工具可從已登記的前版回退。

此公開候選使用合成路由資料。建服仍能驗證並準備 LuckPerms 核心插件；預設不替任何身分授予權限。
固定測試管理員的自動安裝與核對只保留在原私人歷史。請由使用者自行管理伺服器權限。
