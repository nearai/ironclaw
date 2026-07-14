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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab, ChannelConnectSections, OAuthChannelConnectionSection, isSlackPackage, channelSurface, channelConnection, isInboundProofCodeConnection, isOauthConnection };`;
}

function channelConnectSectionsForTest(item) {
  const context = {
    globalThis: {},
    PairingSection() {},
    SlackAdminManagedSection() {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    redeemPairingCode() {},
    onConfigure() {},
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.ChannelConnectSections({ item }),
    OAuthChannelConnectionSection: context.globalThis.__testExports.OAuthChannelConnectionSection,
    PairingSection: context.PairingSection,
    SlackAdminManagedSection: context.SlackAdminManagedSection,
    redeemPairingCode: context.redeemPairingCode,
    onConfigure: context.onConfigure,
  };
}

function channelsTabForTest(props) {
  const context = {
    ExtensionCard() {},
    PairingSection() {},
    RegistryCard() {},
    SlackAdminManagedSection() {},
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
    ChannelConnectSections: context.globalThis.__testExports.ChannelConnectSections,
    ExtensionCard: context.ExtensionCard,
    PairingSection: context.PairingSection,
    RegistryCard: context.RegistryCard,
    SlackAdminManagedSection: context.SlackAdminManagedSection,
    redeemPairingCode: context.redeemPairingCode,
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

function componentPropAfter(node, component) {
  const index = node.values.indexOf(component);
  assert.notEqual(index, -1, "expected the component reference in the node");
  return node.values[index + 1];
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

function elementUses(node, component) {
  return Boolean(
    node && typeof node === "object" && Array.isArray(node.values) && node.values[0] === component,
  );
}

function renderedNodeWhoseChildrenContain(rendered, components) {
  if (!rendered || typeof rendered !== "object") return undefined;
  const nodes = Array.isArray(rendered) ? rendered.flat(Infinity) : [rendered];
  for (const node of nodes) {
    if (!node || typeof node !== "object") continue;
    const children = Array.isArray(node.children) ? node.children.flat(Infinity) : [];
    if (
      components.every((component) => children.some((child) => elementUses(child, component)))
    ) {
      return node;
    }
    for (const child of children) {
      const found = renderedNodeWhoseChildrenContain(child, components);
      if (found !== undefined) return found;
    }
  }
  return undefined;
}

const TAB_PROPS = {
  status: { enabled_channels: [], sse_connections: 0, ws_connections: 0 },
  channels: [],
  channelRegistry: [],
  isBusy: false,
  onActivate() {},
  onConfigure() {},
  onInstall() {},
  onRemove() {},
};

test("isSlackPackage recognizes the Slack extension package", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackPackage } = context.globalThis.__testExports;

  assert.equal(isSlackPackage({ package_ref: { id: "slack" } }), true);
  assert.equal(isSlackPackage({ package_ref: { id: "slack_v2" } }), false);
  assert.equal(isSlackPackage({}), false);
});

test("channelSurface and channelConnection extract the typed channel surface", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { channelSurface, channelConnection, isInboundProofCodeConnection, isOauthConnection } =
    context.globalThis.__testExports;

  const connection = { channel: "telegram", strategy: "inbound_proof_code" };
  const surface = {
    kind: "channel",
    inbound: true,
    outbound: true,
    connected: false,
    connection,
  };
  const item = {
    package_ref: { id: "telegram" },
    surfaces: [{ kind: "tool" }, { kind: "auth" }, surface],
  };

  assert.equal(channelSurface(item), surface);
  assert.equal(channelSurface({ surfaces: [{ kind: "tool" }, { kind: "auth" }] }), null);
  assert.equal(channelSurface({}), null);

  assert.equal(channelConnection(item), connection);
  assert.equal(
    channelConnection({ surfaces: [{ kind: "channel", inbound: true, outbound: false }] }),
    null,
    "a channel surface without a connect affordance yields no connection",
  );

  assert.equal(isInboundProofCodeConnection(connection), true);
  assert.equal(isInboundProofCodeConnection({ strategy: "oauth" }), false);
  assert.equal(isInboundProofCodeConnection({ strategy: "admin_managed_channels" }), false);
  assert.equal(isInboundProofCodeConnection(null), false);
  assert.equal(isOauthConnection({ strategy: "oauth" }), true);
  assert.equal(isOauthConnection({ strategy: "inbound_proof_code" }), false);
  assert.equal(isOauthConnection(null), false);
});

test("ChannelConnectSections renders Slack admin management only for the Slack package", () => {
  const slackView = channelConnectSectionsForTest({
    package_ref: { id: "slack" },
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connected: false,
        // Even an inbound-proof-code connection must not turn Slack into the
        // generic pairing card: the Slack branch wins.
        connection: { channel: "slack", strategy: "inbound_proof_code" },
      },
    ],
  });
  const sections = slackView.rendered.children;
  assert.equal(sections.length, 1);
  assert.equal(
    sections[0].values[0],
    slackView.SlackAdminManagedSection,
    "the Slack package renders the admin-managed section",
  );
  assert.equal(
    sections[0].strings.join(" ").includes("action="),
    false,
    "the Slack section takes no action prop: it self-gates on the operator setup query",
  );
  assert.equal(renderedComponentCount(slackView.rendered, slackView.PairingSection), 0);

  const teamsAdminView = channelConnectSectionsForTest({
    package_ref: { id: "teams" },
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connection: { channel: "teams", strategy: "admin_managed_channels" },
      },
    ],
  });
  assert.equal(
    teamsAdminView.rendered,
    null,
    "the admin-managed picker is Slack-only; other channels get nothing here",
  );
});

test("ChannelConnectSections renders inbound-proof-code surfaces as pairing with the connection copy", () => {
  const connection = {
    channel: "telegram",
    strategy: "inbound_proof_code",
    instructions: "Message the bot, then paste the code it replies with.",
    input_placeholder: "ABC123",
    submit_label: "Connect Telegram",
    error_message: "Invalid or expired pairing code.",
  };
  const view = channelConnectSectionsForTest({
    package_ref: { id: "telegram" },
    surfaces: [
      { kind: "channel", inbound: true, outbound: true, connected: false, connection },
    ],
  });

  const section = view.rendered.children[0];
  assert.equal(section.values[0], view.PairingSection);
  const props = componentProps(view.rendered, view.PairingSection);
  assert.equal(props.channel, "telegram");
  assert.equal(props.copy, connection, "the pairing copy IS the surface connection");
  assert.equal(props.redeemFn, view.redeemPairingCode);
  assert.equal(props.showPendingRequests, false, "no operator pending-requests list on the user card");
  assert.deepEqual(JSON.parse(JSON.stringify(props.queryKeys)), [
    ["extensions"],
    ["pairing", "telegram"],
  ]);

  // A connection without an explicit channel falls back to the package id.
  const fallbackView = channelConnectSectionsForTest({
    package_ref: { id: "telegram" },
    surfaces: [{ kind: "channel", connection: { strategy: "inbound_proof_code" } }],
  });
  const fallbackProps = componentProps(fallbackView.rendered, fallbackView.PairingSection);
  assert.equal(fallbackProps.channel, "telegram");
  assert.deepEqual(JSON.parse(JSON.stringify(fallbackProps.queryKeys)), [
    ["extensions"],
    ["pairing", "telegram"],
  ]);

  const oauthConnection = {
    channel: "telegram",
    strategy: "oauth",
    instructions: "Connect Telegram with OAuth from the extension configuration.",
    submit_label: "Connect Telegram",
  };
  const oauthView = channelConnectSectionsForTest({
    package_ref: { id: "telegram" },
    surfaces: [{ kind: "channel", connection: oauthConnection }],
  });
  const oauthSection = oauthView.rendered.children[0];
  assert.equal(
    oauthSection.values[0],
    oauthView.OAuthChannelConnectionSection,
    "OAuth channel surfaces render their typed connection copy section",
  );
  assert.equal(
    componentProps(oauthView.rendered, oauthView.OAuthChannelConnectionSection).connection,
    oauthConnection,
    "the OAuth card copy is the surface connection",
  );
  assert.equal(
    channelConnectSectionsForTest({ package_ref: { id: "telegram" }, surfaces: [] }).rendered,
    null,
  );
});

test("ChannelConnectSections renders Slack admin setup alongside Slack OAuth connection copy", () => {
  const connection = {
    channel: "slack",
    strategy: "oauth",
    instructions: "Connect Slack with OAuth from the extension configuration.",
    submit_label: "Connect Slack",
  };
  const view = channelConnectSectionsForTest({
    package_ref: { id: "slack" },
    surfaces: [
      { kind: "tool" },
      { kind: "auth" },
      { kind: "channel", inbound: true, outbound: true, connected: false, connection },
    ],
  });

  assert.equal(view.rendered.children.length, 2);
  assert.equal(view.rendered.children[0].values[0], view.SlackAdminManagedSection);
  assert.equal(view.rendered.children[1].values[0], view.OAuthChannelConnectionSection);
  assert.equal(
    componentProps(view.rendered, view.OAuthChannelConnectionSection).connection,
    connection,
    "Slack OAuth uses the typed surface copy instead of hardcoded onboarding text",
  );
});

test("ChannelsTab renders Slack admin management under the installed Slack card", () => {
  const slackItem = {
    package_ref: { id: "slack" },
    runtime: "first_party",
    activation_status: "installed",
    surfaces: [
      { kind: "tool" },
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connected: false,
        connection: { channel: "slack", strategy: "admin_managed_channels" },
      },
    ],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [slackItem] });

  const installedCard = renderedNodeContainingComponent(
    view.rendered,
    view.ChannelConnectSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed Slack card wrapper");
  const cardWrapper = renderedNodeWhoseChildrenContain(view.rendered, [
    view.ExtensionCard,
    view.ChannelConnectSections,
  ]);
  assert.notEqual(
    cardWrapper,
    undefined,
    "the connect sections render inside the installed extension card wrapper",
  );
  assert.equal(
    componentPropAfter(installedCard, view.ChannelConnectSections),
    slackItem,
    "the installed extension item flows into the connect sections",
  );

  const sections = view.ChannelConnectSections({ item: slackItem });
  assert.equal(sections.children[0].values[0], view.SlackAdminManagedSection);
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "installed Slack must not fall back to the generic code-entry section",
  );
});

test("ChannelsTab renders no builtin Slack row when Slack is not installed", () => {
  const view = channelsTabForTest({
    ...TAB_PROPS,
    channelRegistry: [
      {
        package_ref: { id: "slack" },
        runtime: "first_party",
        surfaces: [{ kind: "channel", inbound: true, outbound: true }],
        installed: false,
      },
    ],
  });

  assert.equal(
    renderedComponentCount(view.rendered, view.ChannelConnectSections),
    0,
    "no installed channels means no connect sections anywhere",
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.SlackAdminManagedSection),
    0,
    "the built-in section no longer hosts a Slack setup row",
  );
  assert.equal(renderedComponentCount(view.rendered, view.PairingSection), 0);

  const registryCard = renderedNodeContainingComponent(view.rendered, view.RegistryCard);
  assert.notEqual(
    registryCard,
    undefined,
    "uninstalled Slack is offered through the available-channels registry instead",
  );
});

test("ChannelsTab renders generic connect controls under installed non-Slack channels", () => {
  const connection = {
    channel: "telegram",
    strategy: "inbound_proof_code",
    instructions: "Message the bot, then paste the code it replies with.",
    input_placeholder: "ABC123",
    submit_label: "Connect Telegram",
    error_message: "Invalid or expired pairing code.",
  };
  const telegramItem = {
    package_ref: { id: "telegram" },
    runtime: "wasm",
    activation_status: "installed",
    surfaces: [
      { kind: "channel", inbound: true, outbound: true, connected: false, connection },
    ],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [telegramItem] });

  const installedCard = renderedNodeContainingComponent(
    view.rendered,
    view.ChannelConnectSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed channel card wrapper");
  assert.equal(componentPropAfter(installedCard, view.ChannelConnectSections), telegramItem);

  const sections = view.ChannelConnectSections({ item: telegramItem });
  const section = sections.children[0];
  assert.equal(section.values[0], view.PairingSection);
  assert.equal(
    componentProps(sections, view.PairingSection).copy,
    connection,
    "the pairing card copy is the surface connection",
  );
});

test("ChannelsTab does not render duplicate fallback pairing when the channel surface owns pairing", () => {
  const surfaceOwned = {
    package_ref: { id: "telegram" },
    runtime: "wasm",
    activation_status: "installed",
    onboarding_state: "pairing_required",
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connected: false,
        connection: { channel: "telegram", strategy: "inbound_proof_code" },
      },
    ],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [surfaceOwned] });

  assert.equal(
    renderedComponentCount(view.rendered, view.ChannelConnectSections),
    1,
    "the surface connection owns the pairing UI",
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "the legacy fallback pairing section must not duplicate the surface connection",
  );
});

test("ChannelsTab never invents pairing from legacy onboarding state", () => {
  const bareItem = {
    package_ref: { id: "telegram" },
    runtime: "wasm",
    activation_status: "installed",
    onboarding_state: "pairing_required",
    surfaces: [{ kind: "channel", inbound: true, outbound: true }],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [bareItem] });

  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "pairing_required without a typed connection must not expose an unsupported flow",
  );

  const pairingView = channelsTabForTest({
    ...TAB_PROPS,
    channels: [{ ...bareItem, onboarding_state: "pairing" }],
  });
  assert.equal(renderedComponentCount(pairingView.rendered, pairingView.PairingSection), 0);

  const activeView = channelsTabForTest({
    ...TAB_PROPS,
    channels: [{ ...bareItem, onboarding_state: "active" }],
  });
  assert.equal(
    renderedComponentCount(activeView.rendered, activeView.PairingSection),
    0,
    "a connected channel renders no pairing at all",
  );
});
