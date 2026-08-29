// pattern: Functional Core

export type CharacterAvatarUrlConverter = (path: string) => string;

/** Maps character avatar references at the presentation boundary. */
export function mapCharacterAvatarUrl(
  path: string,
  convertFileSrc: CharacterAvatarUrlConverter,
): string {
  if (/^(?:https?|asset|blob|data|character-instance-resource):/i.test(path)) {
    return path;
  }
  if (path.startsWith("/") || path.startsWith("./") || path.startsWith("../")) {
    return path;
  }
  return convertFileSrc(path);
}
