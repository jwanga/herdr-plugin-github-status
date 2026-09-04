#!/usr/bin/env bash
# Action entrypoint: `pane.sh toggle|open|close` → `herdr-github-status dock <mode>`.
exec "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/launch.sh" dock "${1:-toggle}"
