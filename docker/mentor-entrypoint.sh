#!/bin/bash
# IronClaw Mentor Worker Entrypoint
# Validates checksums and starts the mentor worker

set -e

PERSONA_FILE="/agents/mentor/persona.md"
SKILLS_FILE="/agents/mentor/skills.md"
VOICE_FILE="/agents/mentor/master-voice.wav"

echo "=== IronClaw Mentor Worker Starting ==="

# Validate persona.md exists
if [ ! -f "$PERSONA_FILE" ]; then
    echo "ERROR: Persona file not found: $PERSONA_FILE"
    exit 1
fi

# Validate skills.md exists
if [ ! -f "$SKILLS_FILE" ]; then
    echo "ERROR: Skills file not found: $SKILLS_FILE"
    exit 1
fi

# Validate master-voice.wav exists (optional for text-only mode)
if [ ! -f "$VOICE_FILE" ]; then
    echo "WARNING: Voice sample not found: $VOICE_FILE"
    echo "Mentor will operate in text-only mode"
    export MENTOR_VOICE_MODE="text_only"
else
    echo "Voice sample found, voice mode enabled"
    export MENTOR_VOICE_MODE="enabled"
    
    # Verify checksum if provided
    EXPECTED_CHECKSUM="${MASTER_VOICE_CHECKSUM:-}"
    if [ -n "$EXPECTED_CHECKSUM" ]; then
        ACTUAL_CHECKSUM=$(sha256sum "$VOICE_FILE" | cut -d' ' -f1)
        if [ "$ACTUAL_CHECKSUM" != "$EXPECTED_CHECKSUM" ]; then
            echo "ERROR: Voice sample checksum mismatch!"
            echo "Expected: $EXPECTED_CHECKSUM"
            echo "Actual:   $ACTUAL_CHECKSUM"
            exit 1
        fi
        echo "Voice sample checksum verified"
    fi
fi

# Log configuration
echo "Persona: $PERSONA_FILE"
echo "Skills:  $SKILLS_FILE"
echo "Voice:   $VOICE_FILE (mode: $MENTOR_VOICE_MODE)"

# Start worker
exec /app/worker --agent mentor "$@"
