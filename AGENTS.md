# InstanceHarbor agent router

本 repository 是公開專案。2026-11-22 前的私人開發歷史只保留在維護者本機封存，不屬於本 repository。

## 硬性規則

- **公開內容去識別化**：commit、Issue、PR 與文件不得包含本機絕對路徑、使用者名稱、私人插件專案名稱、
  真人測服的 Port 或路由、玩家資料、帳密或測試管理員身分。從本機封存搬入內容前，先逐項移除上述資料。
- **測試資料邊界**：Agent 測試只使用隔離的 Agent 工作區（開發版為 `%LOCALAPPDATA%/InstanceHarbor Dev/workspaces/`）。
  不得接入、覆寫、重建或刪除 Human 測服、其世界與插件資料，也不得停止非本次測試啟動的伺服器、Host 或 Manager。
- 使用者於 2026-09-28 明確表示已接受 Minecraft EULA，並授權 Agent 在**本 InstanceHarbor 專案**建立或啟動
  Minecraft 測試實例時，自動使用 `-AcceptEula` 或寫入 `eula=true`。此授權不適用其他 repository，也不授權
  覆寫、重建或刪除既有實例資料。

## 按需文件

- 建置、測試、隔離驗收、系統匣與回退相容：讀 [v2/README.md](v2/README.md)。
- 打包、Setup、授權通知或發版：讀 [docs/windows-rc2.md](docs/windows-rc2.md) 與
  [docs/windows-distribution-licenses.md](docs/windows-distribution-licenses.md)。
- 使用專案用語或記錄架構決策：讀 [docs/agents/domain.md](docs/agents/domain.md)。
- 建立或處理 Issue、PR：讀 [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md) 與
  [docs/agents/triage-labels.md](docs/agents/triage-labels.md)。
