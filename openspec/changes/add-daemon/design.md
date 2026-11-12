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

- macOS：`Rosamta.app/Contents/Library/LoginItems/RosamtaAgent.app` 是 helper，執行的是同一個 `rosamta` 執行檔的 `daemon run`。主程式（GUI）負責呼叫 `SMAppService.loginItem(identifier:).register()`；CLI 的 `rsmt daemon enable` 透過開啟主程式的 URL scheme 完成註冊。
- Linux：`rsmt daemon enable` 寫入 `~/.config/systemd/user/rosamta.service`，再執行 `systemctl --user enable --now rosamta`。沒有 systemd 使用者服務時說明原因，使用者可自行以 `rsmt daemon run` 執行。
- 單一實例：在狀態目錄對 `daemon.lock` 取得獨占的檔案鎖。

### 取得 login shell 的環境

啟動時執行 `$SHELL -l -i -c 'printf "%s\0" __ROSAMTA_ENV_START__; env -0; printf "%s\0" __ROSAMTA_ENV_END__'`，逾時 10 秒，只解析兩個 marker 之間的內容，以忽略 p10k instant prompt 等雜訊（VS Code 與 Zed 的做法）。設定 `ROSAMTA_RESOLVING_ENV=1`，讓 loader 跳過開 shell 提醒。失敗時退回 launchd 或 systemd 提供的環境，並在狀態中記錄。

### 檔案監看

用 `notify` 加 `notify-debouncer-full`（2 秒），監看 watched file 的上層目錄（非遞迴），再依路徑過濾；這樣改名寫入也不會讓監看失效。另外監看 repo 目錄（遞迴，排除 `.git/`）與 `host.toml`。rosamta 自己寫入檔案時，在狀態中記錄寫入後的檔案雜湊，監看到相同雜湊的事件就略過，避免自己觸發自己。

### 排程

拉取間隔用單調時鐘計時；macOS 透過 `NSWorkspace.didWakeNotification`、Linux 透過 logind 的 `PrepareForSleep` 訊號偵測喚醒，喚醒後立即同步一次。

### 執行檔更新

監看自己的執行檔路徑；檔案被替換後，等待目前的工作完成，再以 `exec` 重新啟動自己（macOS 由 launchd 依 `KeepAlive` 重啟）。

### 通知

- macOS：helper app 以 UNUserNotificationCenter 註冊一個 category，含「接管」、「忽略」、「查看差異」三個 action。「接管」、「忽略」在 helper 內直接處理；「查看差異」以 `rosamta://review/<id>` 開啟主程式。
- Linux：以 D-Bus 通知並附上 action；`get_capabilities()` 沒有 `actions` 時只發純文字通知；D-Bus 上沒有通知服務時不發通知。
- 問題類通知以「類型＋原因」為鍵記錄在狀態中，問題解決前不重複發送。
- 文字依系統語系選擇英文或繁體中文，字串與 GUI 共用同一份翻譯檔。

### 不做 IPC

GUI 與 CLI 只讀寫設定、佇列與狀態檔；背景服務監看這些檔案的變化。修改共用檔案時一律取得檔案鎖，寫入以「寫暫存檔再改名」的方式完成。

## Risks / Trade-offs

- [spike 證實 helper 無法顯示按鈕] → 依 ADR 0004 的退路，由主程式發通知；需要時主程式可在背景短暫啟動。
- [login shell 撈環境變數時卡住或很慢] → 10 秒逾時並退回系統環境，狀態中提示使用者。
- [大量檔案事件，例如套件管理器一次改很多檔] → 只監看 watched file 的上層目錄並依路徑過濾，加上 2 秒 debounce。
- [背景執行 GPG 簽章時跳出 pinentry 視窗] → 這是使用者既有的設定；簽章失敗時依 `add-git-sync` 標記受阻並通知。
