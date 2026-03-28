---
name: cloudinary
version: "1.0.0"
description: Cloudinary API — Cloudinary is a cloud-based media management platform that enables developers an
activation:
  keywords:
    - "cloudinary"
    - "media management"
  patterns:
    - "(?i)cloudinary"
  tags:
    - "tools"
    - "media-management"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CLOUDINARY_USERNAME, CLOUDINARY_PASSWORD, CLOUDINARY_CLOUD_NAME]
---

# Cloudinary API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://api.cloudinary.com/v1_1/{CLOUDINARY_CLOUD_NAME}`

**Content-Type**: `application/x-www-form-urlencoded` for POST/PUT requests.

## Actions

**Upload image:**
```
http(method="POST", url="https://api.cloudinary.com/v1_1/{CLOUDINARY_CLOUD_NAME}/image/upload", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="file=https://example.com/image.jpg&upload_preset=my_preset")
```

**Get resource details:**
```
http(method="GET", url="https://api.cloudinary.com/v1_1/{CLOUDINARY_CLOUD_NAME}/resources/image/upload/{public_id}")
```

**List resources:**
```
http(method="GET", url="https://api.cloudinary.com/v1_1/{CLOUDINARY_CLOUD_NAME}/resources/image?max_results=10")
```

**Delete resource:**
```
http(method="DELETE", url="https://api.cloudinary.com/v1_1/{CLOUDINARY_CLOUD_NAME}/resources/image/upload?public_ids[]={public_id}")
```

## Notes

- Uses Basic auth with API key and secret.
- Upload accepts URL, base64, or file.
- Transformations via URL: `https://res.cloudinary.com/{cloud}/image/upload/w_300,h_200,c_fill/{public_id}`.
- Upload presets define default transformations and folders.
