import { sep } from 'node:path';

/** Append a path component while leaving symlink and parent traversal to the filesystem. */
export function appendFilesystemPath(base: string, child: string): string {
  return base.endsWith(sep) ? base + child : base + sep + child;
}
