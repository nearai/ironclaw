import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function channelsTabSourceForTest() {
  const source = readFileSync(new URL("./channels-tab.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab, SlackConnectActionSections, isSlackPackage, isSlackAdminManagedAction, isSlackInboundProofCodeAction, findSlackConnectAction, findSlackConnectActions };`;
}

function slackConnectActionSectionsForTest(slackConnectAction, slackConnectActions) {
  const context = {
    globalThis: {},
    SlackChannelPicker() {},
    SlackPairingSection() {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.SlackConnectActionSections({
      slackConnectAction,
      slackConnectActions,
    }),
    SlackChannelPicker: context.SlackChannelPicker,
    SlackPairingSection: context.SlackPairingSection,
  };
}

function channelsTabForTest(props) {
  const context = {
    ExtensionCard() {},
    PairingSection() {},
    RegistryCard() {},
    SlackChannelPicker() {},
    SlackPairingSection() {},
    StatusPill() {},
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useT: () => (key) => key,
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.ChannelsTab(props),
    RegistryCard: context.RegistryCard,
    SlackConnectActionSections: context.globalThis.__testExports.SlackConnectActionSections,
    SlackChannelPicker: context.SlackChannelPicker,
    SlackPairingSection: context.SlackPairingSection,
  };
}

function renderedContainsComponent(rendered, component) {
  if (!rendered || typeof rendered !== "object") {
    return rendered === component;
  }
  if (Array.isArray(rendered)) {
    return rendered.some((value) => renderedContainsComponent(value, component));
  }
  if (Array.isArray(rendered.values)) {
    return rendered.values.some((value) => renderedContainsComponent(value, component));
  }
  return false;
}

function renderedContainsSlackActionPair(rendered) {
  if (!rendered || typeof rendered !== "object") {
    return false;
  }
  if (Array.isArray(rendered)) {
    return rendered.some((value) => renderedContainsSlackActionPair(value));
  }
  if (Array.isArray(rendered.values)) {
    for (const value of rendered.values) {
      if (
        Array.isArray(value) &&
        value.length === 2 &&
        value.every(
          (action) =>
            action?.channel === "slack" &&
            (action.strategy === "admin_managed_channels" ||
              action.strategy === "inbound_proof_code"),
        )
      ) {
        return true;
      }
      if (renderedContainsSlackActionPair(value)) {
        return true;
      }
    }
  }
  return false;
}

function renderedNodeContainingComponent(rendered, component) {
  if (!rendered || typeof rendered !== "object") {
    return undefined;
  }
  if (Array.isArray(rendered)) {
    for (const value of rendered) {
      const found = renderedNodeContainingComponent(value, component);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  if (Array.isArray(rendered.values)) {
    for (const value of rendered.values) {
      const found = renderedNodeContainingComponent(value, component);
      if (found !== undefined) return found;
    }
    if (renderedContainsComponent(rendered.values, component)) {
      return rendered;
    }
  }
  return undefined;
}

test("isSlackPackage recognizes the Slack extension package", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackPackage } = context.globalThis.__testExports;

  assert.equal(isSlackPackage({ package_ref: { id: "slack" } }), true);
  assert.equal(isSlackPackage({ package_ref: { id: "slack_v2" } }), false);
  assert.equal(isSlackPackage({}), false);
});

test("Slack action predicates keep admin picker and proof-code pairing distinct", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackAdminManagedAction, isSlackInboundProofCodeAction } =
    context.globalThis.__testExports;

  assert.equal(
    isSlackAdminManagedAction({ channel: "slack", strategy: "admin_managed_channels" }),
    true,
  );
  assert.equal(
    isSlackInboundProofCodeAction({ channel: "slack", strategy: "inbound_proof_code" }),
    true,
  );
  assert.equal(
    isSlackAdminManagedAction({ channel: "slack", strategy: "inbound_proof_code" }),
    false,
  );
  assert.equal(
    isSlackInboundProofCodeAction({ channel: "teams", strategy: "inbound_proof_code" }),
    false,
  );
});

test("findSlackConnectActions keeps admin channel management and personal pairing", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { findSlackConnectAction, findSlackConnectActions } =
    context.globalThis.__testExports;
  const personal = { channel: "slack", strategy: "inbound_proof_code" };
  const admin = { channel: "slack", strategy: "admin_managed_channels" };

  assert.equal(findSlackConnectAction([personal]), personal);
  assert.equal(findSlackConnectAction([personal, admin]), admin);
  const actions = findSlackConnectActions([personal, admin]);
  assert.equal(actions.length, 2);
  assert.equal(actions[0].strategy, "admin_managed_channels");
  assert.equal(actions[1].strategy, "inbound_proof_code");
});

test("SlackConnectActionSections renders every supported Slack action", () => {
  const personal = { channel: "slack", strategy: "inbound_proof_code", action: {} };
  const admin = { channel: "slack", strategy: "admin_managed_channels", action: {} };

  const adminView = slackConnectActionSectionsForTest(admin);
  assert.equal(adminView.rendered.values[0][0].values[0], adminView.SlackChannelPicker);

  const personalView = slackConnectActionSectionsForTest(personal);
  assert.equal(personalView.rendered.values[0][0].values[0], personalView.SlackPairingSection);

  const combinedView = slackConnectActionSectionsForTest(null, [admin, personal]);
  assert.equal(combinedView.rendered.values[0][0].values[0], combinedView.SlackChannelPicker);
  assert.equal(combinedView.rendered.values[0][1].values[0], combinedView.SlackPairingSection);

  const unhandledView = slackConnectActionSectionsForTest({
    channel: "slack",
    strategy: "admin_managed_unknown",
    action: {},
  });
  assert.equal(unhandledView.rendered, null);
});

test("ChannelsTab keeps Slack controls in the legacy builtin location when Slack is not installed", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [],
    connectableChannels: [
      { channel: "slack", strategy: "admin_managed_channels", action: {} },
      { channel: "slack", strategy: "inbound_proof_code", action: {} },
    ],
    channelRegistry: [{ package_ref: { id: "slack" } }],
    isBusy: false,
    onActivate() {},
    onConfigure() {},
    onInstall() {},
    onRemove() {},
  });

  const builtinSlackSection = renderedNodeContainingComponent(
    view.rendered,
    view.SlackConnectActionSections,
  );
  assert.notEqual(builtinSlackSection, undefined, "expected legacy Slack section");
  assert.equal(renderedContainsComponent(builtinSlackSection, view.SlackConnectActionSections), true);
  assert.equal(renderedContainsSlackActionPair(builtinSlackSection), true);

  // The registry heading is now localized via t(...), so it is an interpolated
  // value rather than a literal in the template strings; locate the registry
  // section by the RegistryCard component instead of by heading text.
  const registryCard = renderedNodeContainingComponent(
    view.rendered,
    view.RegistryCard,
  );
  assert.notEqual(registryCard, undefined, "expected available channels registry card");

  assert.equal(renderedContainsComponent(registryCard, view.RegistryCard), true);
  assert.equal(
    renderedContainsComponent(registryCard, view.SlackConnectActionSections),
    false,
  );
  assert.equal(
    renderedContainsSlackActionPair(registryCard),
    false,
  );
});

test("ChannelsTab renders Slack connect controls under the installed Slack card", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [{ package_ref: { id: "slack" }, kind: "channel", activation_status: "installed" }],
    connectableChannels: [
      { channel: "slack", strategy: "admin_managed_channels", action: {} },
      { channel: "slack", strategy: "inbound_proof_code", action: {} },
    ],
    channelRegistry: [],
    isBusy: false,
    onActivate() {},
    onConfigure() {},
    onInstall() {},
    onRemove() {},
  });

  const installedCard = renderedNodeContainingComponent(
    view.rendered,
    view.SlackConnectActionSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed Slack card wrapper");

  assert.equal(renderedContainsSlackActionPair(installedCard), true);
});
