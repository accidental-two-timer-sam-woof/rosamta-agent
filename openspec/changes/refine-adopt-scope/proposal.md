# Proposal

## Why

使用者希望不必想就有備份：背景服務發現 rc 檔的新內容就自動接管，接管後的行為與接管前完全相同，要套用到其他 OS、shell 或主機時再手動放寬。換電腦時，要能很快找到其他主機最近新增的設定並把新主機加進去；兩台主機各自裝了同一個工具時，相同的設定應該合併成一份，而不是在 repo 裡出現 `bun` 與 `bun-2`。沒有網路時，變更要留著，連上網路後馬上推送。

## What Changes

- 接管的套用範圍記錄主機、OS 與來源 shell：結構化 module 不再套用到所有 shell，只套用到來源檔案所屬的 shell；選「所有主機」時只去掉主機與 OS 的限制。
- 預設接管偏好改為「自動」（只影響新建立的 `host.toml`）。
- 跨主機合併：本機接管到的內容與 repo 中只套用到其他主機的 module 相同（正規化後完全一致、來源 shell 相同）時，把本機加進該 module 的套用範圍，不另建 module；內容不同則照常另建。
- `rsmt recent` 列出其他主機最近新增、本機還沒套用的 module 與 dotfile；`rsmt include` 把本機加進它們的套用範圍，需要放寬 OS、架構或 shell 時先列出並確認。
- 沒有網路而無法推送時通知一次；有尚未推送的 commit 時每分鐘重試推送，成功後恢復原本的間隔。
- App：總覽顯示其他主機最近新增的項目數，並可勾選加入本機；待處理頁顯示合併時對既有 module 的變更。
- **BREAKING**：從 `.zshrc`、`.bashrc` 等檔案新接管的結構化內容不再套用到其他 shell；既有的 module 不改寫。

## Capabilities

### New Capabilities

（無）

### Modified Capabilities

- `adopt`: 套用範圍的內容、結構化 module 的 shell、跨主機合併。
- `host-config`: 預設接管偏好。
- `git-sync`: 離線時的重試。
- `daemon`: 有尚未推送的 commit 時每分鐘重試。
- `notifications`: 離線通知。
- `config-repo`: `rsmt recent` 與 `rsmt include`。
- `gui`: 總覽的最近新增項目、待處理頁的合併項目。

## Impact

- `adopt` 的掃描、計畫與還原，`config::Module` 的寫出格式。
- 新增 `rsmt recent`、`rsmt include`。
- 背景服務的同步排程與問題通知。
- 接管偏好也決定複製部署的 dotfile 在本機被修改時是否自動寫回 repo；預設改為自動後，新主機上的這類修改也會自動寫回，不再列入待處理清單（寫回的 commit 仍會檢查秘密）。已在 `host.toml` 寫明偏好的主機不受影響。
- `add-providers` 的接管規則建立在這裡的合併規則上。
