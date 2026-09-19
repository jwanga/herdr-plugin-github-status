#!/usr/bin/env bash
# Action and hook entrypoint: `pane.sh toggle|open|close|ensure|startup` →
# `herdr-github-status dock <mode>`.
exec "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/launch.sh" dock "${1:-toggle}"
