#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# ZeroClaw instance scaffolder for zerowrapper
#
# Creates a self-contained ZeroClaw config dir (custom path, NOT ~/.zeroclaw)
# so the `zw` binary can point its `zeroclaw_socket` at it and the daemon can
# be started per-project. Safe to re-run: existing files are not overwritten.
#
# Usage:
#   ./setup.sh                          # ./zeroclaw-instance, agent alias "craw"
#   ./setup.sh -d /path/to/instance     # custom target dir
#   ./setup.sh -a assistant             # agent alias "assistant"
#   ./setup.sh --no-seed                # skip writing workspace memory docs
#
# After scaffolding:
#   1. edit  <target>/config.toml  (model provider, api keys, MCP servers)
#   2. start the daemon:
#        ZEROCLAW_CONFIG_DIR=<target> zeroclaw daemon
#   3. point zw at it:
#        zeroclaw_socket = "<target>/data/daemon.sock"   (zerowrapper config.toml)
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE="$SCRIPT_DIR/config.toml.example"

TARGET="${TARGET:-./zeroclaw-instance}"
AGENT_ALIAS="${AGENT_ALIAS:-craw}"
SEED_WORKSPACE=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--dir)     TARGET="$2"; shift 2 ;;
        -a|--agent)   AGENT_ALIAS="$2"; shift 2 ;;
        --no-seed)    SEED_WORKSPACE=0; shift ;;
        -h|--help)
            grep '^#' "$0" | sed 's/^# \{0,1\}//' | head -40
            exit 0
            ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ ! -f "$TEMPLATE" ]]; then
    echo "error: template not found: $TEMPLATE" >&2
    exit 1
fi

if ! command -v zeroclaw >/dev/null 2>&1; then
    echo "warning: 'zeroclaw' binary not on PATH — install it first, or pass the" >&2
    echo "         full path when you run the daemon." >&2
fi

# ── Create the directory skeleton ────────────────────────────────────────────
mkdir -p "$TARGET"
TARGET="$(cd "$TARGET" && pwd)"

DATA_DIR="$TARGET/data"
WORKSPACE="$TARGET/agents/$AGENT_ALIAS/workspace"
mkdir -p "$DATA_DIR" "$WORKSPACE" "$TARGET/shared"

# ── Write config.toml (never clobber an existing one) ───────────────────────
CONFIG_FILE="$TARGET/config.toml"
if [[ -f "$CONFIG_FILE" ]]; then
    echo "> keeping existing $CONFIG_FILE"
else
    sed "s/\[agents\.craw\]/[agents.$AGENT_ALIAS]/" "$TEMPLATE" > "$CONFIG_FILE"
    echo "> wrote $CONFIG_FILE"
fi

# ── Seed the agent workspace with memory / identity docs ────────────────────
if [[ "$SEED_WORKSPACE" == "1" ]]; then
    for f in MEMORY.md IDENTITY.md SOUL.md; do
        if [[ -f "$WORKSPACE/$f" ]]; then
            echo "> keeping existing $WORKSPACE/$f"
        else
            case "$f" in
                MEMORY.md)   printf '# MEMORY.md — persistent memories for %s\n\n' "$AGENT_ALIAS" > "$WORKSPACE/$f" ;;
                IDENTITY.md) printf '# IDENTITY.md — Who I am\n\nI am %s, an agent running under ZeroClaw, wrapped by zerowrapper.\n' "$AGENT_ALIAS" > "$WORKSPACE/$f" ;;
                SOUL.md)     printf '# SOUL.md — Who I am (voice & persona)\n\nHelpful, precise, honest.\n' > "$WORKSPACE/$f" ;;
            esac
            echo "> seeded $WORKSPACE/$f"
        fi
    done
fi

# ── Launcher helpers ─────────────────────────────────────────────────────────
LAUNCHER="$TARGET/run-zeroclaw.sh"
cat > "$LAUNCHER" <<EOF
#!/usr/bin/env bash
# Start the ZeroClaw daemon against this instance's config dir.
# Socket will appear at: $DATA_DIR/daemon.sock
export ZEROCLAW_CONFIG_DIR="$TARGET"
export ZEROCLAW_DATA_DIR="$TARGET/data"
exec zeroclaw daemon "\$@"
EOF
chmod +x "$LAUNCHER"
echo "> wrote $LAUNCHER"

# ── Report ───────────────────────────────────────────────────────────────────
cat <<REPORT

ZeroClaw instance scaffolded at: $TARGET
  config.toml       : $CONFIG_FILE
  data / socket     : $DATA_DIR           (socket: $DATA_DIR/daemon.sock)
  agent workspace   : $WORKSPACE
  shared            : $TARGET/shared

Start the daemon:
  $LAUNCHER
  # or
  ZEROCLAW_CONFIG_DIR="$TARGET" zeroclaw daemon

Wire it into zerowrapper (config.toml):
  [zerowrapper]
  zeroclaw_socket = "$DATA_DIR/daemon.sock"

Automation examples (after the daemon is running):
  zeroclaw config set providers.models.anthropic.main.api_key '<key>'
  zeroclaw config set agents.$AGENT_ALIAS.model_provider anthropic.main
REPORT
