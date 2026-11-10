# Tasks

## 1. git 操作基礎

- [x] 1.1 實作呼叫系統 `git` 的包裝（`GIT_TERMINAL_PROMPT=0`、錯誤分類），以本機 bare repo 測試網路錯誤與 non-fast-forward 的分類
- [x] 1.2 實作 commit 與簽章失敗偵測（不使用 `--no-gpg-sign`、失敗時 `git reset` 取消暫存），以設定了無效簽章金鑰的測試 repo 確認不會產生未簽章的 commit
- [x] 1.3 實作 `sync.json` 狀態檔，確認推送、拉取、受阻、離線各狀態都會正確寫入

## 2. 推送、拉取與同步

- [ ] 2.1 實作 `rsmt publish`（自動 commit、推送前秘密掃描、push），確認含 API key 的手改內容會被擋下且遠端沒有新 commit
- [ ] 2.2 實作 `rsmt pull`（fetch、rebase、衝突時 abort、格式版本檢查、apply），確認衝突時 repo 不處於 rebase 中途、smoke test 失敗時 generated script 不切換
- [ ] 2.3 實作 `rsmt sync` 與推送偏好（自動、手動），以兩個本機 clone 模擬兩台 host 的整合測試確認雙向同步
- [ ] 2.4 實作離線重試：沒有網路時保留 commit，恢復後下一次同步推送成功

## 3. GitHub 連結

- [ ] 3.1 註冊 rosamta 的 GitHub OAuth App 並啟用 device flow，把 client ID 寫進設定
- [ ] 3.2 實作 device flow 登入（含逾時、拒絕、`slow_down`），以 mock server 測試各種回應
- [ ] 3.3 實作 token 儲存（keychain，無 keychain 時 0600 檔案並警告）與 `rsmt remote github logout`，確認登出後 token 消失
- [ ] 3.4 實作 `rsmt init --github <name>` 與 `<owner>/<name>`，以測試帳號確認建立的是 private repo
- [ ] 3.5 實作 SSH 測試與 HTTPS credential helper（`rosamta git-credential`），確認 `~/.gitconfig` 未被修改

## 4. 文件

- [ ] 4.1 在 README 說明推送、拉取、同步、GitHub 連結與受阻狀態的處理方式，確認範例可照抄執行
