# 連結 GitHub 用 OAuth App 的 device flow

使用者在 GUI 按「連結到 GitHub」後，以 device flow 授權，不需要先安裝 `gh`。我們為 rosamta 註冊一個 OAuth App（device flow 只需要 client ID），token 存在 OS keychain，只用來列出 repo 與建立 private repo。git 傳輸優先走使用者既有的 SSH；SSH 連不上時才用這個 token 走 HTTPS，由 rosamta 自己的 credential helper 提供，不改使用者全域的 git 設定。

## Consequences

- 建立 private repo 需要 `repo` scope，它能存取使用者所有的 private repo。這是為了讓流程簡單而接受的權限範圍。

## Considered Options

- GitHub App 搭配細粒度權限：權限可以限縮到單一 repo，但「幫使用者建立新 repo」的流程較複雜。
