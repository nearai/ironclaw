// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// `i18n.tsx` imports React and contains JSX, so load it through the crate's
// established vm-context harness (see pages/chat/lib/chat-input.test.ts):
// read the source, strip the import, stub the module's free variables,
// and capture the otherwise-private helpers via `globalThis.__testExports`.
// Each call evaluates a FRESH module instance, so the module-level
// `packs` / `pending` caches start empty per test.
function loadI18n() {
  let source = readFileSync(new URL("./i18n.tsx", import.meta.url), "utf8");
  source = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n");
  // The real locale loaders call dynamic `import("../i18n/<lang>")`,
  // which a vm script cannot resolve. Route them through an injected
  // hook that tests override per-locale; an un-overridden call throws so
  // a missing override is loud rather than a silent hang.
  source = source.replaceAll("() => import(", "() => __dynamicImport(");
  source = source.replaceAll("export function ", "function ");
  source = source.replaceAll("export const ", "const ");
  source +=
    "\nglobalThis.__testExports = { ensurePack, registerPack, packs, loaders, I18nProvider };";

  const setItemCalls = [];
  const stateSetters = [];
  let stateIndex = 0;

  const context = {
    __dynamicImport: () => {
      throw new Error("locale loader was not overridden in this test");
    },
    Promise,
    React: {
      createContext: (value) => ({ Provider: function Provider() {}, _default: value }),
      useState: (initial) => {
        const index = stateIndex++;
        let value = typeof initial === "function" ? initial() : initial;
        return [
          value,
          (next) => {
            value = typeof next === "function" ? next(value) : next;
            stateSetters.push({ index, value });
          },
        ];
      },
      useRef: (initial) => ({ current: initial }),
      useCallback: (fn) => fn,
      useEffect: () => {},
      useMemo: (fn) => fn(),
    },
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    localStorage: {
      getItem: () => null,
      setItem: (key, value) => setItemCalls.push({ key, value }),
    },
    navigator: { language: "en" },
    document: { documentElement: {} },
    globalThis: {},
  };

  vm.runInNewContext(source, context);
  return { ...context.globalThis.__testExports, setItemCalls, stateSetters };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const LOCALES = ["ar", "de", "en", "es", "fr", "hi", "ja", "ko", "pt-BR", "uk", "zh-CN"];

function loadLocalePack(locale) {
  let registeredId = null;
  let registeredPack = null;
  let source = readFileSync(new URL(`../i18n/${locale}.ts`, import.meta.url), "utf8");
  source = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n");

  vm.runInNewContext(source, {
    registerPack: (id, pack) => {
      registeredId = id;
      registeredPack = { ...(registeredPack || {}), ...pack };
    },
  });

  assert.equal(registeredId, locale);
  assert.ok(registeredPack, `${locale} pack should register`);
  return registeredPack;
}

test("ensurePack: unknown locale resolves null (no loader, not registered)", async () => {
  const { ensurePack } = loadI18n();
  assert.equal(await ensurePack("zz-unknown"), null);
});

test("ensurePack: a known locale resolves and populates its pack", async () => {
  const { ensurePack, registerPack, loaders, packs } = loadI18n();
  loaders.es = () => {
    registerPack("es", { greet: "hola" });
    return Promise.resolve();
  };
  assert.deepEqual(await ensurePack("es"), { greet: "hola" });
  assert.deepEqual(packs.es, { greet: "hola" });
});

test("registerPack merges repeated locale registrations", () => {
  const { registerPack, packs } = loadI18n();

  registerPack("es", { greet: "hola", shared: "first" });
  registerPack("es", { bye: "adios", shared: "second" });

  assert.deepEqual(packs.es, { greet: "hola", bye: "adios", shared: "second" });
});

test("ensurePack: an already-registered pack resolves without invoking the loader", async () => {
  const { ensurePack, registerPack, loaders } = loadI18n();
  registerPack("es", { greet: "hola" });
  let calls = 0;
  loaders.es = () => {
    calls += 1;
    return Promise.resolve();
  };
  assert.deepEqual(await ensurePack("es"), { greet: "hola" });
  assert.equal(calls, 0, "a cached pack short-circuits before the loader");
});

test("ensurePack: concurrent calls fire the import exactly once", async () => {
  const { ensurePack, registerPack, loaders } = loadI18n();
  let calls = 0;
  loaders.es = () => {
    calls += 1;
    registerPack("es", { greet: "hola" });
    return Promise.resolve();
  };
  const [a, b] = await Promise.all([ensurePack("es"), ensurePack("es")]);
  assert.equal(calls, 1, "the in-flight promise is memoized in pending[lang]");
  assert.deepEqual(a, { greet: "hola" });
  assert.deepEqual(b, { greet: "hola" });
});

test("ensurePack: a failed import resolves null, never rejects, and is retryable", async () => {
  const { ensurePack, registerPack, loaders } = loadI18n();
  let calls = 0;
  loaders.es = () => {
    calls += 1;
    return Promise.reject(new Error("network"));
  };
  assert.equal(await ensurePack("es"), null, "rejection is swallowed into a null resolution");

  // pending[lang] is cleared on failure, so a later attempt retries
  // instead of replaying the cached failure.
  loaders.es = () => {
    calls += 1;
    registerPack("es", { greet: "hola" });
    return Promise.resolve();
  };
  assert.deepEqual(await ensurePack("es"), { greet: "hola" });
  assert.equal(calls, 2, "a failed import is not cached: the retry invokes the loader again");
});

test("setLang: a stale pack load resolving last does not clobber the newer language", async () => {
  const { I18nProvider, registerPack, loaders, setItemCalls } = loadI18n();

  const defer = () => {
    let resolve;
    const promise = new Promise((r) => {
      resolve = r;
    });
    return { promise, resolve };
  };
  const esLoad = defer();
  const frLoad = defer();
  loaders.es = () => {
    registerPack("es", { greet: "hola" });
    return esLoad.promise;
  };
  loaders.fr = () => {
    registerPack("fr", { greet: "bonjour" });
    return frLoad.promise;
  };

  const tree = I18nProvider({ children: null });
  const ctx = tree.values.find(
    (value) => value && typeof value === "object" && typeof value.setLang === "function",
  );
  assert.ok(ctx, "provider context exposes setLang");

  // Two rapid switches before either pack has loaded.
  ctx.setLang("es");
  ctx.setLang("fr");

  // Resolve the NEWER request (fr) first, then let the older es import
  // land last — the out-of-order case the staleness guard defends.
  frLoad.resolve();
  await tick();
  esLoad.resolve();
  await tick();

  assert.deepEqual(
    setItemCalls.map((call) => call.value),
    ["fr"],
    "only the most recently requested language is committed/persisted",
  );
});

test("locale packs include skill auto-activation controls", () => {
  const requiredKeys = [
    "skills.defaultAutoActivationEnabled",
    "skills.defaultAutoActivationDisabled",
    "skills.defaultAutoActivationOnDesc",
    "skills.defaultAutoActivationOffDesc",
    "skills.defaultAutoActivationOnButton",
    "skills.defaultAutoActivationOffButton",
    "skills.autoActivateOnTitle",
    "skills.autoActivateOffTitle",
    "skills.autoActivateOnLabel",
    "skills.autoActivateOffLabel",
  ];

  for (const locale of LOCALES) {
    const pack = loadLocalePack(locale);
    for (const key of requiredKeys) {
      assert.equal(typeof pack[key], "string", `${locale} missing ${key}`);
      assert.notEqual(pack[key].trim(), "", `${locale} ${key} should not be empty`);
    }
  }
});

test("locale packs include admin write-only secret management copy", () => {
  const requiredKeys = [
    "admin.user.secrets.title",
    "admin.user.secrets.description",
    "admin.user.secrets.loading",
    "admin.user.secrets.loadFailed",
    "admin.user.secrets.empty",
    "admin.user.secrets.handle",
    "admin.user.secrets.value",
    "admin.user.secrets.writeOnlyHint",
    "admin.user.secrets.replace",
    "admin.user.secrets.delete",
    "admin.user.secrets.save",
    "admin.user.secrets.saving",
    "admin.user.secrets.saved",
    "admin.user.secrets.deleted",
    "admin.user.secrets.actionFailed",
    "admin.user.secrets.deleteTitle",
    "admin.user.secrets.deleteDesc",
    "admin.user.secrets.deleting",
  ];

  for (const locale of LOCALES) {
    const pack = loadLocalePack(locale);
    for (const key of requiredKeys) {
      assert.equal(typeof pack[key], "string", `${locale} missing ${key}`);
      assert.notEqual(pack[key].trim(), "", `${locale} ${key} should not be empty`);
    }
  }
});

test("locale packs include workspace labels and accept a formatted size", () => {
  for (const locale of LOCALES) {
    const pack = loadLocalePack(locale);
    for (const key of [
      "workspace.area.home",
      "workspace.area.memory",
      "workspace.downloadFailed",
    ]) {
      assert.equal(typeof pack[key], "string", `${locale} missing ${key}`);
      assert.notEqual(pack[key].trim(), "", `${locale} ${key} should not be empty`);
    }
    assert.equal(
      pack["workspace.fileMeta"],
      "{mime} · {size}",
      `${locale} must not append a raw byte unit to the formatted size`,
    );
  }
});

test("zh-CN localizes Reborn settings copy and compact automation filters", () => {
  const pack = loadLocalePack("zh-CN");

  assert.equal(pack["settings.traceCommons"], "跟踪共享");
  assert.equal(pack["traceCommons.title"], "跟踪共享积分");
  assert.match(pack["traceCommons.emptyState"], /跟踪共享/);
  assert.equal(pack["skills.defaultAutoActivationOnButton"], "默认：开");
  assert.equal(pack["skills.defaultAutoActivationOffButton"], "默认：关");
  assert.equal(pack["skills.autoActivateOnLabel"], "自动激活：开");
  assert.equal(pack["skills.autoActivateOffLabel"], "自动激活：关");
  assert.equal(pack["automations.filterLabel"], "自动化状态筛选");
  assert.equal(pack["automations.filter.all"], "全部");
  assert.equal(pack["automations.filter.active"], "活跃");
  assert.equal(pack["automations.filter.paused"], "已暂停");
});

test("zh-CN localizes Reborn Projects summary copy", () => {
  const pack = loadLocalePack("zh-CN");

  assert.equal(pack["projects.summary.projects"], "项目");
  assert.equal(pack["projects.summary.attentionQueue"], "关注队列");
  assert.equal(pack["projects.summary.spendToday"], "今日花费");
  assert.equal(pack["projects.card.pendingGates"], "待处理门禁：{count}");
  assert.equal(pack["projects.files.label"], "文件");
});

test("ja localizes Trace Commons settings navigation label", () => {
  const pack = loadLocalePack("ja");

  assert.equal(pack["settings.traceCommons"], "トレース共有");
});
