// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function channelsTabSourceForTest() {
  const source = readFileSync(new URL("./channels-tab.tsx", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab, ChannelConnectActionSections, SlackConnectActionSections, isSlackPackage, isAdminManagedChannelsAction, isInboundProofCodeAction, isWebGeneratedCodeAction, isGenericInboundProofCodeAction, isSlackAdminManagedAction, isTelegramAdminManagedAction, isTelegramWebGeneratedCodeAction, connectActionsForChannel, connectActionsForPackage, findSlackConnectAction, findSlackConnectActions };`;
}

function connectActionSectionsForTest(connectAction, connectActions) {
  const context = {
    globalThis: {},
    PairingSection() {},
    SlackAdminManagedSection() {},
    TelegramAdminManagedSection() {},
    TelegramPairingPanel() {},
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
    TelegramAdminManagedSection: context.TelegramAdminManagedSection,
    TelegramPairingPanel: context.TelegramPairingPanel,
  };
}

function channelsTabForTest(props) {
  const context = {
    ExtensionCard() {},
    PairingSection() {},
    RegistryCard() {},
    SlackChannelPicker() {},
    SlackAdminManagedSection() {},
    TelegramAdminManagedSection() {},
    TelegramPairingPanel() {},
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
    TelegramAdminManagedSection: context.TelegramAdminManagedSection,
    TelegramPairingPanel: context.TelegramPairingPanel,
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

function renderedContainsSlackAdminAction(rendered) {
  return renderedContainsChannelAction(rendered, "slack", "admin_managed_channels");
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

function componentProps(rendered, component) {
  const node = renderedNodeContainingComponent(rendered, component);
  assert.notEqual(node, undefined, "expected rendered component");
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

test("isSlackPackage recognizes the Slack extension package", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackPackage } = context.globalThis.__testExports;

  assert.equal(isSlackPackage({ package_ref: { id: "slack" } }), true);
  assert.equal(isSlackPackage({ package_ref: { id: "slack_v2" } }), false);
  assert.equal(isSlackPackage({}), false);
});

test("connect action predicates keep admin picker and generic proof-code pairing distinct", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const {
    isAdminManagedChannelsAction,
    isInboundProofCodeAction,
    isGenericInboundProofCodeAction,
    isSlackAdminManagedAction,
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
    isGenericInboundProofCodeAction({ channel: "teams", strategy: "inbound_proof_code" }),
    true,
  );
  assert.equal(
    isSlackAdminManagedAction({ channel: "slack", strategy: "oauth" }),
    false,
  );
  assert.equal(
    isGenericInboundProofCodeAction({ channel: "slack", strategy: "oauth" }),
    false,
  );
});

test("connectActionsForChannel keeps Slack admin management and generic code-entry setup separate", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { connectActionsForChannel, findSlackConnectAction, findSlackConnectActions } =
    context.globalThis.__testExports;
  const personal = { channel: "slack", strategy: "oauth" };
  const admin = { channel: "slack", strategy: "admin_managed_channels" };
  const telegram = { channel: "telegram", strategy: "inbound_proof_code" };

  assert.equal(findSlackConnectAction([personal]), null);
  assert.equal(findSlackConnectAction([personal, admin]), admin);
  const actions = findSlackConnectActions([personal, admin]);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].strategy, "admin_managed_channels");
  const telegramActions = connectActionsForChannel([personal, admin, telegram], "telegram");
  assert.equal(telegramActions.length, 1);
  assert.equal(telegramActions[0].channel, "telegram");
  assert.equal(telegramActions[0].strategy, "inbound_proof_code");
});

test("ChannelConnectActionSections renders Slack admin setup and code entry only for other channels", () => {
  const personal = { channel: "slack", strategy: "oauth", action: {} };
  const admin = { channel: "slack", strategy: "admin_managed_channels", action: {} };

  const adminView = connectActionSectionsForTest(admin);
  assert.deepEqual(
    componentProps(adminView.rendered, adminView.SlackAdminManagedSection).action,
    admin.action,
  );

  const personalView = connectActionSectionsForTest(personal);
  assert.equal(personalView.rendered, null);

  const combinedView = connectActionSectionsForTest(null, [admin, personal]);
  assert.equal(
    renderedComponentCount(combinedView.rendered, combinedView.SlackAdminManagedSection),
    1,
  );
  assert.equal(renderedComponentCount(combinedView.rendered, combinedView.PairingSection), 0);

  const unhandledView = connectActionSectionsForTest({
    channel: "slack",
    strategy: "admin_managed_unknown",
    action: {},
  });
  assert.equal(unhandledView.rendered, null);
});

test("ChannelConnectActionSections renders manual-code channel actions with generic pairing", () => {
  const telegram = {
    channel: "telegram",
    strategy: "inbound_proof_code",
    action: { title: "Telegram account connection" },
  };

  const view = connectActionSectionsForTest(telegram);
  const props = componentProps(view.rendered, view.PairingSection);

  assert.equal(props.channel, "telegram");
  assert.deepEqual(props.copy, telegram.action);
});

test("ChannelsTab keeps Slack controls in the builtin location when Slack is not installed", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [],
    connectableChannels: [
      { channel: "slack", strategy: "admin_managed_channels", action: {} },
      { channel: "slack", strategy: "oauth", action: {} },
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
  assert.equal(renderedContainsSlackAdminAction(builtinSlackSection), true);

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
});

test("ChannelsTab keeps the Telegram bot-setup card in the builtin location when Telegram is not installed", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [],
    connectableChannels: [
      { channel: "telegram", strategy: "admin_managed_channels", action: {} },
    ],
    channelRegistry: [{ package_ref: { id: "telegram" } }],
    isBusy: false,
    onActivate() {},
    onConfigure() {},
    onInstall() {},
    onRemove() {},
  });

  // The operator's bot-setup card must render pre-install (design spec §2:
  // the Channels tab shows the Telegram card exactly like Slack's), not be
  // dropped until the extension is installed.
  const builtinTelegramSection = renderedNodeContainingComponent(
    view.rendered,
    view.ChannelConnectActionSections,
  );
  assert.notEqual(builtinTelegramSection, undefined, "expected builtin Telegram section");
  assert.equal(
    renderedContainsChannelAction(builtinTelegramSection, "telegram", "admin_managed_channels"),
    true,
  );

  const registryCard = renderedNodeContainingComponent(view.rendered, view.RegistryCard);
  assert.notEqual(registryCard, undefined, "expected available channels registry card");
  assert.equal(
    renderedContainsComponent(registryCard, view.ChannelConnectActionSections),
    false,
  );
});

test("ChannelsTab moves Telegram connect controls under the installed Telegram card", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [
      { package_ref: { id: "telegram" }, kind: "channel", activation_status: "installed" },
    ],
    connectableChannels: [
      { channel: "telegram", strategy: "admin_managed_channels", action: {} },
      { channel: "telegram", strategy: "web_generated_code", action: {} },
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
  assert.notEqual(installedCard, undefined, "expected installed Telegram card wrapper");
  assert.equal(
    renderedContainsChannelAction(installedCard, "telegram", "admin_managed_channels"),
    true,
  );
  // Exactly one connect-actions section: the builtin row must not duplicate
  // the installed card's controls.
  assert.equal(
    renderedComponentCount(view.rendered, view.ChannelConnectActionSections),
    1,
  );
});

test("ChannelsTab renders Slack connect controls under the installed Slack card", () => {
  const view = channelsTabForTest({
    status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
    channels: [{ package_ref: { id: "slack" }, kind: "channel", activation_status: "installed" }],
    connectableChannels: [
      { channel: "slack", strategy: "admin_managed_channels", action: {} },
      { channel: "slack", strategy: "oauth", action: {} },
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

  assert.equal(renderedContainsSlackAdminAction(installedCard), true);
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "installed Slack must not fall back to the generic code-entry section",
  );
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

test("telegram connect action predicates route admin setup and web-generated pairing", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const {
    isWebGeneratedCodeAction,
    isTelegramAdminManagedAction,
    isTelegramWebGeneratedCodeAction,
    isGenericInboundProofCodeAction,
  } = context.globalThis.__testExports;

  assert.equal(
    isWebGeneratedCodeAction({ channel: "telegram", strategy: "web_generated_code" }),
    true,
  );
  assert.equal(
    isTelegramAdminManagedAction({ channel: "telegram", strategy: "admin_managed_channels" }),
    true,
  );
  assert.equal(
    isTelegramAdminManagedAction({ channel: "slack", strategy: "admin_managed_channels" }),
    false,
  );
  assert.equal(
    isTelegramWebGeneratedCodeAction({ channel: "telegram", strategy: "web_generated_code" }),
    true,
  );
  assert.equal(
    isTelegramWebGeneratedCodeAction({ channel: "telegram", strategy: "inbound_proof_code" }),
    false,
  );
  assert.equal(
    isGenericInboundProofCodeAction({ channel: "telegram", strategy: "web_generated_code" }),
    false,
    "web-generated pairing must not fall into the paste-code section",
  );
});

test("connectActionsForChannel returns telegram admin and web-generated-code actions together", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { connectActionsForChannel } = context.globalThis.__testExports;
  const slackAdmin = { channel: "slack", strategy: "admin_managed_channels" };
  const telegramAdmin = { channel: "telegram", strategy: "admin_managed_channels" };
  const telegramPairing = { channel: "telegram", strategy: "web_generated_code" };

  const actions = connectActionsForChannel(
    [slackAdmin, telegramPairing, telegramAdmin],
    "telegram",
  );
  assert.equal(actions.length, 2);
  assert.equal(actions[0].strategy, "admin_managed_channels");
  assert.equal(actions[1].strategy, "web_generated_code");
  assert.ok(actions.every((action) => action.channel === "telegram"));
});

test("ChannelConnectActionSections renders telegram admin setup and the web-generated pairing panel", () => {
  const admin = {
    channel: "telegram",
    strategy: "admin_managed_channels",
    action: { title: "Telegram bot" },
  };
  const pairing = { channel: "telegram", strategy: "web_generated_code", action: {} };

  const view = connectActionSectionsForTest(null, [admin, pairing]);
  assert.equal(
    renderedComponentCount(view.rendered, view.TelegramAdminManagedSection),
    1,
  );
  assert.deepEqual(
    componentProps(view.rendered, view.TelegramAdminManagedSection).action,
    admin.action,
  );
  assert.equal(renderedComponentCount(view.rendered, view.TelegramPairingPanel), 1);
  // Neither telegram surface leaks into the Slack or paste-code sections.
  assert.equal(renderedComponentCount(view.rendered, view.SlackAdminManagedSection), 0);
  assert.equal(renderedComponentCount(view.rendered, view.PairingSection), 0);
});

test("ChannelsTab renders no fallback paste pairing when telegram web-generated pairing owns onboarding", () => {
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
      { channel: "telegram", strategy: "admin_managed_channels", action: {} },
      { channel: "telegram", strategy: "web_generated_code", action: {} },
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
  assert.notEqual(installedCard, undefined, "expected installed telegram card wrapper");
  assert.equal(
    renderedContainsChannelAction(installedCard, "telegram", "admin_managed_channels"),
    true,
  );
  assert.equal(
    renderedContainsChannelAction(installedCard, "telegram", "web_generated_code"),
    true,
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.ChannelConnectActionSections),
    1,
    "the connect actions own both telegram surfaces",
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "the web-generated pairing panel owns onboarding; no paste-code fallback",
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
