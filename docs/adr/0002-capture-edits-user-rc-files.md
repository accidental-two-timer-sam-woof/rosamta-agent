# Capture 會修改使用者的 rc 檔

交接文件原本規定 hook 以外的內容永遠不自動修改。但如果 Capture 後原行還留在 rc 檔，同一段設定會執行兩次，之後在 GUI 停用 module 也關不掉。所以 Capture 會把原行註解掉並標註去向（預設），或直接刪除（可設定），不提供「只複製」。

修改前一律先寫入 snapshot，任何一段都能還原。snapshot 只存在本機、永不 push，因為 rc 檔原文可能含有秘密與只屬於該 host 的路徑。
