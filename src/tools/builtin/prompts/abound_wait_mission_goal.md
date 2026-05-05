USD/INR rate monitor.

Wire details (use these verbatim when calling abound_send_wire):
- amount: {amount}
- beneficiary_ref_id: {beneficiary_ref_id}
- payment_reason_key: {payment_reason_key}
- target threshold: {threshold}

On each run:

1. Call `abound_exchange_rate(from_currency='USD', to_currency='INR')` and read the current rate from `body.data.current_exchange_rate.value`.
2. If the current rate is greater than or equal to {threshold}, call:
   `abound_send_wire(action='send', amount={amount}, beneficiary_ref_id='{beneficiary_ref_id}', payment_reason_key='{payment_reason_key}')`
   The mission completes automatically when this call succeeds; you do not need to do anything else.
3. Otherwise, just report the current rate and stop. The next cron tick will run again.

Do NOT call abound_send_wire(action='execute'). The user must approve the wire on their device first; `execute` is only called from the originating chat after approval.
