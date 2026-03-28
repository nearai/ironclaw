---
name: microsoft-dynamics-365-business-central
version: "1.0.0"
description: Microsoft Dynamics 365 Business Central API — A comprehensive business management solution that helps small and mid-sized comp
activation:
  keywords:
    - "microsoft-dynamics-365-business-central"
    - "microsoft dynamics 365 business central"
    - "accounting"
  patterns:
    - "(?i)microsoft.?dynamics.?365.?business.?central"
  tags:
    - "accounting"
    - "finance"
    - "Accounting"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ENVIRONMENT_NAME, BUSINESS_CENTRAL_TENANT_ID]
---

# Microsoft Dynamics 365 Business Central API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.businesscentral.dynamics.com/v2.0/{DYNAMICS_TENANT_ID}/{DYNAMICS_ENVIRONMENT}/api/v2.0`

## Actions

**List companies:**
```
http(method="GET", url="https://api.businesscentral.dynamics.com/v2.0/{DYNAMICS_TENANT_ID}/{DYNAMICS_ENVIRONMENT}/api/v2.0/companies")
```

**List customers:**
```
http(method="GET", url="https://api.businesscentral.dynamics.com/v2.0/{DYNAMICS_TENANT_ID}/{DYNAMICS_ENVIRONMENT}/api/v2.0/companies({company_id})/customers?$top=10")
```

**List items:**
```
http(method="GET", url="https://api.businesscentral.dynamics.com/v2.0/{DYNAMICS_TENANT_ID}/{DYNAMICS_ENVIRONMENT}/api/v2.0/companies({company_id})/items?$top=10")
```

**List sales orders:**
```
http(method="GET", url="https://api.businesscentral.dynamics.com/v2.0/{DYNAMICS_TENANT_ID}/{DYNAMICS_ENVIRONMENT}/api/v2.0/companies({company_id})/salesOrders?$top=10")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Company ID is a GUID.
- OData query params: `$filter`, `$select`, `$expand`, `$top`, `$skip`.
- Resources: customers, vendors, items, salesOrders, purchaseOrders, journals.
