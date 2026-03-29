# Live2D Semantic Cue Architecture

## Rule

Live2D playback must follow a single chain:

`semantic event -> cue -> model resources`

There is no direct playback path from emotion names, intent names, interaction names, expressions, or motion groups.

## Layers

### Semantic Event

Semantic events are model-agnostic meanings produced by upper-layer systems.

Current namespaces:

- `emotion:*`
- `interaction:*`

Examples:

- `emotion:very_happy`
- `interaction:tap_face`

Semantic events do not reference concrete Live2D resources.

### Cue

A cue is the only valid playback target.

A cue is a per-model performance key such as:

- `welcome`
- `blush`
- `annoyed_look`

All runtime playback APIs must resolve to a cue before touching the model.

### Model Resources

Resources are the imported model's raw assets:

- expressions
- motion groups

They are bound through `cue_map` only.

## Profile Structure

Each imported model profile stores:

- `available_expressions`
- `available_motion_groups`
- `cue_map`
- `semantic_cue_map`

`cue_map` binds:

- `cue -> expression`
- `cue -> motion_group`
- `cue -> expression + motion_group`

`semantic_cue_map` binds:

- `semantic event -> cue`

## Current Producers

- emotion system emits `emotion:*`
- touch interaction resolves to `interaction:*`

## Non-Goals

- No automatic guessing from raw resource names to semantic meaning
- No direct `emotion -> expression` or `action -> motion` path
- No compatibility aliases for legacy `set_expression`, `change_expression`, or `interaction_cue_map`
- No `sequence` layer until single-cue mappings are proven insufficient
