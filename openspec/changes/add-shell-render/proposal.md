# Proposal

## Why

使用者的 PATH、環境變數與 alias 分散在多個 rc 檔裡，寫在 bash 檔案的設定 zsh 讀不到，每個 shell 又要用不同語法重寫一次。rosamta 要把同一份宣告轉成每個 shell 的腳本，並且只用一個 hook 載入，讓 rc 檔本身保持簡潔。

## What Changes

- 從 module render 出 sh、bash、zsh、fish 的 generated script，依載入階段排序；低於版本下限的 shell 不產生，並提示原因。
- 結構化 module（環境變數、PATH、alias）自動轉成各 shell 的語法；PATH 自動去重。
- 每個 shell 只在 rc 檔插入一個有 marker 的 hook，由 loader 載入 generated script；任一腳本出錯不影響其他腳本與 shell 啟動。
- `rsmt apply`：render、smoke test，通過才以原子方式切換；失敗則保留舊版本。`--dry-run` 只顯示差異。
- 修改任何 rc 檔之前先存 snapshot（只存本機）；`rsmt restore --original` 可還原到安裝 rosamta 之前的狀態。
- `rsmt eject`：把目前的設定直接寫回 rc 檔並移除 hook，移除 rosamta 後 shell 仍然正常。
- loader 本身每次開 shell 增加的時間不超過 5 ms。

## Capabilities

### New Capabilities

- `shell-render`: module 轉成各 shell generated script 的規則、支援的 shell 與版本下限。
- `shell-loader`: shell 啟動時載入 generated script 的行為、錯誤隔離、效能預算與量測模式。
- `shell-hooks`: 各 shell 的 hook 位置、安裝與移除規則。
- `apply`: 套用設定的流程，包含預覽、smoke test、回滾、原子切換與 eject。
- `snapshots`: 修改 rc 檔前保存原文，以及還原的方式。

### Modified Capabilities

（無）

## Impact

- `rosamta-core` 新增 renderer、hook 管理、snapshot store；`rosamta` 新增 `apply`、`unhook`、`eject`、`restore` 子指令。
- 會修改使用者的 rc 檔（只在 marker 區塊內），修改前一律存 snapshot。
- 依賴 `add-repo-and-host-config` 的資料模型與套用範圍。
