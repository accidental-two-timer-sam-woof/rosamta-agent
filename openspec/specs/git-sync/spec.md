# git-sync Specification

## Purpose
在本機 repo 與遠端之間雙向同步設定，讓每台 host 都持有最新的設定，且同步失敗時不會破壞本機已生效的狀態。

## Requirements

### Requirement: Publish
`rsmt publish` SHALL 把 repo 中尚未 commit 的變更建立成 commit，再把所有尚未推送的 commit push 到遠端。

#### Scenario: Uncommitted edit
- **WHEN** 使用者在編輯器中修改了 `modules/zoxide.toml` 後執行 `rsmt publish`
- **THEN** repo 多一個描述此修改的 commit，且遠端包含這個 commit

### Requirement: Publish policy
推送偏好為「自動」時，rosamta SHALL 在每次接管或透過 rosamta 修改 repo 之後自動推送；為「手動」時 SHALL 只在使用者執行推送或同步時推送。

#### Scenario: Manual policy
- **WHEN** 推送偏好為「手動」，使用者接受一個接管項目
- **THEN** repo 多一個 commit，但遠端沒有變化

#### Scenario: Automatic push leaves hand edits alone
- **WHEN** 推送偏好為「自動」，使用者正在編輯 `modules/zoxide.toml` 且尚未儲存成 commit 時接受一個接管項目
- **THEN** 遠端收到接管的 commit，`modules/zoxide.toml` 的修改仍未 commit

### Requirement: Pull
`rsmt pull` SHALL 取得遠端的變更並以 rebase 合併本機尚未推送的 commit，成功後依 `apply` 的流程套用到本機。拉取到的內容不改變本機 shell 載入的 script 時，SHALL 不啟動任何 shell。

#### Scenario: Another host added a module
- **WHEN** 另一台 host 推送了一個套用到所有主機的 module，使用者在本機執行 `rsmt pull`
- **THEN** 本機 repo 包含該 module，新開的 shell 已載入它

#### Scenario: Only another host's module came in
- **WHEN** 另一台 host 推送了一個只套用到它自己的 module，本機執行 `rsmt pull`
- **THEN** 本機 repo 包含該 module，過程中沒有啟動任何 shell

#### Scenario: Host that has never applied
- **WHEN** 使用者以 `rsmt init <url>` 加入 repo 後尚未執行 `rsmt apply`，接著執行 `rsmt pull`
- **THEN** repo 包含遠端的變更，rc 檔沒有被修改

### Requirement: Conflicts keep the local state
拉取時發生 rebase 衝突，rosamta SHALL 中止這次合併，讓 repo、generated script 與 rc 檔維持拉取前的狀態，並回報衝突的檔案。

#### Scenario: Same module edited on two hosts
- **WHEN** 兩台 host 都修改了同一個 module 的同一行，後者執行 `rsmt pull`
- **THEN** 指令以 1 結束並列出衝突的檔案，本機 repo 沒有處於 rebase 中途的狀態

### Requirement: Failed apply after pull
拉取成功但套用時 smoke test 失敗，rosamta SHALL 保留 repo 的新內容、維持原本生效的 generated script，並回報失敗的 module。

#### Scenario: Broken module from another host
- **WHEN** 拉取到的 module 讓本機的 zsh 啟動時出錯
- **THEN** 新開的 zsh 仍使用拉取前的設定，回報中指出該 module

### Requirement: Sync
`rsmt sync` SHALL 先執行拉取，成功後再執行推送。

#### Scenario: Both directions
- **WHEN** 本機有一個未推送的 commit，遠端也有一個新的 commit
- **THEN** `rsmt sync` 之後本機包含遠端的 commit，遠端也包含本機的 commit

### Requirement: Commit signing follows git config
rosamta 建立的每個 commit SHALL 依使用者的 git 設定決定是否簽章。需要簽章但簽章失敗時，rosamta SHALL 不建立未簽章的 commit，變更保留在工作目錄，並把推送狀態標記為受阻、說明原因。

#### Scenario: Pinentry unavailable
- **WHEN** 使用者設定 `commit.gpgsign = true`，背景中無法輸入 GPG 密碼
- **THEN** 沒有新的 commit 產生，推送狀態為受阻，原因指出簽章失敗

### Requirement: Offline
推送因網路或遠端無法連線而失敗時，rosamta SHALL 保留本機的 commit，並在下一次推送或同步時重試；狀態 SHALL 顯示尚未推送的 commit 數量。推送偏好為「自動」時，背景服務 SHALL 在 1 分鐘後重試；連續 3 次重試仍連不上之後，每次的等待 SHALL 加倍，最長 15 分鐘；電腦從睡眠喚醒、或 commit 已用其他方式推送出去後，SHALL 從 1 分鐘重新開始。沒有終端機時（背景服務、App），對遠端的拉取或推送超過 10 分鐘沒有結束 SHALL 被中止，並視為連不上遠端。

#### Scenario: No network
- **WHEN** 沒有網路時使用者接受一個接管項目，推送偏好為「自動」
- **THEN** commit 留在本機，狀態顯示有 1 個未推送的 commit，恢復連線後的下一次同步會把它推送出去

#### Scenario: Connection that stalls
- **WHEN** 背景服務推送時連線建立後不再有任何回應
- **THEN** 這次推送在 10 分鐘後被中止，狀態顯示連不上遠端，之後照常重試

#### Scenario: Offline for an hour
- **WHEN** 有 commit 等待推送，而電腦一小時都沒有網路
- **THEN** 背景服務前幾次每分鐘重試一次，之後間隔逐次拉長，最長 15 分鐘重試一次

### Requirement: Any git remote
rosamta SHALL 支援任何 git 能存取的遠端 URL，不限於 GitHub。

#### Scenario: Self-hosted Gitea
- **WHEN** 使用者以 `rsmt init https://git.example.com/me/dotfiles.git` 建立 repo
- **THEN** 推送與拉取都使用該遠端
