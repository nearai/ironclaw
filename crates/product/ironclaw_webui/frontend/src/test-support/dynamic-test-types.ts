/**
 * Test-only value crossing a runtime-generated boundary (Node VM evaluation or
 * the synthetic JSX renderer). Static TypeScript types do not survive either
 * transform, so the unsound value is named and confined to shared harnesses.
 */
export type DynamicTestValue = any;

export type VmModuleExports = Record<string, DynamicTestValue>;
export type VmComponentProps = Record<string, DynamicTestValue>;
export type DynamicTestOptions = Record<string, DynamicTestValue>;
