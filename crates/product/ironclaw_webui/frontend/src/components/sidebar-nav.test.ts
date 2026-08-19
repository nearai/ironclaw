// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, useLocation, useNavigate } from "react-router";
import { test } from "vitest";
import "../i18n/en";
import { I18nProvider } from "../lib/i18n";

import { visibleSidebarSubRoutes } from "./sidebar-nav";
import { SidebarNav } from "./sidebar-nav";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const expandableRoutes = [
  { id: "extensions", firstTab: "registry", isAdmin: false },
  { id: "settings", firstTab: "inference", isAdmin: false },
  { id: "admin", firstTab: "users", isAdmin: true },
];

function CurrentPath({ navigateBackPath }) {
  const navigate = useNavigate();
  return React.createElement(
    React.Fragment,
    null,
    React.createElement(
      "span",
      { "data-testid": "current-path" },
      useLocation().pathname,
    ),
    React.createElement(
      "button",
      {
        "data-testid": "navigate-away",
        onClick: () => navigate("/chat"),
      },
      "navigate away",
    ),
    React.createElement(
      "button",
      {
        "data-testid": "navigate-back",
        onClick: () => navigate(navigateBackPath),
      },
      "navigate back",
    ),
  );
}

function renderSidebar({ initialPath, isAdmin = false, onNavigate = () => {} }) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  act(() => {
    root.render(
      React.createElement(
        MemoryRouter,
        { initialEntries: [initialPath] },
        React.createElement(
          I18nProvider,
          null,
          React.createElement(
            React.Fragment,
            null,
            React.createElement(SidebarNav, {
              isAdmin,
              isCreating: false,
              onNavigate,
              onNewChat: () => {},
            }),
            React.createElement(CurrentPath, {
              navigateBackPath: initialPath,
            }),
          ),
        ),
      ),
    );
  });

  return { container, root };
}

test("member sidebar exposes inference but keeps users admin-only", () => {
  const routes = visibleSidebarSubRoutes("settings", false);
  const ids = routes.map((route) => route.id);

  assert.ok(ids.includes("inference"));
  assert.ok(!ids.includes("users"));
});

test.each(expandableRoutes)(
  "$id sidebar section can be collapsed and expanded without navigating away",
  ({ id, firstTab, isAdmin }) => {
    const path = `/${id}/${firstTab}`;
    let navigations = 0;
    const { container, root } = renderSidebar({
      initialPath: path,
      isAdmin,
      onNavigate: () => {
        navigations += 1;
      },
    });

    try {
      const parent = container.querySelector<HTMLAnchorElement>(
        `a[href="${path}"]`,
      );
      assert.ok(parent, `${id} parent link should render`);
      const childPanel = () => {
        const controlsId = parent.getAttribute("aria-controls");
        return controlsId ? container.querySelector(`#${controlsId}`) : null;
      };
      const childLinks = () => {
        return childPanel()?.querySelectorAll("a").length ?? 0;
      };

      assert.equal(childLinks(), visibleSidebarSubRoutes(id, isAdmin).length);
      assert.equal(parent.getAttribute("aria-expanded"), "true");
      assert.ok(parent.getAttribute("aria-controls"));
      assert.ok(childPanel());
      assert.equal(container.querySelector("[data-testid=current-path]")?.textContent, path);

      act(() => parent.click());
      assert.equal(childLinks(), 0, `${id} children should collapse`);
      assert.equal(parent.getAttribute("aria-expanded"), "false");
      assert.equal(parent.getAttribute("aria-controls"), null);
      assert.equal(childPanel(), null);
      assert.equal(container.querySelector("[data-testid=current-path]")?.textContent, path);
      assert.equal(navigations, 0);

      act(() => parent.click());
      assert.equal(
        childLinks(),
        visibleSidebarSubRoutes(id, isAdmin).length,
        `${id} children should expand again`,
      );
      assert.equal(parent.getAttribute("aria-expanded"), "true");
      assert.equal(
        parent.getAttribute("aria-controls"),
        `sidebar-${id}-subroutes`,
      );
      assert.equal(navigations, 0);

      const navigateAway = container.querySelector<HTMLButtonElement>(
        '[data-testid="navigate-away"]',
      );
      const navigateBack = container.querySelector<HTMLButtonElement>(
        '[data-testid="navigate-back"]',
      );
      assert.ok(navigateAway);
      assert.ok(navigateBack);
      act(() => navigateAway.click());
      assert.equal(childLinks(), 0);
      assert.equal(parent.getAttribute("aria-expanded"), "false");
      assert.equal(
        container.querySelector("[data-testid=current-path]")?.textContent,
        "/chat",
      );
      act(() => navigateBack.click());
      assert.equal(childLinks(), visibleSidebarSubRoutes(id, isAdmin).length);
      assert.equal(parent.getAttribute("aria-expanded"), "true");
      assert.equal(
        container.querySelector("[data-testid=current-path]")?.textContent,
        path,
      );

      assert.equal(navigations, 0);

      act(() => parent.click());
      assert.equal(parent.getAttribute("aria-expanded"), "false");
      assert.equal(childLinks(), 0);

      const metaClick = new MouseEvent("click", {
        bubbles: true,
        metaKey: true,
      });
      act(() => parent.dispatchEvent(metaClick));
      assert.equal(metaClick.defaultPrevented, false);
      assert.equal(parent.getAttribute("aria-expanded"), "false");
      assert.equal(childLinks(), 0);

      const ctrlClick = new MouseEvent("click", {
        bubbles: true,
        ctrlKey: true,
      });
      act(() => parent.dispatchEvent(ctrlClick));
      assert.equal(ctrlClick.defaultPrevented, false);
      assert.equal(parent.getAttribute("aria-expanded"), "false");
      assert.equal(childLinks(), 0);

      act(() => parent.click());
      assert.equal(childLinks(), visibleSidebarSubRoutes(id, isAdmin).length);
      assert.equal(
        container.querySelector("[data-testid=current-path]")?.textContent,
        path,
      );
      assert.equal(navigations, 2);
    } finally {
      act(() => root.unmount());
      container.remove();
    }
  },
);

test.each([
  { initialPath: "/chat", description: "inactive" },
  { initialPath: "/extensions", description: "exact parent route" },
  { initialPath: "/extensions/", description: "trailing-slash parent route" },
])("$description expandable parent still navigates to its first child", ({
  initialPath,
}) => {
  let navigations = 0;
  const { container, root } = renderSidebar({
    initialPath,
    onNavigate: () => {
      navigations += 1;
    },
  });

  try {
    const parent = container.querySelector<HTMLAnchorElement>(
      'nav > div > a[href="/extensions/registry"]',
    );
    assert.ok(parent);
    act(() => parent.click());

    assert.equal(navigations, 1);
    assert.equal(
      container.querySelector("[data-testid=current-path]")?.textContent,
      "/extensions/registry",
    );
    assert.equal(parent.getAttribute("aria-expanded"), "true");
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
