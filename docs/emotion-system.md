# Emotion System

This document describes the current emotion pipeline used by Kokoro Engine.

## Overview

The emotion system is a post-response state machine.

It does not classify the user message and it does not drive the main chat prompt directly.
Instead, it reads the assistant's final reply, updates internal emotion state, and lets background systems consume that state.

## Classification Input

Emotion classification runs after `stream_chat` finishes producing the assistant reply.

The classifier input is:
- the assistant's final text
- with leaked tool tags removed
- with whitespace normalized
- truncated for the local model if the reply is long

Long replies are reduced to:
- the first 160 characters
- the last 160 characters
- joined with `...`

This keeps the classifier focused on what the character actually expressed in the current turn.

## Local Classifier

The runtime uses a local ONNX model from `AdamCodd/tinybert-emotion-balanced`.

Behavior:
- loads from the local model cache when available
- downloads the model and required tokenizer/config files if missing
- runs fully locally once the model exists

Current labels:
- `sadness`
- `joy`
- `love`
- `anger`
- `fear`
- `surprise`
- `neutral`

`neutral` is the engine's baseline state. The model itself usually predicts the first six labels.

## Raw Mood

The classifier returns:
- `label`
- `score`
- `raw_mood`

`raw_mood` is derived from the predicted label and confidence:
- `joy`, `love`, `surprise` push mood above `0.5`
- `sadness`, `anger`, `fear` push mood below `0.5`
- anything else falls back to `0.5`

This gives the state machine a normalized mood input in the `0.0..1.0` range.

## State Update

`AIOrchestrator::update_emotion()` forwards classifier output into `EmotionState::update()`.

`EmotionState` tracks:
- current emotion label
- internal mood
- accumulated inertia
- personality parameters
- recent history

Update behavior:
- mood is blended instead of jumping directly to the new value
- repeated emotions increase inertia
- switching to a different emotion requires enough change to overcome inertia
- expressiveness affects the outward mood intensity

In practice, the final state is smoother and more stable than the raw classifier output.

## Personality

Each character has an `EmotionPersonality` derived from persona text.

Current parameters:
- `inertia`: how resistant the character is to emotional change
- `expressiveness`: how strongly emotion is expressed outward
- `default_mood`: the resting mood when nothing new happens

This lets different characters settle, react, and decay differently even with the same classifier input.

## When Emotion Updates Run

During chat:
- the system waits for the final assistant reply
- builds classifier input from that reply
- runs the local classifier
- updates emotion state only if classification succeeds

Emotion updates are optional:
- if emotion is disabled in `emotion_settings.json`, chat keeps using a neutral fallback for emotion-driven behavior
- disabling emotion also resets the stored emotion state to the personality baseline

If the model is unavailable, download fails, or inference fails:
- the current turn does not update emotion
- the previous state is kept

## Decay And Persistence

Heartbeat periodically calls `decay_toward_default()`.

Without new stimuli:
- mood drifts back toward `default_mood`
- the current emotion gradually returns toward `neutral`

Persistence behavior:
- emotion state is stored in `emotion_state.json`
- persistence only happens when memory is enabled
- disabling memory removes the persisted emotion state file

## Current Consumers

Emotion state is currently used by:
- typing simulation
- Live2D expression-frame generation
- semantic emotion events such as `emotion:very_happy`
- idle behavior selection
- initiative / proactive behavior weighting
- emotion-aware TTS speed and pitch adjustment

Emotion events are mapped into Live2D playback through the semantic cue pipeline.

## Prompt Boundary

The main chat prompt no longer injects the current emotion state.

What still reaches the prompt:
- available Live2D cues for the active model, when cue bindings exist

What does not reach the prompt:
- `emotion.describe()`
- direct emotion-state system instructions

So the current system is best understood as:
- a post-response emotion tracker
- a background behavior input
- not a hard prompt-time controller

## Current Limits

The current design is intentionally lightweight.

Limits:
- it only looks at the final assistant reply, not full conversation semantics
- long replies are classified from a shortened representation
- no update happens if the local classifier is unavailable for that turn

That tradeoff keeps the system local, cheap, and aligned with the character's expressed output.
