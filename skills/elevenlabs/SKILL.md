---
name: elevenlabs
version: "1.0.0"
description: ElevenLabs API — text-to-speech, voices, voice cloning, audio generation
activation:
  keywords:
    - "elevenlabs"
    - "text to speech"
    - "tts"
    - "voice"
  exclude_keywords:
    - "whisper"
    - "google tts"
  patterns:
    - "(?i)elevenlabs.*(voice|speech|audio|tts)"
    - "(?i)(generate|create|convert).*speech"
    - "(?i)text.to.speech"
  tags:
    - "ai"
    - "audio"
    - "tts"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ELEVENLABS_API_KEY]
---

# ElevenLabs API

Use the `http` tool. Include `xi-api-key` header. Audio responses should be saved to file.

## Base URL

`https://api.elevenlabs.io/v1`

## Actions

**Text to speech:**
```
http(method="POST", url="https://api.elevenlabs.io/v1/text-to-speech/<voice_id>", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}], body={"text": "Hello, how are you today?", "model_id": "eleven_multilingual_v2", "voice_settings": {"stability": 0.5, "similarity_boost": 0.75}}, save_to="/tmp/speech.mp3")
```

**List voices:**
```
http(method="GET", url="https://api.elevenlabs.io/v1/voices", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}])
```

**Get voice:**
```
http(method="GET", url="https://api.elevenlabs.io/v1/voices/<voice_id>", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}])
```

**Get models:**
```
http(method="GET", url="https://api.elevenlabs.io/v1/models", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}])
```

**Get subscription info:**
```
http(method="GET", url="https://api.elevenlabs.io/v1/user/subscription", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}])
```

**Get usage:**
```
http(method="GET", url="https://api.elevenlabs.io/v1/user", headers=[{"name": "xi-api-key", "value": "{ELEVENLABS_API_KEY}"}])
```

## Notes

- TTS returns raw audio bytes (mp3). Always use `save_to` to save the file.
- Models: `eleven_multilingual_v2` (best quality), `eleven_turbo_v2_5` (low latency), `eleven_monolingual_v1`.
- `stability` (0-1): lower = more expressive, higher = more consistent.
- `similarity_boost` (0-1): higher = closer to original voice.
- Popular premade voice IDs: check `GET /v1/voices` to list available options.
- Character quota tracked per account. Check `character_count`/`character_limit` in user info.
