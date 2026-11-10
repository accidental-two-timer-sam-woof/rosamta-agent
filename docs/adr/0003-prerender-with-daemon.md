# 預先 render，由 daemon 更新，不在 shell 啟動時 render

另一個做法是像 starship、mise 那樣在 hook 寫 `eval "$(rosamta init zsh)"`，每開一個 shell 就即時 render，產物永遠是最新的。我們選擇預先 render 成 generated script；repo 有變動時，由 daemon 重新 render、跑 smoke test，通過才 atomic 替換。這樣 shell 啟動不依賴 rosamta 執行檔，壞掉的設定也會在進入使用者的 shell 之前被擋下並回滾。代價是沒裝 daemon 的 host 要手動執行 `rosamta apply`。
