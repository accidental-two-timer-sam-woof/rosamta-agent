# 不提供 Flatpak 與 Snap 版本

rosamta 要修改家目錄裡的 rc 檔、啟動各種 shell，並呼叫系統的套件管理器；Flatpak 與 Snap 的沙盒會擋住這些操作，或需要大量例外權限才能運作。Linux 版改以 tarball、`.deb`、`.rpm`、AUR 與 `cargo install` 發布；macOS 版以簽章並 notarize 的 `.dmg` 與 Homebrew cask 發布。
