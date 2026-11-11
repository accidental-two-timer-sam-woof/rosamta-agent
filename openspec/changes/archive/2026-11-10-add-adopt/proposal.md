# Proposal

## Why

安裝程式會自己把設定寫進 rc 檔，使用者也常直接手改 rc 檔，久了就忘記哪些設定該進 repo，chezmoi 這類工具的 repo 因此越落越後面。rosamta 要把 rc 檔裡、hook 以外出現的內容接管進 repo，讓 rc 檔維持只有 hook 的狀態。

## What Changes

- 掃描 watched file（sh、bash、zsh、fish 的 rc 檔與 fish 的 `conf.d`），找出 hook 以外的內容。
- 把內容切成段落並辨識：單純的環境變數、PATH、alias 轉成結構化 module（套用到所有 shell）；其餘保存為片段 module（只套用到同類 shell）。
- 絕對路徑改寫成 `$HOME`；接管的 module 預設只套用到本機。
- 接管後從 rc 檔移除原行（預設刪除，可改為註解）；修改前存 snapshot；每次接管建立一個 commit。
- 接管偏好：自動、詢問、關閉。詢問時放進待處理清單，以 `rsmt review` 處理；開互動 shell 時提醒待處理的數量。
- 疑似秘密的內容不進 repo，改存成只在本機生效的 module。
- 可以把任何一次接管還原回 rc 檔。

## Capabilities

### New Capabilities

- `adopt`: 掃描、切段、辨識、改寫路徑、產生 module、移除原行與記錄的規則。
- `review-queue`: 待處理清單、`rsmt review` 的操作、接管偏好與開 shell 時的提醒。
- `secret-guard`: 辨識疑似秘密的內容，並確保它只存在本機。

### Modified Capabilities

- `snapshots`: 新增「還原單次接管」的行為（此 capability 由 `add-shell-render` 建立）。

## Impact

- `rosamta-core` 新增掃描器、辨識規則、秘密規則；`rosamta` 新增 `adopt`、`review`、`restore <id>` 子指令。
- 會依接管偏好修改使用者的 rc 檔；依賴 `add-shell-render` 的 hook 與 snapshot。
- provider 層級的辨識（例如把 `eval "$(zoxide init zsh)"` 轉成 zoxide module）留給 `add-providers-and-doctor`，本 change 提供擴充點。
