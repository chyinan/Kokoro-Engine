// pattern: Functional Core

export type Live2dModelSource = "user" | "package" | "builtin" | "none";

function isAbsoluteFilesystemPath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\") || path.startsWith("/");
}

/** Maps model assets at the presentation boundary without treating package paths as library IDs. */
export function mapLive2dModelUrl(
  path: string,
  source: Live2dModelSource,
  convertFileSrc: (path: string) => string,
  live2dUrl: (path: string) => string,
): string {
  if (source === "package" || (source === "user" && isAbsoluteFilesystemPath(path))) {
    return convertFileSrc(path);
  }
  return live2dUrl(path);
}
