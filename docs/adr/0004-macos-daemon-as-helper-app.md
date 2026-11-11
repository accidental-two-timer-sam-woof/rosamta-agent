# macOS 的 daemon 是包在主程式裡的簽章 helper app

詢問模式需要帶按鈕的通知。macOS 的 UNUserNotificationCenter 只能在有 app bundle 的程序裡使用，由 launchd 直接啟動的 CLI 執行檔只能用 macOS 11 起已 deprecated 的 NSUserNotification。所以 daemon 做成主程式 `.app` 內的 helper `.app`：設 `LSUIElement`，沒有 Dock 圖示、視窗或 menu bar 圖示；以 SMAppService 註冊為登入項目，並用開發者帳號簽章。Linux 則用 systemd `--user` 執行同一個 daemon，通知走 D-Bus。

## Consequences

- launchd 透過 SMAppService 註冊的 LaunchAgent 啟動 helper：plist 放在主程式的 `Contents/Library/LaunchAgents/`，以 `BundleProgram` 指向 helper 的執行檔，helper 因此帶著自己的 bundle ID 執行。
- 通知權限屬於 helper 的 bundle，系統設定的「通知」中顯示的是 helper 的名稱，所以 helper 的 `CFBundleName` 要是 rosamta。第一次的允許提示若被關掉，系統會記成不允許，之後的要求不再提示、直接失敗（`UNErrorDomain` code 1）。daemon 要偵測這個狀態，退回待處理清單與開 shell 提醒，並由 GUI 引導使用者到系統設定開啟通知。
- launchd 不會讀 shell 的 rc 檔，daemon 啟動時要從 login shell 讀取一次環境變數，才拿得到 `SSH_AUTH_SOCK` 與完整的 PATH。

## Spike 結果

2026-11-10 在 macOS 27.3（26C5035r）上驗證（分支 `spike/macos-helper`）：

- 以 Apple Development 憑證簽章、設 `LSUIElement` 的 helper，透過 UNUserNotificationCenter 發出帶「接管」、「忽略」、「查看差異」三個 action 的通知。按下「接管」與「查看差異」（Foreground）後，helper 的 delegate 都收到點擊，回呼在主執行緒上。
- SMAppService 註冊後狀態直接是已啟用，這次沒有要求使用者核准登入項目。

同日以實際的 `rosamta.app` 打包再驗證一次：`rsmt daemon enable` 註冊後，launchd 啟動 helper 中的 `rosamta daemon run`。在通知上按「接管」會接管該項目；按「查看差異」會以 `rosamta://review/<id>` 打開主程式。app 放在 `/private/tmp` 底下時，通知中心不會登記 helper，授權要求直接失敗；搬到一般目錄後就正常。

尚未驗證：macOS 13、以 Developer ID 簽章並公證後安裝在 `/Applications` 的版本，以及系統要求核准登入項目時的流程。
