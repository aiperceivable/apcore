/**
 * $ref resolution for JSON Schema documents following Algorithm A05.
 */

import { existsSync, readFileSync, realpathSync } from 'node:fs';
import * as nodePath from 'node:path';
import { dirname, resolve } from 'node:path';
import yaml from 'js-yaml';
import {
  SchemaCircularRefError,
  SchemaMaxDepthExceededError,
  SchemaNotFoundError,
  SchemaParseError,
} from '../errors.js';
import { deepCopy } from '../utils/index.js';

const INLINE_SENTINEL = '__inline__';

/**
 * The `$ref` strings that denote the document being resolved itself. Seeding the
 * visited set with them makes a self-reference lazy from the very first
 * encounter, so a recursive schema is never inlined even once (spec §4.15.2).
 * The root `$id` is included because JSON Schema lets a document reference
 * itself by identifier — `{"$id": "TreeNode", ... "$ref": "TreeNode"}`.
 */
function rootRefAliases(document: Record<string, unknown>): string[] {
  const aliases = ['#', '#/'];
  const id = document['$id'];
  if (typeof id === 'string' && id) aliases.push(id);
  return aliases;
}

export class RefResolver {
  private _schemasDir: string;
  private _maxDepth: number;
  private _fileCache: Map<string, Record<string, unknown>> = new Map();
  /**
   * The file {@link resolve} was entered with. The D-104 node fallback is
   * scoped to THAT document — see {@link _resolveLocalPointer}.
   */
  private _originFile: string | null = null;

  constructor(schemasDir: string, maxDepth: number = 32) {
    // Realpath the root as well (Python: `Path(schemas_dir).resolve()`), so a
    // schemas directory reached through a symlink — /tmp on macOS, a checkout
    // under a symlinked home — still contains its own files once
    // `_assertWithinSchemasDir` compares realpaths.
    this._schemasDir = resolve(schemasDir);
    try {
      this._schemasDir = realpathSync.native(this._schemasDir);
    } catch {
      // Directory does not exist yet; the lexical path stays as-is and every
      // ref under it will fail on read instead.
    }
    this._maxDepth = maxDepth;
  }

  resolve(schema: Record<string, unknown>, currentFile?: string | null): Record<string, unknown> {
    const result = deepCopy(schema);
    this._fileCache.set(INLINE_SENTINEL, result);
    const visited = new Set<string>();
    // The schema passed in is the document being resolved, so `#` / `#/` /
    // its own `$id` denote IT and stay lazy from the first encounter
    // (spec §4.15.2), whether or not a `currentFile` is known. D-104 changes
    // only where a POINTERED local reference looks: the file root first, then
    // this node. A bare `#` addresses no sub-document and is a
    // self-reference either way.
    for (const alias of rootRefAliases(result)) visited.add(alias);
    const previousOrigin = this._originFile;
    this._originFile = currentFile ?? null;
    try {
      this._resolveNode(result, currentFile ?? null, visited, 0);
    } finally {
      this._fileCache.delete(INLINE_SENTINEL);
      this._originFile = previousOrigin;
    }
    return result;
  }

  /**
   * Resolve one `$ref`.
   *
   * `fromRefChain` marks the caller as another `$ref` that had this one as its
   * immediate target — a `$ref` → `$ref` hop that never reaches a schema body.
   * Re-entering a reference along such a chain is a genuine cycle (spec §4.15.2
   * "circular reference"): resolution cannot terminate and there is nothing to
   * defer to, so `SCHEMA_CIRCULAR_REF` is raised. Re-entering a reference after
   * descending through `properties` / `items` / a combinator is instead a
   * *self-reference* — a recursive data structure — and the `$ref` is preserved
   * verbatim as a lazy reference for the converter to bind.
   */
  resolveRef(
    refString: string,
    currentFile: string | null,
    visitedRefs?: Set<string>,
    depth: number = 0,
    siblingKeys?: Record<string, unknown> | null,
    fromRefChain: boolean = false,
  ): unknown {
    const visited = visitedRefs ?? new Set<string>();

    if (visited.has(refString)) {
      if (fromRefChain) {
        throw new SchemaCircularRefError(refString);
      }
      return { $ref: refString, ...(siblingKeys ?? {}) };
    }

    if (depth >= this._maxDepth) {
      throw new SchemaMaxDepthExceededError(refString);
    }

    visited.add(refString);

    const [filePath, jsonPointer] = this._parseRef(refString, currentFile);
    const document = this._loadFile(filePath);
    const target = refString.startsWith('#') && filePath !== INLINE_SENTINEL
      ? this._resolveLocalPointer(document, jsonPointer, refString, filePath)
      : this._resolveJsonPointer(document, jsonPointer, refString);

    let result: unknown = deepCopy(target);

    if (siblingKeys && typeof result === 'object' && result !== null && !Array.isArray(result)) {
      Object.assign(result as Record<string, unknown>, siblingKeys);
    }

    const effectiveFile = filePath === INLINE_SENTINEL ? currentFile : filePath;

    if (typeof result === 'object' && result !== null && !Array.isArray(result)) {
      const resultObj = result as Record<string, unknown>;
      if ('$ref' in resultObj) {
        const nestedRef = resultObj['$ref'] as string;
        delete resultObj['$ref'];
        const nestedSiblings = Object.keys(resultObj).length > 0 ? { ...resultObj } : null;
        result = this.resolveRef(nestedRef, effectiveFile, visited, depth + 1, nestedSiblings, true);
      }
    }

    this._resolveNode(result, effectiveFile, visited, depth + 1);
    return result;
  }

  private _resolveNode(
    node: unknown,
    currentFile: string | null,
    visitedRefs: Set<string>,
    depth: number,
  ): unknown {
    if (typeof node === 'object' && node !== null && !Array.isArray(node)) {
      const nodeObj = node as Record<string, unknown>;
      if ('$ref' in nodeObj) {
        const refString = nodeObj['$ref'] as string;
        const siblingKeys: Record<string, unknown> = {};
        for (const [k, v] of Object.entries(nodeObj)) {
          if (k !== '$ref') siblingKeys[k] = v;
        }
        const resolved = this.resolveRef(
          refString,
          currentFile,
          new Set(visitedRefs),
          depth,
          Object.keys(siblingKeys).length > 0 ? siblingKeys : null,
        );
        // Clear and replace
        for (const key of Object.keys(nodeObj)) delete nodeObj[key];
        if (typeof resolved === 'object' && resolved !== null && !Array.isArray(resolved)) {
          Object.assign(nodeObj, resolved as Record<string, unknown>);
        } else {
          return resolved;
        }
      } else {
        for (const key of Object.keys(nodeObj)) {
          const result = this._resolveNode(nodeObj[key], currentFile, visitedRefs, depth);
          if (result !== nodeObj[key]) {
            nodeObj[key] = result;
          }
        }
      }
    } else if (Array.isArray(node)) {
      for (let i = 0; i < node.length; i++) {
        const result = this._resolveNode(node[i], currentFile, visitedRefs, depth);
        if (result !== node[i]) {
          node[i] = result;
        }
      }
    }
    return node;
  }

  private _parseRef(refString: string, currentFile: string | null): [string, string] {
    if (refString.startsWith('#')) {
      const pointer = refString.slice(1);
      if (currentFile) return [currentFile, pointer];
      return [INLINE_SENTINEL, pointer];
    }

    if (refString.startsWith('apcore://')) {
      return this._convertCanonicalToPath(refString);
    }

    if (refString.includes('#')) {
      // maxsplit-1 semantics, NOT JS `split('#', 2)`: the JS limit argument
      // DROPS everything past the second field, so a malformed ref carrying two
      // '#' would silently resolve as if the tail were not there. Python's
      // `split("#", 1)` keeps the tail in the pointer, which then fails to
      // resolve — the correct outcome.
      const hashIndex = refString.indexOf('#');
      const filePart = refString.slice(0, hashIndex);
      const pointer = refString.slice(hashIndex + 1);
      const base = currentFile ? dirname(currentFile) : this._schemasDir;
      const resolvedPath = resolve(base, filePart);
      this._assertWithinSchemasDir(resolvedPath, refString);
      return [resolvedPath, pointer];
    }

    const base = currentFile ? dirname(currentFile) : this._schemasDir;
    const resolvedPath = resolve(base, refString);
    this._assertWithinSchemasDir(resolvedPath, refString);
    return [resolvedPath, ''];
  }

  /**
   * Resolve *path* through the filesystem (symlinks included) as far as it
   * exists, mirroring Python's non-strict `Path.resolve()`.
   *
   * `node:path.resolve` is purely lexical — it never touches the filesystem —
   * so a symlink inside the schemas directory that points outside it passes a
   * lexical containment check and is then read. Missing tail segments are kept
   * verbatim (a `$ref` to a file that does not exist must still be rejected by
   * containment, not by an ENOENT out of the check itself).
   */
  private _realpathLenient(path: string): string {
    let current = path;
    const tail: string[] = [];
    for (;;) {
      try {
        return tail.length === 0
          ? realpathSync.native(current)
          : nodePath.join(realpathSync.native(current), ...tail);
      } catch {
        const parent = dirname(current);
        // Reached the filesystem root without finding an existing ancestor.
        if (parent === current) return path;
        tail.unshift(nodePath.basename(current));
        current = parent;
      }
    }
  }

  private _assertWithinSchemasDir(rawResolvedPath: string, refString: string): void {
    // Compare realpaths on both sides: `_schemasDir` is realpath'd in the
    // constructor, so a symlinked schemas root still matches its own files.
    const resolvedPath = this._realpathLenient(rawResolvedPath);
    if (resolvedPath === this._schemasDir) return;
    const rel = nodePath.relative(this._schemasDir, resolvedPath);
    // A path is inside schemasDir iff its relative form is non-empty, not
    // absolute, and does not start with a parent-directory traversal segment.
    // Using path.relative() makes this check cross-platform — the previous
    // startsWith(schemasDir + '/') check was hard-coded to POSIX separators
    // and silently disabled on Windows where resolve() emits backslash paths.
    if (!rel || nodePath.isAbsolute(rel) || rel === '..' || rel.startsWith('..' + nodePath.sep)) {
      throw new SchemaNotFoundError(
        `Reference '${refString}' resolves outside schemas directory`,
      );
    }
  }

  private _convertCanonicalToPath(uri: string): [string, string] {
    const remainder = uri.slice('apcore://'.length);
    const parts = remainder.split('/');
    const canonicalId = parts[0];
    const pointerParts = parts.slice(1);

    const fileRel = canonicalId.replace(/\./g, '/') + '.schema.yaml';
    const filePath = resolve(this._schemasDir, fileRel);

    const pointer = pointerParts.length > 0 ? '/' + pointerParts.join('/') : '';
    return [filePath, pointer];
  }

  /**
   * Resolve a local (`#`-prefixed) pointer per Algorithm A05 step 4a, as
   * settled by spec v1.50.0 D-104: **the file root first, then the schema
   * node being resolved.**
   *
   * Both layouts are normative and an implementation MUST support both —
   * `definitions:` as a top-level sibling of `input_schema` in the FILE
   * (§4.11's own example, previously loadable on apcore-rust alone), and
   * `$defs:` nested inside the `input_schema` node (what apcore-python and
   * apcore-typescript accepted, and therefore presumably what is in the
   * wild). The two lookups cannot collide: a pointer either resolves at the
   * file root or it does not.
   *
   * The fallback is scoped to the document `resolve()` was entered with.
   * D-104 settled WHICH two bases a local pointer tries; it did not say how
   * far the second one travels, and the answer was "everywhere": a
   * `#/$defs/X` written inside an EXTERNAL schema, for a definition that
   * document does not have, fell back to the calling module's schema node and
   * bound to whatever happened to share the name. Three consequences, in
   * increasing order of cost: an invalid reference reported success where it
   * owes `SCHEMA_NOT_FOUND`; the resolved schema then validated against a
   * contract the external author never wrote; and §10.6 reads `x-sensitive`
   * off the RESOLVED schema, so a field the external document marks sensitive
   * could be replaced by a local definition that does not and be logged in
   * plaintext. A document only ever falls back to its own node.
   */
  private _resolveLocalPointer(
    document: unknown,
    pointer: string,
    refString: string,
    filePath: string,
  ): unknown {
    try {
      return this._resolveJsonPointer(document, pointer, refString);
    } catch (err) {
      if (!(err instanceof SchemaNotFoundError)) throw err;
      if (filePath !== this._originFile) throw err;
      const inline = this._fileCache.get(INLINE_SENTINEL);
      if (inline === undefined) throw err;
      return this._resolveJsonPointer(inline, pointer, refString);
    }
  }

  private _resolveJsonPointer(document: unknown, pointer: string, refString: string): unknown {
    if (!pointer) return document;

    let segments = pointer.split('/');
    if (segments.length > 0 && segments[0] === '') {
      segments = segments.slice(1);
    }

    let current = document;
    for (const rawSegment of segments) {
      const segment = rawSegment.replace(/~1/g, '/').replace(/~0/g, '~');
      if (typeof current === 'object' && current !== null && !Array.isArray(current)) {
        const obj = current as Record<string, unknown>;
        if (segment in obj) {
          current = obj[segment];
        } else {
          throw new SchemaNotFoundError(`${refString} (segment '${segment}' not found)`);
        }
      } else {
        throw new SchemaNotFoundError(`${refString} (segment '${segment}' not found)`);
      }
    }
    return current;
  }

  private _loadFile(filePath: string): Record<string, unknown> {
    if (filePath === INLINE_SENTINEL) {
      return this._fileCache.get(INLINE_SENTINEL) ?? {};
    }

    const resolved = resolve(filePath);
    const cached = this._fileCache.get(resolved);
    if (cached !== undefined) return cached;

    if (!existsSync(resolved)) {
      throw new SchemaNotFoundError(resolved);
    }

    const content = readFileSync(resolved, 'utf-8');
    if (!content.trim()) {
      this._fileCache.set(resolved, {});
      return {};
    }

    let parsed: unknown;
    try {
      parsed = yaml.load(content);
    } catch (e) {
      throw new SchemaParseError(`Invalid YAML in ${resolved}: ${e}`);
    }

    if (parsed === null || parsed === undefined) {
      this._fileCache.set(resolved, {});
      return {};
    }

    if (typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new SchemaParseError(
        `Schema file ${resolved} must be a YAML mapping, got ${typeof parsed}`,
      );
    }

    const result = parsed as Record<string, unknown>;
    this._fileCache.set(resolved, result);
    return result;
  }

  clearCache(): void {
    this._fileCache.clear();
  }
}
