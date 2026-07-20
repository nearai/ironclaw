import vm from "node:vm";
import ts from "typescript";

const originalRunInNewContext = vm.runInNewContext;

function jsxFactorySource() {
  return `
const __jsxFragment = Symbol.for("ironclaw.vm.jsx.fragment");
function __jsx(type, props, ...children) {
  if (Array.isArray(type) && Object.prototype.hasOwnProperty.call(type, "raw")) {
    return { strings: Array.from(type), values: [props, ...children] };
  }
  const flatChildren = children.flat ? children.flat(Infinity) : children;
  const isFragment = type === __jsxFragment;
  if (isFragment) {
    return { strings: [""], values: [flatChildren] };
  }
  const isTag = typeof type === "string" && !/^[A-Z]/.test(type);
  const tag = isTag ? type : "";
  const strings = [isTag ? "<" + tag : "<"];
  const values = [];
  if (!isTag) {
    values.push(type);
    strings.push("");
  }
  const addValue = (prefix, value) => {
    strings[strings.length - 1] += prefix;
    values.push(value);
    strings.push("");
  };
  const attrs = props || {};
  for (const [name, value] of Object.entries(attrs)) {
    let storedValue = value;
    if (isTag && /^on[A-Z]/.test(name) && typeof value === "function") {
      storedValue = (...args) => {
        const event = args[0];
        if (event && typeof event === "object" && !event.currentTarget && event.target) {
          event.currentTarget = event.target;
        }
        return value(...args);
      };
    }
    if (typeof storedValue === "string" || typeof storedValue === "number" || typeof storedValue === "boolean") {
      addValue(" " + name + "=\\"" + String(storedValue) + "\\" " + name + "=", storedValue);
    } else {
      addValue(" " + name + "=", storedValue);
    }
  }
  strings[strings.length - 1] += ">";
  for (const child of flatChildren) {
    if (typeof child === "string" || typeof child === "number" || typeof child === "boolean") {
      strings[strings.length - 1] += String(child);
    }
    addValue("", child);
  }
  strings[strings.length - 1] += tag ? "</" + tag + ">" : "";
  return { type, props: attrs, children: flatChildren, strings, values };
}
__jsx.Fragment = __jsxFragment;
`;
}

function transpileVmSource(code: string) {
  return (
    jsxFactorySource() +
    ts.transpileModule(code, {
      compilerOptions: {
        jsx: ts.JsxEmit.React,
        jsxFactory: "__jsx",
        jsxFragmentFactory: "__jsx.Fragment",
        module: ts.ModuleKind.None,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText
  );
}

vm.runInNewContext = function runInNewContextWithTsx(
  code: string | vm.Script,
  contextObject?: vm.Context,
  options?: vm.RunningScriptOptions | string,
) {
  if (typeof code === "string") {
    return originalRunInNewContext.call(
      this,
      transpileVmSource(code),
      contextObject,
      options,
    );
  }
  return originalRunInNewContext.call(this, code, contextObject, options);
};
