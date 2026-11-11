# Spec Delta

## Purpose

在 host 上執行 repo 中宣告的設定腳本，例如新機器第一次安裝工具，或 Brewfile 改變時重新安裝，並記錄每次執行的輸出。

## ADDED Requirements

### Requirement: Script declarations
每個腳本 SHALL 在 repo 的 `scripts.toml` 中以一筆 `[[script]]` 宣告，包含 `source`（相對於 repo）、`run`（`once` 或 `onchange`）與可選的 `timing`（`before` 或 `after`，預設 `after`）、`watch`（僅 `onchange`，repo 中的路徑）與 `when`。腳本 SHALL 以 `source` 識別，同一個 `source` 只能宣告一次。`rsmt validate` SHALL 回報格式錯誤、不存在的 `source` 與 `watch`，以及重複的 `source`。

#### Scenario: Missing script
- **WHEN** `scripts.toml` 宣告的 `source` 在 repo 中不存在
- **THEN** `rsmt validate` 回報該路徑並以 2 結束

### Requirement: Run conditions
`once` 的腳本 SHALL 在本機成功執行一次後不再執行，即使內容之後改變。`onchange` 的腳本 SHALL 在腳本與 `watch` 的內容（目錄包含其下所有檔案）和上次成功執行時不同時執行；從未成功執行過也 SHALL 執行。執行失敗（結束碼不為 0）SHALL 不算執行過。

#### Scenario: Brewfile changed
- **WHEN** `onchange` 腳本宣告 `watch = ["Brewfile"]`，另一台 host 修改 `Brewfile` 並推送，本機拉取後在終端機執行 `rsmt apply`
- **THEN** 該腳本執行一次，再次 `rsmt apply` 不會再執行

#### Scenario: Once after an edit
- **WHEN** `once` 腳本已在本機成功執行，使用者之後修改了它的內容
- **THEN** 之後的套用都不會再執行它

### Requirement: Script execution
腳本 SHALL 以第一行 shebang 指定的直譯器執行，沒有 shebang 時以 `sh` 執行，不需要執行權限。工作目錄 SHALL 是家目錄，環境變數 SHALL 另外包含 `ROSAMTA_REPO`（repo 的路徑）與 `ROSAMTA_HOST_ID`。同一時機的腳本 SHALL 依宣告順序執行；一個腳本失敗 SHALL 不影響其他腳本與其他步驟，`rsmt apply` 以 1 結束。

#### Scenario: Python script
- **WHEN** 腳本第一行是 `#!/usr/bin/env python3`，檔案沒有執行權限
- **THEN** 腳本以 python3 執行

#### Scenario: One script fails
- **WHEN** 兩個 `after` 腳本中第一個以 1 結束
- **THEN** 第二個腳本照常執行，dotfile 照常部署，`rsmt apply` 列出失敗的腳本並以 1 結束

### Requirement: Who runs scripts
在終端機中執行的 `rsmt apply` SHALL 在執行前印出腳本與執行原因，再執行需要執行的腳本，腳本的 stdin 接終端機、輸出顯示在終端機。其他套用（沒有終端機的 `rsmt apply`、App、拉取後的套用與背景服務）SHALL 依 `host.toml` 的 `scripts` 偏好：`ask` 把腳本放進待處理清單、不執行；`auto` 以 `/dev/null` 為 stdin 執行，失敗的腳本放進待處理清單。

#### Scenario: New host
- **WHEN** 新 host clone repo 後在終端機執行 `rsmt apply`，repo 有一個 `once` 的安裝腳本
- **THEN** 腳本在該次套用中執行，可以要求 sudo 密碼

#### Scenario: Pulled in the background
- **WHEN** `scripts = "ask"`，背景服務拉取到新的腳本
- **THEN** 腳本沒有執行，出現在待處理清單中

### Requirement: Script logs
每次執行腳本 SHALL 把 stdout 與 stderr 依輸出順序寫進狀態目錄中該腳本的紀錄檔，取代上一次的紀錄，並記錄開始時間與結束碼。紀錄檔 SHALL 只有使用者本人可讀，超過 1 MiB 的輸出 SHALL 截斷並註明。

#### Scenario: Failed in the background
- **WHEN** `scripts = "auto"`，背景服務執行的腳本失敗
- **THEN** 紀錄檔有該次的輸出與結束碼，待處理清單中的項目顯示這份紀錄
