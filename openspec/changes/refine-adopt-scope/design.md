# Design

## Context

接管目前只在選「本機」時寫入 `when.hosts`；結構化 module 不設 `shells`，因此套用到所有 shell；與本機已載入的 module 相同的內容視為重新寫回，其他一律另建 module（名稱重複時加數字後綴）。背景服務依拉取間隔同步，推送因無法連線而失敗時不通知，等下一次同步才重試。新主機加入 repo 後，其他主機接管的 module 都只套用到原主機，沒有快速加入的方式。

## Goals / Non-Goals

**Goals:**
- 接管後的行為與接管前相同，放寬範圍一律是使用者的動作。
- 同一個工具在多台主機各裝一次，repo 中只有一份設定。
- 換電腦時一個指令就能把其他主機的設定加進新主機。

**Non-Goals:**
- 近似內容的合併（例如不同版本的路徑）。
- 改寫已經接管的 module。
- 監聽各 OS 的網路狀態。

## Decisions

### 套用範圍的寫法

`adopt::plan` 依範圍寫入：

| 範圍 | `when.hosts` | `when.os` | `shells` |
|---|---|---|---|
| 本機（預設） | 本機識別名稱 | 本機 OS | 來源檔案所屬的 shell |
| 所有主機 | — | — | 來源檔案所屬的 shell |

`shells` 對結構化與片段 module 都寫；片段 module 原本就只有來源 shell 的 `raw` 欄位，寫出 `shells` 讓範圍一眼可見。`module_toml` 加上 `os` 與 `shells` 的輸出。替代方案：只記錄主機。主機已隱含 OS，但記錄 OS 後，刪掉 `hosts` 就是「所有 macOS 主機」，放寬一次只動一個條件。

### 跨主機合併

掃描時，一段內容在判定為結構化或片段 module 之後、命名之前，先在 repo 的 module 中找「只套用到其他主機、內容相同」的 module（條件見 spec）。找到時候選項目的種類為 `include`，`module` 指向既有的檔案，`name` 為既有的名稱。

- 環境變數與 alias 以 `IndexMap` 比較（不看順序）；PATH 以清單比較，因為 prepend 的順序決定查找的優先順序。片段去掉註解行、空白行與行尾空白後逐行比較。
- 加入後是否載入：先把本機（與 OS）加進 `when` 的副本，再用 `When::matches` 判斷，不必先寫檔。
- 順序：module 的 `order` 必須大於本機目前從同一個檔案用到的最大順序，且在同一個檔案的範圍內（`order / 1000` 相同）；合併後，這個檔案接下來另建的 module 從該順序之後編號。另一台主機的順序來自它自己的檔案，若排在本機已接管的內容之前，合併會讓檔案中後面的內容先執行，因此改為另建 module。
- 計畫以 `toml_edit` 修改既有檔案（保留註解）：本機範圍加入識別名稱與 OS；所有主機範圍則移除 `hosts` 與 `os`，並把原本的值記在接管紀錄（`limits`）。
- 接管紀錄加上 `include: true`，待處理清單的項目也加上 `include`，讓 CLI 與 app 標示「加入本機」。

### 還原時的 module

還原一次接管時，module 依序判斷：

1. 本機另一筆接管紀錄仍指向這個 module（例如 `~/.zprofile` 與 `~/.zshrc` 的相同內容都合併進來）：不動。
2. 紀錄有 `limits`：放回原本的 `hosts` 與 `os`。
3. `hosts` 還列著其他主機：只移除本機。`os` 保留，因為其他主機可能是同一個 OS，而 `hosts` 已限制範圍。
4. 其他情況：刪除檔案。

建立 module 的主機與合併進來的主機適用同一套規則，因此先建立的主機還原時，不會刪掉其他主機還在用的 module。套用到所有主機的 module 無從得知其他主機是否在用，維持原本的行為（刪除）。

替代方案：以相似度比對（例如編輯距離）。版本號不同的路徑看起來幾乎相同，合併後其中一台會載入錯的路徑，因此只接受完全相同。

### recent 與 include

- 加入時間：一次 `git log --no-renames --diff-filter=A --name-only` 取得每個檔案最近一次加入 repo 的時間（移除後再加入、搬到其他資料夾都從那時算起）；dotfile 以 `source` 為準，目錄取其中最早的檔案。同一路徑有多個不在本機部署的 dotfile 項目時，列出加入本機需要放寬最少的一項，相同時取最新的。
- 是否在本機載入：module 以 `render::select`，dotfile 以同一套 `when` 判斷。
- 放寬：依 spec 的規則算出每項要加入的值，先全部列出；確認後以 `toml_edit` 修改 module 檔案與 `dotfiles.toml`，建立一個 commit，然後套用（`apply::apply_after`，失敗時還原檔案），依推送偏好推送。
- `--json` 輸出每項的名稱、種類、加入時間、套用範圍與要放寬的條件；需要確認的指令在 `--json` 下必須加 `--yes`，與 `rsmt adopt` 相同。

### 離線

- `report_problems` 中，推送的 `Offline` 且尚未推送的 commit 大於 0 時，產生 `offline` 問題；通知鍵不含數量，因此數量增加不會重複通知，推送成功後問題消失，下一次離線再通知。
- `tick` 在同步後檢查：推送偏好為自動、尚未推送的 commit 大於 0、上一次推送為 `Offline` 時，下一次同步排在 60 秒後；拉取偏好為手動時這次只推送。推送成功或失敗原因改變後回到原本的間隔。

### 預設接管偏好

`AdoptPolicy` 的預設值改為 `Auto`；已在 `host.toml` 寫明的值不變。設定頁與 README 跟著更新。

### App

總覽的「其他主機最近新增」與勾選加入、待處理頁的合併項目，實作前先以 artifact 預覽並經使用者確認。

## Risks / Trade-offs

- [行為改變] 新接管的結構化內容不再套用到其他 shell，使用者可能以為 bash 也會有 → 套用範圍明確寫在 module 中，`rsmt include` 與 GUI 可以放寬。
- [合併造成的放寬] 合併時 `os` 加入本機的 OS，刪掉 `hosts` 後會套用到兩種 OS → 這是使用者手動放寬時才發生，與 spec 的意圖一致。
- [離線每分鐘重試] 每分鐘一次推送嘗試的成本 → 只在有尚未推送的 commit 且上一次是無法連線時才發生。

## Migration Plan

既有 module 不改寫；只有之後的接管採用新範圍。既有 `host.toml` 的接管偏好不變。舊的結構化 module 沒有 `shells`（套用到所有 shell），與新接管的內容 shell 不同，因此不會被合併；使用者可以在舊 module 加上 `shells` 後讓它參與合併。
