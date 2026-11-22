# GitHub Issues 與 Pull Request

[Command1264/InstanceHarbor](https://github.com/Command1264/InstanceHarbor) 是 InstanceHarbor 的公開 GitHub 儲存庫。

## 工作項目

- 使用 [GitHub Issues](https://github.com/Command1264/InstanceHarbor/issues) 建立、查詢和更新工作項目。
- 建立前搜尋相關 Issue，沿用 [Triage 標籤](triage-labels.md)。Issue 記錄觀察到的行為、影響範圍、重現步驟與驗收條件。後續補充放在留言。
- 大型工作可用一張 Issue 記錄共通規格，以任務清單連結子 Issue。以 Issue 連結標示相依項，並在子 Issue 記錄結論。
- Issue 與 PR 都是公開內容，須遵守 [AGENTS.md](../../AGENTS.md) 的去識別化規則。原始日誌及本機驗收產物留在本機。
- 1.0.0-rc2 以前的 Issue 與 PR 編號屬於已移除的私人儲存庫，在本儲存庫中不存在。需要時向維護者查詢本機封存。

## 合併

- 一般開發從 `develop` 開任務分支，PR 目標明確指定為 `develop`。`main` 是穩定發行分支。
- PR 附相關 Issue、候選 commit、已完成的測試與待驗收項目。若 Issue 需在特定版本才結案，於該版本驗收後更新 Issue。
- 推送、開 PR 與執行合併依當前任務的授權辦理。使用者驗收須對應候選 commit 及目標分支。
