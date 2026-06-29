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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab, ChannelConnectActionSections, SlackConnectActionSections, isSlackPackage, isAdminManagedChannelsAction, isInboundProofCodeAction, isSlackAdminManagedAction, isSlackInboundProofCodeAction, connectActionsForChannel, connectActionsForPackage, findSlackConnectAction, findSlackConnectActions };`;
}

function connectActionSectionsForTest(connectAction, connectActions) {
  const context = {
    globalThis: {},
    PairingSection() {},
    SlackAdminManagedSection() {},
    SlackPairingSection() {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    redeemPairingCode() {},
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.ChannelConnectActionSections({
      connectAction,
      connectActions,
    }),
    PairingSection: context.PairingSection,
    SlackAdminManagedSection: context.SlackAdminManagedSection,
    SlackPairingSection: context.SlackPairingSection,
  };
}

function channelsTabForTest(props) {
  const context = {
    ExtensionCard() {},
    PairingSection() {},
    RegistryCard() {},
    SlackChannelPicker() {},
    SlackAdminManagedSection() {},
    SlackPairingSection() {},
    StatusPill() {},
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useT: () => (key) => key,
    redeemPairingCode() {},
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.ChannelsTab(props),
    ChannelConnectActionSections: context.globalThis.__testExports.ChannelConnectActionSections,
    PairingSection: context.PairingSection,
    RegistryCard: context.RegistryCard,
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

function renderedContainsChannelAction(rendered, channel, strategy) {
  if (!rendered || typeof rendered !== "object") {
    return false;
  }
  if (Array.isArray(rendered)) {
    return rendered.some((value) => renderedContainsChannelAction(value, channel, strategy));
  }
  if (Array.isArray(rendered.values)) {
    for (const value of rendered.values) {
      if (
        Array.isArray(value) &&
        value.some((action) => action?.channel === channel && action.strategy === strategy)
      ) {
        return true;
      }
      if (renderedContainsChannelAction(value, channel, strategy)) {
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

function renderedComponentCount(rendered, component) {
  if (!rendered || typeof rendered !== "object") {
    return rendered === component ? 1 : 0;
  }
  if (Array.isArray(rendered)) {
    return rendered.reduce(
      (count, value) => count + renderedComponentCount(value, component),
      0,
    );
  }
  if (Array.isArray(rendered.values)) {
    return rendered.values.reduce(
      (count, value) => count + renderedComponentCount(value, component),
      0,
    );
  }
  return 0;
}

test("isSlackPackage recognizes the Slack extension package", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackPackage } = context.globalThis.__testExports;

  assert.equal(isSlackPackage({ package_ref: { id: "slack" } }), true);
  assert.equal(isSlackPackage({ package_ref: { id: "slack_v2" } }), false);
  assert.equal(isSlackPackage({}), false);
});

test("connect action predicates keep admin picker and proof-code pairing distinct", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const {
    isAdminManagedChannelsAction,
    isInboundProofCodeAction,
    isSlackAdminManagedAction,
    isSlackInboundProofCodeAction,
  } = context.globalThis.__testExports;

  assert.equal(
    isAdminManagedChannelsAction({ channel: "teams", strategy: "admin_managed_channels" }),
    true,
  );
  assert.equal(
    isInboundProofCodeAction({ channel: "teams", strategy: "inbound_proof_code" }),
    true,
  );
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

test("connectActionsForChannel keeps admin channel management and personal pairing", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { connectActionsForChannel, findSlackConnectAction, findSlackConnectActions } =
    context.globalThis.__testExports;
  const personal = { channel: "slack", strategy: "inbound_proof_code" };
  const admin = { channel: "slack", strategy: "admin_managed_channels" };
  const telegram = { channel: "telegram", strategy: "inbound_proof_code" };

  assert.equal(findSlackConnectAction([personal]), personal);
  assert.equal(findSlackConnectAction([personal, admin]), admin);
  const actions = findSlackConnectActions([personal, admin]);
  assert.equal(actions.length, 2);
  assert.equal(actions[0].strategy, "admin_managed_channels");
  assert.equal(actions[1].strategy, "inbound_proof_code");
  const telegramActions = connectActionsForChannel([personal, admin, telegram], "telegram");
  assert.equal(telegramActions.length, 1);
  assert.equal(telegramActions[0].channel, "telegram");
  assert.equal(telegramActions[0].strategy, "inbound_proof_code");
});

test("ChannelConnectActionSections renders Slack setup and proof-code actions", () => {
  const personal = { channel: "slack", strategy: "inbound_proof_code", action: {} };
  const admin = { channel: "slack", strategy: "admin_managed_channels", action: {} };

  const adminView = connectActionSectionsForTest(admin);
  assert.equal(adminView.rendered.values[0][0].values[0], adminView.SlackAdminManagedSection);

  const personalView = connectActionSectionsForTest(personal);
  assert.equal(personalView.rendered.values[0][0].values[0], personalView.SlackPairingSection);

  const combinedView = connectActionSectionsForTest(null, [admin, personal]);
  assert.equal(combinedView.rendered.values[0][0].values[0], combinedView.SlackAdminManagedSection);
  assert.equal(combinedView.rendered.values[0][1].values[0], combinedView.SlackPairingSection);

  const unhandledView = connectActionSectionsForTest({
    channel: "slack",
    strategy: "admin_managed_unknown",
    action: {},
  });
  assert.equal(unhandledView.rendered, null);
});

test("ChannelConnectActionSections renders non-Slack proof-code actions with generic pairing", () => {
  const telegram = {
    channel: "telegram",
    strategy: "inbound_proof_code",
    action: { title: "Telegram account connection" },
  };

  const view = connectActionSectionsForTest(telegram);

  assert.equal(view.rendered.values[0][0].values[0], view.PairingSection);
  assert.equal(view.rendered.values[0][0].values[1], "telegram");
  assert.deepEqual(view.rendered.values[0][0].values[2], telegram.action);
});

test("ChannelsTab keeps Slack controls in the builtin location when Slack is not installed", () => {
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
    view.ChannelConnectActionSections,
  );
  assert.notEqual(builtinSlackSection, undefined, "expected builtin Slack section");
  assert.equal(renderedContainsComponent(builtinSlackSection, view.ChannelConnectActionSections), true);
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
    renderedContainsComponent(registryCard, view.ChannelConnectActionSections),
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
    view.ChannelConnectActionSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed Slack card wrapper");

  assert.equal(renderedContainsSlackActionPair(installedCard), true);
});

test("ChannelsTab renders generic connect controls under installed non-Slack channels", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [
      { package_ref: { id: "telegram" }, kind: "channel", activation_status: "installed" },
    ],
    connectableChannels: [
      {
        channel: "telegram",
        strategy: "inbound_proof_code",
        action: { title: "Telegram account connection" },
      },
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
    view.ChannelConnectActionSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed channel card wrapper");
  assert.equal(
    renderedContainsChannelAction(installedCard, "telegram", "inbound_proof_code"),
    true,
  );
});

test("ChannelsTab does not render duplicate fallback pairing when connect action owns pairing", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [
      {
        package_ref: { id: "telegram" },
        kind: "channel",
        activation_status: "installed",
        onboarding_state: "pairing_required",
      },
    ],
    connectableChannels: [
      {
        channel: "telegram",
        strategy: "inbound_proof_code",
        action: { title: "Telegram account connection" },
      },
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
    view.ChannelConnectActionSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed channel card wrapper");
  assert.equal(
    renderedContainsChannelAction(installedCard, "telegram", "inbound_proof_code"),
    true,
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.ChannelConnectActionSections),
    1,
    "the connect action still owns the pairing UI",
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "the legacy fallback pairing section must not duplicate the connect action",
  );
});
