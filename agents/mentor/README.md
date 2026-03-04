# Mentor Agent Directory

This directory contains the Mentor AI configuration files.

## Required Files

### persona.md
Core identity, safety rails, and tone guidelines for the Mentor AI.
- **Permissions**: Read-only for Mentor worker
- **Format**: Markdown with XML-tagged sections

### skills.md
Explicit instructions on how to evaluate the Main Agent's actions.
- **Permissions**: Read-only for Mentor worker
- **Format**: Markdown with capability descriptions

### master-voice.wav (Required for Voice Mode)
Reference audio sample for zero-shot voice cloning via Chutes.ai.

**Requirements:**
- **Format**: WAV, 16-bit, 22050Hz or 44100Hz
- **Duration**: 10-15 seconds (optimal), max 30 seconds
- **Content**: Clean speech without background noise
- **File size**: Max 10MB

**How to create:**
1. Record 10-15 seconds of clear speech
2. Export as WAV format (no compression)
3. Place in this directory as `master-voice.wav`
4. (Optional) Generate SHA256 checksum for verification:
   ```bash
   sha256sum master-voice.wav
   ```
5. Add checksum to `docker-compose.yml` as `MASTER_VOICE_CHECKSUM`

**Security:**
- This file is read-only for the Mentor worker
- Cannot be modified or deleted by any agent
- Checksum verification prevents tampering

## Directory Permissions

| File | Read | Write | Delete |
|------|------|-------|--------|
| persona.md | Mentor | ❌ | ❌ |
| skills.md | Mentor | ❌ | ❌ |
| master-voice.wav | Mentor, chutes_tts | ❌ | ❌ |

## Usage

The Mentor worker reads these files at startup:
1. `persona.md` → System prompt identity
2. `skills.md` → Capability instructions  
3. `master-voice.wav` → Voice cloning reference (if voice mode enabled)

To enable voice mode, set in docker-compose.yml:
```yaml
environment:
  - MENTOR_VOICE_MODE=enabled
  - CHUTES_API_KEY=your_api_key
```
