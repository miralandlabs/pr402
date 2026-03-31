#!/usr/bin/env bash
# Keep static deploy copy in sync with docs/ (run after editing docs/AGENT_INTEGRATION.md).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cp "$ROOT/docs/AGENT_INTEGRATION.md" "$ROOT/public/agent-integration.md"
echo "Synced public/agent-integration.md from docs/AGENT_INTEGRATION.md"
