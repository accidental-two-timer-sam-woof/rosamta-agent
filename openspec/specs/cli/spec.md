# cli Specification

## Purpose
定義 rosamta 命令列介面的共同行為，包括命令名稱、機器可讀輸出、結束碼與訊息語言，讓使用者、GUI 與 AI 都能以一致的方式呼叫。

## Requirements

### Requirement: Two command names
rosamta SHALL 以 `rosamta` 與 `rsmt` 兩個名稱提供同一個命令列程式，兩者行為完全相同。說明與錯誤訊息 SHALL 使用使用者實際輸入的名稱。

#### Scenario: Invoked as rsmt
- **WHEN** 使用者執行 `rsmt --help`
- **THEN** 說明中的用法列以 `rsmt` 開頭

#### Scenario: Same behavior under both names
- **WHEN** 使用者分別執行 `rosamta validate` 與 `rsmt validate`
- **THEN** 兩者的輸出與結束碼相同

### Requirement: Machine-readable output
會產生結果的子指令 SHALL 支援 `--json`，此時 stdout 只輸出一個 JSON 文件，不混入其他文字；進度與警告寫到 stderr。

#### Scenario: JSON output
- **WHEN** 使用者執行 `rsmt validate --json`
- **THEN** stdout 只包含一個可解析的 JSON 文件，描述每個檔案的驗證結果

### Requirement: Exit codes
CLI SHALL 在成功時以 0 結束，使用者輸入或設定錯誤時以 2 結束，其他失敗以 1 結束。

#### Scenario: Invalid config
- **WHEN** repo 中有一個格式錯誤的 module 檔，使用者執行 `rsmt validate`
- **THEN** 指令以 2 結束，stderr 指出檔案路徑與行號

#### Scenario: Unexpected failure
- **WHEN** repo 目錄沒有讀取權限
- **THEN** 指令以 1 結束，並說明無法讀取的路徑

### Requirement: English messages
CLI 的訊息 SHALL 只使用英文，不隨系統語系改變。

#### Scenario: zh-TW locale
- **WHEN** 系統語系為 `zh_TW.UTF-8`，使用者執行任一子指令
- **THEN** 輸出的訊息為英文
