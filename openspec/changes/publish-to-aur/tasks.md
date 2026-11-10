# Tasks

## 1. AUR

- [ ] 1.1 AUR 開放註冊後建立帳號，產生專用的 SSH key 並把公鑰加入帳號，設定 `AUR_SSH_PRIVATE_KEY`、`AUR_USERNAME` 與 `AUR_EMAIL`
- [ ] 1.2 發布第一個正式版本時，確認 `aur` job 成功，且 `rosamta-bin` 在 Arch 容器中以 `makepkg -si` 安裝後 `rosamta` 與 `rsmt` 都能執行
