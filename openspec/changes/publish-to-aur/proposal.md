# Proposal

## Why

`add-project-scaffolding` 已在 `release.yml` 加入推送 AUR 的 job，但 AUR 目前不開放註冊，無法建立帳號驗證。把這項工作獨立出來，讓 scaffolding 可以先封存。

## What Changes

- AUR 開放註冊後建立帳號，設定 `AUR_SSH_PRIVATE_KEY` secret 與 `AUR_USERNAME`、`AUR_EMAIL` variables。
- 以正式版本的 tag 驗證 `rosamta-bin` 推送成功（prerelease 不推送 AUR）。

## Capabilities

### New Capabilities

（無。只完成既有的發布流程。）

### Modified Capabilities

（無）

## Impact

- 不需修改程式；`release.yml` 的 `aur` job 在 `AUR_USERNAME` 設定後自動啟用。
