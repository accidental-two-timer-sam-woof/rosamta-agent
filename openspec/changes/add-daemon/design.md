# Design

## Context

ADR 0004：macOS 的 UNUserNotificationCenter 只能在有 app bundle 的程序中使用，因此 daemon 是主程式內的 helper app；使用者有 Apple 開發者帳號可以簽章。研究結果：launchd 不讀 shell 的 rc 檔；`notify` crate 在 inotify 上監看單一檔案時，改名寫入會讓監看失效。daemon、CLI、GUI 之間不做 IPC，以檔案與檔案鎖溝通（Q14）。

## Goals / Non-Goals

**Goals:**
- 使用者不做任何事，rc 檔的變動也會在數秒內被處理或詢問。
- 背景服務的任何錯誤都不修改 rc 檔、不切換 generated script。
- 在沒有桌面環境的 Linux 上也能運作。

**Non-Goals:**
- menu bar 圖示（使用者決定不需要）。
- Windows。

## Decisions

### 先做 spike

在動工前另開分支，做一個最小的 `.app`：內含 `LSUIElement` 的 helper app，以 SMAppService 註冊為登入項目，由 helper 透過 UNUserNotificationCenter 發出帶三個按鈕的通知並接收點擊。結果寫回 ADR 0004。若不可行，退路是 helper 透過檔案通知主程式，由主程式發出通知。

### 程序結構

- macOS：`rosamta.app/Contents/Library/LoginItems/rosamta-agent.app` 是 helper，執行的是同一個 `rosamta` 執行檔的 `daemon run`。LaunchAgent 的 plist 放在 `rosamta.app/Contents/Library/LaunchAgents/`，以 `BundleProgram` 指向 helper，`KeepAlive` 讓 launchd 在程序結束後重新啟動它。`rsmt daemon enable` 直接呼叫 `SMAppService.agent(plistName:).register()`：SMAppService 需要從 app bundle 內的執行檔呼叫，而 Homebrew 的 `rsmt` 是指向 app 內 CLI 的 symlink，從 symlink 啟動時 macOS 認不出 bundle，所以 CLI 先改用實際路徑重新執行自己。不需要經過 GUI 的 URL scheme。
- helper 不能放在 `/tmp` 底下：實測在 `/private/tmp` 中的 app 不會被通知中心登記，授權要求會直接失敗。和 spike 一樣，先建立 `NSApplication` 再使用 UNUserNotificationCenter。
- Linux：`rsmt daemon enable` 寫入 `~/.config/systemd/user/rosamta.service`，再執行 `systemctl --user enable --now rosamta`。沒有 systemd 使用者服務時說明原因，使用者可自行以 `rsmt daemon run` 執行。
- 單一實例：在狀態目錄對 `daemon.lock` 取得獨占的檔案鎖。執行中的實例把 pid、啟動時間、版本與已通知的問題寫在 `daemon.json`。
- 尚未執行 `rsmt init` 時，背景服務等到 `host.toml` 與 repo 出現才開始監看，避免在 launchd 的 KeepAlive 下反覆啟動失敗。

### 取得 login shell 的環境

啟動時執行 `$SHELL -l -i -c 'echo __ROSAMTA_ENV_START__; env -0; echo __ROSAMTA_ENV_END__'`，逾時 10 秒，只解析兩個 marker 之間的內容，以忽略 p10k instant prompt 與開 shell 提醒等雜訊（VS Code 與 Zed 的做法）；用 `echo` 印 marker，fish 也能執行同一行。`PWD`、`SHLVL` 等描述擷取用 shell 本身的變數，以及 `ROSAMTA_` 開頭的變數不沿用：沿用 `ROSAMTA_ENV` 會讓背景服務啟動的 shell 跳過 env 階段。失敗時退回 launchd 或 systemd 提供的環境，並記錄在 `daemon.json`，`rsmt daemon status` 會顯示原因。

### 檔案監看

用 `notify` 加 `notify-debouncer-full`（2 秒），監看 watched file 的上層目錄（非遞迴）；這樣改名寫入也不會讓監看失效。另外監看 repo 目錄與設定目錄（遞迴，含 `host.toml` 與本機 module），repo 中 `.git/` 的事件不處理。監看與比對都用解析過 symlink 的實際路徑，因為 macOS 的 FSEvents 回報 `/private/var/...`。每次掃描後記錄 watched file 內容的雜湊，事件到來時雜湊沒變就略過：rosamta 自己接管後改寫 rc 檔，不會再觸發一次掃描。repo 也一樣，以內容雜湊判斷是否需要重新套用。

### 排程

拉取間隔用單調時鐘計時。喚醒的偵測不依賴平台 API：每 5 秒比較一次牆上時鐘與單調時鐘經過的時間，兩者在 macOS 與 Linux 上的單調時鐘睡眠時都不前進，牆上時鐘卻會；牆上時鐘多走超過 60 秒就視為剛從睡眠喚醒，立即同步一次。

### 執行檔更新

每 5 秒檢查一次啟動時的執行檔路徑（device、inode、修改時間）；檔案被替換後，等目前的工作完成，再以同一個路徑 `exec` 新版本，pid 不變。路徑要在啟動時記下：Linux 上檔案被取代後，再問 `/proc/self/exe` 會得到 `... (deleted)`。

### 通知

- macOS：helper app 以 UNUserNotificationCenter 註冊一個 category，含「接管」、「忽略」、「查看差異」三個 action。「接管」、「忽略」在 helper 內直接處理；「查看差異」以 `rosamta://review/<id>` 開啟主程式。
- Linux：以 D-Bus 通知並附上 action；`get_capabilities()` 沒有 `actions` 時只發純文字通知；D-Bus 上沒有通知服務時不發通知。
- 問題類通知以「類型＋原因」為鍵記錄在 `daemon.json`，問題解決前不重複發送；問題消失後再出現會再通知一次。
- 新的待處理項目在 10 秒內合併成一則通知。每則通知也寫進 `daemon.log`，沒有通知服務的主機也能從記錄確認。
- 文字依系統語系選擇英文或繁體中文（macOS 讀使用者的偏好語言，launchd 不提供 `LANG`），字串放在 `rosamta-core/locales/*.toml`，GUI 共用。

### 不做 IPC

GUI 與 CLI 只讀寫設定、佇列與狀態檔；背景服務監看這些檔案的變化。修改共用檔案時一律取得檔案鎖，寫入以「寫暫存檔再改名」的方式完成。

## Risks / Trade-offs

- [spike 證實 helper 無法顯示按鈕] → 依 ADR 0004 的退路，由主程式發通知；需要時主程式可在背景短暫啟動。
- [login shell 撈環境變數時卡住或很慢] → 10 秒逾時並退回系統環境，狀態中提示使用者。
- [大量檔案事件，例如套件管理器一次改很多檔] → 只監看 watched file 的上層目錄並依路徑過濾，加上 2 秒 debounce。
- [背景執行 GPG 簽章時跳出 pinentry 視窗] → 這是使用者既有的設定；簽章失敗時依 `add-git-sync` 標記受阻並通知。
