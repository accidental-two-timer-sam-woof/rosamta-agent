# Proposal

## Why

rosamta 的 render、接管、同步與診斷，全都建立在兩份設定上：所有 host 共用的 repo，以及每台 host 自己的設定。先定下兩者的結構、位置與套用範圍，後續的 change 才有共同的資料模型。

## What Changes

- 定義 repo 結構：`rosamta.toml`（含格式版本與 host 清單）、`modules/`（一個 module 一個 TOML 檔）、`home/`、`dotfiles.toml`、`packages.toml`。
- 定義本機的 `host.toml`：識別名稱、標籤、接管偏好、rc 檔原內容的處理方式、推送與拉取偏好、開 shell 提醒。
- 所有平台都使用 XDG 路徑（ADR 0008）。
- 定義套用範圍：`os`、`arch`、`hosts`、`tags` 與判斷規則；`hosts` 指的是識別名稱（ADR 0011）。
- 新增 CLI：`rosamta` 與 `rsmt` 兩個名稱，`rsmt init`、`rsmt validate`、`rsmt schema`，以及 `--json` 輸出與結束碼規則。
- repo 格式版本：舊格式自動遷移；遇到較新的格式拒絕讀取。

## Capabilities

### New Capabilities

- `cli`: 命令名稱、機器可讀輸出、結束碼與訊息語言。
- `config-repo`: repo 的結構、檔案格式、格式版本、驗證與建立方式。
- `host-config`: 本機設定、識別名稱與 rosamta 在 host 上使用的檔案位置。
- `host-conditions`: module 與 dotfile 的套用範圍判斷。

### Modified Capabilities

（無）

## Impact

- `rosamta-core` 新增設定的型別、解析、驗證與寫回；`rosamta` 新增 `init`、`validate`、`schema` 子指令。
- 依賴 `add-project-scaffolding` 建立的 workspace。
- 之後所有 change 都使用這裡定義的資料模型。
