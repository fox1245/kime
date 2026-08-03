#!/usr/bin/env bash

set -Eeuo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
build_dir=${KIME_BUILD_DIR:-"$repo_root/build"}
user_config_root=${XDG_CONFIG_HOME:-"${HOME:?HOME is not set}/.config"}
kime_config_dir="$user_config_root/kime"
kime_config_target="$kime_config_dir/config.yaml"
environment_dir="$user_config_root/environment.d"
environment_target="$environment_dir/99-kime.conf"
kime_config_source="$repo_root/res/kime-overwatch-config.yaml"
environment_source="$repo_root/res/kime-environment.conf"
codex_config_root=${CODEX_HOME:-"${HOME:?HOME is not set}/.codex"}
codex_skill_source="$repo_root/skills/restore-overwatch-kime"
codex_skill_target="$codex_config_root/skills/restore-overwatch-kime"
backup_stamp=$(date +%Y%m%d-%H%M%S)
install_dependencies=false
restart_session=true
check_only=false

usage() {
    cat <<'EOF'
Usage: ./scripts/install-overwatch-kime.sh [options]

Build and install this Kime fork, restore its Overwatch hotkey profile, and
enable the profile watcher for the current user.

Options:
  --install-deps  Install Ubuntu/Debian build dependencies and Rust if needed
  --no-restart    Do not restart Kime through KWin after installation
  --check         Validate this checkout without changing the system
  -h, --help      Show this help

Existing user configuration files are backed up before they are replaced.
EOF
}

log() {
    printf '[kime-setup] %s\n' "$*"
}

warn() {
    printf '[kime-setup] WARNING: %s\n' "$*" >&2
}

die() {
    printf '[kime-setup] ERROR: %s\n' "$*" >&2
    exit 1
}

while (($# > 0)); do
    case "$1" in
        --install-deps)
            install_dependencies=true
            ;;
        --no-restart)
            restart_session=false
            ;;
        --check)
            check_only=true
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
    shift
done

install_debian_dependencies() {
    [[ -r /etc/os-release ]] || die "cannot identify this distribution"

    # shellcheck disable=SC1091
    source /etc/os-release
    case " ${ID:-} ${ID_LIKE:-} " in
        *" ubuntu "* | *" debian "*) ;;
        *) die "--install-deps currently supports Ubuntu and Debian only" ;;
    esac

    log "installing build dependencies"
    sudo apt-get update
    sudo apt-get install -y \
        build-essential ca-certificates curl gcc git libclang-dev meson ninja-build pkg-config zstd \
        libcairo2-dev libfontconfig1-dev libfreetype-dev libglib2.0-dev libgtk-3-dev libgtk-4-dev \
        libpango1.0-dev libqt5gui5 libxcb1 libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev \
        qt6-base-dev qt6-base-private-dev qtbase5-dev qtbase5-private-dev

    if ! command -v rustup >/dev/null 2>&1; then
        log "installing Rust with rustup"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --profile minimal --default-toolchain stable
        export PATH="${HOME:?HOME is not set}/.cargo/bin:$PATH"
    fi

    rustup toolchain install stable
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

backup_and_install() {
    local source_file=$1
    local target_file=$2
    local mode=$3
    local backup_file

    mkdir -p -- "$(dirname -- "$target_file")"

    if [[ -e "$target_file" ]] && ! cmp -s -- "$source_file" "$target_file"; then
        backup_file="${target_file}.backup-${backup_stamp}"
        cp -a -- "$target_file" "$backup_file"
        log "backed up $target_file to $backup_file"
    fi

    install -m "$mode" -- "$source_file" "$target_file"
}

install_codex_skill() {
    [[ -f "$codex_skill_source/SKILL.md" ]] \
        || die "missing Codex skill: $codex_skill_source/SKILL.md"
    [[ -f "$codex_skill_source/agents/openai.yaml" ]] \
        || die "missing Codex skill metadata: $codex_skill_source/agents/openai.yaml"

    backup_and_install \
        "$codex_skill_source/SKILL.md" \
        "$codex_skill_target/SKILL.md" \
        644
    backup_and_install \
        "$codex_skill_source/agents/openai.yaml" \
        "$codex_skill_target/agents/openai.yaml" \
        644
    log "installed the restore-overwatch-kime Codex skill"
}

configure_kde_wayland() {
    local kwin_config="$user_config_root/kwinrc"
    local keyboard_config="$user_config_root/kcminputrc"
    local current_input_method=
    local current_virtual_keyboard=
    local current_repeat_delay=

    if ! command -v kwriteconfig6 >/dev/null 2>&1; then
        warn "kwriteconfig6 is unavailable; select 'kime daemon' in KDE Virtual Keyboard settings"
        return
    fi

    if command -v kreadconfig6 >/dev/null 2>&1; then
        current_input_method=$(kreadconfig6 --file kwinrc --group Wayland --key InputMethod 2>/dev/null || true)
        current_virtual_keyboard=$(kreadconfig6 --file kwinrc --group Wayland --key VirtualKeyboardEnabled 2>/dev/null || true)
        current_repeat_delay=$(kreadconfig6 --file kcminputrc --group Keyboard --key RepeatDelay 2>/dev/null || true)
    fi

    if [[ "$current_input_method" != /usr/share/applications/kime.desktop ]] \
        || [[ "$current_virtual_keyboard" != true ]]; then
        if [[ -e "$kwin_config" ]]; then
            cp -a -- "$kwin_config" "${kwin_config}.backup-${backup_stamp}"
            log "backed up $kwin_config before changing the virtual keyboard"
        fi
        kwriteconfig6 --file kwinrc --group Wayland --key InputMethod \
            /usr/share/applications/kime.desktop
        kwriteconfig6 --file kwinrc --group Wayland --key VirtualKeyboardEnabled \
            --type bool true
    fi

    if [[ "$current_repeat_delay" != 150 ]]; then
        if [[ -e "$keyboard_config" ]]; then
            cp -a -- "$keyboard_config" "${keyboard_config}.backup-${backup_stamp}"
            log "backed up $keyboard_config before changing the repeat delay"
        fi
        kwriteconfig6 --file kcminputrc --group Keyboard --key RepeatDelay 150
        log "set the global keyboard repeat delay to 150 ms"
    fi
}

restart_kime_for_kwin() {
    if [[ "$restart_session" != true ]]; then
        log "skipping the live Kime restart"
        return
    fi

    if ! command -v qdbus6 >/dev/null 2>&1 || ! command -v busctl >/dev/null 2>&1; then
        warn "KDE session tools are unavailable; log out and back in before using Kime"
        return
    fi

    if ! qdbus6 org.kde.KWin /KWin reconfigure >/dev/null 2>&1; then
        warn "KWin is not reachable; log out and back in before using Kime"
        return
    fi
    if ! busctl --user get-property org.kde.KWin /VirtualKeyboard \
        org.kde.kwin.VirtualKeyboard enabled >/dev/null 2>&1; then
        warn "KWin's virtual keyboard interface is unavailable; log out and back in"
        return
    fi

    /usr/bin/kime -k >/dev/null 2>&1 || true
    if ! busctl --user set-property org.kde.KWin /VirtualKeyboard \
        org.kde.kwin.VirtualKeyboard enabled b false; then
        warn "could not disable KWin's virtual keyboard; log out and back in"
        return
    fi
    if ! busctl --user set-property org.kde.KWin /VirtualKeyboard \
        org.kde.kwin.VirtualKeyboard enabled b true; then
        warn "could not re-enable KWin's virtual keyboard; log out and back in"
        return
    fi
    log "restarted Kime through KWin"
}

if [[ "$install_dependencies" == true && "$check_only" == false ]]; then
    install_debian_dependencies
elif [[ "$install_dependencies" == true ]]; then
    warn "ignoring --install-deps because --check never changes the system"
fi

require_command cargo
require_command meson
require_command ninja
require_command sudo
require_command systemctl

[[ -f "$kime_config_source" ]] || die "missing config template: $kime_config_source"
[[ -f "$environment_source" ]] || die "missing environment template: $environment_source"
[[ -f "$codex_skill_source/SKILL.md" ]] || die "missing Codex skill"
[[ -f "$codex_skill_source/agents/openai.yaml" ]] || die "missing Codex skill metadata"

if [[ "$check_only" == true ]]; then
    bash -n "${BASH_SOURCE[0]}"
    cargo metadata --format-version 1 --no-deps >/dev/null
    if [[ -f "$build_dir/build.ninja" ]]; then
        meson configure "$build_dir" >/dev/null
    fi
    log "checkout validation passed; no files or services were changed"
    exit 0
fi

meson_options=(
    --prefix=/usr
    -Dgtk3=enabled
    -Dgtk4=enabled
    -Dqt5=enabled
    -Dqt6=enabled
    -Dxim=enabled
    -Dwayland=enabled
    -Dcheck=disabled
    -Dindicator=disabled
    -Dcandidate_window=disabled
)

if [[ -f "$build_dir/build.ninja" ]]; then
    log "reconfiguring the existing build directory"
    meson setup --reconfigure "$build_dir" "${meson_options[@]}"
else
    log "creating the build directory"
    meson setup "$build_dir" "${meson_options[@]}"
fi

log "building Kime"
ninja -C "$build_dir"

log "installing Kime under /usr"
sudo ninja -C "$build_dir" install

backup_and_install "$kime_config_source" "$kime_config_target" 600
backup_and_install "$environment_source" "$environment_target" 600
install_codex_skill
configure_kde_wayland

if systemctl --user daemon-reload \
    && systemctl --user enable --now kime-overwatch-watcher.service; then
    log "enabled the Overwatch profile watcher"
else
    warn "could not enable the user service; run this from the graphical session:"
    warn "systemctl --user enable --now kime-overwatch-watcher.service"
fi

restart_kime_for_kwin

log "installation complete"
log "normal hotkey: Shift+Space; Overwatch hotkey: Right Alt"
log "after a fresh OS installation, log out and back in once to load 99-kime.conf"
log "start a new Codex task to discover the restore-overwatch-kime skill"
