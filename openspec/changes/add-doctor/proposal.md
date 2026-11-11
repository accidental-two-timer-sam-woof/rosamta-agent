# Proposal

## Why

使用者既有的 shell 設定常有重複的 init、互相覆蓋的 PATH、拖慢啟動的工具，自己很難發現。診斷把這些問題與最佳化建議變成可以直接套用的修正，讓使用者不必研究就感受到 shell 變快、變乾淨；這也是專題的特色重點。

## What Changes

- 診斷規則：
  - 交接文件列出的衝突規則（PowerShell 相關的延後到 Windows 支援時）。
  - 新增四條：同一行執行多次、同一變數在不同檔案有不同值、PATH 寫在別的 shell 讀不到的檔案、指向不存在的路徑。
  - 每一項提供修正，套用前顯示 diff，可還原；有多個可能值時列出所有偵測到的值讓使用者選擇。
- 修正分成兩類：不動使用者自己寫的內容的修正自動套用；會動到的先確認。
- 最佳化（init 輸出的快取已由 `add-providers` 預設提供）：
  - 整理 PATH。
  - `compinit` 只執行一次。
  - 以 `zcompile` 預先編譯 zsh 腳本。
  - nvm、conda 拖慢啟動超過 100 ms 時，建議改為延遲載入。
  - 每一項最佳化都量測套用前後的啟動時間。
- 執行時機：每次 render 之後、手動執行 `rsmt doctor`，以及設定精靈的「掃描與診斷」步驟。

## Capabilities

### New Capabilities

- `doctor`: 診斷規則、修正、最佳化與量測。

### Modified Capabilities

- `gui`: 在 app 中加入診斷的頁面或步驟。

## Impact

- 新增 `rsmt doctor`（`--json`、`--fix <id>`）。
- 依賴 `add-providers` 的互斥、compinit 與 provider 資訊。
