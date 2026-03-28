---
name: google-ads
version: "1.0.0"
description: Google Ads API — Google Ads is an online advertising platform that enables businesses to promote 
activation:
  keywords:
    - "google-ads"
    - "google ads"
    - "ads"
  patterns:
    - "(?i)google.?ads"
  tags:
    - "tools"
    - "ads"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ADS_CUSTOMER_ID]
---

# Google Ads API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://googleads.googleapis.com/v17`

## Actions

**Search campaigns:**
```
http(method="POST", url="https://googleads.googleapis.com/v17/customers/{customer_id}/googleAds:searchStream", body={"query": "SELECT campaign.name, campaign.status, metrics.impressions FROM campaign WHERE campaign.status = 'ENABLED' LIMIT 10"})
```

**List accessible customers:**
```
http(method="GET", url="https://googleads.googleapis.com/v17/customers:listAccessibleCustomers")
```

## Notes

- Uses OAuth 2.0 and requires `developer-token` header.
- Queries use GAQL (Google Ads Query Language).
- Customer IDs are 10-digit numbers without dashes.
- Resources: `campaign`, `ad_group`, `ad_group_ad`, `keyword_view`.
