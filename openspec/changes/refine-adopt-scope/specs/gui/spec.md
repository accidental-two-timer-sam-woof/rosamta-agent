## MODIFIED Requirements

### Requirement: Overview
總覽 SHALL 顯示需要處理的項目、背景服務、接管、推送、拉取與遠端的狀態、各 shell 是否已掛載，以及最近活動，並提供立即同步。有其他主機最近新增、本機尚未套用的項目時，總覽 SHALL 顯示其數量，點擊後列出這些項目，使用者勾選後 SHALL 能把本機加進它們的套用範圍，結果與 `rsmt include` 相同。

#### Scenario: Sync now
- **WHEN** 使用者在總覽按「同步」
- **THEN** rosamta 執行拉取與推送，完成後上次同步時間更新

#### Scenario: Include from another host
- **WHEN** 新主機的總覽顯示其他主機最近新增 3 項，使用者勾選其中 2 項並加入
- **THEN** 結果與對這 2 項執行 `rsmt include --yes` 相同，總覽改為顯示 1 項

### Requirement: Review page
待處理頁 SHALL 列出所有項目；選取的項目 SHALL 顯示 diff，rc 檔段落另顯示會產生的模組，與其他主機的 module 相同時改為顯示該 module 套用範圍的變更。使用者 SHALL 能接管（套用範圍為本機或所有主機）、忽略，dotfile 項目另可採用 repo 版本。`rosamta://review/<id>` SHALL 開啟此頁並選取該項目。

#### Scenario: Open from a notification
- **WHEN** 使用者點通知的「查看差異」，通知對應第 3 項
- **THEN** GUI 開啟待處理頁並選取第 3 項

#### Scenario: Adopt for all hosts
- **WHEN** 使用者把第 1 項的套用範圍切到所有主機並按「接管」
- **THEN** 結果與執行 `rsmt review accept 1 --scope all` 相同，項目從清單移除

#### Scenario: Same as another host
- **WHEN** 選取的項目與 `mbp` 的 `bun` module 相同
- **THEN** 頁面顯示 `bun` 的 `hosts` 會加入本機，而不是新的 module
