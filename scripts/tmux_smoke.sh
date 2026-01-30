#!/usr/bin/env bash
set -euo pipefail

cmd="${1:-}"
session="${2:-fresh-smoke}"

if [[ -z "$cmd" ]]; then
  echo "Usage: tmux_smoke.sh <command> [session]" >&2
  exit 2
fi

tmux kill-session -t "$session" 2>/dev/null || true

inner_cmd="stty -ixon; exec ${cmd}"
inner_cmd_quoted=$(printf '%q' "$inner_cmd")

tmux new-session -d -s "$session" "bash -lc ${inner_cmd_quoted}"

# Wait until the session exists and has a pane.
for _ in {1..100}; do
  if tmux has-session -t "$session" 2>/dev/null; then
    break
  fi
  sleep 0.05
done

# Wait for Fresh to render something (best-effort readiness check).
for _ in {1..200}; do
  if tmux capture-pane -pt "$session:0.0" 2>/dev/null | grep -q "Palette:"; then
    break
  fi
  sleep 0.05
done

send_keys_if_alive() {
  if tmux has-session -t "$session" 2>/dev/null; then
    tmux send-keys -t "$session:0.0" "$@"
  fi
}

# Ask the app to quit. Ctrl+Q is bound to quit by default, but terminals often
# reserve Ctrl+Q for flow control; we disable ixon above.
# Fresh may prompt if there are unsaved buffers; the prompt supports:
#   (d)iscard and quit, (C)ancel
send_keys_if_alive C-q
sleep 0.1
send_keys_if_alive C-q
sleep 0.1
send_keys_if_alive d
send_keys_if_alive Enter

# Wait for the session to terminate.
for _ in {1..200}; do
  if ! tmux has-session -t "$session" 2>/dev/null; then
    exit 0
  fi
  sleep 0.05
done

echo "Fresh did not exit after Ctrl+Q; tmux pane output follows:" >&2
tmux capture-pane -pt "$session:0.0" >&2 || true
tmux kill-session -t "$session" 2>/dev/null || true
exit 1
