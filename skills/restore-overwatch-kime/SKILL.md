---
name: restore-overwatch-kime
description: Restore, rebuild, install, or diagnose the customized Kime fork used for Overwatch on Ubuntu or Debian with KDE Plasma Wayland. Use when setting up a fresh Linux installation, restoring Shift+Space for the normal profile and Right Alt for the Overwatch profile, enabling the game watcher, fixing missing Kime environment variables, or verifying the fork after an OS reinstall.
---

# Restore Overwatch Kime

Use the repository's installer as the source of truth. It preserves existing user
configuration before restoring the known-working profile.

## Locate the repository

First check whether the current Git repository contains all of these files:

- `scripts/install-overwatch-kime.sh`
- `res/kime-overwatch-config.yaml`
- `res/kime-environment.conf`
- `res/kime-overwatch-watcher.service`

If it does not, look for an existing Kime checkout under the user's source
directories. If the repository is not present, clone the customized branch:

```bash
git clone -b codex/overwatch-ime-profile https://github.com/fox1245/kime.git
```

## Choose the operation

- For a fresh Ubuntu or Debian installation, run the installer with
  `--install-deps`.
- When build dependencies are already present, run it without options.
- When changing files without interrupting the active input method, add
  `--no-restart`.
- For inspection or diagnosis requests, do not install or restart anything;
  run `./scripts/install-overwatch-kime.sh --check` and perform the verification
  checks first.

The installer invokes `sudo` to install under `/usr`. Let the user enter the
password interactively if the current environment cannot authenticate.

## Install or restore

Run from the repository root:

```bash
./scripts/install-overwatch-kime.sh --install-deps
```

The installer must:

1. Build the GTK 3/4, Qt 5/6, XIM, and Wayland frontends.
2. Back up differing user configuration with a timestamp suffix.
3. Restore `~/.config/kime/config.yaml` and
   `~/.config/environment.d/99-kime.conf`.
4. Select Kime as KDE's Wayland virtual keyboard when KDE 6 tools exist.
5. Set KDE's global keyboard repeat delay to 150 ms for both Latin and Hangul.
6. Enable `kime-overwatch-watcher.service`.
7. Install this skill under the user's Codex skills directory.
8. Restart Kime through KWin when requested.

Do not launch `kime-wayland` directly under KDE. KWin must launch it with the
private Wayland input-method socket.

## Verify

Run read-only checks after installation:

```bash
pgrep -a kime
systemctl --user status kime-overwatch-watcher.service --no-pager
printf '%s\n' "$GTK_IM_MODULE" "$QT_IM_MODULE" "$XMODIFIERS"
```

Expect `kime`, `kime-xim`, and `kime-wayland` processes in a KDE Plasma Wayland
session. Expect the watcher to be active. On a fresh OS installation, require one
logout and login before judging the environment variables.

Ask the user to manually verify these behaviors:

- `Shift+Space` toggles Korean in the normal profile.
- Right `Alt` toggles Korean while Overwatch is running.
- Holding either a Latin or Korean letter starts repeating after 150 ms without
  sticking the physical key.
- Repeating the same consonant does not create a double consonant unless Shift
  is held.

Never synthesize held keyboard input for this verification. A failed synthetic
key-release path can leave movement or text keys logically stuck.

## Preserve repository state

Inspect `git status` before editing. Preserve unrelated user changes. Run the
workspace tests and release build after code changes. Commit or push only when
the user explicitly requests it.
