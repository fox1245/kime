# 오버워치용 Kime 포크 재설치

이 브랜치는 다음 개인 설정을 함께 보관합니다.

- 평상시 한/영 전환: `Shift+Space`
- 오버워치 실행 중 한/영 전환: 오른쪽 `Alt`
- 오버워치 프로세스를 감지해 단축키 프로필 자동 전환
- 두벌식에서 같은 자음을 연속 입력해도 쌍자음으로 합치지 않음
- KDE Plasma Wayland에서 Kime이 소비한 한글 키의 길게 누르기 반복 지원

## 새 Ubuntu에서 설치

```bash
sudo apt-get update
sudo apt-get install -y git
git clone -b codex/overwatch-ime-profile https://github.com/fox1245/kime.git
cd kime
./scripts/install-overwatch-kime.sh --install-deps
```

스크립트는 다음 작업을 수행합니다.

1. 선택적으로 Ubuntu/Debian 빌드 의존성과 Rust를 설치합니다.
2. GTK 3/4, Qt 5/6, XIM, Wayland 프런트엔드를 빌드하고 `/usr`에 설치합니다.
3. `~/.config/kime/config.yaml`에 저장된 개인 프로필을 설치합니다.
4. `~/.config/environment.d/99-kime.conf`에 입력기 환경변수를 설치합니다.
5. KDE Plasma Wayland의 가상 키보드를 Kime으로 설정합니다.
6. `kime-overwatch-watcher.service`를 활성화합니다.
7. `restore-overwatch-kime` Codex 스킬을 개인 스킬 폴더에 설치합니다.
8. 가능한 경우 로그아웃 없이 KWin을 통해 Kime을 다시 시작합니다.

기존 설정 파일은 덮어쓰기 전에 같은 디렉터리에
`.backup-YYYYMMDD-HHMMSS` 접미사로 보관됩니다.

새 운영체제에서 처음 설치한 경우 환경변수가 전체 데스크톱 세션에 적용되도록
설치가 끝난 뒤 한 번 로그아웃하고 다시 로그인하세요.

## 이미 의존성이 설치된 시스템

```bash
./scripts/install-overwatch-kime.sh
```

현재 Kime을 재시작하지 않고 설치만 하려면 다음 옵션을 사용합니다.

```bash
./scripts/install-overwatch-kime.sh --no-restart
```

시스템을 변경하지 않고 저장소와 필수 도구만 검사할 수도 있습니다.

```bash
./scripts/install-overwatch-kime.sh --check
```

## 설치 확인

```bash
pgrep -a kime
systemctl --user status kime-overwatch-watcher.service
printf '%s\n' "$GTK_IM_MODULE" "$QT_IM_MODULE" "$XMODIFIERS"
```

KDE Plasma Wayland에서는 정상적으로 실행될 때 `kime`, `kime-xim`,
`kime-wayland` 프로세스가 모두 나타납니다.

## Codex 스킬

설치 스크립트가 저장소의 `skills/restore-overwatch-kime`을 `$CODEX_HOME/skills`
아래에 설치합니다. `CODEX_HOME`이 없으면 `~/.codex/skills`를 사용합니다. Codex를
새로 열거나 새 작업을 시작하면 `$restore-overwatch-kime`으로 이 복구 절차를 다시
실행할 수 있습니다.
