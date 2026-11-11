# shell-loader Specification

## Purpose
定義 shell 啟動時載入 generated script 的行為，確保 rosamta 的任何問題都不會讓使用者打不開 shell，而且幾乎不增加啟動時間。

## Requirements

### Requirement: Missing generated scripts are harmless
generated script 不存在或無法讀取時，shell SHALL 正常啟動，且不輸出錯誤。

#### Scenario: Generated directory deleted
- **WHEN** 使用者刪除 rosamta 的 generated 目錄後開啟新的 zsh
- **THEN** zsh 正常啟動，沒有任何錯誤訊息

### Requirement: Error isolation
某個 module 的腳本執行出錯時，loader SHALL 在 stderr 印出該 module 的名稱，並繼續載入其餘的腳本。

#### Scenario: Broken snippet
- **WHEN** 一個片段 module 含有語法錯誤
- **THEN** 新開的 shell 印出一行包含該 module 名稱的警告，其他 module 的設定照常生效

### Requirement: Environment loaded once
同一個 shell 程序中，env 階段 SHALL 只執行一次，即使多個 rc 檔都有 hook。從這個 shell 啟動的另一個 shell SHALL 不再套用一次已經匯出的環境變數，但 SHALL 在該 shell 會重讀片段來源檔的情況下再執行 env 階段的片段：zsh 與 fish 的每個 shell，以及 bash 的 login shell（bash 的 env 片段來自 `.bash_profile` 與 `.profile`）；sh 不重新執行。片段設定的選項、函式與沒有匯出的變數不會傳給子程序。

#### Scenario: Login bash
- **WHEN** 開啟同時讀取 `~/.bash_profile` 與 `~/.bashrc` 的 login bash
- **THEN** PATH 中沒有因重複載入而產生的重複項目

#### Scenario: Nested shell
- **WHEN** env 階段有一個 module 宣告 `CNT = "x$CNT"`，另有片段設定沒有匯出的 `LOCAL=1`，使用者在互動 zsh 中再啟動一個互動 zsh
- **THEN** 內層的 `CNT` 仍是 `x`，`LOCAL` 是 `1`

#### Scenario: Nested bash that is not a login shell
- **WHEN** 同樣的 module，使用者在互動 bash 中再啟動一個不是 login shell 的互動 bash
- **THEN** 內層的 `CNT` 仍是 `x`，片段沒有再執行：bash 在這種 shell 不讀 `.bash_profile`

#### Scenario: Debian profile
- **WHEN** `~/.profile` 在 bash 中載入 `~/.bashrc`（Debian 與 Ubuntu 的預設），使用者開啟 login bash
- **THEN** env 階段與互動階段各只執行一次

### Requirement: Startup overhead budget
在沒有任何 module 的情況下，hook 與 loader 讓每個 shell 啟動時間增加的中位數 SHALL 不超過 5 ms。

#### Scenario: Measured in CI
- **WHEN** CI 在 Linux 上分別量測有 hook 與沒有 hook 的 `zsh -i -c exit` 各 30 次
- **THEN** 兩者中位數的差不超過 5 ms

### Requirement: Profiling mode
設定 `ROSAMTA_PROFILE=1` 時，loader SHALL 記錄每個 module 的載入時間到狀態目錄的紀錄檔；shell 沒有毫秒等級的計時方式時 SHALL 只記錄整體時間。

#### Scenario: zsh profiling
- **WHEN** 使用者以 `ROSAMTA_PROFILE=1 zsh -i -c exit` 啟動
- **THEN** 紀錄檔中有每個 module 的名稱與載入毫秒數

#### Scenario: bash 3.2 profiling
- **WHEN** 使用者在 macOS 內建的 bash 3.2 上以同樣方式啟動
- **THEN** 紀錄檔中只有整體載入時間
