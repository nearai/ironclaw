"""Direct tests against Abound's dev API — verifies endpoints work independently of IronClaw.

Usage:
    python tests/scripts/test_abound_api_direct.py
"""

import os

import requests

# Credentials — set these env vars before running
ABOUND_BEARER_TOKEN = os.environ["ABOUND_BEARER_TOKEN"]
ABOUND_API_KEY = os.environ["ABOUND_API_KEY"]
ABOUND_WRITE_TOKEN = os.environ.get("ABOUND_WRITE_TOKEN", "")

HEADERS = {
    "Authorization": f"Bearer {ABOUND_BEARER_TOKEN}",
    "Content-Type": "application/json",
    "X-API-KEY": ABOUND_API_KEY,
    "device-type": "WEB",
}

WRITE_HEADERS = {
    "Authorization": f"Bearer {ABOUND_WRITE_TOKEN}",
    "Content-Type": "application/json",
    "X-API-KEY": ABOUND_API_KEY,
    "device-type": "WEB",
}

passed = 0
failed = 0


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}")
        if detail:
            print(f"    {detail[:500]}")
        failed += 1


# -----------------------------------------------------------
# 1. Get Account Info
# -----------------------------------------------------------
print("--- 1. Get Account Info ---")
r = requests.get(
    "https://devneobank.timesclub.co/times/bank/remittance/agent/account/info",
    headers=HEADERS,
    timeout=15,
)
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text[:300]}")
if r.status_code == 200:
    data = r.json()
    print('data', data, '\n\n\n')
    check("status success", data.get("status") == "success", str(data)[:300])
    acct = data.get("data", {})
    check("has user_id", "user_id" in acct, str(acct.keys()))
    check("has limits", "limits" in acct)
    check("has payment_reasons", "payment_reasons" in acct)
    check("has recipients", "recipients" in acct)
    check("has funding_sources", "funding_sources" in acct)
    print(f"  Account: {acct.get('user_name', 'N/A')} ({acct.get('user_id', 'N/A')})")
    if acct.get("recipients"):
        print(f"  Recipients: {[r['name'] for r in acct['recipients']]}")
    if acct.get("funding_sources"):
        print(f"  Funding sources: {[f['bank_name'] for f in acct['funding_sources']]}")
print()

# -----------------------------------------------------------
# 2. Get Exchange Rate
# -----------------------------------------------------------
print("--- 2. Get Exchange Rate ---")
r = requests.get(
    "https://devneobank.timesclub.co/times/bank/remittance/agent/exchange-rate",
    params={"from_currency": "USD", "to_currency": "INR"},
    headers=HEADERS,
    timeout=15,
)
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text[:300]}")
if r.status_code == 200:
    data = r.json()
    check("status success", data.get("status") == "success", str(data)[:300])
    rates = data.get("data", {})
    check("has current_exchange_rate", "current_exchange_rate" in rates)
    check("has effective_exchange_rate", "effective_exchange_rate" in rates)
    current = rates.get("current_exchange_rate", {})
    effective = rates.get("effective_exchange_rate", {})
    print(f"  Current rate: {current.get('formatted_value', 'N/A')}")
    print(f"  Effective rate: {effective.get('formatted_value', 'N/A')}")
print()

# -----------------------------------------------------------
# 3. Create Notification
# -----------------------------------------------------------
print("--- 3. Create Notification ---")
r = requests.post(
    "https://dev.timesclub.co/times/users/agent/create-notification",
    headers=HEADERS,
    json={
        "message_id": "test_notif_001",
        "action_type": "notification",
        "meta_data": {
            "score": 72,
            "rate": 85.42,
            "ma50": 84.50,
            "month_bias": 0.65,
        },
    },
    timeout=15,
)
check("status 2xx", r.status_code in (200, 202), f"got {r.status_code}: {r.text[:300]}")
if r.status_code in (200, 202):
    data = r.json()
    check("status accepted", data.get("status") in ("accepted", "success"), str(data)[:300])
print()

# -----------------------------------------------------------
# 4. Send Wire (DRY RUN — only validate request shape, don't actually send)
# -----------------------------------------------------------
print("--- 4. Send Wire (auth check — $1 test) ---")
r = requests.post(
    "https://devneobank.timesclub.co/times/bank/remittance/agent/send-wire",
    headers=WRITE_HEADERS,
    json={
        "funding_source_id": "BEz8mWag3rIKRmkqvdg5sgBQamm41QH4xPrpq",
        "beneficiary_ref_id": "f8048224-6283-4dd6-b473-53e66c05428d",
        "amount": 1.00,
        "payment_reason_key": "IR001",
    },
    timeout=15,
)
check("authenticated (not 401)", r.status_code != 401, f"got {r.status_code}: {r.text[:300]}")
if r.status_code != 401:
    data = r.json()
    print(f"  Response: {r.status_code} — {str(data)[:200]}")
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
