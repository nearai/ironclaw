import { m as BindingReplacePluginConfig } from "./shared/binding-B1q9aYT-.mjs";
import { P as BuiltinPlugin } from "./shared/define-config-BBz954-q.mjs";
import { t as esmExternalRequirePlugin } from "./shared/constructors-2i1T9YcF.mjs";

//#region src/builtin-plugin/replace-plugin.d.ts
/**
* Replaces targeted strings in files while bundling.
*
* @example
* **Basic usage**
* ```js
* replacePlugin({
*   'process.env.NODE_ENV': JSON.stringify('production'),
*    __buildVersion: 15
* })
* ```
* @example
* **With options**
* ```js
* replacePlugin({
*   'process.env.NODE_ENV': JSON.stringify('production'),
*   __buildVersion: 15
* }, {
*   preventAssignment: false,
* })
* ```
*
* @see https://rolldown.rs/builtin-plugins/replace
* @category Builtin Plugins
*/
declare function replacePlugin(values?: BindingReplacePluginConfig["values"], options?: Omit<BindingReplacePluginConfig, "values">): BuiltinPlugin;
//#endregion
export { esmExternalRequirePlugin, replacePlugin };