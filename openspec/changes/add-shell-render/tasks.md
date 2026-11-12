# Tasks

## 1. Renderer 核心

- [ ] 1.1 定義結構化與片段 module 的欄位（env、path、aliases、raw、phase、order、shells），確認範例檔解析測試通過
- [ ] 1.2 實作 module 篩選（套用範圍）與排序（phase、order、名稱），以表格驅動測試確認順序
- [ ] 1.3 實作 shell 版本偵測與快取，在 Docker 映像中確認四種 shell 都能取得版本，低於下限時回報警告

## 2. 各 shell 的語法

- [ ] 2.1 實作 POSIX 輸出（env、PATH 去重、alias 只在互動 shell），以 dash、BusyBox ash、`bash --posix` 執行快照測試
- [ ] 2.2 確認 bash 輸出在 macOS 內建 bash 3.2 上通過全部測試（CI 的 macOS job）
- [ ] 2.3 實作 zsh 輸出與 macOS `.zprofile` 的 PATH 重新排序，確認 login zsh 中 module 的 PATH 排在 `/usr/bin` 之前
- [ ] 2.4 實作 fish 輸出（`set -gx`、`fish_add_path --global --path`），確認執行後 `fish_variables` 沒有新增 universal 變數
- [ ] 2.5 以含引號、`$`、反斜線、`~` 的值撰寫跨 shell 測試，確認四種 shell 得到相同的值

## 3. Loader 與 hook

- [ ] 3.1 產生版本化的 generated 目錄與 `current` symlink，確認切換是單一 `rename` 操作
- [ ] 3.2 撰寫各 shell 的 loader：錯誤隔離、env 防重、`ROSAMTA_PROFILE` 量測，確認含語法錯誤的片段不影響其他 module
- [ ] 3.3 實作 hook 安裝與 `rsmt unhook`（含 `ZDOTDIR`、bash profile 檔選擇、fish `conf.d`），確認重複安裝只有一個區塊、區塊外內容不變
- [ ] 3.4 在 CI 加入啟動時間量測（有無 hook 各 30 次），確認中位數差不超過 5 ms
- [ ] 3.5 在 Docker 矩陣中測試刪除 generated 目錄後各 shell 仍能無錯誤啟動

## 4. Snapshot

- [ ] 4.1 實作本機 snapshot repo，修改 rc 檔前自動 commit 並在第一次時打上 `original` tag，確認 snapshot 目錄沒有任何 remote
- [ ] 4.2 實作 `rsmt restore --original`（顯示差異、確認、`--yes`），確認還原後 rc 檔與安裝前逐位元組相同

## 5. Apply 與 eject

- [ ] 5.1 實作 `rsmt apply --dry-run` 的差異輸出與 `--json`，確認不寫入任何檔案
- [ ] 5.2 實作 smoke test 與回滾，以會輸出錯誤的片段 module 測試：apply 以 1 結束、列出 module 名稱、`current` 不變
- [ ] 5.3 實作 `rsmt eject`，在 Docker 中確認 eject 並移除執行檔後，新 shell 的 PATH、環境變數與 alias 不變
- [ ] 5.4 在 README 說明 `apply`、`unhook`、`eject`、`restore --original`，確認範例可照抄執行
