# Proposal

## Why

同一份設定要在不同 OS、不同用途的主機上產生不同內容，例如工作電腦的 git email 不同；有些設定還需要填入存在密碼管理工具中的 token。這兩件事都需要 template，以及能在背景安全取得秘密的方式。

## What Changes

- 以 minijinja 作為 template 引擎（ADR 0006），預設在沙箱中執行，可用的函式由 rosamta 明確提供。
- template 變數：`host.os`、`host.arch`、`host.name`（識別名稱）、`host.tags`，以及使用者自訂的 `vars.*`（存在 `host.toml`，第一次設定時詢問）。
- 秘密來源：依 rbw → bw → bws 的順序，自動使用第一個已安裝且可用的工具；順序可在 `host.toml` 覆寫，GUI 不顯示這項設定。
- 另提供以指令取值的通用方式（例如 `op read`、`pass show`）：指令集中宣告在 repo 中並命名，template 只能以名稱呼叫，不能直接寫指令字串。
- 背景服務永遠不跳出解鎖提示：只有輸入改變的 template 才重新 render；需要秘密但 vault 鎖住時，保留上一次的結果並在待處理清單標示「需要解鎖」，等使用者在終端機執行 `rsmt apply` 處理。
- `bws` 的 access token 存在系統 keychain，可在背景使用；rosamta 不快取秘密的值。
- Bitwarden SSH agent：依安裝方式（brew 或 dmg、Mac App Store、snap、flatpak）找出 socket 路徑並設定 `SSH_AUTH_SOCK`。

## Capabilities

### New Capabilities

- `templates`: template 的語法、變數、可用函式與 render 時機。
- `secret-sources`: 秘密來源的選擇、取值、鎖住時的行為與 token 保存。

### Modified Capabilities

- `dotfile-deploy`: 啟用 template 部署方式。
- `review-queue`: 新增「需要解鎖」類型的項目。

## Impact

- 新依賴：`minijinja`。
- 依賴 `add-dotfile-deploy`；Bitwarden SSH agent 的 provider 會在 `add-providers-and-doctor` 中使用本 change 的路徑偵測。
- 已決定：以指令取值時只能執行 repo 中宣告過的指令。repo 的 `[raw]` 本來就會被 shell 執行，這項限制不是安全邊界，而是讓 GUI 與 doctor 列得出會執行哪些指令。
