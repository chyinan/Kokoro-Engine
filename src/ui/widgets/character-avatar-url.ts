// pattern: Mixed — pure URL mapping with a browser-platform read at the presentation boundary

export type CharacterAvatarUrlConverter = (path: string) => string;
export type CharacterAvatarPlatform = "windows" | "other";

function detectCharacterAvatarPlatform(): CharacterAvatarPlatform {
  return typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent)
    ? "windows"
    : "other";
}

/** Maps character avatar references at the presentation boundary. */
export function mapCharacterAvatarUrl(
  path: string,
  convertFileSrc: CharacterAvatarUrlConverter,
  platform: CharacterAvatarPlatform = detectCharacterAvatarPlatform(),
): string {
  if (/^character-instance-resource:\/\//i.test(path)) {
    return platform === "windows"
      ? path.replace(/^character-instance-resource:\/\//i, "http://character-instance-resource.localhost/")
      : path;
  }
  if (/^(?:https?|asset|blob|data|character-instance-resource):/i.test(path)) {
    return path;
  }
  if (path.startsWith("/") || path.startsWith("./") || path.startsWith("../")) {
    return path;
  }
  return convertFileSrc(path);
}
