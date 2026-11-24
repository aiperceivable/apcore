import { isAbsolute, relative, sep } from 'node:path';

/** Append a path component while leaving symlink and parent traversal to the filesystem. */
export function appendFilesystemPath(base: string, child: string): string {
  return base.endsWith(sep) ? base + child : base + sep + child;
}

/** Compare physical paths supplied by callers after realpath resolution. */
export function isWithinFilesystemPath(base: string, path: string): boolean {
  const child = relative(base, path);
  return !isAbsolute(child) && child !== '..' && !child.startsWith('..' + sep);
}
