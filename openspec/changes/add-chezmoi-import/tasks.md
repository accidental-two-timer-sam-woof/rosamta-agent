# Tasks

## 1. 讀取 chezmoi source

- [ ] 1.1 找到 source（參數、`chezmoi source-path`、預設位置、`.chezmoiroot`），讀取 `chezmoi data` 與 `chezmoi dump-config`（`mode`、`umask`、`[data]`）；檔名屬性解析（目錄、各類檔案、`literal_`、`.literal`、`.tmpl`、`.age`、`external_` 目錄）與對應的部署方式、權限、只有空白的內容、`exact_` 目錄；以單元測試涵蓋 chezmoi 文件中的每個前綴與後綴
- [ ] 1.2 平台與主機的套用範圍、`.chezmoiignore`（含 OS、架構與 `ne .chezmoi.hostname` 條件區塊、`**`、`!` 例外、子目錄中的）與轉成 `when`；以單元測試驗證

## 2. Template 轉換

- [ ] 2.1 Go template 動作的解析與設計中列出的寫法轉成 minijinja（條件、比較、chezmoi 變數、hostname 換成識別名稱、資料與布林值、Bitwarden 與 1Password、去空白、`raw`）；其他寫法回報行號；整份單一條件改為 `when`；以單元測試與 rosamta 的 template 檢查驗證

## 3. 匯入

- [ ] 3.1 寫入 `home/`、`dotfiles.toml`、`secrets.toml`、`chezmoi-import/`、本機 `[vars]`，略過已存在的項目，以 `chezmoi cat` 比較 template，建立含報告的 commit；加密檔案以 `chezmoi decrypt` 重新加密；以 fixture 的整合測試驗證
- [ ] 3.2 腳本（含 `.chezmoiscripts/`、排序、`include … | sha256sum` 轉 `watch`、記為已執行、`CHEZMOI_` 變數）與 externals（git-repo 鎖定在既有 clone、archive、路徑包含檢查與不支援的欄位）的匯入；以整合測試驗證
- [ ] 3.3 `rsmt import chezmoi`、`--dry-run`、`--json` 與報告的文字；有 chezmoi 時與 `chezmoi apply` 比較結果的測試

## 4. App

- [ ] 4.1 設定精靈的「從 chezmoi 匯入」與結果頁；以 gpui 測試與截圖驗證

## 5. 驗證

- [ ] 5.1 fmt、clippy、全部測試、Linux distro、`cargo deny`，更新 README、GLOSSARY、THIRD_PARTY_NOTICES，列出所有使用者看得到的文字供審閱
