# age 私鑰以密碼加密後放進 repo

加密的 dotfile 要在每台 host 上解開，就需要同一把 age 私鑰。rosamta 把私鑰用使用者設定的密碼（age 的 scrypt）加密後放進 repo；新 host 第一次設定時輸入一次密碼，解開後存成本機的 `~/.config/rosamta/age.key`（權限 0600），之後不再詢問。這樣不必依賴任何密碼管理工具，也不需要使用者自己在機器間複製檔案。

## Considered Options

- 私鑰存在 Bitwarden，bootstrap 時透過 rbw 或 bw 取回：留到之後，因為會讓解密依賴 Bitwarden。
- 使用者自行複製私鑰：容易遺漏，也容易用不安全的方式傳送。
