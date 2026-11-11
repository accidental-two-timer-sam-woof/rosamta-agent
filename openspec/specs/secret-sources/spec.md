# secret-sources Specification

## Purpose
讓 template 取得密碼管理工具中的秘密，而秘密不進 repo、rosamta 也不保存秘密的值；背景服務永遠不跳出解鎖或輸入提示。

## Requirements

### Requirement: Bitwarden
`bitwarden(item, field)` SHALL 依序嘗試 rbw、bw、bws，使用第一個已安裝且已登入的工具；`host.toml` 的 `[secrets] bitwarden` SHALL 可覆寫順序。省略 `field` 時 SHALL 取密碼；`username`、`notes`、`totp` 與自訂欄位名稱 SHALL 取對應的值。bws SHALL 以 key 等於 `item` 的秘密取值，`field` 只接受省略或 `note`。沒有任何可用的工具、或找不到項目時，render SHALL 失敗並說明原因。

#### Scenario: rbw first
- **WHEN** rbw 與 bw 都已安裝並登入，template 使用 `bitwarden("GitHub", "token")`
- **THEN** rosamta 以 rbw 取得 `GitHub` 項目的 `token` 欄位

#### Scenario: Custom order
- **WHEN** `host.toml` 設定 `[secrets] bitwarden = ["bw"]`，rbw 也已安裝
- **THEN** rosamta 只使用 bw

### Requirement: Secret commands
repo 根目錄的 `secrets.toml` SHALL 以 `[commands.<name>] run = [...]` 宣告可用的指令。`secret(name, arg)` SHALL 把 `run` 每個元素中的 `{arg}` 換成參數後直接執行（不經 shell），取 stdout 並去掉結尾的一個換行。template SHALL NOT 能執行未宣告的指令。指令失敗時 render SHALL 失敗，訊息包含指令名稱。`rsmt validate` SHALL 檢查 `secrets.toml`，`rsmt schema secrets` SHALL 輸出其 JSON Schema。

#### Scenario: 1Password
- **WHEN** `secrets.toml` 宣告 `[commands.op] run = ["op", "read", "{arg}"]`，template 使用 `secret("op", "op://Private/GitHub/token")`，使用者在終端機執行 `rsmt apply`
- **THEN** rosamta 執行 `op read op://Private/GitHub/token`，把輸出填進 template

#### Scenario: Undeclared command
- **WHEN** template 使用 `secret("vault", "x")`，`secrets.toml` 沒有 `vault`
- **THEN** 該 template render 失敗，訊息指出 `vault` 未宣告

### Requirement: Never prompt in the background
需要的 Bitwarden 工具鎖住時，終端機中的 `rsmt apply` SHALL 執行該工具的解鎖指令並讓工具自己詢問密碼；非互動執行與背景服務 SHALL NOT 解鎖，也 SHALL NOT 執行秘密指令，而是保留目標原本的內容並放進待處理清單。

#### Scenario: Locked in the background
- **WHEN** rbw 鎖住時拉取到一個使用 `bitwarden()` 的 template 的修改
- **THEN** 目標維持原樣，待處理清單列出該路徑在等待 rbw 解鎖，背景服務通知一次

#### Scenario: Command in the background
- **WHEN** 背景服務需要 render 一個使用 `secret()` 的 template
- **THEN** 不執行該指令，待處理清單列出該路徑在等待終端機中的 `rsmt apply`

### Requirement: Secret values are not stored
rosamta SHALL NOT 把秘密的值寫進 repo、部署紀錄、snapshot 以外的任何檔案或日誌；同一次套用中相同參數的秘密 SHALL 只取一次。bws 的 access token SHALL 存在系統 keychain，以環境變數傳給 bws，不出現在命令列。`rsmt secrets bws login` SHALL 以不回顯的方式讀入 token，`rsmt secrets bws logout` SHALL 刪除它。

#### Scenario: Token login
- **WHEN** 使用者執行 `rsmt secrets bws login` 並貼上 access token
- **THEN** token 存進 keychain，畫面上不顯示 token，之後背景服務可使用 bws

### Requirement: Bitwarden SSH agent socket
`bitwarden_ssh_socket()` SHALL 依 Bitwarden 桌面版的安裝方式（Mac App Store、snap、flatpak 或其他）回傳 SSH agent 的 socket 路徑，沒有安裝時回傳空值。

#### Scenario: Snap
- **WHEN** Bitwarden 以 snap 安裝
- **THEN** `bitwarden_ssh_socket()` 回傳 `~/snap/bitwarden/current/.bitwarden-ssh-agent.sock` 的絕對路徑
