# Tasks

## 1. `create` 部署方式

- [x] 1.1 `deploy = "create"`：目標不存在才寫入（含解密與 `permissions`），不記錄、不監看、不進待處理清單；`rsmt validate` 拒絕搭配 template；`rsmt forget` 不動目標。以整合測試驗證建立、已存在不動、刪除後重建，README 的 `deploy` 欄位加上 `create`

## 2. Externals

- [ ] 2.1 `externals.toml` 的型別、`rsmt schema externals`、`rsmt validate`（格式、鎖定值、路徑互相包含與 `git` external 內的例外）；以測試驗證各種回報
- [ ] 2.2 git external：暫存目錄 fetch、checkout、改名，以及上表的各狀態（乾淨、修改、HEAD 移動、網址改變、已存在的 clone）；以本機 bare repo 的整合測試驗證
- [ ] 2.3 壓縮檔 external：下載上限與逾時、先驗 SHA-256、tar.gz／tar／zip 解壓與 `strip`、拒絕跳出目標的項目、替換與內容雜湊；以本機產生的壓縮檔與惡意項目測試驗證
- [ ] 2.4 接進套用：smoke test 之前處理、外層 `git` external 不在時略過內層與其中的 dotfile、結果與 `--dry-run`／`--json` 輸出、拉取時同樣處理且失敗不算拉取失敗；以整合測試驗證
- [ ] 2.5 `rsmt externals update [<path>...]`：更新鎖定值、commit、依推送偏好推送、再套用；以本機 bare repo 驗證有更新與沒有更新兩種情況，README 新增 externals 一節

## 3. Scripts

- [ ] 3.1 `scripts.toml` 的型別、`rsmt schema scripts`、`rsmt validate`；`host.toml` 的 `scripts` 偏好與 schema；以測試驗證
- [ ] 3.2 執行條件與執行方式：`once`／`onchange` 的狀態與雜湊、shebang、環境變數、工作目錄、紀錄檔（權限、截斷）；以整合測試驗證
- [ ] 3.3 接進套用：`before` 在 smoke test 之前、`after` 在 dotfile 之後；終端機直接執行，其他依偏好放進待處理清單或執行；`--dry-run`／`--json`；以整合測試驗證
- [ ] 3.4 待處理清單的腳本項目：列出、`--json`、`accept` 執行與雜湊檢查、`ignore`、拒絕 `discard`、自動移除；以 CLI 測試驗證
- [ ] 3.5 背景服務：依偏好處理、通知一次（中英文字串）；以背景服務測試驗證，README 新增 scripts 一節

## 4. App

- [ ] 4.1 待處理頁的腳本項目：標籤、詳細內容（腳本或紀錄）、執行與略過按鈕；設定頁的 `scripts` 偏好；以 gpui 測試與截圖驗證

## 5. 驗證

- [ ] 5.1 跑完 fmt、clippy、全部測試、8 個 Linux distro 與 `cargo deny`，更新 THIRD_PARTY_NOTICES、GLOSSARY，列出所有使用者看得到的文字供審閱

## Workflow follow-up

- 使用者審閱文字後，依 `add-dotfile-deploy` → `add-gui` → `add-templates-and-secrets` → 本 change 的順序封存。
