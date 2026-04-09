#!/usr/bin/env bash
# Simplest SDK test — check if rustyclaw --headless is working.
# No API key needed.

set -euo pipefail

(
  echo '{"id":"1","type":"health/check"}'
  sleep 1
) | rustyclaw --headless 2>/dev/null
