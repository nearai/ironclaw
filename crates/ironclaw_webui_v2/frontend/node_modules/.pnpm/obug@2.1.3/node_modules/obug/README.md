# obug

[![npm version][npm-version-src]][npm-version-href]
[![npm downloads][npm-downloads-src]][npm-downloads-href]
[![JSR][jsr-badge-src]][jsr-badge-href]
[![Unit Test][unit-test-src]][unit-test-href]

A lightweight JavaScript debugging utility, forked from [debug](https://www.npmjs.com/package/debug), featuring TypeScript and ESM support.

> [!NOTE]
> obug v1 retains most of the compatibility with [debug](https://github.com/debug-js/debug), but drops support for older browsers and Node.js, making it a drop-in replacement.
>
> obug v2 refactors some API imports and usage for better support of ESM and TypeScript, easier customization, and an even smaller package size.

## Key Differences from `debug`

- ✨ Minimal footprint
  - 7.7 kB package size
  - 1.4 KB minified + gzipped for browsers
- 📦 Zero dependencies
- 📝 Full TypeScript support
- 🚀 Native ESM compatibility
- 🌐 Optimized for modern runtimes
  - ES2015+ browsers
  - Modern Node.js versions
- 🎨 Customizable formatting

## Installation

```bash
npm install obug
```

## Usage

```ts
import { createDebug, disable, enable, enabled, namespaces } from 'obug'

// Get the currently enabled namespaces
console.log(namespaces())

const debug = createDebug('my-namespace', {
  // All options are optional

  useColors: true, // false, true, undefined for auto-detect
  color: 2, // custom color
  // custom formatArgs
  formatArgs(args) {},
  formatters: {},
  // Node.js only
  inspectOpts: {},

  // custom log
  log: console.log,
})

debug('This is a debug message')
console.log(
  debug.namespace, // 'my-namespace'
  debug.enabled, // Check if enabled
  debug.useColors, // true
  debug.color, // 2
  debug.formatArgs, // custom formatArgs
  debug.formatters, // {}
  debug.inspectOpts, // {}
  debug.log, // implemented log function
)

// Create a sub-namespace, and it will inherit options from the parent debugger
const sub = debug.extend('sub-namespace')
sub('This is a sub-namespace debug message')
console.log(sub.namespace) // 'my-namespace:sub-namespace'
```

## How to use

### Enabling debug output

obug picks up enabled namespaces from the `DEBUG` environment variable in Node.js, or from `localStorage.debug` in browsers.

In Node.js:

```bash
DEBUG=app:* node app.js
```

On Windows (CMD):

```cmd
set DEBUG=app:* & node app.js
```

On Windows (PowerShell):

```powershell
$env:DEBUG='app:*'; node app.js
```

In the browser, set the value in DevTools and refresh the page:

```js
localStorage.debug = 'app:*'
```

### Wildcards and exclusion

The `*` character is a wildcard. Suppose your library has debuggers named `connect:bodyParser`, `connect:compress`, and `connect:session` — instead of listing each one, use `DEBUG=connect:*`. Use `DEBUG=*` to enable everything.

Exclude namespaces by prefixing them with `-`:

```bash
DEBUG=*,-connect:* node app.js
```

### Namespace conventions

If you're using obug in a library, prefix your namespaces with the library name and use `:` to separate features (e.g. `connect:bodyParser`). This lets users opt into the parts they care about without guessing names.

### Extending a namespace

Use `.extend()` to create a sub-namespace that inherits options from its parent:

```ts
const log = createDebug('auth')
const logSign = log.extend('sign') // 'auth:sign'
const logLogin = log.extend('login') // 'auth:login'

log('hello') // auth hello
logSign('hello') // auth:sign hello
logLogin('hello') // auth:login hello
```

### Formatters

obug uses [printf-style](https://wikipedia.org/wiki/Printf_format_string) formatting. Built-in formatters:

| Formatter | Representation                                     |
| --------- | -------------------------------------------------- |
| `%O`      | Pretty-print an Object on multiple lines.          |
| `%o`      | Pretty-print an Object all on a single line.       |
| `%%`      | Single percent sign. Does not consume an argument. |

Other format specifiers supported by Node's `util.format` (`%s`, `%d`, `%j`, …) and the browser console pass through to the underlying logger.

#### Custom formatters

Register custom formatters via the `formatters` option. For example, to render a `Buffer` as hex with `%h`:

```ts
const debug = createDebug('foo', {
  formatters: {
    h: (v) => v.toString('hex'),
  },
})

debug('this is hex: %h', Buffer.from('hello world'))
//   foo this is hex: 68656c6c6f20776f726c64 +0ms
```

### Dynamic enable / disable

You can toggle namespaces at runtime:

```ts
import { disable, enable, enabled, namespaces } from 'obug'

enable('app:*')
console.log(enabled('app:server')) // true

const previous = disable()
console.log(enabled('app:server')) // false

// Restore later
enable(previous)
```

`namespaces()` returns the string of currently enabled namespaces. Note that calling `enable()` overrides the value initially read from `DEBUG`.

### Checking whether a debugger is enabled

Guard expensive work behind the `enabled` property:

```ts
const debug = createDebug('http')

if (debug.enabled) {
  // expensive computation only when enabled
}
```

You can also force the state by assigning to `debug.enabled` directly.

### Custom output

By default, obug writes to `stderr` in Node.js and to the console in browsers. Override the `log` option to redirect output per-namespace:

```ts
const log = createDebug('app:log', {
  log: console.log,
})

const error = createDebug('app:error') // still goes to stderr
```

### Environment variables (Node.js)

| Name                | Purpose                                         |
| ------------------- | ----------------------------------------------- |
| `DEBUG`             | Enables/disables specific debugging namespaces. |
| `DEBUG_HIDE_DATE`   | Hide date from debug output (non-TTY).          |
| `DEBUG_COLORS`      | Whether to use colors in the debug output.      |
| `DEBUG_DEPTH`       | Object inspection depth.                        |
| `DEBUG_SHOW_HIDDEN` | Shows hidden properties on inspected objects.   |

Variables prefixed with `DEBUG_` are converted to camelCase keys on the options object passed to Node's [`util.inspect()`](https://nodejs.org/api/util.html#util_util_inspect_object_options) for the `%o` / `%O` formatters.

## Original Authors

As obug is a fork of debug with significant modifications, we would like to acknowledge the original authors:

- TJ Holowaychuk
- Nathan Rajlich
- Andrew Rhyne
- Josh Junon

## Sponsors

<p align="center">
  <a href="https://cdn.jsdelivr.net/gh/sxzz/sponsors/sponsors.svg">
    <img src='https://cdn.jsdelivr.net/gh/sxzz/sponsors/sponsors.svg'/>
  </a>
</p>

## License

[MIT](./LICENSE) License © 2025-PRESENT [Kevin Deng](https://github.com/sxzz)

[The MIT License](./LICENSE) Copyright (c) 2014-2017 TJ Holowaychuk &lt;tj@vision-media.ca&gt;

[The MIT License](./LICENSE) Copyright (c) 2018-2021 Josh Junon

<!-- Badges -->

[npm-version-src]: https://img.shields.io/npm/v/obug.svg
[npm-version-href]: https://npmjs.com/package/obug
[npm-downloads-src]: https://img.shields.io/npm/dm/obug
[npm-downloads-href]: https://www.npmcharts.com/compare/obug?interval=30
[unit-test-src]: https://github.com/sxzz/obug/actions/workflows/unit-test.yml/badge.svg
[unit-test-href]: https://github.com/sxzz/obug/actions/workflows/unit-test.yml
[jsr-badge-src]: https://jsr.io/badges/@sxzz/obug
[jsr-badge-href]: https://jsr.io/@sxzz/obug
