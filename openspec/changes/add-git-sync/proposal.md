# Proposal

## Why

接管與編輯只會更新本機的 repo。使用者有多台機器時，需要 rosamta 自動把變更推到 GitHub、把其他機器的變更拉回來並套用，才不會再出現「repo 落後很多」的情況；同時，推送前必須確保秘密不會外流。

## What Changes

- 推送：把 repo 的變更 commit 並 push 到遠端；推送偏好為「自動」時，在接管或編輯後自動執行。
- 拉取：fetch 後以 rebase 合併，成功後套用到本機；發生衝突時中止並保留本機狀態。
- 同步：`rsmt sync` 先拉取再推送。另有 `rsmt pull`、`rsmt publish`。
- commit 依使用者的 git 設定簽章；簽章失敗時不建立未簽章的 commit，改標記為推送受阻。
- 連結 GitHub：以 device flow 登入（ADR 0010），token 存在系統 keychain；可建立 private repo 或選擇既有 repo；優先使用 SSH，連不上才用 token 走 HTTPS。
- 推送前再掃描一次要推送的內容，發現疑似秘密就停止推送。

## Capabilities

### New Capabilities

- `git-sync`: 推送、拉取、同步、簽章、衝突與離線的行為。
- `github-link`: 連結 GitHub 帳號、建立或選擇 repo、選擇傳輸方式。

### Modified Capabilities

- `secret-guard`: 新增「推送前掃描」的行為（此 capability 由 `add-adopt` 建立）。

## Impact

- `rosamta-core` 新增 git 操作（呼叫系統 `git`）、GitHub API client、keychain 存取；`rosamta` 新增 `sync`、`pull`、`publish`、`remote github` 子指令，以及 `rsmt init --github`。
- 需要為 rosamta 註冊 GitHub OAuth App 並啟用 device flow。
- 依賴 `add-adopt` 建立的 commit 與 `add-shell-render` 的套用流程。
