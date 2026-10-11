#!/usr/bin/env bash
set -euo pipefail

cargo package \
  --workspace \
  --exclude vui-hello \
  --exclude vui-minimal \
  --locked \
  --allow-dirty
