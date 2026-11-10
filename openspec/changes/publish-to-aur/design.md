# Design

## Context

`release.yml` 的 `aur` job 用 `KSXGitHub/github-actions-deploy-aur@v4.2.0` 推送 `packaging/aur/PKGBUILD.in` 填好的 PKGBUILD。job 在 `AUR_USERNAME` 未設定（或只有空白）以及 prerelease 時跳過。

## Goals / Non-Goals

**Goals:**
- 第一個正式版本發布時，AUR 上出現可安裝的 `rosamta-bin`。

**Non-Goals:**
- 從原始碼建置的 `rosamta` AUR 套件。

## Decisions

AUR 帳號專用一把 SSH key，不和其他用途共用，方便撤銷。

## Risks / Trade-offs

- [AUR 長期不開放註冊] → Arch 使用者先用 release 的 tarball 或 `cargo install`。
