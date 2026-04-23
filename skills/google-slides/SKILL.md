---
name: google-slides
version: "1.0.0"
description: Create, read, edit, and format Google Slides presentations via HTTP tool with automatic OAuth credential injection
activation:
  keywords:
    - "slides"
    - "presentation"
    - "google slides"
    - "slideshow"
  patterns:
    - "(?i)(create|edit|make|build).*(slides|presentation|slideshow)"
    - "(?i)google slides"
  tags:
    - "productivity"
    - "slides"
    - "google"
  max_context_tokens: 2500
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "slides.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/presentations"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "slides.googleapis.com"
---

# Google Slides Skill

You have access to the Google Slides API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**.

All Google tools share the same `google_oauth_token`.

## API Patterns

Base URL: `https://slides.googleapis.com/v1/presentations`

### Create a presentation

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations", body={"title": "My Presentation"})
```

Returns `presentationId`.

### Get presentation

```
http(method="GET", url="https://slides.googleapis.com/v1/presentations/{presentationId}")
```

Returns slides with `objectId`, `layoutObjectId`, and page elements.

### Create a slide

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"createSlide": {"insertionIndex": 1, "slideLayoutReference": {"predefinedLayout": "TITLE_AND_BODY"}}}]})
```

- `predefinedLayout`: `BLANK`, `TITLE`, `TITLE_AND_BODY`, `TITLE_AND_TWO_COLUMNS`, `TITLE_ONLY`, `SECTION_HEADER`, etc.
- `insertionIndex`: 0-based position (1 = after title slide)

### Insert text into a shape/text box

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"insertText": {"objectId": "{shapeObjectId}", "text": "Hello World", "insertionIndex": 0}}]})
```

### Replace all text

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"replaceAllText": {"containsText": {"text": "{{title}}", "matchCase": true}, "replaceText": "Q1 Results"}}]})
```

### Create a shape (text box, rectangle)

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"createShape": {"objectId": "myTextBox", "shapeType": "TEXT_BOX", "elementProperties": {"pageObjectId": "{slideObjectId}", "size": {"width": {"magnitude": 400, "unit": "PT"}, "height": {"magnitude": 50, "unit": "PT"}}, "transform": {"scaleX": 1, "scaleY": 1, "translateX": 100, "translateY": 100, "unit": "PT"}}}}]})
```

- `shapeType`: `TEXT_BOX`, `RECTANGLE`, `ROUND_RECTANGLE`, `ELLIPSE`, `ARROW`, etc.
- Positions and sizes use points (PT) with `magnitude` + `unit`.

### Insert an image

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"createImage": {"objectId": "myImage", "elementProperties": {"pageObjectId": "{slideObjectId}", "size": {"width": {"magnitude": 300, "unit": "PT"}, "height": {"magnitude": 200, "unit": "PT"}}, "transform": {"scaleX": 1, "scaleY": 1, "translateX": 50, "translateY": 150, "unit": "PT"}}, "url": "https://example.com/image.png"}}]})
```

### Format text

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"updateTextStyle": {"objectId": "{shapeObjectId}", "style": {"bold": true, "italic": false, "fontSize": {"magnitude": 24, "unit": "PT"}, "fontFamily": "Arial", "foregroundColor": {"opaqueColor": {"rgbColor": {"red": 0.0, "green": 0.0, "blue": 0.0}}}}, "fields": "bold,italic,fontSize,fontFamily,foregroundColor", "range": {"startIndex": 0, "endIndex": 10}}}]})
```

### Delete an object

```
http(method="POST", url="https://slides.googleapis.com/v1/presentations/{presentationId}:batchUpdate", body={"requests": [{"deleteObject": {"objectId": "{objectId}"}}]})
```

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- Almost all mutations use **batchUpdate** with a `requests` array. Group multiple operations in one call.
- Slide and element IDs are string object IDs (not numeric). Get them from the GET response.
- Positions use `transform` with `translateX`/`translateY` in points.
- Sizes use `width`/`height` with `magnitude` + `unit: "PT"`.
- Colors use `rgbColor` with float 0.0–1.0 (not hex).
