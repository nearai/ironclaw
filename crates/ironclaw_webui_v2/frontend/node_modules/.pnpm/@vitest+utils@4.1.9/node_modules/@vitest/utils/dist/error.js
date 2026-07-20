import { format } from '@vitest/pretty-format';
import { printDiffOrStringify, getDefaultFormatOptions } from './diff.js';
import { serializeValue } from './serialize.js';
import 'tinyrainbow';
import './display.js';
import './helpers.js';
import './constants.js';

function processError(_err, diffOptions, seen = new WeakSet()) {
	if (!_err || typeof _err !== "object") {
		return { message: String(_err) };
	}
	const err = _err;
	if (err.showDiff || err.showDiff === undefined && err.expected !== undefined && err.actual !== undefined) {
		const options = {
			...diffOptions,
			...err.diffOptions
		};
		err.diff = printDiffOrStringify(err.actual, err.expected, options, err);
		err.expected = prettifyValue(err.expected, options);
		err.actual = prettifyValue(err.actual, options);
	}
	// some Error implementations may not allow rewriting cause
	// in most cases, the assignment will lead to "err.cause = err.cause"
	try {
		if (!seen.has(err) && typeof err.cause === "object") {
			seen.add(err);
			err.cause = processError(err.cause, diffOptions, seen);
		}
	} catch {}
	try {
		return serializeValue(err);
	} catch (e) {
		return serializeValue(new Error(`Failed to fully serialize error: ${e?.message}\nInner error message: ${err?.message}`));
	}
}
function prettifyValue(value, options) {
	if (typeof value !== "string") {
		return format(value, getDefaultFormatOptions(options));
	}
	return value;
}

export { processError, serializeValue as serializeError };
