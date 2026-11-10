# Tasks

## 1. 掃描與切段

- [x] 1.1 實作 watched file 清單（含 `ZDOTDIR` 與 fish `conf.d`），確認在 Docker 映像中找到每個 shell 的 rc 檔
- [x] 1.2 實作 hook 區塊外內容的擷取與切段（註解併入、marker 區塊整段），以安裝程式 fixture 撰寫快照測試

## 2. 辨識

- [x] 2.1 實作 POSIX、zsh、fish 的結構化行規則與 `case ":$PATH:"` 寫法，確認 fixture 中的 PATH、環境變數、alias 都被正確辨識
- [x] 2.2 實作「任一行無法辨識就整段當片段」與片段的 shell 範圍、載入階段規則，確認混合內容的測試通過
- [x] 2.3 定義辨識器介面並以結構化辨識器實作，確認新增辨識器不需修改掃描流程

## 3. 產生 module 與修改 rc 檔

- [x] 3.1 實作 module 命名、`$HOME` 改寫、預設本機範圍與 `modules/adopted/` 位置，確認名稱衝突時加上數字後綴
- [x] 3.2 實作刪除與註解兩種模式、修改前存 snapshot、每次接管一個 commit，確認 commit 訊息格式
- [x] 3.3 實作接管紀錄與重複寫入偵測，以「安裝程式重跑」fixture 確認不會產生第二個 module
- [x] 3.4 實作 `rsmt restore <id>`，確認原行回到原位置、module 被刪除並有 commit

## 4. 秘密

- [x] 4.1 匯入 gitleaks 規則子集並在 build 時檢查可編譯，加上熵檢查與變數名稱規則，確認範例 token 被標記、`$(...)` 參照不被標記
- [x] 4.2 實作本機 `local/modules/` 並納入 render，確認接管秘密後 shell 中有值、`git log -p` 找不到該值
- [x] 4.3 在 `THIRD_PARTY_NOTICES.md` 加入 gitleaks 規則的來源與授權，確認 `cargo about` 產出包含它

## 5. 待處理清單與 CLI

- [x] 5.1 實作接管偏好（自動、詢問、關閉）與 `queue.json`、`ignored.json`，確認被忽略的段落修改後會重新出現
- [x] 5.2 實作 `rsmt review`（列出、accept、ignore、`--scope`、`--json`）與 `rsmt adopt`（確認、`--yes`），以 `assert_cmd` 測試
- [x] 5.3 實作 `notice` 檔與 loader 的提醒，確認只在互動 shell 印出、關閉提醒後不印，且啟動時間預算測試仍通過

## 6. 整合驗證

- [x] 6.1 在 Docker 矩陣中套用安裝程式 fixture 後執行 `rsmt adopt --yes`，比對接管前後新 shell 的環境變數、PATH 與 alias，確認完全相同且 rc 檔只剩 hook
- [x] 6.2 在 README 說明接管、待處理與還原的流程，確認範例可照抄執行
