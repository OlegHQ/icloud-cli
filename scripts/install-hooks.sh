#!/usr/bin/env sh
set -eu

repo="$(git rev-parse --show-toplevel)"
git -C "$repo" config core.hooksPath .githooks
chmod +x "$repo/.githooks/pre-push"

echo "Configured Git hooks from .githooks"
