# Design

## Context

依 Q16 與 ADR 0010：git 操作一律呼叫系統的 `git`，沿用使用者既有的 SSH agent、credential helper 與 GPG 設定；GitHub 連結用 OAuth App 的 device flow。秘密的偵測規則由 `add-adopt` 提供，本 change 在推送前再用一次。背景排程（自動推送、定時拉取）由 `add-daemon` 觸發，本 change 提供它呼叫的操作。

## Goals / Non-Goals

**Goals:**
- 推送與拉取任何一步失敗，本機已生效的設定都不受影響。
- 不降低使用者既有的安全設定（簽章、全域 git 設定）。

**Non-Goals:**
- 在 GUI 中解決 rebase 衝突（`add-gui` 只顯示衝突並提供在編輯器開啟）。
- GitHub 以外平台的帳號整合。

## Decisions

### 呼叫系統 git，不用 libgit2 或 gitoxide

系統 `git` 會自動使用使用者的 `~/.ssh/config`、SSH agent、credential helper 與 `commit.gpgsign`，rosamta 不需要重新實作。代價是每次操作都要啟動子程序，但同步的頻率很低，這點成本可以接受。所有呼叫都設定 `GIT_TERMINAL_PROMPT=0`，避免在背景卡在輸入提示；也設定 `LC_ALL=C`，讓錯誤訊息固定是英文，才能分類。

### 推送流程

1. 若工作目錄有變更：`git add -A`，先以秘密規則掃描暫存的內容（命中就取消暫存並停止，秘密不會進 commit），再 commit，訊息依變更類型產生（例如 `edit(modules): update zoxide`）。接管等 rosamta 自己的 commit 之後觸發的自動推送跳過這一步，不替使用者 commit 還在編輯的檔案。
2. 以秘密規則掃描 `@{upstream}..HEAD` 每個 commit 新增的行（還沒有 upstream 時是 `HEAD --not --remotes=<remote>`）；命中就停止，回報檔案、行號與規則。
3. `git push`。失敗時依錯誤分類：網路或連線問題標記為離線並保留 commit；被遠端拒絕（non-fast-forward）時先拉取再推送一次；憑證被拒或遠端沒有這個 repo 時標記為受阻。

### 拉取流程

1. `git fetch`。沒有新的 commit 就結束。
2. 以 `git show @{upstream}:rosamta.toml` 檢查格式版本；較新就停止並提示升級。在 rebase 之前檢查，repo 才不會先變成看不懂的格式。
3. 工作目錄有變更時先 commit（rebase 需要乾淨的工作目錄），再 `git rebase @{upstream}`；失敗就 `git rebase --abort`，回報衝突的檔案。
4. 以 `apply_after` 套用：smoke test 和拉取前的 shell 啟動結果比較，只有新出現的錯誤才算失敗；失敗時保留 repo 的新內容，但不切換 generated script。從未套用過的 host 不套用，維持「使用者執行 apply 之前不動 rc 檔」。

### 簽章失敗的偵測

`git commit` 失敗且 stderr 含有 `gpg failed to sign`、`error: gpg` 或 `failed to write commit object` 時，視為簽章失敗：簽章失敗時 git 不會建立 commit，所以只需要以 `git reset`（不加 `--soft`）取消暫存、保留工作目錄，再把推送狀態寫成受阻並附上原因。rosamta 從不加上 `--no-gpg-sign`。接管時遇到簽章失敗，接管本身照常完成，module 留在工作目錄，下一次推送再 commit。

### GitHub device flow

- 向 `https://github.com/login/device/code` 申請代碼（scope `repo`），以預設瀏覽器開啟 `verification_uri`，依回傳的 `interval` 輪詢 `https://github.com/login/oauth/access_token`，並處理 `slow_down`、`expired_token`、`access_denied`。
- token 以 `keyring` crate 存取 macOS Keychain 與 Linux Secret Service；沒有 Secret Service 時寫入 `$XDG_CONFIG_HOME/rosamta/github-token`（0600），與 `gh` 的做法相同。

### 傳輸方式的選擇

以 `ssh -T -o BatchMode=yes git@github.com` 測試：輸出含 `successfully authenticated` 即使用 SSH URL。否則使用 HTTPS URL，並在 repo 的 `.git/config` 中設定 `credential.helper = !rosamta git-credential`；這個 helper 從 keychain 讀出 token 回傳給 git。

### 推送狀態

`$XDG_STATE_HOME/rosamta/sync.json` 記錄上次推送、上次拉取的時間與結果、未推送的 commit 數量，以及受阻原因。GUI 的「上次同步」與背景服務的通知都讀這個檔案。

## Risks / Trade-offs

- [`repo` scope 權限過大] → token 只存在本機 keychain、只用於建立與列出 repo；登出時一併刪除。ADR 0010 記錄這個取捨。
- [使用者的 GPG 金鑰需要在背景輸入密碼] → 不降級為未簽章，改為標記受阻並通知；使用者解鎖 gpg-agent 後下一次同步即可完成。
- [rebase 中途被中斷，例如程序被終止] → 每次拉取開始前檢查 repo 是否處於 rebase 中，是的話先 `git rebase --abort`。
