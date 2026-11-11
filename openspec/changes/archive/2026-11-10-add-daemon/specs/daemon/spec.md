# Spec Delta

## Purpose

在使用者登入後常駐背景，於 rc 檔或 repo 變動時自動執行接管、套用與同步，讓使用者不需要記得手動操作。

## ADDED Requirements

### Requirement: Runs at login
`rsmt daemon enable` SHALL 讓背景服務在使用者登入後自動啟動：macOS 以登入項目註冊主程式內的 helper app，Linux 以 systemd 使用者服務註冊。系統要求使用者核准時，SHALL 說明如何到系統設定中允許。

#### Scenario: macOS approval required
- **WHEN** 使用者在 macOS 上第一次執行 `rsmt daemon enable`
- **THEN** 指令說明需要到「系統設定 › 一般 › 登入項目」允許 rosamta，`rsmt daemon status` 顯示等待核准

#### Scenario: Linux
- **WHEN** 使用者在有 systemd 使用者服務的 Linux 上執行 `rsmt daemon enable`
- **THEN** 背景服務立即啟動，且重新登入後仍會自動啟動

### Requirement: Single instance
同一個使用者同時 SHALL 只有一個背景服務在執行；第二個實例 SHALL 偵測到後立即結束。

#### Scenario: Started twice
- **WHEN** 背景服務已在執行時，使用者再執行 `rsmt daemon run`
- **THEN** 新的程序說明已有實例在執行並以 1 結束

### Requirement: Waits for setup
背景服務在尚未執行 `rsmt init` 的 host 上啟動時，SHALL 持續執行並等待，在 repo 與 `host.toml` 出現後開始監看，而不是結束後被反覆重新啟動。

#### Scenario: Enabled before init
- **WHEN** 背景服務在使用者執行 `rsmt init` 之前啟動，之後使用者執行 `rsmt init`
- **THEN** 背景服務不需要重新啟動，就開始處理 rc 檔的變動

### Requirement: Login shell environment
背景服務啟動時 SHALL 從使用者的 login shell 讀取一次環境變數，並以這組環境執行 git 與 shell 相關的操作。

#### Scenario: SSH agent set in zprofile
- **WHEN** 使用者在 `~/.zprofile` 設定 `SSH_AUTH_SOCK` 指向 Bitwarden 的 SSH agent
- **THEN** 背景服務推送到 GitHub 時使用該 agent

### Requirement: React to rc file changes
watched file 被修改後，背景服務 SHALL 在變動停止約 2 秒後掃描一次，並依接管偏好處理新內容。安裝程式以「寫新檔再改名」的方式寫入時 SHALL 同樣被偵測到。

#### Scenario: Installer appends to zshrc
- **WHEN** 接管偏好為「詢問」，某個安裝程式在 `~/.zshrc` 加了兩行
- **THEN** 數秒內待處理清單出現這兩行，rc 檔不變

#### Scenario: Atomic rename write
- **WHEN** 某個程式以寫入暫存檔再改名的方式更新 `~/.bashrc`
- **THEN** 背景服務仍偵測到變動並掃描

### Requirement: React to repo changes
repo 的內容被修改（使用者手動編輯或拉取）後，背景服務 SHALL 驗證並套用；驗證失敗時 SHALL 不套用並通知。推送偏好為「自動」時 SHALL 接著推送。

#### Scenario: Manual edit
- **WHEN** 使用者在編輯器中修改 `modules/aliases.toml` 並存檔
- **THEN** 新開的 shell 已使用新的 alias，且推送偏好為「自動」時遠端也已更新

### Requirement: Periodic sync
拉取偏好為「自動」時，背景服務 SHALL 依設定的間隔執行同步；電腦從睡眠喚醒後 SHALL 立即同步一次。

#### Scenario: Interval
- **WHEN** 拉取間隔為 15 分鐘
- **THEN** 背景服務大約每 15 分鐘執行一次同步

### Requirement: Settings reload
`host.toml` 被修改後，背景服務 SHALL 不需重新啟動就套用新的設定。

#### Scenario: Change adopt policy
- **WHEN** 使用者在 GUI 把接管偏好改成「自動」
- **THEN** 下一次 rc 檔變動時直接接管，不再加入待處理清單

### Requirement: Restart after upgrade
rosamta 的執行檔被新版本取代後，背景服務 SHALL 自行重新啟動成新版本。

#### Scenario: Upgrade via Homebrew
- **WHEN** 使用者以 Homebrew 升級 rosamta
- **THEN** `rsmt daemon status` 顯示的版本為新版本

### Requirement: Status
`rsmt daemon status` SHALL 顯示是否執行中、啟動時間、版本、上次同步的時間與結果，以及待處理項目的數量；支援 `--json`。

#### Scenario: Status output
- **WHEN** 使用者執行 `rsmt daemon status --json`
- **THEN** stdout 是包含上述欄位的 JSON 文件
