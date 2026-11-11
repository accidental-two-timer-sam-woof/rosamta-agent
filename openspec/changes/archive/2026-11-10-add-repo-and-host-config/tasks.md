# Tasks

## 1. 設定型別與驗證

- [x] 1.1 定義 `rosamta.toml`、module 與 `host.toml` 的型別（含 `deny_unknown_fields`），以範例檔撰寫解析測試並確認通過
- [x] 1.2 實作 `validate`：錯誤附檔案與行號、偵測重複的 module 名稱、未定義欄位與不合法的 `os`／`arch` 值，確認對應的單元測試通過
- [x] 1.3 實作以 `toml_edit` 寫回單一欄位，確認寫回後註解與欄位順序不變、且能重新解析
- [x] 1.4 以 `schemars` 產生 JSON Schema，確認範例檔都能通過 schema 驗證

## 2. Host 設定與路徑

- [x] 2.1 實作 XDG 路徑解析，確認未設定、已設定、相對路徑三種情況的測試通過
- [x] 2.2 實作 `host.toml` 的預設值與識別名稱產生，確認 hostname 改變後識別名稱不變
- [x] 2.3 實作標籤排序與去重，以及識別名稱改名時更新整個 repo 並 commit，確認在暫存 repo 上的整合測試通過

## 3. 套用範圍

- [x] 3.1 實作 `when` 的判斷規則，以表格驅動測試涵蓋「欄位內任一符合、欄位間全部符合」與沒有 `when` 的情況
- [x] 3.2 對不在 host 清單中的識別名稱輸出警告，確認 `rsmt validate` 仍以 0 結束

## 4. CLI

- [x] 4.1 建立 CLI 骨架：依 argv[0] 顯示命令名稱、`--json`、結束碼，確認以 `assert_cmd` 撰寫的測試涵蓋 `rosamta` 與 `rsmt` 兩個名稱
- [x] 4.2 實作 `rsmt init` 與 `rsmt init <git-url>`，以本機 bare repo 做整合測試，確認不會修改任何 rc 檔
- [x] 4.3 實作格式版本檢查與遷移框架，確認較舊格式會遷移並 commit、較新格式會拒絕
- [x] 4.4 實作 `rsmt validate` 與 `rsmt schema`，並在 README 撰寫這三個指令的用法，確認文件中的範例可照抄執行
