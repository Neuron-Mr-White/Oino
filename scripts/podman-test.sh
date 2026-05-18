#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${OINO_PODMAN_IMAGE:-localhost/oino-test:latest}"
CONTAINER="${OINO_PODMAN_CONTAINER:-oino-test}"
WORKSPACE_VOLUME="${OINO_PODMAN_WORKSPACE_VOLUME:-oino-test-workspace}"
LEGACY_WORK_VOLUME="${OINO_PODMAN_WORK_VOLUME:-oino-test-work}"
HOME_VOLUME="${OINO_PODMAN_HOME_VOLUME:-oino-test-home}"
PROFILE="${OINO_PODMAN_PROFILE:-debug}"
TMP_SIZE="${OINO_PODMAN_TMP_SIZE:-4G}"
TMUX_SESSION="${OINO_PODMAN_TMUX_SESSION:-oino}"
ENV_FILE="${OINO_PODMAN_ENV_FILE:-$PROJECT_ROOT/.env.podman}"

if [[ -n "${OINO_PODMAN_BIN:-}" ]]; then
  BIN_PATH="$OINO_PODMAN_BIN"
else
  if [[ "$PROFILE" == "release" ]]; then
    BIN_PATH="$PROJECT_ROOT/target/release/oino"
  else
    BIN_PATH="$PROJECT_ROOT/target/debug/oino"
  fi
fi

BIN_DIR="$(cd "$(dirname "$BIN_PATH")" 2>/dev/null && pwd || true)"
BIN_NAME="$(basename "$BIN_PATH")"

usage() {
  cat <<EOF
Usage: $0 <command>

Commands:
  up       Build the Oino binary, create/start the Podman sandbox, attach tmux
  start    Create/start the Podman sandbox without attaching
  attach   Attach to the sandbox tmux session, creating the sandbox if needed
  shell    Open a bash shell in the sandbox, creating the sandbox if needed
  reset    Clear and reinitialize the fresh /workspace project
  binary   Build the host Oino binary used by the sandbox
  image    Build the sandbox image with tmux and test utilities
  status   Show container, image, and volume status
  clean    Stop/remove the sandbox container, volumes, and image

Environment:
  OINO_PODMAN_CONTAINER      Container name (default: oino-test)
  OINO_PODMAN_IMAGE          Runtime image tag (default: localhost/oino-test:latest)
  OINO_PODMAN_PROFILE        Host cargo profile: debug or release (default: debug)
  OINO_PODMAN_BIN            Existing Linux Oino binary to mount instead of cargo build
  OINO_PODMAN_WORKSPACE_VOLUME Workspace volume name (default: oino-test-workspace)
  OINO_PODMAN_ENV_FILE       Env file to inject (default: .env.podman when present)
  OPENROUTER_API_KEY         Passed through when set
  OINO_MODEL                 Passed through when set

Inside tmux, run:
  oino
  oino --help
  cd /workspace
EOF
}

need_podman() {
  if ! command -v podman >/dev/null 2>&1; then
    echo "error: podman is required but was not found" >&2
    exit 127
  fi
}

build_binary() {
  if [[ -n "${OINO_PODMAN_BIN:-}" ]]; then
    if [[ ! -x "$BIN_PATH" ]]; then
      echo "error: OINO_PODMAN_BIN is not executable: $BIN_PATH" >&2
      exit 1
    fi
    return
  fi

  if [[ "$PROFILE" == "release" ]]; then
    cargo build --release -p oino-app --bin oino
  elif [[ "$PROFILE" == "debug" ]]; then
    cargo build -p oino-app --bin oino
  else
    echo "error: OINO_PODMAN_PROFILE must be debug or release, got: $PROFILE" >&2
    exit 1
  fi
}

refresh_binary_path() {
  BIN_DIR="$(cd "$(dirname "$BIN_PATH")" && pwd)"
  BIN_NAME="$(basename "$BIN_PATH")"
}

build_image() {
  need_podman
  podman build \
    --tag "$IMAGE" \
    --file "$PROJECT_ROOT/scripts/podman/Containerfile" \
    "$PROJECT_ROOT/scripts/podman"
}

image_exists() {
  podman image exists "$IMAGE" >/dev/null 2>&1
}

container_exists() {
  podman container exists "$CONTAINER" >/dev/null 2>&1
}

container_running() {
  [[ "$(podman inspect --format '{{.State.Running}}' "$CONTAINER" 2>/dev/null || true)" == "true" ]]
}

container_uses_legacy_source_layout() {
  podman inspect --format '{{range .Mounts}}{{println .Destination}}{{end}}' "$CONTAINER" 2>/dev/null \
    | grep -Eq '^/(source|work)$'
}

ensure_image() {
  need_podman
  if ! image_exists; then
    build_image
  fi
}

env_file_args() {
  local env_file_args=()
  if [[ -f "$ENV_FILE" ]]; then
    env_file_args+=(--env-file "$ENV_FILE")
  elif [[ -n "${OINO_PODMAN_ENV_FILE:-}" ]]; then
    echo "error: OINO_PODMAN_ENV_FILE does not exist: $ENV_FILE" >&2
    exit 1
  fi

  if ((${#env_file_args[@]})); then
    printf '%s\0' "${env_file_args[@]}"
  fi
}

base_env_args() {
  local utf8_locale="${OINO_PODMAN_LOCALE:-C.UTF-8}"
  local env_args=(
    --env "TERM=${TERM:-xterm-256color}"
    --env "COLORTERM=${COLORTERM:-truecolor}"
    --env "LANG=$utf8_locale"
    --env "LC_ALL=$utf8_locale"
    --env "LC_CTYPE=$utf8_locale"
    --env "OINO_PODMAN_BINARY_NAME=$BIN_NAME"
  )

  printf '%s\0' "${env_args[@]}"
}

passthrough_env_args() {
  local env_args=()
  local passthrough=(
    OPENROUTER_API_KEY
    OINO_MODEL
    OINO_OPENROUTER_REFERER
    OINO_OPENROUTER_TITLE
    RUST_BACKTRACE
  )

  for name in "${passthrough[@]}"; do
    if [[ -n "${!name:-}" ]]; then
      env_args+=(--env "$name=${!name}")
    fi
  done

  if ((${#env_args[@]})); then
    printf '%s\0' "${env_args[@]}"
  fi
}

create_container() {
  ensure_image
  build_binary
  refresh_binary_path

  if [[ ! -x "$BIN_PATH" ]]; then
    echo "error: Oino binary does not exist or is not executable: $BIN_PATH" >&2
    exit 1
  fi

  if container_exists; then
    if container_uses_legacy_source_layout; then
      echo "error: existing sandbox '$CONTAINER' uses the old source-copy layout" >&2
      echo "run 'mise run podman:clean' once, then rerun this command for a fresh /workspace sandbox" >&2
      exit 1
    fi
    return
  fi

  local base_env_args=()
  while IFS= read -r -d '' item; do
    base_env_args+=("$item")
  done < <(base_env_args)

  local env_file_args=()
  while IFS= read -r -d '' item; do
    env_file_args+=("$item")
  done < <(env_file_args)

  local passthrough_env_args=()
  while IFS= read -r -d '' item; do
    passthrough_env_args+=("$item")
  done < <(passthrough_env_args)

  podman run \
    --detach \
    --name "$CONTAINER" \
    --hostname "$CONTAINER" \
    --label dev.oino.sandbox=true \
    --security-opt no-new-privileges \
    --cap-drop ALL \
    --tmpfs "/tmp:exec,size=$TMP_SIZE" \
    --workdir /workspace \
    --volume "$WORKSPACE_VOLUME:/workspace" \
    --volume "$HOME_VOLUME:/root" \
    --volume "$BIN_DIR:/opt/oino-bin:ro" \
    "${base_env_args[@]}" \
    "${env_file_args[@]}" \
    "${passthrough_env_args[@]}" \
    "$IMAGE" \
    sleep infinity >/dev/null
}

ensure_started() {
  create_container
  if ! container_running; then
    podman start "$CONTAINER" >/dev/null
  fi
}

podman_exec_args() {
  local exec_args=()
  while IFS= read -r -d '' item; do
    exec_args+=("$item")
  done < <(base_env_args)

  while IFS= read -r -d '' item; do
    exec_args+=("$item")
  done < <(env_file_args)

  while IFS= read -r -d '' item; do
    exec_args+=("$item")
  done < <(passthrough_env_args)

  if ((${#exec_args[@]})); then
    printf '%s\0' "${exec_args[@]}"
  fi
}

podman_exec() {
  local exec_args=()
  while IFS= read -r -d '' item; do
    exec_args+=("$item")
  done < <(podman_exec_args)

  podman exec "${exec_args[@]}" "$CONTAINER" "$@"
}

podman_exec_tty() {
  local exec_args=()
  while IFS= read -r -d '' item; do
    exec_args+=("$item")
  done < <(podman_exec_args)

  podman exec -it "${exec_args[@]}" "$CONTAINER" "$@"
}

env_file_names() {
  [[ -f "$ENV_FILE" ]] || return 0
  awk '
    /^[[:space:]]*($|#)/ { next }
    {
      line = $0
      sub(/^[[:space:]]*export[[:space:]]+/, "", line)
      if (match(line, /^[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=/)) {
        name = substr(line, 1, RLENGTH - 1)
        sub(/[[:space:]]*$/, "", name)
        print name
      }
    }
  ' "$ENV_FILE"
}

sync_tmux_environment() {
  local names=()
  if [[ -f "$ENV_FILE" ]]; then
    mapfile -t names < <(env_file_names)
  fi

  podman_exec bash -lc '
    session="$1"
    shift
    if tmux has-session -t "$session" 2>/dev/null; then
      tmux set-environment -g LANG "${LANG-}"
      tmux set-environment -g LC_ALL "${LC_ALL-}"
      tmux set-environment -g LC_CTYPE "${LC_CTYPE-}"
      for name in "$@"; do
        tmux set-environment -g "$name" "${!name-}"
      done
    fi
  ' bash "$TMUX_SESSION" "${names[@]}"
}

initialize_workspace() {
  podman_exec bash -lc '
    set -euo pipefail
    mkdir -p /workspace /root/.oino
    if [[ ! -d /workspace/.git ]]; then
      git init -b main /workspace >/dev/null 2>&1 || git init /workspace >/dev/null
    fi
    git config --global --add safe.directory /workspace >/dev/null 2>&1 || true
    git config --global user.name "Oino Podman Test" >/dev/null 2>&1 || true
    git config --global user.email "oino-podman-test@example.invalid" >/dev/null 2>&1 || true
  '
}

seed_workspace() {
  ensure_started
  initialize_workspace
}

reset_workspace() {
  ensure_started
  podman_exec bash -lc '
    set -euo pipefail
    mkdir -p /workspace
    find /workspace -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
  '
  initialize_workspace
  echo "reset $CONTAINER:/workspace to a fresh empty git workspace"
}

attach_tmux() {
  seed_workspace
  sync_tmux_environment
  podman_exec_tty bash -lc "cd /workspace && tmux -u new-session -A -s '$TMUX_SESSION'"
}

open_shell() {
  seed_workspace
  podman_exec_tty bash -lc 'cd /workspace && exec bash'
}

show_status() {
  need_podman
  echo "container: $CONTAINER"
  podman ps -a --filter "name=^${CONTAINER}$" --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}' || true
  echo
  echo "image: $IMAGE"
  podman images "$IMAGE" --format 'table {{.Repository}}\t{{.Tag}}\t{{.ID}}\t{{.Size}}' || true
  echo
  echo "volumes: $WORKSPACE_VOLUME $HOME_VOLUME"
  podman volume ls --filter "name=^${WORKSPACE_VOLUME}$" --filter "name=^${HOME_VOLUME}$" || true
}

clean_all() {
  need_podman
  if container_exists; then
    podman rm -f "$CONTAINER" >/dev/null
  fi
  podman volume rm -f "$WORKSPACE_VOLUME" "$LEGACY_WORK_VOLUME" "$HOME_VOLUME" >/dev/null 2>&1 || true
  podman image rm -f "$IMAGE" >/dev/null 2>&1 || true
  echo "removed sandbox container, volumes, and image for $CONTAINER"
}

cmd="${1:-}"
case "$cmd" in
  up)
    attach_tmux
    ;;
  start)
    seed_workspace
    echo "sandbox started: $CONTAINER"
    echo "attach with: mise run podman:attach"
    ;;
  attach)
    attach_tmux
    ;;
  shell)
    open_shell
    ;;
  reset)
    reset_workspace
    ;;
  binary)
    build_binary
    ;;
  image)
    build_image
    ;;
  status)
    show_status
    ;;
  clean)
    clean_all
    ;;
  -h|--help|help|"")
    usage
    ;;
  *)
    echo "error: unknown command: $cmd" >&2
    usage >&2
    exit 2
    ;;
esac
