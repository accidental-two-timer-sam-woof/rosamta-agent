# host-config Specification

## Purpose
定義每台 host 自己的設定，以及 rosamta 在 host 上使用的檔案位置，讓同一個 repo 能在不同 host 上有不同的行為，而這些差異不會被推到其他 host。

## Requirements

### Requirement: XDG file locations
rosamta SHALL 在所有平台使用 XDG 路徑：設定在 `$XDG_CONFIG_HOME/rosamta`，repo 在 `$XDG_DATA_HOME/rosamta/repo`，generated script 在 `$XDG_DATA_HOME/rosamta/generated`，狀態在 `$XDG_STATE_HOME/rosamta`。變數未設定或不是絕對路徑時 SHALL 使用 XDG 規範的預設值。

#### Scenario: macOS without XDG variables
- **WHEN** macOS 上沒有設定任何 `XDG_*` 變數
- **THEN** host 設定檔位於 `~/.config/rosamta/host.toml`，repo 位於 `~/.local/share/rosamta/repo`

#### Scenario: XDG override
- **WHEN** `XDG_CONFIG_HOME` 設為 `/srv/cfg`
- **THEN** host 設定檔位於 `/srv/cfg/rosamta/host.toml`

#### Scenario: Relative value ignored
- **WHEN** `XDG_STATE_HOME` 設為 `state`（相對路徑）
- **THEN** rosamta 使用 `~/.local/state/rosamta`

### Requirement: Host settings stay local
`host.toml` SHALL 只存在本機、不進 repo。它記錄識別名稱、標籤、接管偏好、rc 檔原內容的處理方式、推送偏好、拉取偏好與間隔、開 shell 提醒。

#### Scenario: Defaults
- **WHEN** rosamta 第一次建立 `host.toml`
- **THEN** 接管偏好為「詢問」、rc 檔原內容為「刪除」、推送偏好為「自動」、拉取偏好為「自動」且間隔 15 分鐘、開 shell 提醒為開啟

### Requirement: Stable host ID
rosamta 第一次建立 `host.toml` 時 SHALL 從系統 hostname 產生識別名稱並寫入；之後系統 hostname 改變時，識別名稱 SHALL 維持不變。

#### Scenario: Hostname changes
- **WHEN** macOS 因為區網名稱衝突，把 hostname 改成原本名稱加上 `-2`
- **THEN** rosamta 仍以原本的識別名稱判斷套用範圍

### Requirement: Rename host ID
使用者修改識別名稱時，rosamta SHALL 同時更新 repo 中所有引用舊名稱的套用範圍與 host 清單，並以一個 commit 記錄。

#### Scenario: Rename
- **WHEN** 使用者把識別名稱從 `mbp` 改成 `work-mbp`
- **THEN** 所有 `hosts` 含有 `mbp` 的 module 與 dotfile 改為 `work-mbp`，host 清單也一併更新

### Requirement: Sorted tags
host 的標籤 SHALL 以字母順序儲存與顯示，且不重複。

#### Scenario: Add a tag
- **WHEN** 已有標籤 `personal`，使用者新增 `lab`
- **THEN** 標籤依序為 `lab`、`personal`

#### Scenario: Duplicate tag
- **WHEN** 使用者再次新增已存在的標籤 `lab`
- **THEN** 標籤清單維持不變
