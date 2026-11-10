# macOS 的 daemon 是包在主程式裡的簽章 helper app

詢問模式需要帶按鈕的通知。macOS 的 UNUserNotificationCenter 只能在有 app bundle 的程序裡使用，由 launchd 直接啟動的 CLI 執行檔只能用 macOS 11 起已 deprecated 的 NSUserNotification。所以 daemon 做成主程式 `.app` 內的 helper `.app`：設 `LSUIElement`，沒有 Dock 圖示、視窗或 menu bar 圖示；以 SMAppService 註冊為登入項目，並用開發者帳號簽章。Linux 則用 systemd `--user` 執行同一個 daemon，通知走 D-Bus。

## Consequences

- helper 發出的通知能否正常顯示按鈕仍未驗證，實作前要先做 spike。若不可行，退路是由 helper 通知主程式，改由主程式發出通知。
- launchd 不會讀 shell 的 rc 檔，daemon 啟動時要從 login shell 讀取一次環境變數，才拿得到 `SSH_AUTH_SOCK` 與完整的 PATH。
