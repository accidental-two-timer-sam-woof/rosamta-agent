## 1. 重現

- [ ] 1.1 為 #34、#35、#36、#37、#38 與 `.zlogin` 各寫一個目前會失敗的測試：接管前後，同一種啟動方式下的環境、alias、shell 選項與執行次數相同

## 2. 資料模型

- [ ] 2.1 `Module` 加上 `file`；`rsmt validate` 檢查它只出現在只有 `raw` 的 module，且不與 `phase` 並存；更新 JSON Schema
- [ ] 2.2 `watched_files` 以來源檔取代階段；接管產生的片段寫入 `file`
- [x] 2.3 執行 `.bashrc` 的行（`. ~/.bashrc`、`source ~/.bashrc`）不接管
- [ ] 2.4 執行 `.profile` 的行（`. ~/.profile`、`emulate sh -c '. ~/.profile'`）不接管

## 3. 產生與載入

- [ ] 3.1 每個來源檔產生一份 script；`.profile` 的片段只產生在 sh
- [ ] 3.2 每個 watched file 的 hook 載入自己的那一份；加上 `.zlogin` 與 `.bash_login` 的 hook
- [x] 3.3 巢狀 shell 重新執行 env 階段的片段（zsh、fish 的每個 shell 與 bash 的 login shell），匯出的環境變數不重複套用（#35）
- [ ] 3.5 刪除 `NOT_BASH` 與 `profile_readers`
- [ ] 3.4 `.bash_profile` 的 hook 在互動 shell 載入 `rc.bash`，以不 export 的變數避免同一個程序載入兩次；不再替既有的 `.bash_profile` 加上執行 `.bashrc` 的行；新建的 `.bash_profile` 以 `.bashrc` hook 的記號判斷 `.bashrc` 是否已經執行過

## 4. 格式 2

- [ ] 4.1 migration：從接管紀錄與 module 的註解取回來源檔；無法判斷時保留 `phase` 並由 `rsmt validate` 列出
- [ ] 4.2 migration 前後比對每個 shell 的 login 互動狀態，不同時還原並回報；其他啟動方式的差異列在輸出中

## 5. 驗證

- [ ] 5.1 9 個發行版的 docker 測試全過
- [ ] 5.2 在你的 Mac 上以暫存的 HOME 複製目前的 repo 執行 migration，比對 zsh 與 bash 的 login、互動與巢狀狀態
- [ ] 5.3 更新 README 與 GLOSSARY；關閉 #34–#38
