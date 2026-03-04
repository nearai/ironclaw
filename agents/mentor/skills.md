# Mentor AI Skills

## Core Skills

### Log Reader
Read and analyze execution logs from the Main Agent's DuckDB database.

**Capability**: Read-only access to `/logs/agent_history.db`

**Usage**:
```sql
SELECT action, result, timestamp, status 
FROM agent_logs 
ORDER BY timestamp DESC 
LIMIT 10;
```

### Action Evaluator
Evaluate the Main Agent's proposed actions against safety and best practice criteria.

**Checklist**:
- [ ] Does this action have proper error handling?
- [ ] Are there security implications?
- [ ] Is this the most efficient approach?
- [ ] Are there edge cases not considered?
- [ ] Does this follow project conventions?

### Pattern Recognizer
Identify common code patterns, anti-patterns, and potential refactoring opportunities.

**Patterns to detect**:
- Code duplication
- Missing abstractions
- Over-engineering
- Tight coupling
- Missing test coverage

### Voice Responder
Generate voice responses using cloned voice from master-voice.wav sample.

**Pipeline**:
1. Receive text response from Mentor LLM
2. Call chutes_tts WASM tool with text + reference_audio_path
3. Receive synthesized audio from Chutes.ai
4. Return audio to Telegram channel

### Checkpoint Manager
Read and validate checkpoint files for state persistence.

**Read-only paths**:
- `/agents/mentor/checkpoints/`
- `/agents/mentor/persona.md`
- `/agents/mentor/skills.md`
- `/agents/mentor/master-voice.wav`

## Telegram Integration

### Command Routing
- `/mentor <query>` - Route to Mentor for text response
- `/mentor_voice <query>` - Route to Mentor for voice response
- Voice notes with caption `/mentor` - Transcribe → Mentor → TTS → Voice response

### Voice Note Handling
1. Download voice note from Telegram (`.ogg` format)
2. Call chutes_stt WASM tool for transcription
3. Pass transcribed text to Mentor
4. If voice reply requested, call chutes_tts with master-voice.wav
5. Send audio back via Telegram sendVoice API

## Security Constraints

### Filesystem Access
| Path | Read | Write | Delete |
|------|------|-------|--------|
| `/agents/mentor/` | ✓ | ✗ | ✗ |
| `/logs/` | ✓ | ✗ | ✗ |
| `/tmp/stt_*` | ✓ | ✓ | ✗ |
| `/tmp/tts_*` | ✓ | ✓ | ✗ |
| `/workspace/` | ✗ | ✗ | ✗ |

### Network Access
- `llm.chutes.ai` - TTS/STT API calls only
- No arbitrary HTTP requests

### Resource Limits
- Memory: 256MB max
- Execution time: 30s max per tool call
- Audio file size: 10MB max for voice cloning
