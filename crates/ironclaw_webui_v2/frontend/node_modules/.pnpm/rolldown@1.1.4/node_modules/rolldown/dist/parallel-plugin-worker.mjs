import { n as __toESM, t as require_binding } from "./shared/binding-BmkJW3Wy.mjs";
import { i as PluginContextData, n as bindingifyPlugin } from "./shared/bindingify-input-options-BsDhfYrS.mjs";
import { parentPort, workerData } from "node:worker_threads";
//#region src/parallel-plugin-worker.ts
var import_binding = /* @__PURE__ */ __toESM(require_binding(), 1);
const { registryId, pluginInfos, threadNumber } = workerData;
(async () => {
	try {
		const plugins = await Promise.all(pluginInfos.map(async (pluginInfo) => {
			const definePluginImpl = (await import(pluginInfo.fileUrl)).default;
			const plugin = await definePluginImpl(pluginInfo.options, { threadNumber });
			return {
				index: pluginInfo.index,
				plugin: bindingifyPlugin(plugin, {}, {}, new PluginContextData(() => {}, {}, [], []), [], () => {}, "info", false)
			};
		}));
		(0, import_binding.registerPlugins)(registryId, plugins);
		parentPort.postMessage({ type: "success" });
	} catch (error) {
		parentPort.postMessage({
			type: "error",
			error
		});
	} finally {
		parentPort.unref();
	}
})();
//#endregion
export {};
