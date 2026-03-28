---
name: salesforce
version: "1.0.0"
description: Salesforce REST API — accounts, contacts, leads, opportunities, SOQL
activation:
  keywords:
    - "salesforce"
    - "sfdc"
    - "sobject"
    - "lead"
    - "opportunity"
  exclude_keywords:
    - "hubspot"
    - "pipedrive"
  patterns:
    - "(?i)salesforce.*(account|contact|lead|opportunity|case)"
    - "(?i)(create|update|query).*salesforce"
    - "(?i)soql"
  tags:
    - "crm"
    - "sales"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [SALESFORCE_INSTANCE_URL, SALESFORCE_ACCESS_TOKEN]
---

# Salesforce REST API

Use the `http` tool. Credentials are automatically injected for your Salesforce instance.

## Base URL

`https://{SALESFORCE_INSTANCE_URL}/services/data/v59.0`

Example: `https://yourorg.my.salesforce.com/services/data/v59.0`

## Actions

**SOQL query:**
```
http(method="GET", url="https://{instance}/services/data/v59.0/query?q=SELECT+Id,Name,Email+FROM+Contact+WHERE+LastName='Smith'+LIMIT+10")
```

**Get record:**
```
http(method="GET", url="https://{instance}/services/data/v59.0/sobjects/Account/<record_id>")
```

**Create record:**
```
http(method="POST", url="https://{instance}/services/data/v59.0/sobjects/Lead", body={"FirstName": "John", "LastName": "Doe", "Company": "Acme", "Email": "john@acme.com"})
```

**Update record:**
```
http(method="PATCH", url="https://{instance}/services/data/v59.0/sobjects/Lead/<record_id>", body={"Status": "Contacted"})
```

**Delete record:**
```
http(method="DELETE", url="https://{instance}/services/data/v59.0/sobjects/Lead/<record_id>")
```

**Search (SOSL):**
```
http(method="GET", url="https://{instance}/services/data/v59.0/search?q=FIND+{John}+IN+ALL+FIELDS+RETURNING+Contact(Name,Email),Lead(Name,Company)")
```

**Describe object (get fields):**
```
http(method="GET", url="https://{instance}/services/data/v59.0/sobjects/Account/describe")
```

**List recent records:**
```
http(method="GET", url="https://{instance}/services/data/v59.0/recent?limit=10")
```

## Common Objects

| Object | Key Fields |
|--------|-----------|
| Account | Name, Industry, Website, Phone |
| Contact | FirstName, LastName, Email, AccountId |
| Lead | FirstName, LastName, Company, Status, Email |
| Opportunity | Name, StageName, Amount, CloseDate, AccountId |
| Case | Subject, Description, Status, Priority, ContactId |

## Notes

- SOQL must be URL-encoded. `+` replaces spaces in query param.
- Record IDs are 15 or 18 character strings (e.g., `001xx000003DGbYAAW`).
- Create returns `{"id": "...", "success": true}`.
- Update returns 204 No Content on success.
- Pagination: if `nextRecordsUrl` is present in query response, fetch it for more results.
- Date format: `YYYY-MM-DD`. DateTime: `YYYY-MM-DDThh:mm:ss.000+0000`.
