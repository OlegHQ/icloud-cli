$ErrorActionPreference = "Stop"

$repo = git rev-parse --show-toplevel
git -C $repo config core.hooksPath .githooks

Write-Host "Configured Git hooks from .githooks"
