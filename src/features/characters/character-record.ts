import type { CharacterRecord } from "@/lib/kokoro-bridge";

/** Defaults for instances assembled locally before a SQLite record exists. */
export const emptyCharacterDetails = {
  template_id: null,
  template_version: null,
  template_snapshot_json: null,
  description: "",
  avatar_path: null,
  greeting: "",
  greeting_consumed_at: null,
  greeting_message_id: null,
  example_dialogue: "",
  runtime_profile_json: "{}",
  user_modified_at: null,
} satisfies Partial<CharacterRecord>;
