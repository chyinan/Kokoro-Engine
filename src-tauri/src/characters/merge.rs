// pattern: Functional Core

use crate::characters::manifest::{
    CharacterAssets, CharacterRuntimeProfile, CharacterTemplateManifest, CharacterTtsProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterTemplateFields {
    pub name: String,
    pub description: String,
    pub avatar: Option<String>,
    pub persona: String,
    pub greeting: String,
    pub example_dialogue: Option<String>,
    pub assets: Option<CharacterAssets>,
    pub runtime: Option<CharacterRuntimeProfile>,
}

impl From<&CharacterTemplateManifest> for CharacterTemplateFields {
    fn from(manifest: &CharacterTemplateManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            avatar: manifest.avatar.clone(),
            persona: manifest.persona.clone(),
            greeting: manifest.greeting.clone(),
            example_dialogue: manifest.example_dialogue.clone(),
            assets: manifest.assets.clone(),
            runtime: manifest.runtime.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MergeConflict {
    pub field: String,
    pub old: Value,
    pub user: Value,
    pub new: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterMergeResult {
    pub merged: CharacterTemplateFields,
    pub conflicts: Vec<MergeConflict>,
}

pub fn merge_character_template(
    old: &CharacterTemplateFields,
    user: &CharacterTemplateFields,
    new: &CharacterTemplateFields,
) -> CharacterMergeResult {
    let mut conflicts = Vec::new();

    let old_assets = old.assets.clone().unwrap_or_default();
    let user_assets = user.assets.clone().unwrap_or_default();
    let new_assets = new.assets.clone().unwrap_or_default();
    let assets = CharacterAssets {
        live2d_model: merge_leaf(
            "assets.live2d_model",
            &old_assets.live2d_model,
            &user_assets.live2d_model,
            &new_assets.live2d_model,
            &mut conflicts,
        ),
        background: merge_leaf(
            "assets.background",
            &old_assets.background,
            &user_assets.background,
            &new_assets.background,
            &mut conflicts,
        ),
        cue_profile: merge_leaf(
            "assets.cue_profile",
            &old_assets.cue_profile,
            &user_assets.cue_profile,
            &new_assets.cue_profile,
            &mut conflicts,
        ),
    };

    let old_runtime = old.runtime.clone().unwrap_or_default();
    let user_runtime = user.runtime.clone().unwrap_or_default();
    let new_runtime = new.runtime.clone().unwrap_or_default();
    let old_tts = old_runtime.tts.clone().unwrap_or_default();
    let user_tts = user_runtime.tts.clone().unwrap_or_default();
    let new_tts = new_runtime.tts.clone().unwrap_or_default();
    let tts = CharacterTtsProfile {
        provider_type: merge_leaf(
            "runtime.tts.provider_type",
            &old_tts.provider_type,
            &user_tts.provider_type,
            &new_tts.provider_type,
            &mut conflicts,
        ),
        provider_id: merge_leaf(
            "runtime.tts.provider_id",
            &old_tts.provider_id,
            &user_tts.provider_id,
            &new_tts.provider_id,
            &mut conflicts,
        ),
        local_preset: merge_leaf(
            "runtime.tts.local_preset",
            &old_tts.local_preset,
            &user_tts.local_preset,
            &new_tts.local_preset,
            &mut conflicts,
        ),
        voice: merge_leaf(
            "runtime.tts.voice",
            &old_tts.voice,
            &user_tts.voice,
            &new_tts.voice,
            &mut conflicts,
        ),
        speed: merge_leaf(
            "runtime.tts.speed",
            &old_tts.speed,
            &user_tts.speed,
            &new_tts.speed,
            &mut conflicts,
        ),
        pitch: merge_leaf(
            "runtime.tts.pitch",
            &old_tts.pitch,
            &user_tts.pitch,
            &new_tts.pitch,
            &mut conflicts,
        ),
    };
    let tts = has_tts_values(&tts).then_some(tts);
    let runtime = CharacterRuntimeProfile {
        tts,
        response_language: merge_leaf(
            "runtime.response_language",
            &old_runtime.response_language,
            &user_runtime.response_language,
            &new_runtime.response_language,
            &mut conflicts,
        ),
        proactive_enabled: merge_leaf(
            "runtime.proactive_enabled",
            &old_runtime.proactive_enabled,
            &user_runtime.proactive_enabled,
            &new_runtime.proactive_enabled,
            &mut conflicts,
        ),
    };

    CharacterMergeResult {
        merged: CharacterTemplateFields {
            name: merge_leaf("name", &old.name, &user.name, &new.name, &mut conflicts),
            description: merge_leaf(
                "description",
                &old.description,
                &user.description,
                &new.description,
                &mut conflicts,
            ),
            avatar: merge_leaf(
                "avatar",
                &old.avatar,
                &user.avatar,
                &new.avatar,
                &mut conflicts,
            ),
            persona: merge_leaf(
                "persona",
                &old.persona,
                &user.persona,
                &new.persona,
                &mut conflicts,
            ),
            greeting: merge_leaf(
                "greeting",
                &old.greeting,
                &user.greeting,
                &new.greeting,
                &mut conflicts,
            ),
            example_dialogue: merge_leaf(
                "example_dialogue",
                &old.example_dialogue,
                &user.example_dialogue,
                &new.example_dialogue,
                &mut conflicts,
            ),
            assets: has_asset_values(&assets).then_some(assets),
            runtime: has_runtime_values(&runtime).then_some(runtime),
        },
        conflicts,
    }
}

fn merge_leaf<T>(field: &str, old: &T, user: &T, new: &T, conflicts: &mut Vec<MergeConflict>) -> T
where
    T: Clone + PartialEq + Serialize,
{
    if user == old {
        return new.clone();
    }
    if new == old || user == new {
        return user.clone();
    }

    conflicts.push(MergeConflict {
        field: field.to_string(),
        old: serde_json::to_value(old).expect("merge values must serialize"),
        user: serde_json::to_value(user).expect("merge values must serialize"),
        new: serde_json::to_value(new).expect("merge values must serialize"),
    });
    user.clone()
}

fn has_asset_values(assets: &CharacterAssets) -> bool {
    assets.live2d_model.is_some() || assets.background.is_some() || assets.cue_profile.is_some()
}

fn has_tts_values(tts: &CharacterTtsProfile) -> bool {
    tts.provider_type.is_some()
        || tts.provider_id.is_some()
        || tts.local_preset.is_some()
        || tts.voice.is_some()
        || tts.speed.is_some()
        || tts.pitch.is_some()
}

fn has_runtime_values(runtime: &CharacterRuntimeProfile) -> bool {
    runtime.tts.is_some()
        || runtime.response_language.is_some()
        || runtime.proactive_enabled.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::characters::manifest::{
        CharacterAssets, CharacterRuntimeProfile, CharacterTtsProfile,
    };
    use proptest::prelude::*;

    fn fields() -> CharacterTemplateFields {
        CharacterTemplateFields {
            name: "Kokoro".into(),
            description: "Old description".into(),
            avatar: Some("avatar.webp".into()),
            persona: "Old persona".into(),
            greeting: "Old greeting".into(),
            example_dialogue: Some("Old examples".into()),
            assets: Some(CharacterAssets {
                live2d_model: Some("live2d/old.model3.json".into()),
                background: Some("old-background.webp".into()),
                cue_profile: Some("cues.json".into()),
            }),
            runtime: Some(CharacterRuntimeProfile {
                tts: Some(CharacterTtsProfile {
                    provider_type: Some("edge".into()),
                    provider_id: None,
                    local_preset: Some("edge-default".into()),
                    voice: Some("old-voice".into()),
                    speed: Some(1.0),
                    pitch: Some(0.0),
                }),
                response_language: Some("en".into()),
                proactive_enabled: Some(false),
            }),
        }
    }

    #[test]
    fn preserves_user_changes_and_adopts_template_only_changes() {
        let old = fields();
        let mut user = old.clone();
        user.description = "User description".into();
        let mut new = old.clone();
        new.persona = "New persona".into();

        let result = merge_character_template(&old, &user, &new);

        assert_eq!(result.merged.description, "User description");
        assert_eq!(result.merged.persona, "New persona");
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn reports_old_user_and_new_values_when_both_sides_change() {
        let old = fields();
        let mut user = old.clone();
        user.greeting = "User greeting".into();
        let mut new = old.clone();
        new.greeting = "Template greeting".into();

        let result = merge_character_template(&old, &user, &new);

        assert_eq!(result.merged.greeting, "User greeting");
        assert_eq!(result.conflicts.len(), 1);
        let conflict = &result.conflicts[0];
        assert_eq!(conflict.field, "greeting");
        assert_eq!(conflict.old, serde_json::json!("Old greeting"));
        assert_eq!(conflict.user, serde_json::json!("User greeting"));
        assert_eq!(conflict.new, serde_json::json!("Template greeting"));
    }

    #[test]
    fn merges_runtime_settings_by_semantic_field() {
        let old = fields();
        let mut user = old.clone();
        user.runtime.as_mut().unwrap().tts.as_mut().unwrap().voice = Some("user-voice".into());
        let mut new = old.clone();
        new.runtime.as_mut().unwrap().tts.as_mut().unwrap().speed = Some(1.2);
        new.runtime.as_mut().unwrap().response_language = Some("ja".into());

        let result = merge_character_template(&old, &user, &new);
        let runtime = result.merged.runtime.unwrap();
        let tts = runtime.tts.unwrap();

        assert_eq!(tts.voice.as_deref(), Some("user-voice"));
        assert_eq!(tts.speed, Some(1.2));
        assert_eq!(runtime.response_language.as_deref(), Some("ja"));
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn reports_semantic_runtime_conflicts_without_dropping_user_value() {
        let old = fields();
        let mut user = old.clone();
        user.runtime.as_mut().unwrap().tts.as_mut().unwrap().voice = Some("user-voice".into());
        let mut new = old.clone();
        new.runtime.as_mut().unwrap().tts.as_mut().unwrap().voice = Some("new-voice".into());

        let result = merge_character_template(&old, &user, &new);

        assert_eq!(
            result.merged.runtime.unwrap().tts.unwrap().voice.as_deref(),
            Some("user-voice")
        );
        assert_eq!(result.conflicts[0].field, "runtime.tts.voice");
    }

    proptest! {
        #[test]
        fn disjoint_user_and_template_edits_are_both_preserved(
            user_description in ".{1,40}",
            template_persona in ".{1,40}",
        ) {
            let old = fields();
            let mut user = old.clone();
            user.description = user_description.clone();
            let mut new = old.clone();
            new.persona = template_persona.clone();

            let result = merge_character_template(&old, &user, &new);

            prop_assert_eq!(result.merged.description, user_description);
            prop_assert_eq!(result.merged.persona, template_persona);
            prop_assert!(result.conflicts.is_empty());
        }
    }
}
