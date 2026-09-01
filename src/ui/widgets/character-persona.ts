// pattern: Functional Core

export type CharacterPersonaIdentity = Readonly<{
    name: string;
    userNickname: string;
}>;

/**
 * Returns the editable character persona without runtime-generated prompt glue.
 *
 * The character record is the source of truth. Identity, nickname, user profile,
 * and XML wrappers belong to the resolved runtime prompt and must never be
 * persisted back into the record as if they were user-authored persona text.
 */
export function normalizeCharacterPersona(
    input: string,
    character: CharacterPersonaIdentity,
    userName: string,
    userPersona = "",
): string {
    let value = input.trim();
    const openTag = "<character_persona>";
    const closeTag = "</character_persona>";
    const openIndex = value.indexOf(openTag);
    const closeIndex = value.lastIndexOf(closeTag);
    if (openIndex >= 0 && closeIndex > openIndex) {
        value = value.slice(openIndex + openTag.length, closeIndex).trim();
    }

    const generatedName = `Your name is ${character.name.trim()}.`;
    while (generatedName && value.startsWith(generatedName)) {
        value = value.slice(generatedName.length).trimStart();
    }

    const generatedSuffixes = [
        userPersona.trim() ? `About the user: ${userPersona.trim()}` : "",
        userName.trim() ? `The user's name is ${userName.trim()}.` : "",
        character.userNickname.trim() && character.userNickname !== "{{user}}"
            ? `Address the user as "${character.userNickname.trim()}".`
            : "",
    ].filter((segment) => segment.length > 0);

    let removedSuffix = true;
    while (removedSuffix) {
        removedSuffix = false;
        for (const suffix of generatedSuffixes) {
            if (!value.endsWith(suffix)) continue;
            value = value.slice(0, -suffix.length).trimEnd();
            removedSuffix = true;
            break;
        }
    }

    return value.trim();
}
