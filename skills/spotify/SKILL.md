---
name: spotify
version: "1.0.0"
description: Spotify Web API — search, playlists, tracks, albums, player control
activation:
  keywords:
    - "spotify"
    - "playlist"
    - "spotify track"
  exclude_keywords:
    - "apple music"
    - "soundcloud"
  patterns:
    - "(?i)spotify.*(playlist|track|album|artist|search)"
    - "(?i)(play|search|add).*spotify"
  tags:
    - "music"
    - "media"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SPOTIFY_ACCESS_TOKEN]
---

# Spotify Web API

Use the `http` tool. Credentials are automatically injected for `api.spotify.com`.

## Base URL

`https://api.spotify.com/v1`

## Actions

**Search:**
```
http(method="GET", url="https://api.spotify.com/v1/search?q=radiohead&type=track,artist&limit=10")
```

**Get my playlists:**
```
http(method="GET", url="https://api.spotify.com/v1/me/playlists?limit=20")
```

**Get playlist tracks:**
```
http(method="GET", url="https://api.spotify.com/v1/playlists/<playlist_id>/tracks?limit=50")
```

**Create playlist:**
```
http(method="POST", url="https://api.spotify.com/v1/users/<user_id>/playlists", body={"name": "My Playlist", "description": "Created via API", "public": false})
```

**Add tracks to playlist:**
```
http(method="POST", url="https://api.spotify.com/v1/playlists/<playlist_id>/tracks", body={"uris": ["spotify:track:4iV5W9uYEdYUVa79Axb7Rh"]})
```

**Get currently playing:**
```
http(method="GET", url="https://api.spotify.com/v1/me/player/currently-playing")
```

**Get artist:**
```
http(method="GET", url="https://api.spotify.com/v1/artists/<artist_id>")
```

**Get album:**
```
http(method="GET", url="https://api.spotify.com/v1/albums/<album_id>")
```

**Get my top tracks:**
```
http(method="GET", url="https://api.spotify.com/v1/me/top/tracks?time_range=medium_term&limit=20")
```

## Notes

- Track URIs: `spotify:track:<id>`. Album: `spotify:album:<id>`.
- Search types: `track`, `artist`, `album`, `playlist`, `show`, `episode`.
- `time_range` for top items: `short_term` (4 weeks), `medium_term` (6 months), `long_term` (years).
- Player control endpoints (play/pause/skip) require `user-modify-playback-state` scope and active device.
- Pagination: `limit` + `offset`. Check `total` and `next` URL.
- Rate limit: varies. Check `Retry-After` header on 429.
