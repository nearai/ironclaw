// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import {
  channelConnection,
  isInboundProofCodeConnection,
} from "../lib/extensions-schema";

function channelsTabSourceForTest() {
  const source = readFileSync(new URL("./channels-tab.tsx", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChannelsTab, ChannelConnectSections };`;
}

function channelConnectSectionsForTest(item) {
  const context = {
    globalThis: {},
    PairingSection() {},
    // The real surface-taxonomy helpers: connect sections must derive from the
    // wire connection strategy exactly as production does.
    channelConnection,
    isInboundProofCodeConnection,
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    redeemPairingCode() {},
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.ChannelConnectSections({ item }),
    PairingSection: context.PairingSection,
    redeemPairingCode: context.redeemPairingCode,
  };
}

function channelsTabForTest(props) {
  const context = {
    ExtensionCard() {},
    PairingSection() {},
    RegistryCard() {},
    StatusPill() {},
    globalThis: {},
    channelConnection,
    isInboundProofCodeConnection,
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

test("ChannelConnectSections renders the same generic sections for every package", () => {
  // A proof-code connection renders the generic pairing section regardless of
  // the package id — there is no per-extension branch.
  const slackView = channelConnectSectionsForTest({
    package_ref: { id: "slack" },
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connection: { channel: "slack", strategy: "inbound_proof_code" },
      },
    ],
  });
  const sections = slackView.rendered.children;
  assert.equal(sections.length, 1);
  assert.equal(
    sections[0].values[0],
    slackView.PairingSection,
    "a proof-code connection renders the generic pairing section for any package",
  );

  // Admin-managed connections have no user-facing connect section: allowed
  // channels are installation config, not a per-user affordance.
  const adminView = channelConnectSectionsForTest({
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
    adminView.rendered,
    null,
    "admin-managed connections render no per-user connect section",
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
      { kind: "channel", inbound: true, outbound: true, connection },
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

  // OAuth connections (and channels without a connect affordance) render
  // nothing here — OAuth connect lives in the configure modal.
  assert.equal(
    channelConnectSectionsForTest({
      package_ref: { id: "telegram" },
      surfaces: [{ kind: "channel", connection: { channel: "telegram", strategy: "oauth" } }],
    }).rendered,
    null,
  );
  assert.equal(
    channelConnectSectionsForTest({ package_ref: { id: "telegram" }, surfaces: [] }).rendered,
    null,
  );
});

test("ChannelsTab renders an installed OAuth-connect channel without pairing or admin sections", () => {
  const slackItem = {
    package_ref: { id: "slack" },
    runtime: "first_party",
    installation_state: "installed",
    surfaces: [
      { kind: "tool" },
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connection: { channel: "slack", strategy: "oauth" },
      },
    ],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [slackItem] });

  const installedCard = renderedNodeContainingComponent(
    view.rendered,
    view.ChannelConnectSections,
  );
  assert.notEqual(installedCard, undefined, "expected installed channel card wrapper");
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

  assert.equal(
    view.ChannelConnectSections({ item: slackItem }),
    null,
    "an OAuth connection renders no connect section: connect lives in the configure modal",
  );
  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    0,
    "an OAuth-connect channel must not fall back to the generic code-entry section",
  );
});

test("ChannelsTab renders no builtin channel row when the channel is not installed", () => {
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
  assert.equal(renderedComponentCount(view.rendered, view.PairingSection), 0);

  const registryCard = renderedNodeContainingComponent(view.rendered, view.RegistryCard);
  assert.notEqual(
    registryCard,
    undefined,
    "an uninstalled channel is offered through the available-channels registry instead",
  );
});

test("ChannelsTab renders generic connect controls under installed channels", () => {
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
    installation_state: "installed",
    surfaces: [
      { kind: "channel", inbound: true, outbound: true, connection },
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
    installation_state: "installed",
    onboarding_state: "pairing_required",
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
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

test("ChannelsTab falls back to pairing only when the surface connection did not handle it", () => {
  const bareItem = {
    package_ref: { id: "telegram" },
    runtime: "wasm",
    installation_state: "installed",
    onboarding_state: "pairing_required",
    surfaces: [{ kind: "channel", inbound: true, outbound: true }],
  };
  const view = channelsTabForTest({ ...TAB_PROPS, channels: [bareItem] });

  assert.equal(
    renderedComponentCount(view.rendered, view.PairingSection),
    1,
    "pairing_required without a surface connection still gets the fallback pairing card",
  );
  const fallback = renderedNodeContainingComponent(view.rendered, view.PairingSection);
  assert.equal(componentPropAfter(fallback, view.PairingSection), "telegram");
  assert.equal(fallback.values[2], view.redeemPairingCode);

  const pairingView = channelsTabForTest({
    ...TAB_PROPS,
    channels: [{ ...bareItem, onboarding_state: "pairing" }],
  });
  assert.equal(renderedComponentCount(pairingView.rendered, pairingView.PairingSection), 1);

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
