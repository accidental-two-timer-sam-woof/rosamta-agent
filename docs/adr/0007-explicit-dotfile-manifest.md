# Dotfile 的屬性寫在 dotfiles.toml，不編在檔名裡

chezmoi 把屬性編在檔名裡（`dot_`、`private_`、`executable_`、`.tmpl`），repo 裡的檔名跟實際檔名不同，也看不出完整的設定。rosamta 的 `home/` 目錄使用實際檔名，deploy mode、權限、加密與 condition 一律寫在 `dotfiles.toml`，每個 dotfile 都必須有一筆宣告。沒有宣告的檔案由 doctor 回報，不會被部署。代價是從 chezmoi 匯入時，要把檔名上的屬性轉成設定。
