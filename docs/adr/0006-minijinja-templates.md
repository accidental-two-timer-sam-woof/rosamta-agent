# Template 用 minijinja，不相容 chezmoi 的 Go template

chezmoi 使用者最熟悉的是 Go text/template，相容它就能讓 chezmoi 的 template 幾乎原封不動匯入。但 Rust 的 Go template crate 都很新、使用者極少、Sprig 函式不完整，而且 chezmoi 自己的函式（`output`、password manager 相關）一樣要重寫。我們選擇 minijinja：Jinja2 語法、維護活躍、預設在沙箱中執行，不能讀檔或執行指令，可用的函式由我們明確加入。從 chezmoi 匯入時，自動轉換常見寫法，轉不了的標出來讓使用者手動修改。

## Considered Options

- tera：mise 的 dotfile 功能使用 tera，選它能跟 mise 使用者相容，但預設沒有 minijinja 這種沙箱限制。
