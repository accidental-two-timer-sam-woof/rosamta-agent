# Tasks

## 1. Spike（可丟棄）

- [x] 1.1 在獨立分支做最小的簽章 `.app` 與 helper app，以 SMAppService 註冊並由 helper 發出帶三個按鈕的通知，確認在 macOS 13 與最新版上都能顯示按鈕並收到點擊
- [x] 1.2 把 spike 的結果與限制寫回 ADR 0004，若不可行則記錄改採的退路

## 2. 程序與生命週期

- [ ] 2.1 實作 `rsmt daemon run`、單一實例鎖與 `status --json`，確認第二個實例會以 1 結束
- [ ] 2.2 實作 login shell 環境擷取（marker、逾時、退路），以含 p10k instant prompt 的 zsh 設定測試能正確取得 `SSH_AUTH_SOCK`
- [ ] 2.3 實作 Linux 的 `daemon enable`、`disable`（systemd 使用者服務），在有 systemd 的容器或 VM 中確認登入後自動啟動
- [ ] 2.4 實作 macOS 的 helper app 打包、LaunchAgent plist 與透過主程式註冊登入項目，確認等待核准時 `status` 顯示正確狀態
- [ ] 2.5 實作執行檔更新後自動重新啟動，確認替換執行檔後 `status` 顯示新版本

## 3. 監看與觸發

- [ ] 3.1 實作 watched file 上層目錄監看與 2 秒 debounce，確認一般寫入與改名寫入都會觸發掃描
- [ ] 3.2 實作 rosamta 自身寫入的略過機制，確認接管修改 rc 檔後不會再觸發一次掃描
- [ ] 3.3 實作 repo 變動觸發驗證、套用與（依偏好）推送，確認手動編輯 module 後新 shell 生效
- [ ] 3.4 實作 `host.toml` 重新載入與定時同步、喚醒後同步，確認改變接管偏好後立即生效

## 4. 通知

- [ ] 4.1 實作 macOS 通知 category 與三個 action，「查看差異」以 `rosamta://review/<id>` 開啟主程式，在實機上確認
- [ ] 4.2 實作 Linux D-Bus 通知（含 action 能力檢查與無通知服務的退路），在 GNOME 與無桌面的容器中確認
- [ ] 4.3 實作通知合併與問題類通知去重，確認 10 秒內三個變動只產生一則通知、同一原因不重複通知
- [ ] 4.4 建立英文與繁體中文的通知字串並依系統語系選擇，把所有通知文字列出給使用者審閱

## 5. 文件

- [ ] 5.1 在 README 說明啟用背景服務、macOS 登入項目核准與 Linux 無 systemd 時的做法，確認範例可照抄執行
