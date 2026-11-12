# GUI 用 GPUI，透過 gpui-component 引入並鎖定版本

crates.io 上的 `gpui` 停在 2025-10 的 0.2.2，GPUI 沒有提供 semver 保證的正式版本。我們透過 Longbridge 的 `gpui-component` 引入 GPUI：它提供現成的輸入框、列表、表格與開關，並鎖定 `gpui-pre`，這是一份 Zed 原始碼的 snapshot，但不是 Zed 團隊發布的。GUI 用不到 GPUI 的通知或 Accessory API，這些都交給 daemon。依賴鎖定確切版本，升級一律當成獨立的 change 處理。

## Considered Options

- 直接 git 依賴 Zed repo 並鎖定 commit：元件得全部自己寫。
- Tauri 2：tray 與桌面通知都較成熟，但使用者選擇維持 GPUI。
