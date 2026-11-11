# Proposal

## Why

取代 chezmoi 還需要它的兩項常用功能：下載外部內容（例如 oh-my-zsh、zsh 插件的 git repo 或壓縮檔），以及在新機器上執行設定腳本。少了這些，使用者仍得保留其他工具。

## What Changes

- externals：在 repo 中宣告 git repo 或壓縮檔，鎖定 commit 或版本，套用時下載到指定位置；`rsmt externals update` 更新到新版本並以 commit 記錄。
- scripts：
  - `run_once`：每台 host 只執行一次。
  - `run_onchange`：腳本內容（或宣告的相依檔案）改變時才執行。
  - 腳本可宣告套用範圍與執行時機（部署 dotfile 之前或之後）。
- `create`：目標檔案不存在時才建立，之後不再管理內容。
- 背景服務拉取並套用新設定後，依 `host.toml` 的 `scripts` 偏好處理需要執行的腳本：`ask`（預設）列進待處理清單並通知，`auto` 直接執行。
- 腳本執行的輸出寫入狀態目錄的紀錄檔，GUI 可檢視。

## Capabilities

### New Capabilities

- `externals`: 外部內容的宣告、鎖定、下載與更新。
- `scripts`: 設定腳本的種類、執行條件、時機與紀錄。

### Modified Capabilities

- `dotfile-deploy`: 新增 `create` 部署方式。
- `apply`: 套用流程加入 externals 與 scripts 的步驟。
- `host-config`: 新增 `scripts` 偏好。
- `review-queue`: 新增腳本項目。
- `daemon`: 背景服務的 externals 與腳本。
- `notifications`: 腳本項目的通知。

## Impact

- 依賴 `add-dotfile-deploy` 與 `add-shell-render` 的套用流程。
- 已決定：背景服務預設不執行腳本，只在 `scripts = "auto"` 的主機上執行。腳本可能需要 sudo、執行很久或要求輸入，預設由使用者以 `rsmt apply` 或在 GUI 確認後執行。
- 修改 `host-config`：新增 `scripts` 偏好。
