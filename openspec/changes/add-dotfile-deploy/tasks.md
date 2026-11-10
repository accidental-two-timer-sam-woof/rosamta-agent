# Tasks

## 1. 宣告與驗證

- [x] 1.1 新增 `dotfiles.toml` 的型別與 `rsmt schema dotfiles`，`rsmt validate` 檢查路徑格式、來源存在、互相包含的路徑、搭配 link 的 `encrypted` 與權限格式
- [x] 1.2 實作選擇符合本機的宣告與「指名本機者優先」，以單元測試涵蓋重複宣告的情況

## 2. 部署

- [ ] 2.1 實作檔案與目錄的內容雜湊與 `deployed.json` 部署紀錄
- [ ] 2.2 實作 link 部署（新建、內容相同時取代、衝突判斷）與權限設定
- [ ] 2.3 實作 copy 部署（新建、目標未改時更新、本機修改的判斷）與權限設定
- [ ] 2.4 實作 link 被換成一般檔案時寫回 repo 並重建 symlink，以及 copy 目標被改時依接管偏好處理
- [ ] 2.5 實作不再宣告的路徑：copy 留在原地，link 換成一般檔案，來源已刪除時從 git 歷史取回
- [ ] 2.6 把 dotfile 部署接進 `rsmt apply`（在 generated script 切換之後），含 `--dry-run`、`--json`、輸出與 template 的略過，覆蓋前存 snapshot

## 3. 衝突與待處理清單

- [ ] 3.1 `QueueItem` 加上 dotfile 類型，衝突與本機修改進入清單並在內容改變時更新，`rsmt review` 與 `--json` 列出
- [ ] 3.2 實作 dotfile 項目的 `accept --scope host|all`、`discard`、`ignore`，並以 commit 記錄與依偏好推送
- [ ] 3.3 實作互動終端機中 `rsmt apply` 逐一詢問衝突（採用 repo 版本、保留本機版本、稍後處理）

## 4. 加密

- [ ] 4.1 加入 `age`（armor）與 `rpassword`，實作產生金鑰、以密碼加密成 `key.age`、本機 `age.key` 與解開金鑰的 core 函式；更新 `THIRD_PARTY_NOTICES.md` 並通過 `cargo deny check`
- [ ] 4.2 部署時解密、寫回時重新加密；本機沒有私鑰時終端機要求密碼（最多三次），非互動執行略過並回報

## 5. 新增與移除

- [ ] 5.1 實作 `rsmt add <path>... [--copy] [--encrypt]`：路徑檢查、自動記錄權限、秘密內容的拒絕、第一次加密時設定密碼、commit 與推送
- [ ] 5.2 實作 `rsmt forget <path>`：移除宣告與內容、symlink 換成一般檔案、commit 與推送
- [ ] 5.3 `edit(...)` commit 訊息新增 `home` 範圍

## 6. 停止使用

- [ ] 6.1 `rsmt eject` 把 link 換成一般檔案並清除部署紀錄；`rsmt restore --original` 還原 dotfile 目標（含目錄），確認還原後與部署前相同

## 7. 背景服務

- [ ] 7.1 依部署紀錄監看 link 的上層目錄與 copy 的目標，以內容雜湊略過自己的寫入，確認編輯器以改名儲存後數秒內寫回 repo
- [ ] 7.2 dotfile 項目加入合併通知，新增需要密碼的問題通知與英文、繁體中文字串

## 8. 驗證與文件

- [ ] 8.1 在 macOS 與 8 個 Linux distro 上跑完整測試，含 symlink、權限與加密
- [ ] 8.2 README 說明 `dotfiles.toml`、`rsmt add`、`forget`、衝突處理與加密（含密碼無法找回），並把新增的 CLI 與通知文字列出給使用者審閱
