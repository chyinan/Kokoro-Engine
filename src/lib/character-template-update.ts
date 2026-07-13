// pattern: Functional Core

export type CharacterTemplateJsonValue =
  | string
  | number
  | boolean
  | null
  | ReadonlyArray<CharacterTemplateJsonValue>
  | { readonly [key: string]: CharacterTemplateJsonValue };

export type CharacterTemplateConflict = {
  readonly field: string;
  readonly old_value: CharacterTemplateJsonValue;
  readonly user_value: CharacterTemplateJsonValue;
  readonly new_value: CharacterTemplateJsonValue;
};

export type CharacterTemplateConflictChoice = "keep_user" | "accept_template";

export type CharacterTemplateConflictChoices = Readonly<
  Record<string, CharacterTemplateConflictChoice>
>;

export type CharacterTemplateSelectedValue = {
  readonly field: string;
  readonly value: CharacterTemplateJsonValue;
};

function findConflictChoice(
  choices: CharacterTemplateConflictChoices,
  field: string,
): CharacterTemplateConflictChoice {
  for (const [choiceField, choice] of Object.entries(choices)) {
    if (choiceField === field) {
      return choice;
    }
  }
  return "keep_user";
}

export function selectCharacterTemplateConflictValues(
  conflicts: ReadonlyArray<CharacterTemplateConflict>,
  choices: CharacterTemplateConflictChoices,
): ReadonlyArray<CharacterTemplateSelectedValue> {
  return conflicts.map((conflict) => ({
    field: conflict.field,
    value:
      findConflictChoice(choices, conflict.field) === "accept_template"
        ? conflict.new_value
        : conflict.user_value,
  }));
}
