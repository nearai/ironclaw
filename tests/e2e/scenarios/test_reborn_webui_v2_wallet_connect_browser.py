"""Browser smoke for the isolated WebUI v2 NEAR wallet connect popup.

The real wallet provider is intentionally not used here. The test drives the
served /v2/wallet/connect page through Chromium and intercepts the remote
@hot-labs/near-connect module with a deterministic in-browser connector. That
proves the committed popup can import, connect, sign the fixed NEAR AI message,
and post the success payload over BroadcastChannel without live wallet traffic.
"""

import json

import pytest
from playwright.async_api import expect


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


WALLET_CONNECTOR_STUB = """
export class NearConnector {
  constructor(options) {
    globalThis.__nearConnectorOptions = options;
  }

  async connect() {
    globalThis.__nearConnectorConnectCalled = true;
  }

  async wallet() {
    return {
      async signMessage(input) {
        globalThis.__nearConnectorSignInput = {
          message: input.message,
          recipient: input.recipient,
          nonce: Array.from(input.nonce),
        };
        return {
          accountId: "alice.near",
          publicKey: "ed25519:test-public-key",
          signature: "stub-signature",
        };
      },
    };
  }
}
"""


async def test_wallet_connect_popup_posts_signed_nearai_payload(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 460, "height": 640})
    await context.route(
        "https://esm.sh/@hot-labs/near-connect",
        lambda route: route.fulfill(
            status=200,
            content_type="text/javascript",
            body=WALLET_CONNECTOR_STUB,
        ),
    )
    await context.add_init_script(
        """
        (() => {
          const messages = [];
          const channel = new BroadcastChannel("wallet-smoke-channel");
          window.__walletConnectMessages = messages;
          window.__walletConnectChannel = channel;
          channel.addEventListener("message", (event) => {
            messages.push(event.data);
          });
        })();
        """
    )
    page = await context.new_page()
    try:
        await page.goto(
            f"{reborn_v2_server}/v2/wallet/connect?channel=wallet-smoke-channel",
            wait_until="domcontentloaded",
        )
        await expect(page.locator("#status")).to_contain_text(
            "Signed. You can close this window.",
            timeout=10000,
        )

        messages = []
        for _ in range(50):
            messages = await page.evaluate("window.__walletConnectMessages")
            if messages:
                break
            await page.wait_for_timeout(200)

        assert len(messages) == 1
        payload = messages[0]
        assert payload["type"] == "nearai-wallet-login"
        assert payload["ok"] is True
        assert payload["accountId"] == "alice.near"
        assert payload["publicKey"] == "ed25519:test-public-key"
        assert payload["signature"] == "stub-signature"
        assert payload["message"] == "Sign in to NEAR AI Cloud"
        assert payload["recipient"] == "cloud.near.ai"
        assert len(payload["nonce"]) == 32

        sign_input = await page.evaluate("window.__nearConnectorSignInput")
        assert sign_input["message"] == "Sign in to NEAR AI Cloud"
        assert sign_input["recipient"] == "cloud.near.ai"
        assert sign_input["nonce"] == payload["nonce"]
        connector_options = await page.evaluate(
            "JSON.stringify(window.__nearConnectorOptions)"
        )
        assert json.loads(connector_options) == {
            "network": "mainnet",
            "features": {"signMessage": True},
        }
    finally:
        await context.close()
