// pattern: Functional Core

export type FileAssetUrlConverter = (path: string) => string;

/** Converts filesystem-backed character backgrounds at the presentation boundary. */
export function mapBackgroundAssetUrl(
  path: string | null,
  convertFileSrc: FileAssetUrlConverter,
): string | null {
  if (!path) return null;
  if (/^(?:https?|asset|blob|data):/i.test(path)) return path;
  // Public Vite assets and relative web paths are already browser-addressable.
  if (path.startsWith("/") || path.startsWith("./") || path.startsWith("../")) return path;
  return convertFileSrc(path);
}
