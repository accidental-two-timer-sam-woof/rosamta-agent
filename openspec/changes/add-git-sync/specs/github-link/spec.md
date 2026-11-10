# Spec Delta

## Purpose

讓使用者不需要事先安裝或設定任何 GitHub 工具，就能把 rosamta 連結到自己的 GitHub 帳號、建立或選擇存放設定的 repo。

## ADDED Requirements

### Requirement: Device flow sign-in
`rsmt remote github login` SHALL 以 GitHub 的 device flow 登入：顯示一組驗證碼、開啟瀏覽器到 GitHub 的驗證頁，並在使用者授權後完成登入；驗證碼逾時或使用者拒絕時 SHALL 回報失敗且不儲存任何資料。

#### Scenario: Successful sign-in
- **WHEN** 使用者執行 `rsmt remote github login` 並在瀏覽器中輸入驗證碼、按下授權
- **THEN** 指令顯示已連結的 GitHub 帳號名稱並以 0 結束

#### Scenario: Code expired
- **WHEN** 使用者在驗證碼逾時之前沒有完成授權
- **THEN** 指令以 1 結束，系統 keychain 中沒有新的 token

### Requirement: Token storage
GitHub token SHALL 存在系統的 keychain（macOS Keychain、Linux Secret Service）。系統沒有可用的 keychain 時，SHALL 存在 rosamta 設定目錄中權限為 0600 的檔案，並警告使用者。

#### Scenario: Headless Linux
- **WHEN** 在沒有 Secret Service 的 Linux 伺服器上登入
- **THEN** token 存在權限 0600 的檔案中，指令輸出一則說明儲存位置的警告

### Requirement: Create or choose a repo
連結 GitHub 後，`rsmt init --github <name>` SHALL 在使用者的帳號下建立指定名稱的 private repo 並設為遠端；`rsmt init --github <owner>/<name>` SHALL clone 既有的 repo。

#### Scenario: New private repo
- **WHEN** 使用者執行 `rsmt init --github dotfiles`
- **THEN** GitHub 上出現一個名為 `dotfiles` 的 private repo，本機 repo 的遠端指向它

### Requirement: Prefer SSH transport
設定 GitHub 遠端時，使用者的 SSH 金鑰能通過 GitHub 驗證，rosamta SHALL 使用 SSH URL；否則 SHALL 使用 HTTPS URL，並只在該 repo 的 git 設定中加入 rosamta 的 credential helper，不修改使用者全域的 git 設定。

#### Scenario: SSH available
- **WHEN** 使用者的 SSH agent 中有已加入 GitHub 的金鑰
- **THEN** 遠端 URL 是 `git@github.com:` 開頭

#### Scenario: SSH unavailable
- **WHEN** 使用者沒有可用的 SSH 金鑰
- **THEN** 遠端 URL 是 `https://github.com/` 開頭，推送時使用已儲存的 token，`~/.gitconfig` 沒有被修改

### Requirement: Sign out
`rsmt remote github logout` SHALL 刪除已儲存的 token，並移除 rosamta 加在 repo 中的 credential helper 設定。

#### Scenario: Logout
- **WHEN** 使用者執行 `rsmt remote github logout`
- **THEN** keychain 中不再有 rosamta 的 GitHub token
