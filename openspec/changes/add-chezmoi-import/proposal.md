# Proposal

## Why

rosamta 的目標之一是完全取代 chezmoi。現有的 chezmoi 使用者需要一條能把 source 目錄轉成 rosamta repo 的路，否則換工具的成本太高。

## What Changes

- `rsmt import chezmoi [<source-dir>]`：預設讀取 `chezmoi source-path` 回報的位置。
- 檔名屬性（`dot_`、`private_`、`executable_`、`symlink_`、`encrypted_`、`create_`、`readonly_` 等）轉成 `home/` 中的實際檔名與 `dotfiles.toml` 的宣告（ADR 0007）。
- Go template 中常見的寫法自動轉成 minijinja（例如 `{{ if eq .chezmoi.os "darwin" }}`）；無法轉換的部分標示出來，請使用者手動修改。
- `run_once_`、`run_onchange_` 腳本轉成 rosamta 的 scripts；`.chezmoiexternal.*` 轉成 externals；`.chezmoiignore` 轉成套用範圍。
- 加密的檔案以 chezmoi 解密，再以 rosamta 的金鑰（ADR 0012）重新加密；不沿用 chezmoi 的金鑰，因為 repo 只有一把金鑰。
- 產生匯入報告：成功轉換、需要手動處理、不支援的項目各有幾個，以及對應的檔案。
- 設定精靈的「從 chezmoi 遷移」使用這個功能。

## Capabilities

### New Capabilities

- `chezmoi-import`: 讀取 chezmoi source 目錄並轉換成 rosamta repo 的規則與報告。

### Modified Capabilities

（無）
- GUI：在 `add-gui` 建立的 app 中加入本 change 對應的頁面或步驟。

## Impact

- 依賴 `add-dotfile-deploy`、`add-templates-and-secrets`、`add-externals-and-scripts` 提供的目標格式；應在它們之後實作。
- chezmoi 是 MIT 授權；若移植它的測試案例或程式碼，需在 `THIRD_PARTY_NOTICES.md` 保留聲明。
