/*
	MIT License http://www.opensource.org/licenses/mit-license.php
	Author Natsu @xiaoxiaojx
*/

"use strict";

const stripJsonComments = require("./strip-json-comments");

/** @typedef {import("../Resolver").FileSystem} FileSystem */
/** @typedef {import("../Resolver").JsonObject} JsonObject */

/**
 * @typedef {object} ReadJsonOptions
 * @property {boolean=} stripComments Whether to strip JSONC comments
 */

/** @type {WeakMap<Buffer, JsonObject>} */
const _stripCommentsCache = new WeakMap();

/**
 * Read and parse JSON file (supports JSONC with comments).
 * Callback-based so a synchronous `fileSystem` stays synchronous all the
 * way through — Promise wrapping would defer resolution by a Promise tick
 * and break `resolveSync` when `tsconfig` is used together with
 * `useSyncFileSystemCalls: true`.
 * @param {FileSystem} fileSystem the file system
 * @param {string} jsonFilePath absolute path to JSON file
 * @param {ReadJsonOptions} options Options
 * @param {(err: NodeJS.ErrnoException | Error | null, content?: JsonObject) => void} callback callback
 * @returns {void}
 */
function readJson(fileSystem, jsonFilePath, options, callback) {
	const { stripComments = false } = options;
	const { readJson: fsReadJson } = fileSystem;
	if (fsReadJson && !stripComments) {
		fsReadJson(jsonFilePath, (err, content) => {
			if (err) return callback(err);
			callback(null, /** @type {JsonObject} */ (content));
		});
		return;
	}

	fileSystem.readFile(jsonFilePath, (err, data) => {
		if (err) return callback(err);
		const buf = /** @type {Buffer} */ (data);

		if (stripComments) {
			const cached = _stripCommentsCache.get(buf);
			if (cached !== undefined) return callback(null, cached);
		}

		let result;
		try {
			const jsonText = buf.toString();
			const jsonWithoutComments = stripComments
				? stripJsonComments(jsonText, {
						trailingCommas: true,
						whitespace: true,
					})
				: jsonText;
			result = JSON.parse(jsonWithoutComments);
		} catch (parseErr) {
			return callback(/** @type {Error} */ (parseErr));
		}

		if (stripComments) {
			_stripCommentsCache.set(buf, result);
		}

		callback(null, result);
	});
}

module.exports.readJson = readJson;
