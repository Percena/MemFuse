#!/usr/bin/env bash
# Quick restart of the memfuse user service after config changes.
# Usage: ./memfuse-reload.sh [install]
#
# Commands:
#   (no args)   — restart the service, applying latest config
#   install     — install/update the systemd unit and start
#   uninstall   — stop and remove the systemd unit
#   status      — show service status
#   logs        — tail journal logs

set -euo pipefail

SERVICE_NAME="memfuse"
UNIT_FILE="$HOME/.config/systemd/user/${SERVICE_NAME}.service"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE_UNIT="$SCRIPT_DIR/memfuse.service"

cmd="${1:-restart}"

case "$cmd" in
  install)
    mkdir -p "$(dirname "$UNIT_FILE")"
    cp "$SOURCE_UNIT" "$UNIT_FILE"
    systemctl --user daemon-reload
    systemctl --user enable --now "$SERVICE_NAME"
    echo "Installed and started ${SERVICE_NAME}.service"
    systemctl --user status "$SERVICE_NAME" --no-pager
    ;;
  uninstall)
    systemctl --user disable --now "$SERVICE_NAME" 2>/dev/null || true
    rm -f "$UNIT_FILE"
    systemctl --user daemon-reload
    echo "Uninstalled ${SERVICE_NAME}.service"
    ;;
  restart)
    systemctl --user restart "$SERVICE_NAME"
    echo "Restarted ${SERVICE_NAME}.service"
    systemctl --user status "$SERVICE_NAME" --no-pager
    ;;
  status)
    systemctl --user status "$SERVICE_NAME" --no-pager
    ;;
  logs)
    journalctl --user -u "$SERVICE_NAME" -f --no-pager
    ;;
  *)
    echo "Usage: $0 {install|uninstall|restart|status|logs}"
    exit 1
    ;;
esac
