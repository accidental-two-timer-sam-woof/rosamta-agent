## 1. 套用範圍

- [x] 1.1 接管依範圍寫入 `hosts`、`os` 與 `shells`，結構化 module 只套用到來源檔案所屬的 shell，`module_toml` 輸出 `os` 與 `shells`；更新受影響的測試
- [x] 1.2 預設接管偏好改為自動，既有設定不變；更新 README 與測試

## 2. 跨主機合併

- [ ] 2.1 掃描時找出只套用到其他主機、內容相同的 module，產生 `include` 候選（結構化比較集合、片段比較正規化文字、shell 相同、加入後會在本機載入、秘密不合併）
- [ ] 2.2 計畫與執行：以 `toml_edit` 修改既有 module（本機範圍加入主機與 OS，所有主機範圍移除兩者），commit 訊息 `adopt(<shell>): include …`，接管紀錄標記 include
- [ ] 2.3 還原 include：放回原行並從 `hosts` 移除本機；兩台主機的整合測試涵蓋合併、不同版本、不同 shell 與還原

## 3. 離線

- [ ] 3.1 背景服務在有尚未推送的 commit 且上一次推送無法連線時每分鐘重試，拉取偏好為手動時只推送
- [ ] 3.2 離線通知只發一次，推送成功後重置；語系檔加入文字；測試

## 4. recent 與 include

- [ ] 4.1 core：每個 module 與 dotfile 加入 repo 的時間、不在本機載入的項目、放寬計畫與執行（`toml_edit`、一個 commit、套用失敗時還原）
- [ ] 4.2 CLI：`rsmt recent`、`rsmt include <名稱>... | --all`，確認、`--yes`、`--json` 與結束碼；測試

## 5. App

- [ ] 5.1 以 artifact 預覽總覽的最近新增與勾選加入、待處理頁的合併項目，經使用者確認
- [ ] 5.2 實作並更新截圖與 view 測試

## 6. 收尾

- [ ] 6.1 fmt、clippy、全部測試、Linux distro、`cargo deny`；更新 README 與 GLOSSARY，列出所有使用者看得到的文字供審閱
